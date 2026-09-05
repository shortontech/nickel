use std::{
    cmp::Ordering,
    collections::{HashMap, HashSet},
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

extern crate self as nickel_file;

#[path = "main.rs"]
#[allow(dead_code)]
pub(crate) mod app;
pub(crate) mod components;
pub mod desktop;
pub(crate) mod host;
pub mod icons;
pub(crate) mod layout;
pub mod operations;
pub(crate) mod platform;
pub mod properties;
pub mod selection_summary;
pub mod watch;

pub use app::{FileApp, FileFixtureProvider, FileMessage, FileViewMode, run};
pub use watch::DirectoryWatch;

pub fn desktop_directory() -> PathBuf {
    platform::places()
        .into_iter()
        .find_map(|(label, path)| (label == "Desktop").then_some(path))
        .unwrap_or_else(|| platform::home_directory().join("Desktop"))
}

pub fn open_path(path: &Path) -> Result<(), String> {
    match open_target_kind(path) {
        #[cfg(target_os = "linux")]
        OpenTargetKind::Launcher => {
            platform::open_launcher(path).map_err(|error| error.to_string())
        }
        OpenTargetKind::Document => {
            nickel_platform::open_with_default(path).map_err(|error| error.to_string())
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum OpenTargetKind {
    #[cfg(target_os = "linux")]
    Launcher,
    Document,
}

fn open_target_kind(path: &Path) -> OpenTargetKind {
    #[cfg(target_os = "linux")]
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("desktop"))
    {
        return OpenTargetKind::Launcher;
    }
    #[cfg(not(target_os = "linux"))]
    let _ = path;
    OpenTargetKind::Document
}

pub fn is_application_launcher(path: &Path) -> bool {
    path.extension().is_some_and(|extension| {
        let extension = extension.to_string_lossy();
        extension.eq_ignore_ascii_case("desktop")
            || extension.eq_ignore_ascii_case("lnk")
            || extension.eq_ignore_ascii_case("app")
    })
}

/// Delivers a negotiated native file drop to an application launcher.
pub fn open_with_launcher(launcher: &Path, source: &Path) -> Result<(), String> {
    platform::open_with_launcher(launcher, source).map_err(|error| error.to_string())
}

pub fn publish_file_clipboard(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    platform::publish_file_clipboard(paths, cut).map_err(|error| error.to_string())
}

/// Applies the native file clipboard through Nickel File's bounded operation planner.
pub fn paste_native_file_clipboard(destination: &Path) -> Result<usize, String> {
    use operations::{
        ClipboardOffer, ConflictPolicy, ItemCapabilities, TransferIntent, TransferSource,
        execute_local_transfer, plan_paste,
    };
    let (cut, paths) = platform::read_file_clipboard()?;
    let intent = if cut {
        TransferIntent::Move
    } else {
        TransferIntent::Copy
    };
    let sources = paths
        .into_iter()
        .enumerate()
        .map(|(index, path)| TransferSource {
            provider: "native".into(),
            identity: FileIdentity(0, index as u64),
            path,
            capabilities: ItemCapabilities {
                readable: true,
                removable: cut,
            },
        })
        .collect();
    let offer = ClipboardOffer::new(intent, sources)
        .map_err(|error| format!("invalid clipboard offer: {error:?}"))?;
    let effect = plan_paste(&offer, "local", destination, true, ConflictPolicy::Ask)
        .map_err(|error| format!("invalid paste target: {error:?}"))?;
    let report = execute_local_transfer(
        &effect,
        &std::sync::atomic::AtomicBool::new(false),
        |_, _| {},
    );
    if report.failed.is_empty() {
        Ok(report.affected.len())
    } else {
        Err(format!("{} item(s) failed", report.failed.len()))
    }
}

pub fn native_file_clipboard_available() -> bool {
    platform::read_file_clipboard().is_ok_and(|(_, paths)| !paths.is_empty())
}

pub fn directory_is_writable(path: &Path) -> bool {
    std::fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_dir() && !metadata.permissions().readonly())
}

/// Creates a conventionally named folder through the shared file-operation boundary.
pub fn create_new_folder(parent: &Path) -> Result<PathBuf, String> {
    for suffix in 0..10_000 {
        let name = if suffix == 0 {
            "New folder".to_owned()
        } else {
            format!("New folder ({suffix})")
        };
        let path = parent.join(name);
        if path.exists() {
            continue;
        }
        match std::fs::create_dir(&path) {
            Ok(()) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.to_string()),
        }
    }
    Err("could not choose an unused folder name".into())
}

pub fn file_identity(path: &Path) -> io::Result<FileIdentity> {
    let metadata = fs::metadata(path)?;
    metadata_identity(path, &metadata).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::Unsupported,
            "stable file identities are unavailable on this platform",
        )
    })
}

