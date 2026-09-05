//! Shared authority for Nickel's small, platform-local configuration files.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub(crate) fn config_path(file_name: &str) -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let root = std::env::var_os("LOCALAPPDATA")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))?;
        Ok(PathBuf::from(root).join("Nickel").join(file_name))
    }
    #[cfg(target_os = "macos")]
    {
        let root = std::env::var_os("HOME")
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))?;
        Ok(PathBuf::from(root)
            .join("Library/Application Support/Nickel")
            .join(file_name))
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let root = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_CONFIG_HOME and HOME are not set",
                )
            })?;
        Ok(root.join("nickel").join(file_name))
    }
}

/// Replaces a complete settings file without exposing readers to a partial
/// write. This is intentionally for small files owned by one Nickel process.
pub(crate) fn atomic_write(path: &Path, contents: impl AsRef<[u8]>) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, contents)?;
    match replace_file(&temporary, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(temporary);
            Err(error)
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(temporary, destination)
}

#[cfg(target_os = "windows")]
fn replace_file(temporary: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            MOVE_FILE_FLAGS, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
        },
        core::PCWSTR,
    };
    let temporary = temporary
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let destination = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: both paths are terminated and remain live for the synchronous call.
    unsafe {
        MoveFileExW(
            PCWSTR(temporary.as_ptr()),
            PCWSTR(destination.as_ptr()),
            MOVE_FILE_FLAGS(MOVEFILE_REPLACE_EXISTING.0 | MOVEFILE_WRITE_THROUGH.0),
        )
    }
    .map_err(io::Error::other)
}

#[cfg(test)]
mod tests {
    use super::atomic_write;

    #[test]
    fn atomic_replacement_never_leaves_the_temporary_peer() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("settings");
        atomic_write(&path, "first").unwrap();
        atomic_write(&path, "second").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "second");
        assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
