//! Operating-system-owned default application associations.
//!
//! This module is deliberately the only place where Nickel applications deal
//! with MIME databases, Windows consent UI, or Launch Services limitations.

use std::{
    collections::{HashSet, VecDeque},
    fmt,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum AssociationTarget {
    Extension(String),
    Mime(String),
    Scheme(String),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum AssociationFamily {
    Web,
    Documents,
    Images,
    Audio,
    Video,
    Archives,
    OtherFiles,
    Protocols,
}

impl AssociationFamily {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Web => "Web",
            Self::Documents => "Documents",
            Self::Images => "Images",
            Self::Audio => "Audio",
            Self::Video => "Video",
            Self::Archives => "Archives",
            Self::OtherFiles => "Other files",
            Self::Protocols => "Protocols",
        }
    }
}

impl AssociationTarget {
    pub fn extension(value: impl Into<String>) -> Self {
        Self::Extension(value.into())
    }

    pub fn mime(value: impl Into<String>) -> Self {
        Self::Mime(value.into())
    }

    pub fn scheme(value: impl Into<String>) -> Self {
        Self::Scheme(value.into())
    }

    pub fn platform_key(&self) -> String {
        match self {
            Self::Extension(value) => value.clone(),
            Self::Mime(value) => value.clone(),
            Self::Scheme(value) => format!("x-scheme-handler/{value}"),
        }
    }