/// Copies one external drag/clipboard source into a provider directory without overwriting.
pub fn copy_into(source: &Path, destination: &Path) -> io::Result<PathBuf> {
    use operations::{
        ClipboardOffer, ConflictPolicy, ItemCapabilities, TransferIntent, TransferSource,
        execute_local_transfer, plan_paste,
    };
    let offer = ClipboardOffer::new(
        TransferIntent::Copy,
        vec![TransferSource {
            provider: "local".into(),
            identity: file_identity(source)?,
            path: source.to_path_buf(),
            capabilities: ItemCapabilities {
                readable: true,
                removable: false,
            },
        }],
    )
    .map_err(|error| io::Error::other(format!("{error:?}")))?;
    let effect = plan_paste(&offer, "local", destination, true, ConflictPolicy::KeepBoth)
        .map_err(|error| io::Error::other(format!("{error:?}")))?;
    let cancelled = std::sync::atomic::AtomicBool::new(false);
    let report = execute_local_transfer(&effect, &cancelled, |_, _| {});
    report.affected.into_iter().next().ok_or_else(|| {
        let detail = report
            .failed
            .into_iter()
            .next()
            .map_or_else(|| "copy produced no output".into(), |(_, error)| error);
        io::Error::other(detail)
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FileEntry {
    pub name: OsString,
    pub display_name_override: Option<String>,
    pub path: PathBuf,
    pub is_directory: bool,
    pub size: Option<u64>,
    pub modified: Option<SystemTime>,
}

impl FileEntry {
    pub fn display_name(&self) -> String {
        self.display_name_override
            .clone()
            .unwrap_or_else(|| self.name.to_string_lossy().into_owned())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EntrySortKey {
    Name,
    Type,
    Modified,
    Size,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Debug)]
pub struct DirectoryBrowser {
    current: PathBuf,
    history: Vec<PathBuf>,
    forward_history: Vec<PathBuf>,
    entries: Vec<FileEntry>,
    identities: HashMap<PathBuf, FileIdentity>,
    show_hidden: bool,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct FileIdentity(pub u64, pub u64);

impl DirectoryBrowser {
    #[doc(hidden)]
    pub fn fixture(entries: Vec<FileEntry>) -> Self {
        let identities = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.path.clone(), FileIdentity(0, index as u64)))
            .collect();
        Self {
            current: PathBuf::from("/fixture"),
            history: Vec::new(),
            forward_history: Vec::new(),
            entries,
            identities,
            show_hidden: true,
        }
    }

    #[doc(hidden)]
    pub fn loading(path: impl Into<PathBuf>, show_hidden: bool) -> Self {
        Self {
            current: path.into(),
            history: Vec::new(),
            forward_history: Vec::new(),
            entries: Vec::new(),
            identities: HashMap::new(),
            show_hidden,
        }
    }
    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        Self::open_with_hidden(path, true)
    }

    pub fn open_with_hidden(path: impl Into<PathBuf>, show_hidden: bool) -> io::Result<Self> {
        let current = path.into();
        let (entries, identities) = read_entries_with_identities(&current, show_hidden)?;
        Ok(Self {
            current,
            history: Vec::new(),
            forward_history: Vec::new(),
            entries,
            identities,
            show_hidden,
        })
    }

    pub fn current(&self) -> &Path {
        &self.current
    }

    pub fn entries(&self) -> &[FileEntry] {
        &self.entries
    }

    pub fn identity_at(&self, index: usize) -> Option<FileIdentity> {
        self.entries
            .get(index)
            .and_then(|entry| self.identities.get(&entry.path).copied())
    }

    pub(crate) fn index_of_identity(&self, identity: FileIdentity) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| self.identities.get(&entry.path).copied() == Some(identity))
    }

    pub fn can_go_back(&self) -> bool {
        !self.history.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward_history.is_empty()
    }

    pub fn can_go_up(&self) -> bool {
        self.current.parent().is_some()
    }

    pub fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    pub fn enter(&mut self, path: impl Into<PathBuf>) -> io::Result<()> {
        let next = path.into();
        let (entries, identities) = read_entries_with_identities(&next, self.show_hidden)?;
        self.history.push(self.current.clone());
        self.forward_history.clear();
        self.current = next;
        self.entries = entries;
        self.identities = identities;
        Ok(())
    }

    pub fn back(&mut self) -> io::Result<bool> {
        let Some(previous) = self.history.last().cloned() else {
            return Ok(false);
        };
        let (entries, identities) = read_entries_with_identities(&previous, self.show_hidden)?;
        self.history.pop();
        self.forward_history.push(self.current.clone());
        self.current = previous;
        self.entries = entries;
        self.identities = identities;
        Ok(true)
    }

    pub fn forward(&mut self) -> io::Result<bool> {
        let Some(next) = self.forward_history.last().cloned() else {
            return Ok(false);
        };
        let (entries, identities) = read_entries_with_identities(&next, self.show_hidden)?;
        self.forward_history.pop();
        self.history.push(self.current.clone());
        self.current = next;
        self.entries = entries;
        self.identities = identities;
        Ok(true)
    }

    pub fn up(&mut self) -> io::Result<bool> {
        let Some(parent) = self.current.parent().map(Path::to_path_buf) else {
            return Ok(false);
        };
        self.enter(parent)?;
        Ok(true)
    }

    pub fn refresh(&mut self) -> io::Result<()> {
        (self.entries, self.identities) =
            read_entries_with_identities(&self.current, self.show_hidden)?;
        Ok(())
    }

    pub fn set_show_hidden(&mut self, show_hidden: bool) -> io::Result<()> {
        self.show_hidden = show_hidden;
        (self.entries, self.identities) =
            read_entries_with_identities(&self.current, self.show_hidden)?;
        Ok(())
    }

    pub fn sort(&mut self, key: EntrySortKey, direction: SortDirection) {
        self.entries.sort_by(|left, right| {
            right.is_directory.cmp(&left.is_directory).then_with(|| {
                let order = match key {
                    EntrySortKey::Name => compare_names(&left.name, &right.name),
                    EntrySortKey::Type => entry_type(left)
                        .cmp(&entry_type(right))
                        .then_with(|| compare_names(&left.name, &right.name)),
                    EntrySortKey::Modified => left
                        .modified
                        .cmp(&right.modified)
                        .then_with(|| compare_names(&left.name, &right.name)),
                    EntrySortKey::Size => left
                        .size
                        .cmp(&right.size)
                        .then_with(|| compare_names(&left.name, &right.name)),
                };
                match direction {
                    SortDirection::Ascending => order,
                    SortDirection::Descending => order.reverse(),
                }
            })
        });
    }
}

