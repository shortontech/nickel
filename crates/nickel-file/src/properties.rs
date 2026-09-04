//! Platform-neutral presentation state for a filesystem Properties dialog.

use std::{
    fs, io,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
    },
    time::SystemTime,
};

use crate::{FileEntry, FileIdentity, metadata_identity};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EntryProperties {
    pub identity: Option<FileIdentity>,
    pub path: PathBuf,
    pub name: String,
    pub kind: String,
    pub logical_size: Option<u64>,
    pub allocated_size: Option<u64>,
    pub modified: Option<SystemTime>,
    pub accessed: Option<SystemTime>,
    pub created: Option<SystemTime>,
    pub readonly: bool,
    pub permissions: String,
    pub owner: Option<String>,
    pub hidden: bool,
    pub symlink_target: Option<PathBuf>,
    confirmed_metadata_len: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PropertyEdits {
    pub readonly: bool,
    pub hidden: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyApplyOutcome {
    pub path: PathBuf,
    pub readonly: Result<(), String>,
    pub hidden: Result<(), String>,
}

pub fn apply_edits(properties: &EntryProperties, edits: PropertyEdits) -> PropertyApplyOutcome {
    if properties.is_stale() {
        let error = Err("the target changed; refresh Properties before applying changes".into());
        return PropertyApplyOutcome {
            path: properties.path.clone(),
            readonly: error.clone(),
            hidden: error,
        };
    }
    let readonly =
        set_readonly(&properties.path, edits.readonly).map_err(|error| error.to_string());
    let path = properties.path.clone();
    let hidden = if properties.hidden == edits.hidden {
        Ok(None)
    } else {
        set_hidden(&path, edits.hidden)
    };
    let path = hidden
        .as_ref()
        .ok()
        .and_then(|updated| updated.clone())
        .unwrap_or(path);
    PropertyApplyOutcome {
        path,
        readonly,
        hidden: hidden.map(|_| ()),
    }
}

#[cfg(not(target_os = "windows"))]
fn set_hidden(path: &Path, hidden: bool) -> Result<Option<PathBuf>, String> {
    if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
        let next_name = if hidden {
            format!(".{name}")
        } else {
            name.trim_start_matches('.').to_owned()
        };
        if next_name.is_empty() {
            Err("the hidden-state change would produce an empty name".into())
        } else {
            let next = path.with_file_name(next_name);
            if next != path && next.exists() {
                Err(format!("{} already exists", next.display()))
            } else {
                fs::rename(path, &next)
                    .map(|()| Some(next))
                    .map_err(|error| error.to_string())
            }
        }
    } else {
        Err("hidden-state editing is unavailable for this filename".into())
    }
}

#[cfg(target_os = "windows")]
fn set_hidden(path: &Path, hidden: bool) -> Result<Option<PathBuf>, String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::Storage::FileSystem::{
            FILE_ATTRIBUTE_HIDDEN, FILE_FLAGS_AND_ATTRIBUTES, GetFileAttributesW,
            INVALID_FILE_ATTRIBUTES, SetFileAttributesW,
        },
        core::PCWSTR,
    };
    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        let attributes = GetFileAttributesW(PCWSTR(wide.as_ptr()));
        if attributes == INVALID_FILE_ATTRIBUTES {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let updated = if hidden {
            attributes | FILE_ATTRIBUTE_HIDDEN.0
        } else {
            attributes & !FILE_ATTRIBUTE_HIDDEN.0
        };
        SetFileAttributesW(PCWSTR(wide.as_ptr()), FILE_FLAGS_AND_ATTRIBUTES(updated))
            .map_err(|error| error.to_string())?;
    }
    Ok(None)
}

#[cfg(unix)]
fn set_readonly(path: &Path, readonly: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let metadata = fs::metadata(path)?;
    let mut permissions = metadata.permissions();
    let mode = permissions.mode();
    permissions.set_mode(if readonly {
        mode & !0o222
    } else {
        mode | 0o200
    });
    fs::set_permissions(path, permissions)
}

#[cfg(not(unix))]
fn set_readonly(path: &Path, readonly: bool) -> io::Result<()> {
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_readonly(readonly);
    fs::set_permissions(path, permissions)
}

