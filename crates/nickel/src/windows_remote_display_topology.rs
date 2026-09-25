//! Read-only DisplayConfig identity and completeness checks for remote layouts.
//! The existing monitor inventory contains active monitors only. It cannot be
//! presented as a complete transactional layout while a connected target is
//! disabled or while a cloned source obscures target identity.

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