fn entry_type(entry: &FileEntry) -> String {
    if entry.is_directory {
        String::new()
    } else {
        entry
            .path
            .extension()
            .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default()
    }
}

pub fn read_entries(path: &Path) -> io::Result<Vec<FileEntry>> {
    read_entries_with_hidden(path, true)
}

fn read_entries_with_hidden(path: &Path, show_hidden: bool) -> io::Result<Vec<FileEntry>> {
    read_entries_with_identities(path, show_hidden).map(|(entries, _)| entries)
}

fn read_entries_with_identities(
    path: &Path,
    show_hidden: bool,
) -> io::Result<(Vec<FileEntry>, HashMap<PathBuf, FileIdentity>)> {
    let hidden_names = hidden_names(path);
    let mut identities = HashMap::new();
    let mut entries = fs::read_dir(path)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let metadata = entry.metadata().ok()?;
            if !show_hidden && entry_is_hidden(&entry, &metadata, &hidden_names) {
                return None;
            }
            let is_directory = metadata_is_directory(&metadata);
            let path = entry.path();
            let display_name_override = (!is_directory)
                .then(|| nickel_platform::path_display_name(&path))
                .flatten();
            if let Some(identity) = metadata_identity(&path, &metadata) {
                identities.insert(path.clone(), identity);
            }
            Some(FileEntry {
                name: entry.file_name(),
                display_name_override,
                path,
                is_directory,
                size: (!is_directory && metadata.is_file()).then_some(metadata.len()),
                modified: metadata.modified().ok(),
            })
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| compare_names(&left.name, &right.name))
    });
    Ok((entries, identities))
}