#[derive(Debug)]
pub enum RecursiveSizeUpdate {
    Progress { entries: u64, bytes: u64 },
    Complete(u64),
    Failed(String),
    Cancelled,
}

pub struct RecursiveSizeJob {
    cancel: Arc<AtomicBool>,
    pub receiver: Receiver<RecursiveSizeUpdate>,
}

impl RecursiveSizeJob {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}

pub fn calculate_recursive_size(root: PathBuf) -> RecursiveSizeJob {
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = cancel.clone();
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut stack = vec![root];
        let mut entries = 0u64;
        let mut bytes = 0u64;
        while let Some(path) = stack.pop() {
            if worker_cancel.load(Ordering::Acquire) {
                let _ = sender.send(RecursiveSizeUpdate::Cancelled);
                return;
            }
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) => {
                    let _ = sender.send(RecursiveSizeUpdate::Failed(error.to_string()));
                    return;
                }
            };
            entries = entries.saturating_add(1);
            if metadata.is_file() {
                bytes = bytes.saturating_add(metadata.len());
            }
            if metadata.is_dir() {
                let children = match fs::read_dir(&path) {
                    Ok(children) => children,
                    Err(error) => {
                        let _ = sender.send(RecursiveSizeUpdate::Failed(error.to_string()));
                        return;
                    }
                };
                stack.extend(children.filter_map(Result::ok).map(|entry| entry.path()));
            }
            if entries.is_multiple_of(128) {
                let _ = sender.send(RecursiveSizeUpdate::Progress { entries, bytes });
            }
        }
        let _ = sender.send(RecursiveSizeUpdate::Complete(bytes));
    });
    RecursiveSizeJob { cancel, receiver }
}

impl EntryProperties {
    pub fn load(entry: &FileEntry, identity: Option<FileIdentity>) -> io::Result<Self> {
        let metadata = fs::symlink_metadata(&entry.path)?;
        let file_type = metadata.file_type();
        let kind = if file_type.is_symlink() {
            "Symbolic link"
        } else if metadata.is_dir() {
            "Folder"
        } else if metadata.is_file() {
            "File"
        } else {
            "Special file"
        };
        Ok(Self {
            identity,
            path: entry.path.clone(),
            name: entry.display_name(),
            kind: kind.into(),
            logical_size: metadata.is_file().then_some(metadata.len()),
            allocated_size: allocated_size(&metadata),
            modified: metadata.modified().ok(),
            accessed: metadata.accessed().ok(),
            created: metadata.created().ok(),
            readonly: metadata.permissions().readonly(),
            permissions: permission_label(&metadata),
            owner: owner_label(&metadata),
            hidden: hidden_state(&entry.path, &metadata),
            symlink_target: file_type
                .is_symlink()
                .then(|| fs::read_link(&entry.path).ok())
                .flatten(),
            confirmed_metadata_len: metadata.len(),
        })
    }

    /// Revalidates the stable provider identity; a renamed object remains the
    /// same target while replacement or disappearance becomes stale.
    pub fn is_stale(&self) -> bool {
        let Ok(metadata) = fs::symlink_metadata(&self.path) else {
            return true;
        };
        match self.identity {
            Some(expected) if metadata_identity(&self.path, &metadata) != Some(expected) => true,
            _ => metadata.len() != self.confirmed_metadata_len,
        }
    }
}

#[cfg(not(target_os = "windows"))]
fn hidden_state(path: &Path, _: &fs::Metadata) -> bool {
    path.file_name()
        .is_some_and(|name| name.to_string_lossy().starts_with('.'))
}

#[cfg(target_os = "windows")]
fn hidden_state(_: &Path, metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    use windows::Win32::Storage::FileSystem::FILE_ATTRIBUTE_HIDDEN;
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN.0 != 0
}

