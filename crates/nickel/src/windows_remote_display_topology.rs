//! Read-only DisplayConfig identity and completeness checks for remote layouts.
//! The existing monitor inventory contains active monitors only. It cannot be
//! presented as a complete transactional layout while a connected target is
//! disabled or while a cloned source obscures target identity.

use nickel_remote_control::diagnostics::OutputInventory;
use nickel_remote_control::display_layout::Layout;
use std::collections::{BTreeMap, BTreeSet};
use windows::Win32::{
    Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO, DISPLAYCONFIG_SOURCE_DEVICE_NAME,
        DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ALL_PATHS,
        QDC_ONLY_ACTIVE_PATHS, QUERY_DISPLAY_CONFIG_FLAGS, QueryDisplayConfig,
    },
    Foundation::ERROR_SUCCESS,
};

const MAX_NATIVE_PATHS: usize = 4096;
type TargetKey = (i32, u32, u32);

pub(crate) struct Observation {
    pub layout: Layout,
    pub topology_generation: u64,
    pub topology_complete: bool,
    pub transaction_unavailable_reason: Option<String>,
    pub dimensions: BTreeMap<String, (u32, u32)>,
    pub native_names: BTreeMap<String, String>,
}

type NativeMode = (
    Vec<u16>,
    windows::Win32::Graphics::Gdi::DEVMODEW,
    windows::Win32::Graphics::Gdi::DEVMODEW,
    bool,
    bool,
);

pub(crate) struct RecoveryPlan {
    modes: Vec<NativeMode>,
    identities: BTreeMap<String, String>,
}

pub(crate) struct ApplyFailure {
    pub reason: String,
    pub recovery: Option<RecoveryPlan>,
}

impl From<String> for ApplyFailure {
    fn from(reason: String) -> Self {
        Self {
            reason,
            recovery: None,
        }
    }
}

impl From<&str> for ApplyFailure {
    fn from(reason: &str) -> Self {
        reason.to_owned().into()
    }
}

fn uncertain_recovery(
    observation: &Observation,
    modes: &[NativeMode],
    reason: &str,
) -> ApplyFailure {
    ApplyFailure {
        reason: reason.into(),
        recovery: Some(RecoveryPlan {
            modes: modes.to_vec(),
            identities: observation.native_names.clone(),
        }),
    }
}

fn restore_captured_modes(observation: &Observation, modes: &[NativeMode]) -> bool {
    RecoveryPlan {
        modes: modes.to_vec(),
        identities: observation.native_names.clone(),
    }
    .restore()
    .is_ok()
}

pub(crate) fn observe(inventory: &OutputInventory) -> Result<Observation, String> {
    if inventory.truncated || inventory.outputs.is_empty() {
        return Err("complete Windows display layout is unavailable".into());
    }
    let identities = complete_active_target_identities(
        inventory.outputs.iter().map(|output| output.name.clone()),
    );
    project_inventory(inventory, identities)
}

fn project_inventory(
    inventory: &OutputInventory,
    identities: Result<BTreeMap<String, String>, String>,
) -> Result<Observation, String> {
    use nickel_remote_control::{display_layout::Placement, leases::ResourceId};
    let (identities, topology_complete, transaction_unavailable_reason) = match identities {
        Ok(identities) => (identities, true, None),
        Err(reason) => (
            inventory
                .outputs
                .iter()
                .map(|output| (output.name.clone(), output.name.clone()))
                .collect(),
            false,
            Some(reason),
        ),
    };
    let primary = inventory
        .outputs
        .iter()
        .find(|output| output.primary && output.enabled)
        .ok_or("Windows display layout has no enabled primary output")?;
    let primary = ResourceId {
        id: identities
            .get(&primary.name)
            .ok_or("Windows primary display identity changed")?
            .clone(),
        generation: primary.generation,
    };
    let mut dimensions = BTreeMap::new();
    let mut native_names = BTreeMap::new();
    let mut outputs = Vec::with_capacity(inventory.outputs.len());
    for output in &inventory.outputs {
        let id = identities
            .get(&output.name)
            .ok_or("Windows display identity changed")?
            .clone();
        let size = (
            u32::try_from(output.geometry[2]).map_err(|_| "Windows display width is invalid")?,
            u32::try_from(output.geometry[3]).map_err(|_| "Windows display height is invalid")?,
        );
        dimensions.insert(id.clone(), size);
        native_names.insert(id.clone(), output.name.clone());
        outputs.push(Placement {
            output: ResourceId {
                id,
                generation: output.generation,
            },
            x: output.geometry[0],
            y: output.geometry[1],
            enabled: output.enabled,
            scale_120: output.scale_120,
        });
    }
    outputs.sort_by(|left, right| left.output.id.cmp(&right.output.id));
    let layout = Layout { primary, outputs };
    if !layout.valid_representation() {
        return Err("Windows display owner reported an invalid layout".into());
    }
    validate_position_change(
        inventory.topology_generation,
        inventory.topology_generation,
        &layout,
        &layout,
        &layout,
        &dimensions,
    )?;
    Ok(Observation {
        layout,
        topology_generation: inventory.topology_generation,
        topology_complete,
        transaction_unavailable_reason,
        dimensions,
        native_names,
    })
}

