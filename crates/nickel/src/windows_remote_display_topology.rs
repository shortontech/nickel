//! Read-only DisplayConfig identity and completeness checks for remote layouts.
//! The existing monitor inventory contains active monitors only. It cannot be
//! presented as a complete transactional layout while an available target is
//! inactive or while a cloned source obscures target identity.

use nickel_remote_control::diagnostics::OutputInventory;
use nickel_remote_control::display_layout::Layout;
use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::{
    Devices::Display::{
        DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_HEADER,
        DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE, DISPLAYCONFIG_PATH_INFO,
        DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TOPOLOGY_ID, DisplayConfigGetDeviceInfo,
        GetDisplayConfigBufferSizes, QDC_ALL_PATHS, QDC_DATABASE_CURRENT, QDC_ONLY_ACTIVE_PATHS,
        QUERY_DISPLAY_CONFIG_FLAGS, QueryDisplayConfig,
    },
    Foundation::ERROR_SUCCESS,
};

const MAX_NATIVE_PATHS: usize = 4096;
const MAX_NATIVE_MODES: usize = 8192;
type TargetKey = (i32, u32, u32);
type SourceGeometry = (i32, i32, u32, u32);

pub(crate) struct Observation {
    pub layout: Layout,
    pub topology_generation: u64,
    pub topology_complete: bool,
    pub transaction_supported: bool,
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
type NativeConfiguration = (Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>);

pub(crate) struct RecoveryPlan {
    modes: Vec<NativeMode>,
    identities: BTreeMap<String, String>,
    original_configuration: NativeConfiguration,
    saved_configuration: NativeConfiguration,
    requested_configuration: NativeConfiguration,
    persistence_attempted: AtomicBool,
}

pub(crate) struct ApplyFailure {
    pub reason: String,
    pub recovery: Option<Box<RecoveryPlan>>,
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
    original_configuration: &NativeConfiguration,
    saved_configuration: &NativeConfiguration,
    requested_configuration: &NativeConfiguration,
    reason: &str,
) -> ApplyFailure {
    ApplyFailure {
        reason: reason.into(),
        recovery: Some(Box::new(RecoveryPlan {
            modes: modes.to_vec(),
            identities: observation.native_names.clone(),
            original_configuration: original_configuration.clone(),
            saved_configuration: saved_configuration.clone(),
            requested_configuration: requested_configuration.clone(),
            persistence_attempted: AtomicBool::new(false),
        })),
    }
}

pub(crate) fn observe(inventory: &OutputInventory) -> Result<Observation, String> {
    if inventory.truncated || inventory.outputs.is_empty() {
        return Err("complete Windows display layout is unavailable".into());
    }
    let identities = complete_active_target_identities(
        inventory.outputs.iter().map(|output| output.name.clone()),
    );
    let readiness = if identities.is_ok() {
        native_transaction_prerequisites(inventory)
    } else {
        Ok(())
    };
    project_inventory(inventory, identities, readiness)
}

fn project_inventory(
    inventory: &OutputInventory,
    identities: Result<BTreeMap<String, String>, String>,
    readiness: Result<(), String>,
) -> Result<Observation, String> {
    use nickel_remote_control::{display_layout::Placement, leases::ResourceId};
    let (identities, topology_complete, transaction_unavailable_reason) = match identities {
        Ok(identities) => (identities, true, readiness.err()),
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
        transaction_supported: topology_complete && transaction_unavailable_reason.is_none(),
        transaction_unavailable_reason,
        dimensions,
        native_names,
    })
}

