//! Operating-system-owned default application associations.
//!
//! This module is deliberately the only place where Nickel applications deal
//! with MIME databases, Windows consent UI, or Launch Services limitations.

use std::{fmt, path::Path};

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum AssociationTarget {
    Mime(String),
    Scheme(String),
}

impl AssociationTarget {
    pub fn mime(value: impl Into<String>) -> Self {
        Self::Mime(value.into())
    }

    pub fn scheme(value: impl Into<String>) -> Self {
        Self::Scheme(value.into())
    }

    pub fn platform_key(&self) -> String {
        match self {
            Self::Mime(value) => value.clone(),
            Self::Scheme(value) => format!("x-scheme-handler/{value}"),
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

pub trait AssociationBackend {
    fn inspect(&self, target: &AssociationTarget) -> Result<AssociationSnapshot, AssociationError>;
    fn request_change(
        &self,
        target: &AssociationTarget,
        handler_id: &str,
    ) -> Result<ChangeOutcome, AssociationError>;
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
    #[cfg(target_os = "macos")]
    return Box::new(MacAssociations);
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

#[cfg(target_os = "linux")]
struct LinuxAssociations;

#[cfg(target_os = "linux")]
impl LinuxAssociations {
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

    fn desktop_name(id: &str) -> String {
        desktop_file_paths(id)
            .into_iter()
            .find_map(|path| std::fs::read_to_string(path).ok())
            .and_then(|contents| {
                contents.lines().find_map(|line| {
                    line.strip_prefix("Name=")
                        .filter(|name| !name.trim().is_empty())
                        .map(str::to_owned)
                })
            })
            .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned())
    }

    fn handlers(target: &AssociationTarget) -> Vec<ApplicationHandler> {
        let key = target.platform_key();
        let mut handlers = Vec::new();
        for root in desktop_data_roots() {
            let Ok(entries) = std::fs::read_dir(root.join("applications")) else {
                continue;
            };
            for entry in entries.flatten().take(2048) {
                let path = entry.path();
                let Some(id) = path.file_name().and_then(|name| name.to_str()) else {
                    continue;
                };
                if !id.ends_with(".desktop")
                    || handlers
                        .iter()
                        .any(|item: &ApplicationHandler| item.id == id)
                {
                    continue;
                }
                let Ok(contents) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let supports = contents
                    .lines()
                    .filter_map(|line| line.strip_prefix("MimeType="))
                    .flat_map(|types| types.split(';'))
                    .any(|kind| kind == key);
                let hidden = contents
                    .lines()
                    .any(|line| line == "Hidden=true" || line == "NoDisplay=true");
                if supports && !hidden {
                    handlers.push(ApplicationHandler {
                        id: id.into(),
                        name: Self::desktop_name(id),
                        icon: contents
                            .lines()
                            .find_map(|line| line.strip_prefix("Icon=").map(str::to_owned)),
                        source: path.display().to_string(),
                    });
                    if handlers.len() == 128 {
                        break;
                    }
                }
            }
        }
        handlers.sort_by(|left, right| left.name.to_lowercase().cmp(&right.name.to_lowercase()));
        handlers
    }
}

#[cfg(target_os = "linux")]
impl AssociationBackend for LinuxAssociations {
    fn inspect(&self, target: &AssociationTarget) -> Result<AssociationSnapshot, AssociationError> {
        let effective = Self::query(target)?.map(|id| ApplicationHandler {
            name: Self::desktop_name(&id),
            id,
            icon: None,
            source: "freedesktop MIME default".into(),
        });
        let mut handlers = Self::handlers(target);
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
        if !desktop_file_paths(handler_id)
            .iter()
            .any(|path| path.is_file())
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
fn desktop_file_paths(id: &str) -> Vec<std::path::PathBuf> {
    desktop_data_roots()
        .into_iter()
        .map(|root| root.join("applications").join(id))
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

#[cfg(target_os = "windows")]
struct WindowsAssociations;

#[cfg(target_os = "windows")]
impl AssociationBackend for WindowsAssociations {
    fn inspect(&self, target: &AssociationTarget) -> Result<AssociationSnapshot, AssociationError> {
        Ok(AssociationSnapshot {
            target: target.clone(), effective: None, handlers: Vec::new(),
            capability: AssociationCapability::NativeConsent,
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

#[cfg(target_os = "macos")]
struct MacAssociations;

#[cfg(target_os = "macos")]
impl AssociationBackend for MacAssociations {
    fn inspect(&self, target: &AssociationTarget) -> Result<AssociationSnapshot, AssociationError> {
        Ok(AssociationSnapshot {
            target: target.clone(),
            effective: None,
            handlers: Vec::new(),
            capability: AssociationCapability::ReadOnly,
            detail: "No public supported setter is available in this Nickel build".into(),
        })
    }
    fn request_change(
        &self,
        _: &AssociationTarget,
        _: &str,
    ) -> Result<ChangeOutcome, AssociationError> {
        Ok(ChangeOutcome::Rejected {
            detail: "Change this association with the macOS Open With workflow".into(),
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
    use std::cell::RefCell;

    struct Fixture {
        current: RefCell<String>,
        confirm: bool,
    }

    impl AssociationBackend for Fixture {
        fn inspect(
            &self,
            target: &AssociationTarget,
        ) -> Result<AssociationSnapshot, AssociationError> {
            let id = self.current.borrow().clone();
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
                detail: String::new(),
            })
        }
        fn request_change(
            &self,
            target: &AssociationTarget,
            handler_id: &str,
        ) -> Result<ChangeOutcome, AssociationError> {
            if self.confirm {
                *self.current.borrow_mut() = handler_id.into();
            }
            Ok(ChangeOutcome::Confirmed(self.inspect(target)?))
        }
    }

    #[test]
    fn successful_setters_are_requeried_before_confirmation() {
        let fixture = Fixture {
            current: RefCell::new("old.desktop".into()),
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
            current: RefCell::new("old.desktop".into()),
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
}
