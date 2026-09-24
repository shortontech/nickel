//! Recycle Bin operations for Nickel File's Windows adapter.

use std::{os::windows::ffi::OsStrExt, path::PathBuf};
use windows::{
    Win32::{
        System::Com::{
            CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED, CoCreateInstance, CoInitializeEx,
            CoUninitialize,
        },
        UI::Shell::{
            FOF_NOERRORUI, FOFX_ADDUNDORECORD, FOFX_EARLYFAILURE, FOFX_RECYCLEONDELETE,
            FileOperation, IFileOperation, IFileOperationProgressSink, IShellItem,
            SHCreateItemFromParsingName,
        },
    },
    core::PCWSTR,
};

pub(crate) fn move_to_trash(sources: &[(PathBuf, crate::FileIdentity)]) -> Result<(), String> {
    if sources.is_empty() {
        return Err("no items selected".into());
    }
    // Verify the context-menu snapshot before asking the shell to act on any
    // path. A replaced file must not inherit the previous file's command.
    let paths = sources
        .iter()
        .map(|(path, identity)| {
            if crate::file_identity(path).ok() != Some(*identity) {
                return Err(format!("{} changed since selection", path.display()));
            }
            std::path::absolute(path)
                .map_err(|error| format!("resolve {}: {error}", path.display()))
        })
        .collect::<Result<Vec<_>, _>>()?;

    // SAFETY: Nickel File runs this function on a dedicated worker thread.
    // The guard balances this successful STA initialization on that thread.
    unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }
        .ok()
        .map_err(|error| format!("initialize Recycle Bin apartment: {error}"))?;
    let apartment = ComApartment;
    let result = recycle_paths(&paths);
    drop(apartment);
    result
}

struct ComApartment;

impl Drop for ComApartment {
    fn drop(&mut self) {
        // SAFETY: Paired with successful CoInitializeEx on this thread.
        unsafe { CoUninitialize() };
    }
}

fn recycle_paths(paths: &[PathBuf]) -> Result<(), String> {
    // SAFETY: COM is initialized on this STA and Windows owns the shell class.
    let operation: IFileOperation =
        unsafe { CoCreateInstance(&FileOperation, None, CLSCTX_INPROC_SERVER) }
            .map_err(|error| format!("create Recycle Bin operation: {error}"))?;
    // Recycle instead of permanently deleting. Suppress error dialogs and stop
    // on the first error; Nickel File reports failure and refreshes its listing.
    // SAFETY: The flags and operation interface are valid on this STA.
    unsafe {
        operation
            .SetOperationFlags(
                FOFX_RECYCLEONDELETE | FOFX_ADDUNDORECORD | FOF_NOERRORUI | FOFX_EARLYFAILURE,
            )
            .map_err(|error| format!("configure Recycle Bin operation: {error}"))?;
        for path in paths {
            let wide = path
                .as_os_str()
                .encode_wide()
                .chain(std::iter::once(0))
                .collect::<Vec<_>>();
            let item: IShellItem = SHCreateItemFromParsingName(PCWSTR(wide.as_ptr()), None)
                .map_err(|error| format!("inspect {}: {error}", path.display()))?;
            operation
                .DeleteItem(&item, None::<&IFileOperationProgressSink>)
                .map_err(|error| format!("queue {}: {error}", path.display()))?;
        }
        let performed = operation.PerformOperations();
        let aborted = operation
            .GetAnyOperationsAborted()
            .map_err(|error| format!("check Recycle Bin outcome: {error}"))?;
        performed.map_err(|error| format!("move to Recycle Bin: {error}"))?;
        if aborted.as_bool() {
            return Err("Windows stopped the Recycle Bin operation".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn changed_file_identity_is_rejected_before_recycling() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("old.txt");
        let replacement = directory.path().join("replacement.txt");
        std::fs::write(&path, b"old").unwrap();
        let identity = crate::file_identity(&path).unwrap();
        std::fs::write(&replacement, b"new").unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::rename(&replacement, &path).unwrap();

        assert!(move_to_trash(&[(path.clone(), identity)]).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"new");
    }

    #[test]
    #[ignore = "live Windows integration: places a fixture in the user's Recycle Bin"]
    fn moves_file_to_windows_recycle_bin() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nickel-recycle-test.txt");
        std::fs::write(&path, b"Nickel Recycle Bin fixture").unwrap();
        let identity = crate::file_identity(&path).unwrap();

        move_to_trash(&[(path.clone(), identity)]).unwrap();
        assert!(!path.exists());
    }
}