/// Apply only positions and primary selection. Capture each current DEVMODE
/// for guarded recovery, then apply a supplied DisplayConfig temporarily.
/// The caller must hold exclusive remote authority and verify fresh readback.
pub(crate) fn apply_position_change(
    observation: &Observation,
    prior: &Layout,
    requested: &Layout,
    requested_topology_generation: u64,
    check_commit: impl Fn() -> Result<(), String>,
) -> Result<RecoveryPlan, ApplyFailure> {
    use windows::{
        Win32::Devices::Display::{
            QDC_VIRTUAL_MODE_AWARE, SDC_USE_SUPPLIED_DISPLAY_CONFIG, SDC_VALIDATE,
            SDC_VIRTUAL_MODE_AWARE, SetDisplayConfig,
        },
        Win32::Graphics::Gdi::{
            DEVMODEW, DM_POSITION, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW,
        },
        core::PCWSTR,
    };

    if !observation.transaction_supported || !observation.topology_complete {
        return Err(observation
            .transaction_unavailable_reason
            .as_deref()
            .unwrap_or("complete Windows display topology is unavailable")
            .into());
    }

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
        modes.push((
            wide,
            original,
            requested_mode,
            placement.output == requested.primary,
            placement.output == observation.layout.primary,
        ));
    }

    let (paths, original_modes) =
        native_configuration(QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE)?;
    let saved_configuration = native_configuration(QDC_DATABASE_CURRENT | QDC_VIRTUAL_MODE_AWARE)?;
    configuration_positions(&saved_configuration.0, &saved_configuration.1)?;
    let requested_modes = requested_source_modes(observation, requested, &paths, &original_modes)?;
    let original_configuration = (paths.clone(), original_modes);
    let requested_configuration = (paths, requested_modes);
    check_commit()?;
    let flags = SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_VIRTUAL_MODE_AWARE;
    // SAFETY: The captured paths and modes remain valid for this synchronous
    // validation call, which does not change the display configuration.
    let validation = unsafe {
        SetDisplayConfig(
            Some(&requested_configuration.0),
            Some(&requested_configuration.1),
            SDC_VALIDATE | flags,
        )
    };
    if validation != ERROR_SUCCESS.0 as i32 {
        return Err(format!("Windows rejected display configuration: {validation}").into());
    }
    check_commit()?;
    if let Err(error) = apply_supplied_configuration(&requested_configuration, false) {
        return Err(uncertain_recovery(
            observation,
            &modes,
            &original_configuration,
            &saved_configuration,
            &requested_configuration,
            &format!("{error}; recovery is pending"),
        ));
    }
    Ok(RecoveryPlan {
        modes,
        identities: observation.native_names.clone(),
        original_configuration,
        saved_configuration,
        requested_configuration,
        persistence_attempted: AtomicBool::new(false),
    })
}

