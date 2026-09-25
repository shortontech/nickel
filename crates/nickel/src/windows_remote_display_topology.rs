//! Read-only DisplayConfig identity and completeness checks for remote layouts.
//! The existing monitor inventory contains active monitors only. It cannot be
//! presented as a complete transactional layout while a connected target is
//! disabled or while a cloned source obscures target identity.

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

#[cfg(test)]
mod tests {
    use super::*;
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
                    y: -100,
                    enabled: true,
                    scale_120: 120,
                },
                Placement {
                    output: ResourceId {
                        id: "right".into(),
                        generation: 2,
                    },
                    x: right_x,
                    y: 0,
                    enabled: true,
                    scale_120: 180,
                },
            ],
        }
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
        let current = layout("left", -1920, 0);
        let requested = layout("right", 0, -1920);
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
        let overflowing = layout("left", i32::MAX, 0);
        assert!(
            validate_position_change(7, 7, &current, &overflowing, &current, &dimensions).is_err()
        );
        assert!(
            validate_position_change(6, 7, &current, &requested, &current, &dimensions).is_err()
        );
    }
}