#[cfg(unix)]
pub(crate) fn metadata_identity(_path: &Path, metadata: &fs::Metadata) -> Option<FileIdentity> {
    use std::os::unix::fs::MetadataExt;
    Some(FileIdentity(metadata.dev(), metadata.ino()))
}

#[cfg(target_os = "windows")]
pub(crate) fn metadata_identity(path: &Path, _metadata: &fs::Metadata) -> Option<FileIdentity> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::{
            Foundation::CloseHandle,
            Storage::FileSystem::{
                BY_HANDLE_FILE_INFORMATION, CreateFileW, FILE_FLAG_BACKUP_SEMANTICS,
                FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE, GetFileInformationByHandle,
                OPEN_EXISTING,
            },
        },
        core::PCWSTR,
    };

    let wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: `wide` is a live, NUL-terminated UTF-16 path; no security or template pointers are
    // supplied. The returned owned kernel handle is closed below on every subsequent path.
    let handle = unsafe {
        CreateFileW(
            PCWSTR(wide.as_ptr()),
            0,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            None,
            OPEN_EXISTING,
            FILE_FLAG_BACKUP_SEMANTICS,
            None,
        )
        .ok()?
    };
    let mut information = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `handle` is valid and `information` points to writable storage of the required type.
    let result = unsafe { GetFileInformationByHandle(handle, &mut information) };
    // SAFETY: `handle` was returned by `CreateFileW` and is closed exactly once here.
    let _ = unsafe { CloseHandle(handle) };
    result.ok()?;
    let index =
        (u64::from(information.nFileIndexHigh) << 32) | u64::from(information.nFileIndexLow);
    Some(FileIdentity(
        u64::from(information.dwVolumeSerialNumber),
        index,
    ))
}

#[cfg(not(any(unix, target_os = "windows")))]
fn metadata_identity(_path: &Path, _metadata: &fs::Metadata) -> Option<FileIdentity> {
    None
}