fn apply_supplied_configuration(
    configuration: &NativeConfiguration,
    save: bool,
) -> Result<(), String> {
    use windows::Win32::Devices::Display::{
        SDC_APPLY, SDC_SAVE_TO_DATABASE, SDC_USE_SUPPLIED_DISPLAY_CONFIG, SDC_VIRTUAL_MODE_AWARE,
        SetDisplayConfig,
    };
    let flags = SDC_APPLY
        | SDC_USE_SUPPLIED_DISPLAY_CONFIG
        | SDC_VIRTUAL_MODE_AWARE
        | if save {
            SDC_SAVE_TO_DATABASE
        } else {
            Default::default()
        };
    // SAFETY: The captured paths and modes remain owned by the caller through
    // the synchronous SetDisplayConfig call.
    let result = unsafe { SetDisplayConfig(Some(&configuration.0), Some(&configuration.1), flags) };
    (result == ERROR_SUCCESS.0 as i32)
        .then_some(())
        .ok_or_else(|| format!("Windows display configuration failed ({result})"))
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
    #[cfg(test)]
    pub(crate) fn clone_for_test_saved_recovery(&self) -> Self {
        // A Keep attempt may have updated the database before a test fails.
        // Always restore both captured configurations from this test guard.
        Self {
            modes: self.modes.clone(),
            identities: self.identities.clone(),
            original_configuration: self.original_configuration.clone(),
            saved_configuration: self.saved_configuration.clone(),
            requested_configuration: self.requested_configuration.clone(),
            persistence_attempted: AtomicBool::new(true),
        }
    }

    pub(crate) fn persist(&self) -> Result<(), String> {
        use windows::Win32::Devices::Display::QDC_VIRTUAL_MODE_AWARE;
        use windows::{
            Win32::Graphics::Gdi::{
                DEVMODEW, DM_POSITION, ENUM_CURRENT_SETTINGS, EnumDisplaySettingsW,
            },
            core::PCWSTR,
        };
        let current = complete_active_target_identities(self.identities.values().cloned())?;
        if self
            .identities
            .iter()
            .any(|(id, name)| current.get(name) != Some(id))
        {
            return Err("Windows display targets changed before confirmation".into());
        }
        let expected = configuration_positions(
            &self.requested_configuration.0,
            &self.requested_configuration.1,
        )?;
        let (active_paths, active_modes) =
            native_configuration(QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE)?;
        if configuration_positions(&active_paths, &active_modes)? != expected {
            return Err("Windows display source modes changed before confirmation".into());
        }
        for (wide, _, requested, _, _) in &self.modes {
            let mut current = DEVMODEW {
                dmSize: std::mem::size_of::<DEVMODEW>() as u16,
                ..Default::default()
            };
            // SAFETY: The captured device name and writable mode remain valid.
            if !unsafe {
                EnumDisplaySettingsW(
                    PCWSTR(wide.as_ptr()),
                    ENUM_CURRENT_SETTINGS,
                    &raw mut current,
                )
            }
            .as_bool()
                || (current.dmFields & DM_POSITION).0 == 0
                || current.dmPelsWidth != requested.dmPelsWidth
                || current.dmPelsHeight != requested.dmPelsHeight
                || current.dmDisplayFrequency != requested.dmDisplayFrequency
                || current.dmBitsPerPel != requested.dmBitsPerPel
            {
                return Err("Windows display mode changed before confirmation".into());
            }
            // SAFETY: DM_POSITION confirms the current position member, and
            // both modes were initialized by EnumDisplaySettingsW.
            if unsafe { current.Anonymous1.Anonymous2.dmPosition }
                != unsafe { requested.Anonymous1.Anonymous2.dmPosition }
                || unsafe { current.Anonymous1.Anonymous2.dmDisplayOrientation }
                    != unsafe { requested.Anonymous1.Anonymous2.dmDisplayOrientation }
                || unsafe { current.Anonymous1.Anonymous2.dmDisplayFixedOutput }
                    != unsafe { requested.Anonymous1.Anonymous2.dmDisplayFixedOutput }
            {
                return Err(
                    "Windows display placement or orientation changed before confirmation".into(),
                );
            }
        }
        self.persistence_attempted.store(true, Ordering::SeqCst);
        apply_supplied_configuration(&self.requested_configuration, true)?;
        let (saved_paths, saved_modes) =
            native_configuration(QDC_DATABASE_CURRENT | QDC_VIRTUAL_MODE_AWARE)?;
        if configuration_positions(&saved_paths, &saved_modes)? != expected {
            return Err("Windows saved display configuration did not match confirmation".into());
        }
        Ok(())
    }

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
        let saved_result = if self.persistence_attempted.load(Ordering::SeqCst) {
            apply_supplied_configuration(&self.saved_configuration, true).and_then(|()| {
                use windows::Win32::Devices::Display::QDC_VIRTUAL_MODE_AWARE;
                let (paths, modes) =
                    native_configuration(QDC_DATABASE_CURRENT | QDC_VIRTUAL_MODE_AWARE)?;
                if configuration_positions(&paths, &modes)?
                    != configuration_positions(
                        &self.saved_configuration.0,
                        &self.saved_configuration.1,
                    )?
                {
                    return Err("Windows saved display recovery did not match".into());
                }
                Ok(())
            })
        } else {
            Ok(())
        };
        let active_result = apply_supplied_configuration(&self.original_configuration, false);
        saved_result?;
        active_result?;
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

fn source_mode_index(path: &DISPLAYCONFIG_PATH_INFO) -> Result<usize, String> {
    use windows::Win32::Graphics::Gdi::{
        DISPLAYCONFIG_PATH_MODE_IDX_INVALID, DISPLAYCONFIG_PATH_SOURCE_MODE_IDX_INVALID,
        DISPLAYCONFIG_PATH_SUPPORT_VIRTUAL_MODE,
    };

    // SAFETY: QueryDisplayConfig initializes the active path's source union;
    // the path flag determines which indexed representation it contains.
    let index = if path.flags & DISPLAYCONFIG_PATH_SUPPORT_VIRTUAL_MODE != 0 {
        unsafe { path.sourceInfo.Anonymous.Anonymous._bitfield >> 16 }
    } else {
        unsafe { path.sourceInfo.Anonymous.modeInfoIdx }
    };
    if index == DISPLAYCONFIG_PATH_MODE_IDX_INVALID
        || index == DISPLAYCONFIG_PATH_SOURCE_MODE_IDX_INVALID
    {
        return Err("Windows active display source mode is unavailable".into());
    }
    usize::try_from(index).map_err(|_| "Windows display source mode index is invalid".into())
}

fn active_source_modes(
    paths: &[DISPLAYCONFIG_PATH_INFO],
    modes: &[DISPLAYCONFIG_MODE_INFO],
) -> Result<BTreeMap<TargetKey, usize>, String> {
    let mut indices = BTreeMap::new();
    let mut used_source_modes = BTreeSet::new();
    for path in paths {
        let index = source_mode_index(path)?;
        let mode = modes
            .get(index)
            .ok_or("Windows active display source mode is missing")?;
        if mode.infoType != DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE
            || mode.adapterId != path.sourceInfo.adapterId
            || mode.id != path.sourceInfo.id
        {
            return Err("Windows active display source mode changed".into());
        }
        // SAFETY: infoType confirms the initialized source-mode union member.
        let source_mode = unsafe { mode.Anonymous.sourceMode };
        if source_mode.width == 0 || source_mode.height == 0 {
            return Err("Windows active display source dimensions are invalid".into());
        }
        if indices.insert(target_key(path), index).is_some() {
            return Err("Windows display target has ambiguous source modes".into());
        }
        if !used_source_modes.insert(index) {
            return Err("Windows display source is cloned or ambiguous".into());
        }
    }
    Ok(indices)
}

fn configuration_positions(
    paths: &[DISPLAYCONFIG_PATH_INFO],
    modes: &[DISPLAYCONFIG_MODE_INFO],
) -> Result<BTreeMap<TargetKey, SourceGeometry>, String> {
    let indices = active_source_modes(paths, modes)?;
    Ok(indices
        .into_iter()
        .map(|(target, index)| {
            // SAFETY: active_source_modes checked every source mode index and type.
            let source = unsafe { modes[index].Anonymous.sourceMode };
            (
                target,
                (
                    source.position.x,
                    source.position.y,
                    source.width,
                    source.height,
                ),
            )
        })
        .collect())
}

/// Preserve every native path and mode field except desktop source position.
/// The observed layout must match the native configuration before constructing
/// a supplied configuration for SetDisplayConfig.
fn requested_source_modes(
    observation: &Observation,
    requested: &Layout,
    paths: &[DISPLAYCONFIG_PATH_INFO],
    modes: &[DISPLAYCONFIG_MODE_INFO],
) -> Result<Vec<DISPLAYCONFIG_MODE_INFO>, String> {
    let indices = active_source_modes(paths, modes)?;
    if indices.len() != requested.outputs.len() {
        return Err("Windows display source count changed before staging".into());
    }
    let mut prepared = modes.to_vec();
    for placement in &requested.outputs {
        let key = paths
            .iter()
            .map(target_key)
            .find(|key| target_identity(*key) == placement.output.id)
            .ok_or("Windows display target changed before staging")?;
        let index = indices[&key];
        // SAFETY: active_source_modes checked infoType and bounds for this index.
        let mut source = unsafe { prepared[index].Anonymous.sourceMode };
        let original = observation
            .layout
            .outputs
            .iter()
            .find(|output| output.output == placement.output)
            .ok_or("Windows display output retired before staging")?;
        let size = observation
            .dimensions
            .get(&placement.output.id)
            .ok_or("Windows display dimensions changed before staging")?;
        if (source.position.x, source.position.y) != (original.x, original.y)
            || (source.width, source.height) != *size
        {
            return Err("Windows display source mode changed before staging".into());
        }
        source.position.x = placement.x;
        source.position.y = placement.y;
        prepared[index].Anonymous.sourceMode = source;
    }
    Ok(prepared)
}

fn native_configuration(
    flags: QUERY_DISPLAY_CONFIG_FLAGS,
) -> Result<(Vec<DISPLAYCONFIG_PATH_INFO>, Vec<DISPLAYCONFIG_MODE_INFO>), String> {
    let mut path_count = 0;
    let mut mode_count = 0;
    // SAFETY: The count pointers are valid writable storage.
    if unsafe { GetDisplayConfigBufferSizes(flags, &mut path_count, &mut mode_count) }
        != ERROR_SUCCESS
    {
        return Err("Windows DisplayConfig topology is unavailable".into());
    }
    if path_count == 0
        || path_count as usize > MAX_NATIVE_PATHS
        || mode_count as usize > MAX_NATIVE_MODES
    {
        return Err("Windows DisplayConfig topology exceeds its bound".into());
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    let mut topology = DISPLAYCONFIG_TOPOLOGY_ID::default();
    let topology_id =
        (flags & QDC_DATABASE_CURRENT == QDC_DATABASE_CURRENT).then_some(&raw mut topology);
    // SAFETY: The vectors have the capacities returned by DisplayConfig and
    // both count pointers remain valid throughout this synchronous call.
    if unsafe {
        QueryDisplayConfig(
            flags,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            topology_id,
        )
    } != ERROR_SUCCESS
    {
        return Err("Windows DisplayConfig changed during observation".into());
    }
    paths.truncate(path_count as usize);
    modes.truncate(mode_count as usize);
    Ok((paths, modes))
}

fn validate_source_geometry(
    inventory: &OutputInventory,
    sources: &BTreeMap<String, SourceGeometry>,
) -> Result<(), String> {
    if inventory.outputs.len() != sources.len() {
        return Err("Windows DisplayConfig sources do not match active monitors".into());
    }
    for output in &inventory.outputs {
        let expected = sources
            .get(&output.name)
            .ok_or("Windows DisplayConfig source identity differs from active monitor")?;
        let observed = (
            output.geometry[0],
            output.geometry[1],
            u32::try_from(output.geometry[2])
                .map_err(|_| "Windows active monitor width is invalid")?,
            u32::try_from(output.geometry[3])
                .map_err(|_| "Windows active monitor height is invalid")?,
        );
        if !output.enabled || *expected != observed {
            return Err("Windows DisplayConfig source geometry differs from active monitor".into());
        }
    }
    Ok(())
}

fn native_transaction_prerequisites(inventory: &OutputInventory) -> Result<(), String> {
    use windows::Win32::Devices::Display::{
        QDC_VIRTUAL_MODE_AWARE, SDC_USE_SUPPLIED_DISPLAY_CONFIG, SDC_VALIDATE,
        SDC_VIRTUAL_MODE_AWARE, SetDisplayConfig,
    };
    let (active_paths, active_modes) =
        native_configuration(QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE)?;
    let active_positions = configuration_positions(&active_paths, &active_modes)?;
    let mut named_positions = BTreeMap::new();
    for path in &active_paths {
        let source = source_name(path)?;
        let geometry = *active_positions
            .get(&target_key(path))
            .ok_or("Windows active display source geometry is unavailable")?;
        if named_positions.insert(source, geometry).is_some() {
            return Err("Windows DisplayConfig source identity is ambiguous".into());
        }
    }
    validate_source_geometry(inventory, &named_positions)?;
    let (saved_paths, saved_modes) =
        native_configuration(QDC_DATABASE_CURRENT | QDC_VIRTUAL_MODE_AWARE)?;
    configuration_positions(&saved_paths, &saved_modes)?;
    // SAFETY: SDC_VALIDATE only checks the captured active configuration and
    // does not change active or saved display settings.
    let result = unsafe {
        SetDisplayConfig(
            Some(&active_paths),
            Some(&active_modes),
            SDC_VALIDATE | SDC_USE_SUPPLIED_DISPLAY_CONFIG | SDC_VIRTUAL_MODE_AWARE,
        )
    };
    if result != ERROR_SUCCESS.0 as i32 {
        return Err(format!(
            "Windows display configuration validation is unavailable ({result})"
        ));
    }
    Ok(())
}

fn native_paths(flags: QUERY_DISPLAY_CONFIG_FLAGS) -> Result<Vec<DISPLAYCONFIG_PATH_INFO>, String> {
    native_configuration(flags).map(|(paths, _)| paths)
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
    available: impl IntoIterator<Item = TargetKey>,
    active: impl IntoIterator<Item = (String, TargetKey)>,
    monitor_names: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>, String> {
    let available = available.into_iter().collect::<BTreeSet<_>>();
    let mut active_targets = BTreeSet::new();
    let mut named = BTreeMap::new();
    for (name, target) in active {
        if !active_targets.insert(target) || named.insert(name, target_identity(target)).is_some() {
            return Err("Windows display source is cloned or ambiguous".into());
        }
    }
    if available.is_empty() {
        return Err("Windows has no available display targets".into());
    }
    if active_targets.is_subset(&available) && available != active_targets {
        return Err("Windows available display target is inactive".into());
    }
    if available != active_targets {
        return Err("Windows active display target changed or is unavailable".into());
    }
    if named.keys().cloned().collect::<BTreeSet<_>>()
        != monitor_names.into_iter().collect::<BTreeSet<_>>()
    {
        return Err("Windows active monitor inventory does not match display sources".into());
    }
    Ok(named)
}

/// Return stable adapter/target IDs only when the active monitor inventory is
/// the complete available topology. Inactive available targets require a
/// separate owner that can observe their placement and scale accurately.
pub(crate) fn complete_active_target_identities(
    monitor_names: impl IntoIterator<Item = String>,
) -> Result<BTreeMap<String, String>, String> {
    let available = native_paths(QDC_ALL_PATHS)?
        .into_iter()
        .filter(|path| path.targetInfo.targetAvailable.as_bool())
        .map(|path| target_key(&path))
        .collect::<Vec<_>>();
    let (active_paths, active_modes) = native_configuration(QDC_ONLY_ACTIVE_PATHS)?;
    active_source_modes(&active_paths, &active_modes)?;
    let active = active_paths
        .into_iter()
        .map(|path| Ok((source_name(&path)?, target_key(&path))))
        .collect::<Result<Vec<_>, String>>()?;
    complete_active_targets(available, active, monitor_names)
}

/// Validate the subset that the current GDI placement API can preserve: all
/// available targets remain enabled at their observed scale, while positions
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
        if placement.output != requested.primary && placement.x == 0 && placement.y == 0 {
            return Err("Windows primary display placement is ambiguous".into());
        }
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

    fn native_inventory() -> OutputInventory {
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
        let incomplete =
            project_inventory(&inventory, Err("extra native target".into()), Ok(())).unwrap();
        assert!(!incomplete.topology_complete);
        assert!(!incomplete.transaction_supported);
        assert_eq!(
            incomplete.transaction_unavailable_reason.as_deref(),
            Some("extra native target")
        );
        assert_eq!(
            apply_position_change(
                &incomplete,
                &incomplete.layout,
                &incomplete.layout,
                incomplete.topology_generation,
                || panic!("incomplete topology reached the native commit boundary"),
            )
            .err()
            .unwrap()
            .reason,
            "extra native target"
        );
        assert_eq!(incomplete.layout.primary.id, r"\\.\DISPLAY1");
        let complete = project_inventory(
            &inventory,
            Ok(BTreeMap::from([(
                r"\\.\DISPLAY1".into(),
                "adapter:target".into(),
            )])),
            Ok(()),
        )
        .unwrap();
        assert!(complete.topology_complete);
        assert!(complete.transaction_supported);
        assert_eq!(complete.layout.primary.id, "adapter:target");
        let unavailable = project_inventory(
            &inventory,
            Ok(BTreeMap::from([(
                r"\\.\DISPLAY1".into(),
                "adapter:target".into(),
            )])),
            Err("saved Windows display configuration unavailable".into()),
        )
        .unwrap();
        assert!(unavailable.topology_complete);
        assert!(!unavailable.transaction_supported);
        assert_eq!(unavailable.layout.primary.id, "adapter:target");
        assert_eq!(
            unavailable.transaction_unavailable_reason.as_deref(),
            Some("saved Windows display configuration unavailable")
        );
        assert_eq!(
            apply_position_change(
                &unavailable,
                &unavailable.layout,
                &unavailable.layout,
                unavailable.topology_generation,
                || panic!("unavailable transaction reached the native commit boundary"),
            )
            .err()
            .unwrap()
            .reason,
            "saved Windows display configuration unavailable"
        );
    }

    #[test]
    fn source_geometry_must_match_active_monitor_before_native_transaction() {
        let inventory = OutputInventory {
            observation_generation: 1,
            observed_at_us: 0,
            topology_generation: 1,
            outputs: vec![OutputDiagnostic {
                name: r"\\.\DISPLAY1".into(),
                generation: 1,
                geometry: [0, 0, 1280, 720],
                work_area: [0, 0, 1280, 700],
                scale_120: 120,
                primary: true,
                enabled: true,
            }],
            truncated: false,
        };
        let mut sources = BTreeMap::from([(r"\\.\DISPLAY1".into(), (0, 0, 1280, 720))]);
        assert!(validate_source_geometry(&inventory, &sources).is_ok());
        sources.insert(r"\\.\DISPLAY1".into(), (0, 0, 1920, 1080));
        assert_eq!(
            validate_source_geometry(&inventory, &sources).unwrap_err(),
            "Windows DisplayConfig source geometry differs from active monitor"
        );
        sources.insert(r"\\.\DISPLAY1".into(), (8, 0, 1280, 720));
        assert_eq!(
            validate_source_geometry(&inventory, &sources).unwrap_err(),
            "Windows DisplayConfig source geometry differs from active monitor"
        );
    }

    #[test]
    fn complete_active_topology_requires_exact_native_targets_and_monitor_names() {
        let left = (1, 2, 3);
        let right = (1, 2, 4);
        let names = [r"\\.\DISPLAY1".to_owned(), r"\\.\DISPLAY2".to_owned()];
        let active = [(names[0].clone(), left), (names[1].clone(), right)];
        let mapped = complete_active_targets([left, right], active.clone(), names.clone()).unwrap();
        assert_ne!(mapped[&names[0]], mapped[&names[1]]);
        assert_eq!(
            complete_active_targets([], active.clone(), names.clone()).unwrap_err(),
            "Windows has no available display targets"
        );
        assert_eq!(
            complete_active_targets([left, right, (1, 2, 5)], active.clone(), names.clone())
                .unwrap_err(),
            "Windows available display target is inactive"
        );
        assert_eq!(
            complete_active_targets([left, right], active.clone(), [names[0].clone()]).unwrap_err(),
            "Windows active monitor inventory does not match display sources"
        );
        assert_eq!(
            complete_active_targets([left], active.clone(), names.clone()).unwrap_err(),
            "Windows active display target changed or is unavailable"
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
    fn active_source_modes_reject_missing_or_mismatched_native_modes() {
        let mut path = DISPLAYCONFIG_PATH_INFO::default();
        path.sourceInfo.adapterId.HighPart = 3;
        path.sourceInfo.adapterId.LowPart = 4;
        path.sourceInfo.id = 5;
        path.targetInfo.id = 6;
        path.sourceInfo.Anonymous.modeInfoIdx = 0;
        let mut mode = DISPLAYCONFIG_MODE_INFO {
            infoType: DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE,
            adapterId: path.sourceInfo.adapterId,
            id: path.sourceInfo.id,
            ..Default::default()
        };
        mode.Anonymous.sourceMode.width = 1920;
        mode.Anonymous.sourceMode.height = 1080;
        assert_eq!(active_source_modes(&[path], &[mode]).unwrap().len(), 1);
        assert!(active_source_modes(&[path], &[]).is_err());
        mode.id += 1;
        assert!(active_source_modes(&[path], &[mode]).is_err());
    }

    #[test]
    fn supplied_configuration_moves_only_validated_source_positions() {
        let current = layout("left", 0, 1920);
        let requested = layout("right", -1920, 0);
        let mut paths = [DISPLAYCONFIG_PATH_INFO::default(); 2];
        let mut modes = [DISPLAYCONFIG_MODE_INFO::default(); 2];
        for index in 0..2 {
            paths[index].sourceInfo.id = index as u32;
            paths[index].targetInfo.id = index as u32;
            paths[index].sourceInfo.Anonymous.modeInfoIdx = index as u32;
            modes[index].infoType = DISPLAYCONFIG_MODE_INFO_TYPE_SOURCE;
            modes[index].id = index as u32;
            modes[index].Anonymous.sourceMode.width = 1920;
            modes[index].Anonymous.sourceMode.height = 1080;
            modes[index].Anonymous.sourceMode.position.x = current.outputs[index].x;
            modes[index].Anonymous.sourceMode.position.y = current.outputs[index].y;
        }
        let mut native_layout = current.clone();
        for (index, output) in native_layout.outputs.iter_mut().enumerate() {
            output.output.id = target_identity(target_key(&paths[index]));
        }
        native_layout.primary = native_layout.outputs[0].output.clone();
        let mut native_requested = requested.clone();
        for (index, output) in native_requested.outputs.iter_mut().enumerate() {
            output.output.id = native_layout.outputs[index].output.id.clone();
        }
        native_requested.primary = native_requested.outputs[1].output.clone();
        let observation = Observation {
            layout: native_layout,
            topology_generation: 1,
            topology_complete: true,
            transaction_supported: true,
            transaction_unavailable_reason: None,
            dimensions: native_requested
                .outputs
                .iter()
                .map(|output| (output.output.id.clone(), (1920, 1080)))
                .collect(),
            native_names: BTreeMap::new(),
        };
        let prepared =
            requested_source_modes(&observation, &native_requested, &paths, &modes).unwrap();
        assert_eq!(
            unsafe { prepared[0].Anonymous.sourceMode.position.x },
            -1920
        );
        assert_eq!(unsafe { prepared[1].Anonymous.sourceMode.position.x }, 0);
        assert_eq!(unsafe { modes[0].Anonymous.sourceMode.position.x }, 0);
        paths[1].sourceInfo.Anonymous.modeInfoIdx = 0;
        assert!(requested_source_modes(&observation, &native_requested, &paths, &modes).is_err());
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
        let mut ambiguous_primary = requested.clone();
        ambiguous_primary.outputs[1].x = 0;
        ambiguous_primary.outputs[1].y = 0;
        assert_eq!(
            validate_position_change(7, 7, &current, &ambiguous_primary, &current, &dimensions)
                .unwrap_err(),
            "Windows primary display placement is ambiguous"
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
            let matches_active_monitor_path = result == 0
                && target.monitorDevicePath[0] != 0
                && active_paths.iter().any(|active| {
                    let mut active_name = DISPLAYCONFIG_TARGET_DEVICE_NAME {
                        header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                            r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                            size: std::mem::size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                            adapterId: active.targetInfo.adapterId,
                            id: active.targetInfo.id,
                        },
                        ..Default::default()
                    };
                    // SAFETY: The initialized packet is writable for this read-only query.
                    (unsafe { DisplayConfigGetDeviceInfo(&raw mut active_name.header) }) == 0
                        && active_name.monitorDevicePath == target.monitorDevicePath
                });
            eprintln!(
                "inactive available target: query={}, monitor_path_present={}, friendly_name_present={}, matches_active_monitor_path={}",
                result,
                target.monitorDevicePath[0] != 0,
                target.monitorFriendlyDeviceName[0] != 0,
                matches_active_monitor_path
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

    #[test]
    #[ignore = "reads the current Windows display modes"]
    fn native_display_source_modes_gate_transaction_support() {
        use windows::Win32::Devices::Display::QDC_VIRTUAL_MODE_AWARE;

        let inventory = native_inventory();
        let (paths, modes) =
            native_configuration(QDC_ONLY_ACTIVE_PATHS | QDC_VIRTUAL_MODE_AWARE).unwrap();
        let indices = active_source_modes(&paths, &modes).unwrap();
        let (saved_paths, saved_modes) =
            native_configuration(QDC_DATABASE_CURRENT | QDC_VIRTUAL_MODE_AWARE).unwrap();
        assert!(
            !configuration_positions(&saved_paths, &saved_modes)
                .unwrap()
                .is_empty()
        );
        let readiness = native_transaction_prerequisites(&inventory);
        assert_eq!(paths.len(), inventory.outputs.len());
        let mut named = BTreeMap::new();
        for path in &paths {
            let name = source_name(path).unwrap();
            // SAFETY: active_source_modes verified the source mode index and type.
            let source = unsafe { modes[indices[&target_key(path)]].Anonymous.sourceMode };
            named.insert(
                name,
                (
                    source.position.x,
                    source.position.y,
                    source.width,
                    source.height,
                ),
            );
        }
        if let Err(reason) = validate_source_geometry(&inventory, &named) {
            assert_eq!(
                readiness.unwrap_err(),
                reason,
                "transaction readiness must refuse mismatched source geometry"
            );
            let observed = observe(&inventory).unwrap();
            assert!(!observed.transaction_supported);
            assert_eq!(
                observed.transaction_unavailable_reason.as_deref(),
                Some(reason.as_str())
            );
            eprintln!("Windows display transactions unavailable on this fixture: {reason}");
        } else {
            readiness.unwrap();
        }
    }

    /// Run only with an explicitly safe multi-monitor fixture. The guard
    /// restores the original native modes even when an assertion panics.
    #[test]
    #[ignore = "requires NICKEL_WINDOWS_DISPLAY_MUTATION_TEST=1 and a reversible multi-monitor setup"]
    fn native_display_position_round_trip_restores_original_modes() {
        assert_eq!(
            std::env::var("NICKEL_WINDOWS_DISPLAY_MUTATION_TEST").as_deref(),
            Ok("1")
        );
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

        let before =
            observe(&native_inventory()).expect("a complete active display topology is required");
        assert!(
            before.transaction_supported && before.layout.outputs.len() >= 2,
            "complete multi-output native transaction fixture required: {:?}",
            before.transaction_unavailable_reason
        );
        assert!(crate::windows_remote_input::physical_input_idle());
        let mut requested = before.layout.clone();
        let primary = requested.primary.clone();
        let secondary = requested
            .outputs
            .iter_mut()
            .find(|output| output.output != primary)
            .unwrap();
        secondary.y = secondary.y.checked_add(8).unwrap();
        if secondary.x == 0 && secondary.y == 0 {
            secondary.y = secondary.y.checked_sub(16).unwrap();
        }
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
                    let _guard = Restore(Some(*plan));
                }
                panic!("{}", failure.reason);
            }
        };
        let mut guard = Restore(Some(plan));
        let applied = observe(&native_inventory()).unwrap();
        assert!(
            matches_physical_layout(&applied.layout, &requested),
            "Windows did not apply the requested position: before={:?}, requested={:?}, applied={:?}",
            before.layout,
            requested,
            applied.layout
        );
        guard.0.as_ref().unwrap().restore().unwrap();
        guard.0 = None;
        let restored = observe(&native_inventory()).unwrap();
        assert!(matches_physical_layout(&restored.layout, &before.layout));
    }
}