/// Apply only positions and primary selection. Each DEVMODE begins with the
/// current native mode, preserving resolution, orientation and refresh rate.
/// The caller must hold exclusive remote authority and verify fresh readback.
pub(crate) fn apply_position_change(
    observation: &Observation,
    prior: &Layout,
    requested: &Layout,
    requested_topology_generation: u64,
    check_commit: impl Fn() -> Result<(), String>,
) -> Result<RecoveryPlan, ApplyFailure> {
    use windows::{
        Win32::Graphics::Gdi::{
            CDS_NORESET, CDS_SET_PRIMARY, CDS_TEST, CDS_TYPE, CDS_UPDATEREGISTRY,
            ChangeDisplaySettingsExW, DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_POSITION,
            ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW,
        },
        core::PCWSTR,
    };

    validate_position_change(
        requested_topology_generation,
        observation.topology_generation,
        prior,
        requested,
        &observation.layout,
        &observation.dimensions,
    )?;
    let mut modes = Vec::with_capacity(requested.outputs.len());
    for placement in &requested.outputs {
        let native_name = observation
            .native_names
            .get(&placement.output.id)
            .ok_or("Windows display source identity is unavailable")?;
        let wide = native_name.encode_utf16().chain([0]).collect::<Vec<_>>();
        let device = PCWSTR(wide.as_ptr());
        let mut original = DEVMODEW {
            dmSize: std::mem::size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: The device name and writable mode remain valid for this call.
        if !unsafe { EnumDisplaySettingsW(device, ENUM_CURRENT_SETTINGS, &raw mut original) }
            .as_bool()
        {
            return Err("Windows display mode changed before staging".into());
        }
        let expected_size = observation
            .dimensions
            .get(&placement.output.id)
            .ok_or("Windows display dimensions changed before staging")?;
        if (original.dmFields & DM_POSITION).0 == 0
            || original.dmPelsWidth != expected_size.0
            || original.dmPelsHeight != expected_size.1
        {
            return Err("Windows display mode no longer matches the observed output".into());
        }
        let prior_placement = prior
            .outputs
            .iter()
            .find(|output| output.output == placement.output)
            .ok_or("Windows display output retired before staging")?;
        // SAFETY: DM_POSITION confirms this DEVMODE union member is initialized.
        let original_position = unsafe { original.Anonymous1.Anonymous2.dmPosition };
        if original_position.x != prior_placement.x || original_position.y != prior_placement.y {
            return Err("Windows display placement changed before staging".into());
        }
        let mut requested_mode = original;
        requested_mode.Anonymous1.Anonymous2.dmPosition.x = placement.x;
        requested_mode.Anonymous1.Anonymous2.dmPosition.y = placement.y;
        // SAFETY: The current native mode is preserved except for a validated
        // desktop position. CDS_TEST does not commit the proposed mode.
        let tested = unsafe {
            ChangeDisplaySettingsExW(
                device,
                Some(&raw const requested_mode),
                None,
                CDS_TEST,
                None,
            )
        };
        if tested != DISP_CHANGE_SUCCESSFUL {
            return Err(format!("Windows rejected display placement test: {}", tested.0).into());
        }
        modes.push((
            wide,
            original,
            requested_mode,
            placement.output == requested.primary,
            placement.output == observation.layout.primary,
        ));
    }

    let mut staged_any = false;
    for (wide, _, requested_mode, primary, _) in &modes {
        if let Err(error) = check_commit() {
            return if !staged_any || restore_captured_modes(observation, &modes) {
                Err(error.into())
            } else {
                Err(uncertain_recovery(
                    observation,
                    &modes,
                    "Windows display request expired and recovery is uncertain",
                ))
            };
        }
        let flags = CDS_UPDATEREGISTRY
            | CDS_NORESET
            | if *primary {
                CDS_SET_PRIMARY
            } else {
                CDS_TYPE::default()
            };
        // SAFETY: The validated mode and device name remain owned by `modes`
        // for this synchronous staging call.
        let result = unsafe {
            ChangeDisplaySettingsExW(
                PCWSTR(wide.as_ptr()),
                Some(requested_mode as *const DEVMODEW),
                None,
                flags,
                None,
            )
        };
        if result != DISP_CHANGE_SUCCESSFUL {
            return if restore_captured_modes(observation, &modes) {
                Err(format!("Windows rejected display placement: {}", result.0).into())
            } else {
                Err(uncertain_recovery(
                    observation,
                    &modes,
                    "Windows display placement failed and recovery is uncertain",
                ))
            };
        }
        staged_any = true;
    }
    if let Err(error) = check_commit() {
        return if restore_captured_modes(observation, &modes) {
            Err(error.into())
        } else {
            Err(uncertain_recovery(
                observation,
                &modes,
                "Windows display request expired and recovery is uncertain",
            ))
        };
    }
    // SAFETY: A null device and mode commit the validated staged positions.
    let result =
        unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE::default(), None) };
    if result == DISP_CHANGE_SUCCESSFUL {
        Ok(RecoveryPlan {
            modes,
            identities: observation.native_names.clone(),
        })
    } else if restore_captured_modes(observation, &modes) {
        Err(format!("Windows rejected display layout commit: {}", result.0).into())
    } else {
        Err(uncertain_recovery(
            observation,
            &modes,
            "Windows display commit failed and recovery is uncertain",
        ))
    }
}