    /// Stable, portable presentation grouping. The underlying association remains an exact
    /// platform target; grouping never aliases settings that the OS may configure independently.
    pub fn family(&self) -> AssociationFamily {
        match self {
            Self::Scheme(value) if matches!(value.as_str(), "http" | "https") => {
                AssociationFamily::Web
            }
            Self::Scheme(_) => AssociationFamily::Protocols,
            Self::Mime(value) if value.starts_with("image/") => AssociationFamily::Images,
            Self::Mime(value) if value.starts_with("audio/") => AssociationFamily::Audio,
            Self::Mime(value) if value.starts_with("video/") => AssociationFamily::Video,
            Self::Mime(value)
                if value.starts_with("text/")
                    || value == "application/pdf"
                    || value.contains("document")
                    || value.contains("presentation")
                    || value.contains("spreadsheet")
                    || value.contains("epub") =>
            {
                AssociationFamily::Documents
            }
            Self::Mime(value)
                if value.contains("zip")
                    || value.contains("archive")
                    || value.contains("compressed")
                    || value.contains("tar") =>
            {
                AssociationFamily::Archives
            }
            Self::Extension(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    ".svg" | ".png" | ".jpg" | ".jpeg" | ".gif" | ".webp" | ".avif"
                ) =>
            {
                AssociationFamily::Images
            }
            Self::Extension(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    ".mp3" | ".wav" | ".flac" | ".ogg" | ".m4a"
                ) =>
            {
                AssociationFamily::Audio
            }
            Self::Extension(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    ".mp4" | ".mkv" | ".webm" | ".avi" | ".mov" | ".mpeg" | ".mpg"
                ) =>
            {
                AssociationFamily::Video
            }
            Self::Extension(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    ".txt"
                        | ".md"
                        | ".pdf"
                        | ".doc"
                        | ".docx"
                        | ".odt"
                        | ".rtf"
                        | ".xls"
                        | ".xlsx"
                        | ".ods"
                        | ".ppt"
                        | ".pptx"
                        | ".odp"
                        | ".epub"
                ) =>
            {
                AssociationFamily::Documents
            }
            Self::Extension(value)
                if matches!(
                    value.to_ascii_lowercase().as_str(),
                    ".zip" | ".tar" | ".gz" | ".bz2" | ".xz" | ".7z" | ".rar"
                ) =>
            {
                AssociationFamily::Archives
            }
            Self::Extension(_) | Self::Mime(_) => AssociationFamily::OtherFiles,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssociationCapability {
    DirectUserChange,
    NativeConsent,
    ReadOnly,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AssociationScope {
    User,
    System,
    Policy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApplicationHandler {
    /// Stable platform identity (`.desktop` id, registered application id, or bundle id).
    pub id: String,
    pub name: String,
    pub icon: Option<String>,
    pub source: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssociationSnapshot {
    pub target: AssociationTarget,
    pub effective: Option<ApplicationHandler>,
    pub handlers: Vec<ApplicationHandler>,
    pub capability: AssociationCapability,
    pub scope: AssociationScope,
    pub detail: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ChangeOutcome {
    Confirmed(AssociationSnapshot),
    NativeConsentRequired { detail: String },
    Rejected { detail: String },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssociationError(pub String);

impl fmt::Display for AssociationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for AssociationError {}

pub trait AssociationBackend: Send + Sync {
    fn available_targets(&self) -> Result<Vec<AssociationTarget>, AssociationError> {
        Ok(Vec::new())
    }

    fn inspect(&self, target: &AssociationTarget) -> Result<AssociationSnapshot, AssociationError>;
    fn request_change(
        &self,
        target: &AssociationTarget,
        handler_id: &str,
    ) -> Result<ChangeOutcome, AssociationError>;
}

const ASSOCIATION_CACHE_CAPACITY: usize = 256;

/// Process-wide association authority shared by Settings and file dialogs.
/// Every inspection is re-resolved through the OS; the bounded cache exists
/// only to assign a confirmed generation and never competes with OS state.
pub struct AssociationService {
    backend: Box<dyn AssociationBackend>,
    state: Mutex<AssociationServiceState>,
}

#[derive(Default)]
struct AssociationServiceState {
    generation: u64,
    projections: VecDeque<(AssociationTarget, AssociationSnapshot)>,
}

impl AssociationService {
    fn new(backend: Box<dyn AssociationBackend>) -> Self {
        Self {
            backend,
            state: Mutex::new(AssociationServiceState::default()),
        }
    }

    pub fn inspect(
        &self,
        target: &AssociationTarget,
    ) -> Result<AssociationSnapshot, AssociationError> {
        let snapshot = self.backend.inspect(target)?;
        let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
        let changed = state
            .projections
            .iter()
            .find(|(cached, _)| cached == target)
            .is_none_or(|(_, cached)| cached != &snapshot);
        if changed {
            state.generation = state.generation.saturating_add(1);
            state.projections.retain(|(cached, _)| cached != target);
            state
                .projections
                .push_back((target.clone(), snapshot.clone()));
            while state.projections.len() > ASSOCIATION_CACHE_CAPACITY {
                state.projections.pop_front();
            }
        }
        Ok(snapshot)
    }

    pub fn available_targets(&self) -> Result<Vec<AssociationTarget>, AssociationError> {
        self.backend.available_targets()
    }

    pub fn request_change(
        &self,
        target: &AssociationTarget,
        handler_id: &str,
    ) -> Result<ChangeOutcome, AssociationError> {
        change_and_verify(self.backend.as_ref(), target, handler_id).map(|outcome| {
            if matches!(outcome, ChangeOutcome::Confirmed(_)) {
                let mut state = self.state.lock().unwrap_or_else(|error| error.into_inner());
                state.projections.retain(|(cached, _)| cached != target);
            }
            outcome
        })
    }

    pub fn generation(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .generation
    }
}

pub fn association_service() -> Arc<AssociationService> {
    static SERVICE: OnceLock<Arc<AssociationService>> = OnceLock::new();
    Arc::clone(SERVICE.get_or_init(|| Arc::new(AssociationService::new(association_backend()))))
}

/// Re-query after every request. A setter returning success is not evidence
/// that policy, registration, or an external settings application accepted it.
pub fn change_and_verify(
    backend: &dyn AssociationBackend,
    target: &AssociationTarget,
    handler_id: &str,
) -> Result<ChangeOutcome, AssociationError> {
    match backend.request_change(target, handler_id)? {
        ChangeOutcome::Confirmed(_) => {
            let confirmed = backend.inspect(target)?;
            if confirmed
                .effective
                .as_ref()
                .map(|handler| handler.id.as_str())
                == Some(handler_id)
            {
                Ok(ChangeOutcome::Confirmed(confirmed))
            } else {
                Ok(ChangeOutcome::Rejected {
                    detail: "the platform did not confirm the requested default".into(),
                })
            }
        }
        outcome => Ok(outcome),
    }
}

pub fn association_backend() -> Box<dyn AssociationBackend> {
    #[cfg(target_os = "linux")]
    return Box::new(LinuxAssociations);
    #[cfg(target_os = "windows")]
    return Box::new(WindowsAssociations);
    #[allow(unreachable_code)]
    Box::new(UnsupportedAssociations)
}

/// Resolves the operating-system type authority for a concrete file.
pub fn association_target_for_file(path: &Path) -> Result<AssociationTarget, AssociationError> {
    #[cfg(target_os = "linux")]
    {
        let output = std::process::Command::new("xdg-mime")
            .arg("query")
            .arg("filetype")
            .arg(path)
            .output()
            .map_err(|error| AssociationError(format!("could not run xdg-mime: {error}")))?;
        if output.status.success() {
            let mime = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if !mime.is_empty() {
                return Ok(AssociationTarget::mime(mime));
            }
        }
    }
    infer_portable_mime(path)
        .map(AssociationTarget::mime)
        .ok_or_else(|| {
            AssociationError("the operating system could not resolve this file type".into())
        })
}

/// Opens the Nickel Settings surface backed by this same association service.
pub fn open_default_application_settings() -> Result<(), AssociationError> {
    let current = std::env::current_exe()
        .map_err(|error| AssociationError(format!("could not locate Nickel Settings: {error}")))?;
    let executable = current.with_file_name(if cfg!(target_os = "windows") {
        "nickel-settings.exe"
    } else {
        "nickel-settings"
    });
    std::process::Command::new(&executable)
        .args(["--screen", "default-apps"])
        .spawn()
        .map(|_| ())
        .map_err(|error| {
            AssociationError(format!("could not open {}: {error}", executable.display()))
        })
}

fn infer_portable_mime(path: &Path) -> Option<&'static str> {
    match path
        .extension()?
        .to_string_lossy()
        .to_ascii_lowercase()
        .as_str()
    {
        "txt" | "log" => Some("text/plain"),
        "md" => Some("text/markdown"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "pdf" => Some("application/pdf"),
        "mp3" => Some("audio/mpeg"),
        "mp4" => Some("video/mp4"),
        _ => None,
    }
}

/// Opens exactly one file with a selected handler without changing its default.
pub fn open_once_with(path: &Path, handler: &ApplicationHandler) -> Result<(), AssociationError> {
    #[cfg(target_os = "linux")]
    {
        let status = std::process::Command::new("gtk-launch")
            .arg(handler.id.trim_end_matches(".desktop"))
            .arg(path)
            .status()
            .map_err(|error| {
                AssociationError(format!("could not launch {}: {error}", handler.name))
            })?;
        status
            .success()
            .then_some(())
            .ok_or_else(|| AssociationError(format!("{} exited with {status}", handler.name)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (path, handler);
        Err(AssociationError(
            "open once with a chosen handler is unavailable on this platform build".into(),
        ))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DefaultLaunchError {
    AssociationMissing,
    PermissionDenied,
    TargetMissing,
    Platform(String),
}

impl fmt::Display for DefaultLaunchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AssociationMissing => formatter.write_str("no default application is available"),
            Self::PermissionDenied => formatter.write_str("permission was denied"),
            Self::TargetMissing => formatter.write_str("the target no longer exists"),
            Self::Platform(error) => formatter.write_str(error),
        }
    }
}

/// Opens a validated filesystem target through the operating system's default
/// association authority. Portable applications receive only typed outcomes.
pub fn open_with_default(path: &Path) -> Result<(), DefaultLaunchError> {
    if !path.exists() {
        return Err(DefaultLaunchError::TargetMissing);
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::{
            Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
            core::PCWSTR,
        };
        let path = path
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect::<Vec<_>>();
        let verb = "open\0".encode_utf16().collect::<Vec<_>>();
        // SAFETY: both strings are terminated and live for this synchronous call.
        let result = unsafe {
            ShellExecuteW(
                None,
                PCWSTR(verb.as_ptr()),
                PCWSTR(path.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        match result.0 as isize {
            code if code > 32 => Ok(()),
            2 | 3 => Err(DefaultLaunchError::TargetMissing),
            5 => Err(DefaultLaunchError::PermissionDenied),
            31 => Err(DefaultLaunchError::AssociationMissing),
            code => Err(DefaultLaunchError::Platform(format!(
                "Windows shell error {code}"
            ))),
        }
    }
    #[cfg(target_os = "linux")]
    {
        let program = "xdg-open";
        let status = std::process::Command::new(program)
            .arg(path)
            .status()
            .map_err(|error| match error.kind() {
                std::io::ErrorKind::NotFound => DefaultLaunchError::AssociationMissing,
                std::io::ErrorKind::PermissionDenied => DefaultLaunchError::PermissionDenied,
                _ => DefaultLaunchError::Platform(error.to_string()),
            })?;
        if status.success() {
            Ok(())
        } else if status.code() == Some(3) {
            Err(DefaultLaunchError::AssociationMissing)
        } else {
            Err(DefaultLaunchError::Platform(format!(
                "{program} exited with {status}"
            )))
        }
    }
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    {
        Err(DefaultLaunchError::Platform(
            "default application launch is unavailable on this platform".into(),
        ))
    }
}

pub const fn open_once_supported() -> bool {
    cfg!(target_os = "linux")
}

#[cfg(target_os = "linux")]
struct LinuxAssociations;

#[cfg(target_os = "linux")]
impl LinuxAssociations {
    fn available_targets() -> Vec<AssociationTarget> {
        let mut keys = HashSet::new();
        for root in desktop_data_roots() {
            let applications = root.join("applications");
            extend_association_keys_from_cache(&applications.join("mimeinfo.cache"), &mut keys);
            extend_association_keys_from_mimeapps(&applications.join("mimeapps.list"), &mut keys);
            let Ok(entries) = std::fs::read_dir(&applications) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|extension| extension.to_str()) != Some("desktop") {
                    continue;
                }
                let Ok(contents) = std::fs::read_to_string(path) else {
                    continue;
                };
                keys.extend(
                    contents
                        .lines()
                        .filter_map(|line| line.strip_prefix("MimeType="))
                        .flat_map(|types| types.split(';'))
                        .filter(|kind| !kind.is_empty())
                        .map(str::to_owned),
                );
            }
        }
        for root in desktop_config_roots() {
            extend_association_keys_from_mimeapps(&root.join("mimeapps.list"), &mut keys);
        }
        let mut targets = keys
            .into_iter()
            .map(|key| {
                key.strip_prefix("x-scheme-handler/")
                    .map(AssociationTarget::scheme)
                    .unwrap_or_else(|| AssociationTarget::mime(key))
            })
            .collect::<Vec<_>>();
        targets.sort_by_key(AssociationTarget::platform_key);
        targets
    }

    fn query(target: &AssociationTarget) -> Result<Option<String>, AssociationError> {
        let output = std::process::Command::new("xdg-mime")
            .args(["query", "default", &target.platform_key()])
            .output()
            .map_err(|error| AssociationError(format!("could not run xdg-mime: {error}")))?;
        if !output.status.success() {
            return Err(AssociationError(format!(
                "xdg-mime query failed with {}",
                output.status
            )));
        }
        let id = String::from_utf8_lossy(&output.stdout).trim().to_owned();
        Ok((!id.is_empty()).then_some(id))
    }

    fn desktop_name(
        id: &str,
        entries: &[freedesktop_desktop_entry::DesktopEntry],
        locales: &[String],
    ) -> String {
        entries
            .iter()
            .find(|entry| entry.id() == id.trim_end_matches(".desktop"))
            .and_then(|entry| entry.name(locales))
            .filter(|name| !name.trim().is_empty())
            .map(|name| name.into_owned())
            .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned())
    }

    fn handlers(
        target: &AssociationTarget,
        entries: &[freedesktop_desktop_entry::DesktopEntry],
        locales: &[String],
    ) -> Vec<ApplicationHandler> {
        let key = target.platform_key();
        let mut handlers = Vec::new();
        for entry in entries {
            let id = format!("{}.desktop", entry.id());
            if handlers
                .iter()
                .any(|item: &ApplicationHandler| item.id == id)
            {
                continue;
            }
            let supports = entry
                .mime_type()
                .is_some_and(|types| types.into_iter().any(|kind| kind == key));
            // NoDisplay suppresses launcher/menu presentation; it does not unregister the
            // application as a compatible association handler. Hidden, however, is the
            // freedesktop deletion/override marker and must win over lower-precedence entries.
            if supports && !entry.hidden() {
                handlers.push(ApplicationHandler {
                    name: entry
                        .name(locales)
                        .filter(|name| !name.trim().is_empty())
                        .map_or_else(|| entry.id().to_owned(), |name| name.into_owned()),
                    id,
                    icon: entry.icon().map(str::to_owned),
                    source: entry.path.display().to_string(),
                });
            }
        }
        handlers.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        handlers
    }
}

#[cfg(target_os = "linux")]
fn extend_association_keys_from_cache(path: &Path, keys: &mut HashSet<String>) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    let mut in_cache = false;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') {
            in_cache = line == "[MIME Cache]";
        } else if in_cache
            && let Some((key, _)) = line.split_once('=')
            && !key.is_empty()
        {
            keys.insert(key.to_owned());
        }
    }
}

#[cfg(target_os = "linux")]
fn extend_association_keys_from_mimeapps(path: &Path, keys: &mut HashSet<String>) {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return;
    };
    let mut in_associations = false;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') {
            in_associations = matches!(
                line,
                "[Default Applications]" | "[Added Associations]" | "[Removed Associations]"
            );
        } else if in_associations
            && let Some((key, _)) = line.split_once('=')
            && !key.is_empty()
        {
            keys.insert(key.to_owned());
        }
    }
}

#[cfg(target_os = "linux")]
impl AssociationBackend for LinuxAssociations {
    fn available_targets(&self) -> Result<Vec<AssociationTarget>, AssociationError> {
        Ok(Self::available_targets())
    }

    fn inspect(&self, target: &AssociationTarget) -> Result<AssociationSnapshot, AssociationError> {
        let locales = freedesktop_desktop_entry::get_languages_from_env();
        let entries = linux_desktop_entries(&locales);
        let effective = Self::query(target)?.map(|id| ApplicationHandler {
            name: Self::desktop_name(&id, &entries, &locales),
            id,
            icon: None,
            source: "freedesktop MIME default".into(),
        });
        let mut handlers = Self::handlers(target, &entries, &locales);
        if let Some(current) = effective.as_ref()
            && !handlers.iter().any(|handler| handler.id == current.id)
        {
            handlers.insert(0, current.clone());
        }
        Ok(AssociationSnapshot {
            target: target.clone(),
            effective,
            handlers,
            capability: AssociationCapability::DirectUserChange,
            scope: AssociationScope::User,
            detail: "User-level freedesktop association".into(),
        })
    }

    fn request_change(
        &self,
        target: &AssociationTarget,
        handler_id: &str,
    ) -> Result<ChangeOutcome, AssociationError> {
        if handler_id.is_empty() || !handler_id.ends_with(".desktop") {
            return Ok(ChangeOutcome::Rejected {
                detail: "the selected application is not a desktop-entry identity".into(),
            });
        }
        let locales = freedesktop_desktop_entry::get_languages_from_env();
        if !linux_desktop_entries(&locales)
            .iter()
            .any(|entry| format!("{}.desktop", entry.id()) == handler_id)
        {
            return Ok(ChangeOutcome::Rejected {
                detail: "the selected application is no longer installed".into(),
            });
        }
        let status = std::process::Command::new("xdg-mime")
            .args(["default", handler_id, &target.platform_key()])
            .status()
            .map_err(|error| AssociationError(format!("could not run xdg-mime: {error}")))?;
        if !status.success() {
            return Ok(ChangeOutcome::Rejected {
                detail: format!("xdg-mime rejected the change with {status}"),
            });
        }
        Ok(ChangeOutcome::Confirmed(self.inspect(target)?))
    }
}

#[cfg(target_os = "linux")]
fn linux_desktop_entries(locales: &[String]) -> Vec<freedesktop_desktop_entry::DesktopEntry> {
    freedesktop_desktop_entry::Iter::new(
        desktop_data_roots()
            .into_iter()
            .map(|root| root.join("applications")),
    )
    .entries(Some(locales))
    .collect()
}

#[cfg(target_os = "linux")]
fn desktop_data_roots() -> Vec<std::path::PathBuf> {
    let mut roots = std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .into_iter()
        .collect::<Vec<_>>();
    if roots.is_empty()
        && let Some(home) = std::env::var_os("HOME")
    {
        roots.push(std::path::PathBuf::from(home).join(".local/share"));
    }
    roots.extend(
        std::env::var_os("XDG_DATA_DIRS")
            .unwrap_or_else(|| "/usr/local/share:/usr/share".into())
            .to_string_lossy()
            .split(':')
            .filter(|root| !root.is_empty())
            .map(std::path::PathBuf::from),
    );
    roots
}

#[cfg(target_os = "linux")]
fn desktop_config_roots() -> Vec<std::path::PathBuf> {
    let mut roots = std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .into_iter()
        .collect::<Vec<_>>();
    if roots.is_empty()
        && let Some(home) = std::env::var_os("HOME")
    {
        roots.push(std::path::PathBuf::from(home).join(".config"));
    }
    roots.extend(
        std::env::var_os("XDG_CONFIG_DIRS")
            .unwrap_or_else(|| "/etc/xdg".into())
            .to_string_lossy()
            .split(':')
            .filter(|root| !root.is_empty())
            .map(std::path::PathBuf::from),
    );
    roots
}

#[cfg(target_os = "windows")]
struct WindowsAssociations;

#[cfg(target_os = "windows")]
struct RegistryKey(windows::Win32::System::Registry::HKEY);

#[cfg(target_os = "windows")]
impl Drop for RegistryKey {
    fn drop(&mut self) {
        // SAFETY: this wrapper is constructed only from a successful RegOpenKeyExW call
        // and owns exactly one corresponding close.
        unsafe {
            let _ = windows::Win32::System::Registry::RegCloseKey(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_open_registry_key(
    root: windows::Win32::System::Registry::HKEY,
    path: &str,
) -> Option<RegistryKey> {
    use windows::{Win32::System::Registry::*, core::PCWSTR};
    let path = path.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut key = HKEY::default();
    // SAFETY: path is a valid NUL-terminated UTF-16 string and key points to writable storage.
    let status = unsafe { RegOpenKeyExW(root, PCWSTR(path.as_ptr()), None, KEY_READ, &mut key) };
    (status == windows::Win32::Foundation::ERROR_SUCCESS).then_some(RegistryKey(key))
}

#[cfg(target_os = "windows")]
fn windows_registry_string(key: &RegistryKey, name: &str) -> Option<String> {
    use windows::{Win32::System::Registry::*, core::PCWSTR};
    let name = name.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut value_type = REG_VALUE_TYPE::default();
    let mut bytes = 0_u32;
    // SAFETY: the value name is NUL terminated and the sizing call supplies no data buffer.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            None,
            Some(&mut bytes),
        )
    };
    if status != windows::Win32::Foundation::ERROR_SUCCESS
        || !matches!(value_type, REG_SZ | REG_EXPAND_SZ)
        || bytes == 0
        || bytes > 64 * 1024
    {
        return None;
    }
    let mut data = vec![0_u16; bytes.div_ceil(2) as usize];
    // SAFETY: data contains at least the byte count returned by the sizing call.
    let status = unsafe {
        RegQueryValueExW(
            key.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            Some(data.as_mut_ptr().cast()),
            Some(&mut bytes),
        )
    };
    if status != windows::Win32::Foundation::ERROR_SUCCESS {
        return None;
    }
    let units = (bytes as usize / 2).min(data.len());
    let end = data[..units]
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(units);
    Some(String::from_utf16_lossy(&data[..end]))
}

#[cfg(target_os = "windows")]
fn windows_registry_values(key: &RegistryKey) -> Vec<(String, String)> {
    use windows::{Win32::System::Registry::*, core::PWSTR};
    let mut values = Vec::new();
    for index in 0..u32::MAX {
        let mut name = vec![0_u16; 256];
        let mut data = vec![0_u8; 1024];
        loop {
            let mut name_len = name.len() as u32;
            let mut data_len = data.len() as u32;
            let mut value_type = REG_VALUE_TYPE::default();
            // SAFETY: both buffers report their writable lengths and remain alive for the call.
            let status = unsafe {
                RegEnumValueW(
                    key.0,
                    index,
                    Some(PWSTR(name.as_mut_ptr())),
                    &mut name_len,
                    None,
                    Some(&mut value_type.0),
                    Some(data.as_mut_ptr()),
                    Some(&mut data_len),
                )
            };
            if status == windows::Win32::Foundation::ERROR_NO_MORE_ITEMS {
                return values;
            }
            if status == windows::Win32::Foundation::ERROR_MORE_DATA
                && name.len() < 16 * 1024
                && data.len() < 64 * 1024
            {
                name.resize(name.len() * 2, 0);
                data.resize(data.len() * 2, 0);
                continue;
            }
            if status == windows::Win32::Foundation::ERROR_SUCCESS
                && matches!(value_type, REG_SZ | REG_EXPAND_SZ)
            {
                let name_len = (name_len as usize).min(name.len());
                let data_units = (data_len as usize / 2).min(data.len() / 2);
                // SAFETY: u8 registry storage is suitably sized; read_unaligned avoids relying on
                // its alignment while converting the returned UTF-16 code units.
                let decoded = (0..data_units)
                    .map(|offset| unsafe {
                        std::ptr::read_unaligned(data.as_ptr().add(offset * 2).cast::<u16>())
                    })
                    .take_while(|unit| *unit != 0)
                    .collect::<Vec<_>>();
                values.push((
                    String::from_utf16_lossy(&name[..name_len]),
                    String::from_utf16_lossy(&decoded),
                ));
            }
            break;
        }
    }
    values
}

#[cfg(target_os = "windows")]
fn windows_registered_handlers(target: &AssociationTarget) -> Vec<ApplicationHandler> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let Some(query_key) = windows_association_query_key(target) else {
        return Vec::new();
    };
    let association_group = match target {
        AssociationTarget::Extension(_) | AssociationTarget::Mime(_) => "FileAssociations",
        AssociationTarget::Scheme(_) => "UrlAssociations",
    };
    let mut handlers = Vec::new();
    for (root, scope) in [(HKEY_CURRENT_USER, "user"), (HKEY_LOCAL_MACHINE, "machine")] {
        let Some(registrations) =
            windows_open_registry_key(root, "Software\\RegisteredApplications")
        else {
            continue;
        };
        for (id, capabilities_path) in windows_registry_values(&registrations) {
            if handlers
                .iter()
                .any(|handler: &ApplicationHandler| handler.id.eq_ignore_ascii_case(&id))
            {
                continue;
            }
            let Some(capabilities) = windows_open_registry_key(root, &capabilities_path) else {
                continue;
            };
            let association_path = format!("{capabilities_path}\\{association_group}");
            let Some(associations) = windows_open_registry_key(root, &association_path) else {
                continue;
            };
            if windows_registry_string(&associations, query_key).is_none() {
                continue;
            }
            let name = windows_registry_string(&capabilities, "ApplicationName")
                .filter(|name| !name.trim().is_empty())
                .unwrap_or_else(|| id.clone());
            handlers.push(ApplicationHandler {
                id,
                name,
                icon: windows_registry_string(&capabilities, "ApplicationIcon"),
                source: format!("Windows RegisteredApplications ({scope})"),
            });
        }
    }
    handlers.sort_by(|left, right| {
        left.name
            .to_lowercase()
            .cmp(&right.name.to_lowercase())
            .then_with(|| left.id.cmp(&right.id))
    });
    handlers
}

#[cfg(target_os = "windows")]
fn windows_available_targets() -> Vec<AssociationTarget> {
    use windows::Win32::System::Registry::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    let mut targets = HashSet::new();
    for root in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
        let Some(registrations) =
            windows_open_registry_key(root, "Software\\RegisteredApplications")
        else {
            continue;
        };
        for (_, capabilities_path) in windows_registry_values(&registrations) {
            for (group, is_scheme) in [("FileAssociations", false), ("UrlAssociations", true)] {
                let path = format!("{capabilities_path}\\{group}");
                let Some(associations) = windows_open_registry_key(root, &path) else {
                    continue;
                };
                for (key, _) in windows_registry_values(&associations) {
                    if is_scheme {
                        targets.insert(AssociationTarget::scheme(key));
                    } else if key.starts_with('.') {
                        targets.insert(AssociationTarget::extension(key));
                    }
                }
            }
        }
    }
    let mut targets = targets.into_iter().collect::<Vec<_>>();
    targets.sort_by_key(AssociationTarget::platform_key);
    targets
}

#[cfg(target_os = "windows")]
fn windows_effective_handler(
    target: &AssociationTarget,
) -> Result<Option<ApplicationHandler>, AssociationError> {
    use windows::{
        Win32::UI::Shell::{ASSOCF_NONE, ASSOCSTR_EXECUTABLE, AssocQueryStringW},
        core::{PCWSTR, PWSTR},
    };

    let Some(query_key) = windows_association_query_key(target) else {
        return Ok(None);
    };
    let association = query_key.encode_utf16().chain(Some(0)).collect::<Vec<_>>();
    let mut length = 0_u32;
    // SAFETY: both pointers reference initialized NUL-terminated UTF-16 data;
    // the first call intentionally supplies no output buffer to obtain length.
    let first = unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            PCWSTR(association.as_ptr()),
            PCWSTR::null(),
            None,
            &mut length,
        )
    };
    if length == 0 {
        return if first.is_err() {
            Ok(None)
        } else {
            Err(AssociationError(
                "Windows returned an empty association result".into(),
            ))
        };
    }
    let mut executable = vec![0_u16; length as usize];
    // SAFETY: the output buffer contains `length` writable UTF-16 elements and
    // all other pointer validity requirements match the sizing call above.
    unsafe {
        AssocQueryStringW(
            ASSOCF_NONE,
            ASSOCSTR_EXECUTABLE,
            PCWSTR(association.as_ptr()),
            PCWSTR::null(),
            Some(PWSTR(executable.as_mut_ptr())),
            &mut length,
        )
    }
    .ok()
    .map_err(|error| AssociationError(format!("Windows association query failed: {error}")))?;
    let end = executable
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(executable.len());
    let id = String::from_utf16_lossy(&executable[..end]);
    if id.is_empty() {
        return Ok(None);
    }
    let name = Path::new(&id)
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(&id)
        .to_owned();
    Ok(Some(ApplicationHandler {
        id: id.clone(),
        name,
        icon: Some(id.clone()),
        source: "Windows effective association".into(),
    }))
}

/// Translate Nickel's portable MIME/scheme authority into the association
/// identifiers accepted by Windows `AssocQueryStringW`. Windows does not
/// understand freedesktop MIME names or `x-scheme-handler/` keys.
#[cfg(any(target_os = "windows", test))]
fn windows_association_query_key(target: &AssociationTarget) -> Option<&str> {
    match target {
        AssociationTarget::Extension(extension) => Some(extension.as_str()),
        AssociationTarget::Scheme(scheme) => Some(scheme.as_str()),
        AssociationTarget::Mime(mime) => Some(match mime.as_str() {
            "text/plain" => ".txt",
            "text/markdown" => ".md",
            "image/png" => ".png",
            "image/jpeg" => ".jpg",
            "image/gif" => ".gif",
            "application/pdf" => ".pdf",
            "audio/mpeg" => ".mp3",
            "video/mp4" => ".mp4",
            _ => return None,
        }),
    }
}

#[cfg(target_os = "windows")]
impl AssociationBackend for WindowsAssociations {
    fn available_targets(&self) -> Result<Vec<AssociationTarget>, AssociationError> {
        Ok(windows_available_targets())
    }