fn hidden_names(path: &Path) -> HashSet<OsString> {
    fs::read_to_string(path.join(".hidden"))
        .ok()
        .into_iter()
        .flat_map(|contents| {
            contents
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty() && !line.starts_with('#'))
                .map(OsString::from)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn entry_is_hidden(
    entry: &fs::DirEntry,
    metadata: &fs::Metadata,
    hidden_names: &HashSet<OsString>,
) -> bool {
    let name = entry.file_name();
    name.to_string_lossy().starts_with('.')
        || hidden_names.contains(&name)
        || metadata_has_hidden_attribute(metadata)
}

#[cfg(target_os = "windows")]
fn metadata_has_hidden_attribute(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    metadata.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

#[cfg(not(target_os = "windows"))]
fn metadata_has_hidden_attribute(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(target_os = "windows")]
fn metadata_is_directory(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_DIRECTORY: u32 = 0x10;
    metadata.is_dir() || metadata.file_attributes() & FILE_ATTRIBUTE_DIRECTORY != 0
}

#[cfg(not(target_os = "windows"))]
fn metadata_is_directory(metadata: &fs::Metadata) -> bool {
    metadata.is_dir()
}

fn compare_names(left: &OsString, right: &OsString) -> Ordering {
    left.to_string_lossy()
        .to_lowercase()
        .cmp(&right.to_string_lossy().to_lowercase())
        .then_with(|| left.cmp(right))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use super::{
        DirectoryBrowser, EntrySortKey, FileEntry, OpenTargetKind, SortDirection, open_target_kind,
    };

    fn temporary_directory(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!(
            "nickel-file-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn directories_sort_before_files_case_insensitively() {
        let root = temporary_directory("ordering");
        fs::write(root.join("beta.txt"), b"beta").unwrap();
        fs::write(root.join("Alpha.txt"), b"alpha").unwrap();
        fs::create_dir(root.join("zeta")).unwrap();
        fs::create_dir(root.join("Gamma")).unwrap();

        let browser = DirectoryBrowser::open(&root).unwrap();
        assert!(!browser.can_go_back());
        assert!(!browser.can_go_forward());
        let names = browser
            .entries()
            .iter()
            .map(|entry| entry.display_name())
            .collect::<Vec<_>>();

        assert_eq!(names, ["Gamma", "zeta", "Alpha.txt", "beta.txt"]);
        fs::remove_dir_all(root).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn directory_entries_cache_desktop_launcher_name_instead_of_opaque_filename() {
        let root = temporary_directory("desktop-launcher-name");
        fs::write(
            root.join("chrome-opaque-Default.desktop"),
            b"[Desktop Entry]\nType=Application\nName=GitLab\nExec=true\n",
        )
        .unwrap();

        let browser = DirectoryBrowser::open(&root).unwrap();
        assert_eq!(browser.entries()[0].name, "chrome-opaque-Default.desktop");
        assert_eq!(browser.entries()[0].display_name(), "GitLab");

        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn application_launchers_do_not_use_document_associations() {
        #[cfg(target_os = "linux")]
        assert_eq!(
            open_target_kind(std::path::Path::new("opaque.desktop")),
            OpenTargetKind::Launcher
        );
        assert_eq!(
            open_target_kind(std::path::Path::new("notes.txt")),
            OpenTargetKind::Document
        );
    }

    #[test]
    fn portable_sorting_keeps_directories_first_in_both_directions() {
        let entry = |name: &str, directory: bool, size| FileEntry {
            display_name_override: None,
            name: name.into(),
            path: std::path::PathBuf::from("/fixture").join(name),
            is_directory: directory,
            size,
            modified: None,
        };
        let mut browser = DirectoryBrowser::fixture(vec![
            entry("small.txt", false, Some(2)),
            entry("folder", true, None),
            entry("large.bin", false, Some(90)),
        ]);

        browser.sort(EntrySortKey::Size, SortDirection::Ascending);
        assert_eq!(
            browser
                .entries()
                .iter()
                .map(FileEntry::display_name)
                .collect::<Vec<_>>(),
            ["folder", "small.txt", "large.bin"]
        );
        browser.sort(EntrySortKey::Size, SortDirection::Descending);
        assert_eq!(
            browser
                .entries()
                .iter()
                .map(FileEntry::display_name)
                .collect::<Vec<_>>(),
            ["folder", "large.bin", "small.txt"]
        );
    }

    #[test]
    fn back_returns_to_the_previous_directory() {
        let root = temporary_directory("history");
        let child = root.join("child");
        fs::create_dir(&child).unwrap();
        let mut browser = DirectoryBrowser::open(&root).unwrap();

        browser.enter(&child).unwrap();
        assert_eq!(browser.current(), child);
        assert!(browser.can_go_back());
        assert!(!browser.can_go_forward());
        assert!(browser.back().unwrap());
        assert_eq!(browser.current(), root);
        assert!(!browser.can_go_back());
        assert!(browser.can_go_forward());
        assert!(!browser.back().unwrap());
        assert_eq!(browser.current(), root);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn forward_returns_to_a_directory_after_back() {
        let root = temporary_directory("forward-history");
        let child = root.join("child");
        fs::create_dir(&child).unwrap();
        let mut browser = DirectoryBrowser::open(&root).unwrap();

        browser.enter(&child).unwrap();
        browser.back().unwrap();
        assert!(browser.can_go_forward());
        assert!(browser.forward().unwrap());
        assert_eq!(browser.current(), child);
        assert!(browser.can_go_back());
        assert!(!browser.can_go_forward());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn hidden_policy_covers_dotfiles_and_hidden_manifest() {
        let root = temporary_directory("hidden");
        fs::write(root.join(".secret"), b"dotfile").unwrap();
        fs::write(root.join("listed.txt"), b"listed").unwrap();
        fs::write(root.join("visible.txt"), b"visible").unwrap();
        fs::write(root.join(".hidden"), "listed.txt\n").unwrap();

        let hidden = DirectoryBrowser::open_with_hidden(&root, false).unwrap();
        let hidden_names = hidden
            .entries()
            .iter()
            .map(|entry| entry.display_name())
            .collect::<Vec<_>>();
        assert_eq!(hidden_names, ["visible.txt"]);

        let shown = DirectoryBrowser::open_with_hidden(&root, true).unwrap();
        let shown_names = shown
            .entries()
            .iter()
            .map(|entry| entry.display_name())
            .collect::<Vec<_>>();
        assert_eq!(
            shown_names,
            [".hidden", ".secret", "listed.txt", "visible.txt"]
        );
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn refresh_tracks_external_create_rename_and_remove_by_stable_identity() {
        let root = temporary_directory("live-reconcile");
        let original = root.join("original.txt");
        let renamed = root.join("renamed.txt");
        fs::write(&original, b"content").unwrap();
        let mut browser = DirectoryBrowser::open(&root).unwrap();
        let identity = browser.identity_at(0).expect("native stable identity");

        fs::write(root.join("created.txt"), b"new").unwrap();
        fs::rename(&original, &renamed).unwrap();
        browser.refresh().unwrap();
        assert_eq!(
            browser
                .entries()
                .iter()
                .map(FileEntry::display_name)
                .collect::<Vec<_>>(),
            ["created.txt", "renamed.txt"]
        );
        let renamed_index = browser.index_of_identity(identity).unwrap();
        assert_eq!(browser.entries()[renamed_index].path, renamed);

        fs::remove_file(&renamed).unwrap();
        browser.refresh().unwrap();
        assert!(browser.index_of_identity(identity).is_none());
        assert_eq!(browser.entries().len(), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn external_drop_copy_preserves_directories_and_resolves_collisions() {
        let root = temporary_directory("drop-copy");
        let source = root.join("source");
        let destination = root.join("desktop");
        fs::create_dir(&source).unwrap();
        fs::create_dir(&destination).unwrap();
        fs::write(source.join("nested.txt"), b"contents").unwrap();
        let first = super::copy_into(&source, &destination).unwrap();
        let second = super::copy_into(&source, &destination).unwrap();
        assert_eq!(fs::read(first.join("nested.txt")).unwrap(), b"contents");
        assert_eq!(fs::read(second.join("nested.txt")).unwrap(), b"contents");
        assert_ne!(first, second);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn desktop_launcher_detection_covers_native_platform_shortcuts() {
        assert!(super::is_application_launcher(std::path::Path::new(
            "Editor.desktop"
        )));
        assert!(super::is_application_launcher(std::path::Path::new(
            "Editor.LNK"
        )));
        assert!(super::is_application_launcher(std::path::Path::new(
            "Editor.app"
        )));
        assert!(!super::is_application_launcher(std::path::Path::new(
            "notes.txt"
        )));
    }

    #[test]
    fn shared_new_folder_operation_uses_conventional_collision_names() {
        let root = temporary_directory("new-folder");
        let first = super::create_new_folder(&root).unwrap();
        let second = super::create_new_folder(&root).unwrap();
        assert_eq!(first.file_name().unwrap(), "New folder");
        assert_eq!(second.file_name().unwrap(), "New folder (1)");
        fs::remove_dir_all(root).unwrap();
    }
}