#[cfg(unix)]
fn permission_label(metadata: &fs::Metadata) -> String {
    use std::os::unix::fs::PermissionsExt;
    format!("{:04o}", metadata.permissions().mode() & 0o7777)
}
#[cfg(not(unix))]
fn permission_label(metadata: &fs::Metadata) -> String {
    if metadata.permissions().readonly() {
        "read-only".into()
    } else {
        "read/write".into()
    }
}
#[cfg(unix)]
fn owner_label(metadata: &fs::Metadata) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    Some(format!("UID {} · GID {}", metadata.uid(), metadata.gid()))
}
#[cfg(not(unix))]
fn owner_label(_: &fs::Metadata) -> Option<String> {
    None
}

#[cfg(unix)]
fn allocated_size(metadata: &fs::Metadata) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    metadata.blocks().checked_mul(512)
}

#[cfg(not(unix))]
fn allocated_size(_: &fs::Metadata) -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loads_truthful_metadata_and_detects_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("sample.txt");
        fs::write(&path, b"hello").unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let identity = metadata_identity(&path, &metadata);
        let entry = FileEntry {
            name: "sample.txt".into(),
            path: path.clone(),
            is_directory: false,
            size: Some(5),
            modified: None,
        };
        let properties = EntryProperties::load(&entry, identity).unwrap();
        assert_eq!(properties.logical_size, Some(5));
        assert_eq!(properties.kind, "File");
        assert!(!properties.is_stale());
        fs::remove_file(&path).unwrap();
        assert!(properties.is_stale());
    }

    #[test]
    fn folder_open_does_not_scan_descendants() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("child")).unwrap();
        fs::write(directory.path().join("child/large"), vec![0; 4096]).unwrap();
        let entry = FileEntry {
            name: "child".into(),
            path: directory.path().join("child"),
            is_directory: true,
            size: None,
            modified: None,
        };
        let properties = EntryProperties::load(&entry, None).unwrap();
        assert_eq!(properties.kind, "Folder");
        assert_eq!(properties.logical_size, None);
    }

    #[test]
    fn recursive_size_reports_exact_file_content_total() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir(directory.path().join("child")).unwrap();
        fs::write(directory.path().join("a"), b"123").unwrap();
        fs::write(directory.path().join("child/b"), b"4567").unwrap();
        let job = calculate_recursive_size(directory.path().to_path_buf());
        loop {
            match job.receiver.recv().unwrap() {
                RecursiveSizeUpdate::Complete(bytes) => {
                    assert_eq!(bytes, 7);
                    break;
                }
                RecursiveSizeUpdate::Progress { .. } => {}
                update => panic!("unexpected recursive result: {update:?}"),
            }
        }
    }

    #[test]
    fn property_edits_report_each_field_and_preserve_identity() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("visible.txt");
        fs::write(&path, b"data").unwrap();
        let metadata = fs::metadata(&path).unwrap();
        let entry = FileEntry {
            name: "visible.txt".into(),
            path: path.clone(),
            is_directory: false,
            size: Some(4),
            modified: None,
        };
        let properties =
            EntryProperties::load(&entry, metadata_identity(&path, &metadata)).unwrap();
        let outcome = apply_edits(
            &properties,
            PropertyEdits {
                readonly: true,
                hidden: true,
            },
        );
        assert!(outcome.readonly.is_ok(), "{:?}", outcome.readonly);
        assert!(outcome.hidden.is_ok(), "{:?}", outcome.hidden);
        assert_eq!(outcome.path.file_name().unwrap(), ".visible.txt");
        assert!(fs::metadata(outcome.path).unwrap().permissions().readonly());
    }

    #[test]
    fn hidden_edit_never_replaces_an_existing_entry() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("visible.txt");
        let hidden = directory.path().join(".visible.txt");
        fs::write(&path, b"source").unwrap();
        fs::write(&hidden, b"keep me").unwrap();
        let entry = FileEntry {
            name: "visible.txt".into(),
            path: path.clone(),
            is_directory: false,
            size: Some(6),
            modified: None,
        };
        let properties = EntryProperties::load(&entry, None).unwrap();
        let outcome = apply_edits(
            &properties,
            PropertyEdits {
                readonly: properties.readonly,
                hidden: true,
            },
        );
        assert!(outcome.hidden.is_err());
        assert_eq!(fs::read(path).unwrap(), b"source");
        assert_eq!(fs::read(hidden).unwrap(), b"keep me");
    }
}