    fn inspect(&self, target: &AssociationTarget) -> Result<AssociationSnapshot, AssociationError> {
        let effective = windows_effective_handler(target)?;
        let mut handlers = windows_registered_handlers(target);
        if let Some(current) = effective.as_ref()
            && !handlers
                .iter()
                .any(|handler| handler.id.eq_ignore_ascii_case(&current.id))
        {
            handlers.insert(0, current.clone());
        }
        Ok(AssociationSnapshot {
            target: target.clone(),
            handlers,
            effective,
            capability: AssociationCapability::NativeConsent,
            scope: AssociationScope::User,
            detail: "Windows requires its Default apps consent UI; Nickel does not alter protected UserChoice data".into(),
        })
    }
    fn request_change(
        &self,
        _: &AssociationTarget,
        _: &str,
    ) -> Result<ChangeOutcome, AssociationError> {
        std::process::Command::new("explorer.exe")
            .arg("ms-settings:defaultapps")
            .spawn()
            .map_err(|error| {
                AssociationError(format!("could not open Windows Default apps: {error}"))
            })?;
        Ok(ChangeOutcome::NativeConsentRequired {
            detail: "Windows Default apps was opened; choose the application there, then refresh"
                .into(),
        })
    }
}

struct UnsupportedAssociations;

impl AssociationBackend for UnsupportedAssociations {
    fn inspect(&self, target: &AssociationTarget) -> Result<AssociationSnapshot, AssociationError> {
        Ok(AssociationSnapshot {
            target: target.clone(),
            effective: None,
            handlers: Vec::new(),
            capability: AssociationCapability::Unsupported,
            scope: AssociationScope::System,
            detail: "Default applications are unsupported on this platform".into(),
        })
    }
    fn request_change(
        &self,
        _: &AssociationTarget,
        _: &str,
    ) -> Result<ChangeOutcome, AssociationError> {
        Ok(ChangeOutcome::Rejected {
            detail: "Default applications are unsupported on this platform".into(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_association_queries_use_native_extension_and_scheme_keys() {
        for (mime, extension) in [
            ("text/plain", ".txt"),
            ("text/markdown", ".md"),
            ("image/png", ".png"),
            ("image/jpeg", ".jpg"),
            ("image/gif", ".gif"),
            ("application/pdf", ".pdf"),
            ("audio/mpeg", ".mp3"),
            ("video/mp4", ".mp4"),
        ] {
            assert_eq!(
                windows_association_query_key(&AssociationTarget::mime(mime)),
                Some(extension)
            );
        }
        assert_eq!(
            windows_association_query_key(&AssociationTarget::scheme("https")),
            Some("https")
        );
        assert_eq!(
            windows_association_query_key(&AssociationTarget::mime("application/x-unknown")),
            None
        );
    }

    struct Fixture {
        current: Arc<Mutex<String>>,
        confirm: bool,
    }

    struct LargeFixture;

    impl AssociationBackend for LargeFixture {
        fn inspect(
            &self,
            target: &AssociationTarget,
        ) -> Result<AssociationSnapshot, AssociationError> {
            Ok(AssociationSnapshot {
                target: target.clone(),
                effective: None,
                handlers: (0..250)
                    .map(|index| ApplicationHandler {
                        id: format!("handler-{index:03}.desktop"),
                        name: format!("Handler {index:03}"),
                        icon: None,
                        source: "fixture".into(),
                    })
                    .collect(),
                capability: AssociationCapability::DirectUserChange,
                scope: AssociationScope::User,
                detail: String::new(),
            })
        }

        fn request_change(
            &self,
            _: &AssociationTarget,
            _: &str,
        ) -> Result<ChangeOutcome, AssociationError> {
            unreachable!()
        }
    }

    #[test]
    fn association_service_does_not_truncate_compatible_handlers() {
        let service = AssociationService::new(Box::new(LargeFixture));
        let snapshot = service
            .inspect(&AssociationTarget::mime("application/x-fixture"))
            .unwrap();
        assert_eq!(snapshot.handlers.len(), 250);
        assert_eq!(snapshot.handlers.last().unwrap().id, "handler-249.desktop");
    }

    #[test]
    fn association_families_group_formats_without_merging_platform_targets() {
        let svg = AssociationTarget::mime("image/svg+xml");
        let png = AssociationTarget::mime("image/png");
        let pdf = AssociationTarget::mime("application/pdf");
        let odt = AssociationTarget::mime("application/vnd.oasis.opendocument.text");
        let mp4 = AssociationTarget::mime("video/mp4");
        let webm = AssociationTarget::extension(".webm");

        assert_eq!(svg.family(), AssociationFamily::Images);
        assert_eq!(png.family(), AssociationFamily::Images);
        assert_ne!(svg, png, "grouping must preserve exact OS association keys");
        assert_eq!(pdf.family(), AssociationFamily::Documents);
        assert_eq!(odt.family(), AssociationFamily::Documents);
        assert_eq!(mp4.family(), AssociationFamily::Video);
        assert_eq!(webm.family(), AssociationFamily::Video);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_catalog_combines_mime_cache_and_user_association_keys() {
        let directory = tempfile::tempdir().unwrap();
        let cache = directory.path().join("mimeinfo.cache");
        let mimeapps = directory.path().join("mimeapps.list");
        std::fs::write(
            &cache,
            "[MIME Cache]\nimage/svg+xml=viewer.desktop;\nvideo/webm=player.desktop;\n",
        )
        .unwrap();
        std::fs::write(
            &mimeapps,
            "[Default Applications]\napplication/pdf=reader.desktop;\n\
             [Added Associations]\napplication/vnd.oasis.opendocument.text=writer.desktop;\n",
        )
        .unwrap();

        let mut keys = HashSet::new();
        extend_association_keys_from_cache(&cache, &mut keys);
        extend_association_keys_from_mimeapps(&mimeapps, &mut keys);

        for expected in [
            "image/svg+xml",
            "video/webm",
            "application/pdf",
            "application/vnd.oasis.opendocument.text",
        ] {
            assert!(keys.contains(expected), "missing association {expected}");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_handlers_use_recursive_desktop_ids_and_localized_names() {
        let entry = freedesktop_desktop_entry::DesktopEntry::from_str(
            "/fixture/applications/vendor/viewer.desktop",
            "[Desktop Entry]\nName=Viewer\nName[fr]=Visionneuse\nIcon=image-viewer\nMimeType=image/svg+xml;image/png;\n",
            Some(&["fr"]),
        )
        .unwrap();
        let handlers = LinuxAssociations::handlers(
            &AssociationTarget::mime("image/svg+xml"),
            &[entry],
            &["fr".to_owned()],
        );

        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0].id, "vendor-viewer.desktop");
        assert_eq!(handlers[0].name, "Visionneuse");
        assert_eq!(handlers[0].icon.as_deref(), Some("image-viewer"));

        let no_display = freedesktop_desktop_entry::DesktopEntry::from_str(
            "/fixture/applications/helper.desktop",
            "[Desktop Entry]\nName=Helper\nNoDisplay=true\nMimeType=image/svg+xml;\n",
            None::<&[&str]>,
        )
        .unwrap();
        assert_eq!(
            LinuxAssociations::handlers(
                &AssociationTarget::mime("image/svg+xml"),
                &[no_display],
                &[],
            )
            .len(),
            1,
            "NoDisplay handlers remain valid association candidates"
        );
    }

    impl AssociationBackend for Fixture {
        fn inspect(
            &self,
            target: &AssociationTarget,
        ) -> Result<AssociationSnapshot, AssociationError> {
            let id = self.current.lock().unwrap().clone();
            Ok(AssociationSnapshot {
                target: target.clone(),
                effective: Some(ApplicationHandler {
                    name: id.clone(),
                    id: id.clone(),
                    icon: None,
                    source: "fixture".into(),
                }),
                handlers: Vec::new(),
                capability: AssociationCapability::DirectUserChange,
                scope: AssociationScope::User,
                detail: String::new(),
            })
        }
        fn request_change(
            &self,
            target: &AssociationTarget,
            handler_id: &str,
        ) -> Result<ChangeOutcome, AssociationError> {
            if self.confirm {
                *self.current.lock().unwrap() = handler_id.into();
            }
            Ok(ChangeOutcome::Confirmed(self.inspect(target)?))
        }
    }

    #[test]
    fn successful_setters_are_requeried_before_confirmation() {
        let fixture = Fixture {
            current: Arc::new(Mutex::new("old.desktop".into())),
            confirm: true,
        };
        let result = change_and_verify(
            &fixture,
            &AssociationTarget::mime("text/plain"),
            "new.desktop",
        )
        .unwrap();
        assert!(
            matches!(result, ChangeOutcome::Confirmed(snapshot) if snapshot.effective.as_ref().unwrap().id == "new.desktop")
        );
    }

    #[test]
    fn failed_verification_retains_a_rejection_outcome() {
        let fixture = Fixture {
            current: Arc::new(Mutex::new("old.desktop".into())),
            confirm: false,
        };
        assert!(matches!(
            change_and_verify(&fixture, &AssociationTarget::scheme("https"), "new.desktop")
                .unwrap(),
            ChangeOutcome::Rejected { .. }
        ));
    }

    #[test]
    fn portable_file_type_fallback_is_explicit_and_bounded() {
        assert_eq!(
            infer_portable_mime(Path::new("readme.txt")),
            Some("text/plain")
        );
        assert_eq!(infer_portable_mime(Path::new("archive.unknown")), None);
    }

    #[test]
    fn default_launch_rejects_a_missing_target_before_native_dispatch() {
        let missing = tempfile::tempdir().unwrap().path().join("missing.txt");
        assert_eq!(
            open_with_default(&missing),
            Err(DefaultLaunchError::TargetMissing)
        );
    }

    #[test]
    fn one_service_generation_converges_multiple_consumers_and_external_changes() {
        let current = Arc::new(Mutex::new("old.desktop".into()));
        let service = AssociationService::new(Box::new(Fixture {
            current: Arc::clone(&current),
            confirm: true,
        }));
        let target = AssociationTarget::mime("text/plain");
        let settings = service.inspect(&target).unwrap();
        let properties = service.inspect(&target).unwrap();
        assert_eq!(settings, properties);
        assert_eq!(service.generation(), 1);

        *current.lock().unwrap() = "external.desktop".into();
        let refreshed = service.inspect(&target).unwrap();
        assert_eq!(refreshed.effective.unwrap().id, "external.desktop");
        assert_eq!(service.generation(), 2);
    }
}