fn restore_modes(modes: &[NativeMode]) -> Result<(), String> {
    use windows::{
        Win32::Graphics::Gdi::{
            CDS_NORESET, CDS_SET_PRIMARY, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW,
            DEVMODEW, DISP_CHANGE_SUCCESSFUL,
        },
        core::PCWSTR,
    };
    let mut staged = true;
    for (wide, original, _, _, original_primary) in modes {
        let flags = CDS_UPDATEREGISTRY
            | CDS_NORESET
            | if *original_primary {
                CDS_SET_PRIMARY
            } else {
                CDS_TYPE::default()
            };
        // SAFETY: The captured mode and device name remain owned by this
        // synchronous recovery loop.
        let result = unsafe {
            ChangeDisplaySettingsExW(
                PCWSTR(wide.as_ptr()),
                Some(original as *const DEVMODEW),
                None,
                flags,
                None,
            )
        };
        staged &= result == DISP_CHANGE_SUCCESSFUL;
    }
    if !staged {
        return Err("Windows display recovery staging failed".into());
    }
    // SAFETY: A null device and mode commit all staged recovery settings.
    let committed =
        unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE::default(), None) };
    (committed == DISP_CHANGE_SUCCESSFUL)
        .then_some(())
        .ok_or_else(|| "Windows display recovery commit failed".into())
}

fn active_primary_name() -> Result<String, String> {
    crate::platform::remote_observation::outputs()
        .map_err(|_| "Windows display primary readback is unavailable".to_owned())?
        .into_iter()
        .find(|output| output.primary)
        .map(|output| output.name)
        .ok_or_else(|| "Windows display primary readback is unavailable".to_owned())
}

fn captured_name(wide: &[u16]) -> Option<String> {
    let (&0, units) = wide.split_last()? else {
        return None;
    };
    String::from_utf16(units).ok()
}

impl RecoveryPlan {
    pub(crate) fn restore(&self) -> Result<(), String> {
        use windows::{
            Win32::Graphics::Gdi::{DEVMODEW, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW},
            core::PCWSTR,
        };
        let current = complete_active_target_identities(self.identities.values().cloned())?;
        if self
            .identities
            .iter()
            .any(|(id, name)| current.get(name) != Some(id))
        {
            return Err("Windows display targets changed before recovery".into());
        }
        for (wide, original, requested, _, _) in &self.modes {
            let mut mode = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            // SAFETY: The device name and writable mode remain valid here.
            if !unsafe {
                EnumDisplaySettingsW(PCWSTR(wide.as_ptr()), ENUM_CURRENT_SETTINGS, &raw mut mode)
            }
            .as_bool()
                || mode.dmPelsWidth != requested.dmPelsWidth
                || mode.dmPelsHeight != requested.dmPelsHeight
                || mode.dmDisplayFrequency != requested.dmDisplayFrequency
                || mode.dmBitsPerPel != requested.dmBitsPerPel
                || (mode.dmFields & windows::Win32::Graphics::Gdi::DM_POSITION).0 == 0
            {
                return Err("Windows display mode changed before recovery".into());
            }
            // SAFETY: EnumDisplaySettingsW initialized the display union and
            // both captured modes came from the same initialized native mode.
            if unsafe { mode.Anonymous1.Anonymous2.dmDisplayOrientation }
                != unsafe { requested.Anonymous1.Anonymous2.dmDisplayOrientation }
                || unsafe { mode.Anonymous1.Anonymous2.dmDisplayFixedOutput }
                    != unsafe { requested.Anonymous1.Anonymous2.dmDisplayFixedOutput }
            {
                return Err("Windows display orientation changed before recovery".into());
            }
            // SAFETY: EnumDisplaySettingsW initialized the current position,
            // and both captured modes came from initialized native modes.
            let current_position = unsafe { mode.Anonymous1.Anonymous2.dmPosition };
            let original_position = unsafe { original.Anonymous1.Anonymous2.dmPosition };
            let requested_position = unsafe { requested.Anonymous1.Anonymous2.dmPosition };
            if (current_position.x, current_position.y)
                != (original_position.x, original_position.y)
                && (current_position.x, current_position.y)
                    != (requested_position.x, requested_position.y)
            {
                return Err("Windows display placement changed outside the recovery owner".into());
            }
        }
        let current_primary = active_primary_name()?;
        let known_primary = self.modes.iter().any(|(wide, _, _, requested, original)| {
            (*requested || *original)
                && captured_name(wide).is_some_and(|name| name == current_primary)
        });
        if !known_primary {
            return Err("Windows display primary changed outside the recovery owner".into());
        }
        restore_modes(&self.modes)?;
        for (wide, original, _, _, _) in &self.modes {
            let mut mode = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            // SAFETY: The device name and writable mode remain valid here.
            if !unsafe {
                EnumDisplaySettingsW(PCWSTR(wide.as_ptr()), ENUM_CURRENT_SETTINGS, &raw mut mode)
            }
            .as_bool()
                || (mode.dmFields & windows::Win32::Graphics::Gdi::DM_POSITION).0 == 0
            {
                return Err("Windows display recovery readback is unavailable".into());
            }
            // SAFETY: EnumDisplaySettingsW initialized the returned position
            // and DM_POSITION confirms that the union member is valid. The
            // original mode was captured by the same API before mutation.
            let restored = unsafe { mode.Anonymous1.Anonymous2.dmPosition };
            let expected = unsafe { original.Anonymous1.Anonymous2.dmPosition };
            if restored.x != expected.x || restored.y != expected.y {
                return Err("Windows display recovery readback did not match".into());
            }
        }
        let expected_primary = self
            .modes
            .iter()
            .find(|(_, _, _, _, original_primary)| *original_primary)
            .and_then(|(wide, _, _, _, _)| captured_name(wide))
            .ok_or("Windows original primary display is unavailable")?;
        let actual_primary = active_primary_name()?;
        if actual_primary != expected_primary {
            return Err("Windows display recovery primary readback did not match".into());
        }
        Ok(())
    }
}

fn target_key(path: &DISPLAYCONFIG_PATH_INFO) -> TargetKey {
    (
        path.targetInfo.adapterId.HighPart,
        path.targetInfo.adapterId.LowPart,
        path.targetInfo.id,
    )
}

fn target_identity(key: TargetKey) -> String {
    format!(
        "windows-display:{:08x}:{:08x}:{:08x}",
        key.0 as u32, key.1, key.2
    )
}

fn native_paths(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, String> {
    let mut path_count = 0;
    let mut mode_count = 0;
    // SAFETY: The count pointers are valid writable storage.
    if unsafe { GetDisplayConfigBufferSizes(flags, &mut path_count, &mut mode_count) }
        != ERROR_SUCCESS
    {
        return Err("Windows DisplayConfig topology is unavailable".into());
    }
    if path_count == 0 || path_count as usize > MAX_NATIVE_PATHS {
        return Err("Windows DisplayConfig path count exceeds its bound".into());
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: The vectors have the capacities returned by DisplayConfig and
    // both count pointers remain valid throughout this synchronous call.
    if unsafe {
        QueryDisplayConfig(
            flags,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    } != ERROR_SUCCESS
    {
        return Err("Windows DisplayConfig changed during observation".into());
    }
    paths.truncate(path_count as usize);
    Ok(paths)
}

fn source_name(path: &DISPLAYCONFIG_PATH_INFO) -> Result<String, String> {
    let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
            size: std::mem::size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
        },
        ..Default::default()
    };
    // SAFETY: The header identifies this initialized source-name packet and
    // remains writable for the duration of the synchronous native call.
    if unsafe { DisplayConfigGetDeviceInfo(&raw mut source.header) } != 0 {
        return Err("Windows display source identity is unavailable".into());
    }
    let end = source
        .viewGdiDeviceName
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(source.viewGdiDeviceName.len());
    String::from_utf16(&source.viewGdiDeviceName[..end])
        .ok()
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Windows display source name is invalid".into())
}

fn complete_active_targets(
    connected: impl IntoIterator<Item = TargetKey>,
    active: impl IntoIterator<Item = (String, TargetKey)>,
    monitor_names: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>, String> {
    let connected = connected.into_iter().collect::<BTreeSet<_>>();
    let mut active_targets = BTreeSet::new();
    let mut named = BTreeMap::new();
    for (name, target) in active {
        if !active_targets.insert(target) || named.insert(name, target_identity(target)).is_some() {
            return Err("Windows display source is cloned or ambiguous".into());
        }
    }
    if connected.is_empty()
        || connected != active_targets
        || named.keys().cloned().collect::<BTreeSet<_>>()
            != monitor_names.into_iter().collect::<BTreeSet<_>>()
    {
        return Err("Windows active monitors do not cover every connected display target".into());
    }
    Ok(named)
}

/// Return stable adapter/target IDs only when the active monitor inventory is
/// the complete connected topology. Disabled connected targets require a
/// separate owner that can observe their placement and scale accurately.
pub(crate) fn complete_active_target_identities(
    monitor_names: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>, String> {
    let connected = native_paths(QDC_ALL_PATHS)?
        .into_iter()
        .filter(|path| path.targetInfo.targetAvailable.as_bool())
        .map(|path| target_key(&path))
        .collect::<Vec<_>>();
    let active = native_paths(QDC_ONLY_ACTIVE_PATHS)?
        .into_iter()
        .map(|path| Ok((source_name(&path)?, target_key(&path))))
        .collect::<Result<Vec<_>, String>>()?;
    complete_active_targets(connected, active, monitor_names)
}

/// Validate the subset that the current GDI placement API can preserve: all
/// connected targets remain enabled at their observed scale, while positions
/// and primary selection may change. Dimensions come from the same fresh
/// monitor observation as `current` and are keyed by stable target identity.
pub(crate) fn validate_position_change(
    requested_topology_generation: u64,
    current_topology_generation: u64,
    prior: &Layout,
    requested: &Layout,
    current: &Layout,
    dimensions: &BTreeMap<String, (u32, u32)>,
) -> Result<(), String> {
    if requested_topology_generation != current_topology_generation
        || prior != current
        || !requested.valid_representation()
        || !requested.same_output_incarnations(current)
    {
        return Err("Windows display request has stale or incomplete topology".into());
    }
    if dimensions.len() != current.outputs.len() {
        return Err("Windows display dimensions are incomplete".into());
    }
    let primary = requested
        .outputs
        .iter()
        .find(|placement| placement.output == requested.primary)
        .ok_or("Windows primary display is unavailable")?;
    if primary.x != 0 || primary.y != 0 {
        return Err("Windows primary display must be placed at the desktop origin".into());
    }
    for placement in &requested.outputs {
        let observed = current
            .outputs
            .iter()
            .find(|observed| observed.output == placement.output)
            .ok_or("Windows display target retired")?;
        if !observed.enabled || !placement.enabled {
            return Err("Windows display enable changes require a DisplayConfig owner".into());
        }
        if placement.scale_120 != observed.scale_120 {
            return Err("Windows display scale changes are unavailable".into());
        }
        let (width, height) = dimensions
            .get(&placement.output.id)
            .copied()
            .ok_or("Windows display dimensions are unavailable")?;
        if width == 0
            || height == 0
            || i64::from(placement.x) + i64::from(width) > i64::from(i32::MAX)
            || i64::from(placement.y) + i64::from(height) > i64::from(i32::MAX)
            || i32::try_from(i64::from(placement.x) - i64::from(primary.x)).is_err()
            || i32::try_from(i64::from(placement.y) - i64::from(primary.y)).is_err()
        {
            return Err("Windows display placement exceeds native coordinate bounds".into());
        }
    }
    Ok(())
}

/// Native monitor handles may be recreated during reconfiguration. Stable
/// adapter/target IDs still let the recovery owner verify physical layout.
pub(crate) fn matches_physical_layout(actual: &Layout, expected: &Layout) -> bool {
    actual.primary.id == expected.primary.id
        && actual.outputs.len() == expected.outputs.len()
        && actual.outputs.iter().all(|output| {
            expected.outputs.iter().any(|candidate| {
                output.output.id == candidate.output.id
                    && output.x == candidate.x
                    && output.y == candidate.y
                    && output.enabled == candidate.enabled
                    && output.scale_120 == candidate.scale_120
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_remote_control::diagnostics::OutputDiagnostic;
    use nickel_remote_control::{display_layout::Placement, leases::ResourceId};

    fn layout(primary: &str, left_x: i32, right_x: i32) -> Layout {
        Layout {
            primary: ResourceId {
                id: primary.into(),
                generation: if primary == "right" { 2 } else { 1 },
            },
            outputs: vec![
                Placement {
                    output: ResourceId {
                        id: "left".into(),
                        generation: 1,
                    },
                    x: left_x,
                    y: if primary == "left" { 0 } else { -100 },
                    enabled: true,
                    scale_120: 120,
                },
                Placement {
                    output: ResourceId {
                        id: "right".into(),
                        generation: 2,
                    },
                    x: right_x,
                    y: if primary == "right" { 0 } else { 100 },
                    enabled: true,
                    scale_120: 180,
                },
            ],
        }
    }

    #[test]
    fn incomplete_native_topology_still_exposes_active_layout_without_transactions() {
        let inventory = OutputInventory {
            observation_generation: 1,
            observed_at_us: 0,
            topology_generation: 7,
            outputs: vec![OutputDiagnostic {
                name: r"\\.\DISPLAY1".into(),
                generation: 3,
                geometry: [0, 0, 1920, 1080],
                work_area: [0, 0, 1920, 1040],
                scale_120: 120,
                primary: true,
                enabled: true,
            }],
            truncated: false,
        };
        let incomplete = project_inventory(&inventory, Err("extra native target".into())).unwrap();
        assert!(!incomplete.topology_complete);
        assert_eq!(
            incomplete.transaction_unavailable_reason.as_deref(),
            Some("extra native target")
        );
        assert_eq!(incomplete.layout.primary.id, r"\\.\DISPLAY1");
        let complete = project_inventory(
            &inventory,
            Ok(BTreeMap::from([(
                r"\\.\DISPLAY1".into(),
                "adapter:target".into(),
            )])),
        )
        .unwrap();
        assert!(complete.topology_complete);
        assert_eq!(complete.layout.primary.id, "adapter:target");
    }

    #[test]
    fn complete_active_topology_requires_exact_native_targets_and_monitor_names() {
        let left = (1, 2, 3);
        let right = (1, 2, 4);
        let names = [r"\\.\DISPLAY1".to_owned(), r"\\.\DISPLAY2".to_owned()];
        let active = [(names[0].clone(), left), (names[1].clone(), right)];
        let mapped = complete_active_targets([left, right], active.clone(), names.clone()).unwrap();
        assert_ne!(mapped[&names[0]], mapped[&names[1]]);
        assert!(
            complete_active_targets([left, right, (1, 2, 5)], active.clone(), names.clone())
                .is_err()
        );
        assert!(
            complete_active_targets([left, right], active.clone(), [names[0].clone()]).is_err()
        );
        assert!(
            complete_active_targets(
                [left, right],
                [(names[0].clone(), left), (names[0].clone(), right)],
                names.clone()
            )
            .is_err()
        );
    }

    #[test]
    fn placement_validation_preserves_scale_and_rejects_retired_or_disabled_targets() {
        let current = layout("right", -1920, 0);
        let requested = layout("left", 0, 1920);
        let dimensions = BTreeMap::from([
            ("left".into(), (1920, 1080)),
            ("right".into(), (1920, 1080)),
        ]);
        assert!(
            validate_position_change(7, 7, &current, &requested, &current, &dimensions).is_ok()
        );
        let mut changed_scale = requested.clone();
        changed_scale.outputs[1].scale_120 = 120;
        assert!(
            validate_position_change(7, 7, &current, &changed_scale, &current, &dimensions)
                .is_err()
        );
        let mut disabled = requested.clone();
        disabled.outputs[0].enabled = false;
        assert!(
            validate_position_change(7, 7, &current, &disabled, &current, &dimensions).is_err()
        );
        disabled.outputs[1].enabled = false;
        assert!(
            validate_position_change(7, 7, &current, &disabled, &current, &dimensions).is_err()
        );
        let mut retired = requested.clone();
        retired.outputs[0].output.generation += 1;
        assert!(validate_position_change(7, 7, &current, &retired, &current, &dimensions).is_err());
        assert!(
            validate_position_change(
                7,
                7,
                &current,
                &requested,
                &layout("left", -1919, 0),
                &dimensions
            )
            .is_err()
        );
        let overflowing = layout("left", 0, i32::MAX);
        assert!(
            validate_position_change(7, 7, &current, &overflowing, &current, &dimensions).is_err()
        );
        assert!(
            validate_position_change(6, 7, &current, &requested, &current, &dimensions).is_err()
        );
    }

    #[test]
    fn physical_readback_accepts_recreated_monitor_handles_only_for_same_targets() {
        let expected = layout("right", -1920, 0);
        let mut recreated = expected.clone();
        recreated.primary.generation += 1;
        for output in &mut recreated.outputs {
            output.output.generation += 1;
        }
        assert!(matches_physical_layout(&recreated, &expected));
        recreated.outputs[0].x += 1;
        assert!(!matches_physical_layout(&recreated, &expected));
        recreated.outputs[0].x -= 1;
        recreated.outputs[0].output.id = "replacement".into();
        assert!(!matches_physical_layout(&recreated, &expected));
    }

    #[test]
    #[ignore = "reads the current Windows display topology"]
    fn native_display_target_identity_matches_active_monitors() {
        use windows::Win32::Devices::Display::{
            DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
        };

        let outputs = crate::platform::remote_observation::outputs().unwrap();
        let all_paths = native_paths(QDC_ALL_PATHS).unwrap();
        let active_paths = native_paths(QDC_ONLY_ACTIVE_PATHS).unwrap();
        let available_targets = all_paths
            .iter()
            .filter(|path| path.targetInfo.targetAvailable.as_bool())
            .map(target_key)
            .collect::<BTreeSet<_>>();
        let active_targets = active_paths.iter().map(target_key).collect::<BTreeSet<_>>();
        eprintln!(
            "Windows display topology: {} active monitors, {} active targets, {} available targets across {} paths",
            outputs.len(),
            active_targets.len(),
            available_targets.len(),
            all_paths.len()
        );
        if let Some(path) = all_paths.iter().find(|path| {
            path.targetInfo.targetAvailable.as_bool() && !active_targets.contains(&target_key(path))
        }) {
            let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                    r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                    size: std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                    adapterId: path.targetInfo.adapterId,
                    id: path.targetInfo.id,
                },
                ..Default::default()
            };
            // SAFETY: The initialized packet is writable for this read-only query.
            let result = unsafe { DisplayConfigGetDeviceInfo(&raw mut target.header) };
            eprintln!(
                "inactive available target: query={}, monitor_path_present={}, friendly_name_present={}",
                result,
                target.monitorDevicePath[0] != 0,
                target.monitorFriendlyDeviceName[0] != 0
            );
        }
        let identities =
            complete_active_target_identities(outputs.iter().map(|output| output.name.clone()));
        match identities {
            Ok(identities) => assert_eq!(identities.len(), outputs.len()),
            Err(reason) => {
                assert!(!reason.is_empty());
                eprintln!("Windows display transactions unavailable on this fixture: {reason}");
            }
        }
    }

    /// Run only with an explicitly safe multi-monitor fixture. The guard
    /// restores the original native modes even when an assertion panics.
    #[test]
    #[ignore = "requires NICKEL_WINDOWS_DISPLAY_MUTATION_TEST=1 and a reversible multi-monitor setup"]
    fn native_display_position_round_trip_restores_original_modes() {
        use nickel_remote_control::diagnostics::OutputDiagnostic;

        assert_eq!(
            std::env::var("NICKEL_WINDOWS_DISPLAY_MUTATION_TEST").as_deref(),
            Ok("1")
        );
        fn inventory() -> OutputInventory {
            let outputs = crate::platform::remote_observation::outputs().unwrap();
            OutputInventory {
                observation_generation: 1,
                observed_at_us: 0,
                topology_generation: 1,
                outputs: outputs
                    .into_iter()
                    .enumerate()
                    .map(|(index, output)| OutputDiagnostic {
                        name: output.name,
                        generation: index as u64 + 1,
                        geometry: [
                            output.bounds.x,
                            output.bounds.y,
                            i32::try_from(output.bounds.width).unwrap(),
                            i32::try_from(output.bounds.height).unwrap(),
                        ],
                        work_area: [
                            output.work_area.x,
                            output.work_area.y,
                            i32::try_from(output.work_area.width).unwrap(),
                            i32::try_from(output.work_area.height).unwrap(),
                        ],
                        scale_120: output.scale_120,
                        primary: output.primary,
                        enabled: true,
                    })
                    .collect(),
                truncated: false,
            }
        }
        struct Restore(Option<RecoveryPlan>);
        impl Drop for Restore {
            fn drop(&mut self) {
                if let Some(plan) = self.0.take() {
                    assert!(
                        plan.restore().is_ok(),
                        "Windows display fixture restore failed"
                    );
                }
            }
        }

        let before = observe(&inventory()).expect("a complete active display topology is required");
        assert!(before.layout.outputs.len() >= 2);
        let mut requested = before.layout.clone();
        let primary = requested.primary.clone();
        let secondary = requested
            .outputs
            .iter_mut()
            .find(|output| output.output != primary)
            .unwrap();
        secondary.x = secondary.x.checked_add(8).unwrap();
        let plan = match apply_position_change(
            &before,
            &before.layout,
            &requested,
            before.topology_generation,
            || Ok(()),
        ) {
            Ok(plan) => plan,
            Err(failure) => {
                if let Some(plan) = failure.recovery {
                    let _guard = Restore(Some(plan));
                }
                panic!("{}", failure.reason);
            }
        };
        let mut guard = Restore(Some(plan));
        let applied = observe(&inventory()).unwrap();
        assert!(matches_physical_layout(&applied.layout, &requested));
        guard.0.as_ref().unwrap().restore().unwrap();
        guard.0 = None;
        let restored = observe(&inventory()).unwrap();
        assert!(matches_physical_layout(&restored.layout, &before.layout));
    }
}
