//! Shared authority for printers, removable media, and bounded filesystem usage.

use std::{
    fmt,
    path::PathBuf,
    sync::{Arc, OnceLock},
};

const MAX_PRINTERS: usize = 512;
const MAX_JOBS: usize = 2_048;
const MAX_VOLUMES: usize = 512;
const MAX_FILESYSTEMS: usize = 256;
const MAX_TEXT: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeripheralProvider {
    LinuxCupsAndUDisks2 {
        cups_available: bool,
        udisks2_available: bool,
    },
    WindowsPrintAndStorage,
    Unsupported {
        platform: String,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrinterState {
    Ready,
    Busy,
    Offline,
    Error,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PrintJobState {
    Pending,
    Printing,
    Held,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrintJob {
    pub id: String,
    pub name: String,
    pub state: PrintJobState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Printer {
    pub id: String,
    pub name: String,
    pub is_default: bool,
    pub state: PrinterState,
    pub jobs: Vec<PrintJob>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum VolumeState {
    Unmounted,
    Mounted,
    Busy,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemovableVolume {
    pub id: String,
    pub name: String,
    pub capacity_bytes: Option<u64>,
    pub available_bytes: Option<u64>,
    pub mount_path: Option<PathBuf>,
    pub state: VolumeState,
    pub ejectable: bool,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FilesystemUsage {
    pub id: String,
    pub name: String,
    pub mount_path: PathBuf,
    pub capacity_bytes: u64,
    pub available_bytes: u64,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeripheralSnapshot {
    pub provider: PeripheralProvider,
    pub printers: Result<Vec<Printer>, String>,
    pub volumes: Result<Vec<RemovableVolume>, String>,
    pub filesystems: Result<Vec<FilesystemUsage>, String>,
    pub omitted_printers: usize,
    pub omitted_jobs: usize,
    pub omitted_volumes: usize,
    pub omitted_filesystems: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeripheralAction {
    SetDefaultPrinter(String),
    AddPrinter { address: String },
    RemovePrinter(String),
    CancelPrintJob { printer_id: String, job_id: String },
    PrintTestPage(String),
    MountVolume(String),
    UnmountVolume(String),
    EjectVolume(String),
    OpenCleanupLocation(PathBuf),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PeripheralOutcome {
    Accepted,
    Busy { detail: String },
    AuthorizationRequired { detail: String },
    Unsupported { detail: String },
    Rejected { detail: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PeripheralFailureClass {
    Authorization,
    Busy,
    ProviderUnavailable,
    InvalidTarget,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeripheralError {
    pub class: PeripheralFailureClass,
    pub detail: String,
}

impl fmt::Display for PeripheralError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.class, self.detail)
    }
}
impl std::error::Error for PeripheralError {}

pub trait PeripheralBackend: Send + Sync {
    fn inspect(&self) -> Result<PeripheralSnapshot, PeripheralError>;
    fn request(&self, action: PeripheralAction) -> Result<PeripheralOutcome, PeripheralError>;
}

pub struct PeripheralService {
    backend: Box<dyn PeripheralBackend>,
}

impl PeripheralService {
    pub fn new(backend: Box<dyn PeripheralBackend>) -> Self {
        Self { backend }
    }

    pub fn inspect(&self) -> Result<PeripheralSnapshot, PeripheralError> {
        self.backend
            .inspect()
            .map(sanitize_snapshot)
            .map_err(sanitize_error)
    }

    pub fn request_and_refresh(
        &self,
        action: PeripheralAction,
    ) -> Result<(PeripheralOutcome, PeripheralSnapshot), PeripheralError> {
        validate_action(&action)?;
        let outcome = self
            .backend
            .request(action)
            .map(sanitize_outcome)
            .map_err(sanitize_error)?;
        let snapshot = self.inspect()?;
        Ok((outcome, snapshot))
    }
}

pub fn peripheral_service() -> Arc<PeripheralService> {
    static SERVICE: OnceLock<Arc<PeripheralService>> = OnceLock::new();
    Arc::clone(SERVICE.get_or_init(|| Arc::new(PeripheralService::new(peripheral_backend()))))
}

pub fn peripheral_backend() -> Box<dyn PeripheralBackend> {
    Box::new(UnsupportedPeripherals)
}

struct UnsupportedPeripherals;
impl PeripheralBackend for UnsupportedPeripherals {
    fn inspect(&self) -> Result<PeripheralSnapshot, PeripheralError> {
        #[cfg(target_os = "linux")]
        let provider = PeripheralProvider::LinuxCupsAndUDisks2 {
            cups_available: PathBuf::from("/run/cups/cups.sock").exists(),
            udisks2_available: PathBuf::from("/run/udisks2").exists()
                || PathBuf::from(
                    "/usr/share/dbus-1/system-services/org.freedesktop.UDisks2.service",
                )
                .exists(),
        };
        #[cfg(target_os = "windows")]
        let provider = PeripheralProvider::WindowsPrintAndStorage;
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        let provider = PeripheralProvider::Unsupported {
            platform: std::env::consts::OS.into(),
        };
        Ok(PeripheralSnapshot {
            provider,
            printers: Err("CUPS or native print enumeration is not connected".into()),
            volumes: Err("UDisks2 or native removable-media enumeration is not connected".into()),
            filesystems: Err("Native filesystem-usage enumeration is not connected".into()),
            omitted_printers: 0,
            omitted_jobs: 0,
            omitted_volumes: 0,
            omitted_filesystems: 0,
        })
    }

    fn request(&self, _: PeripheralAction) -> Result<PeripheralOutcome, PeripheralError> {
        Ok(PeripheralOutcome::Unsupported {
            detail: "No supported peripheral mutation provider is connected".into(),
        })
    }
}

fn sanitize_snapshot(mut snapshot: PeripheralSnapshot) -> PeripheralSnapshot {
    if let Ok(printers) = &mut snapshot.printers {
        let original = printers.len();
        printers.truncate(MAX_PRINTERS);
        snapshot.omitted_printers = snapshot
            .omitted_printers
            .saturating_add(original - printers.len());
        let retained_before_deduplication = printers.len();
        let mut seen = std::collections::HashSet::new();
        printers.retain_mut(|printer| {
            printer.id = clean_text(&printer.id);
            printer.name = clean_text(&printer.name);
            !printer.id.is_empty() && seen.insert(printer.id.clone())
        });
        snapshot.omitted_printers = snapshot
            .omitted_printers
            .saturating_add(retained_before_deduplication - printers.len());
        let mut retained_jobs = 0;
        for printer in printers.iter_mut() {
            let original_jobs = printer.jobs.len();
            printer
                .jobs
                .truncate(MAX_JOBS.saturating_sub(retained_jobs));
            let mut job_ids = std::collections::HashSet::new();
            printer.jobs.retain_mut(|job| {
                job.id = clean_text(&job.id);
                job.name = clean_text(&job.name);
                !job.id.is_empty() && job_ids.insert(job.id.clone())
            });
            retained_jobs += printer.jobs.len();
            snapshot.omitted_jobs = snapshot
                .omitted_jobs
                .saturating_add(original_jobs - printer.jobs.len());
        }
        let mut default_seen = false;
        for printer in printers {
            if printer.is_default && std::mem::replace(&mut default_seen, true) {
                printer.is_default = false;
            }
        }
    } else if let Err(detail) = &mut snapshot.printers {
        *detail = clean_text(detail);
    }
    if let Ok(volumes) = &mut snapshot.volumes {
        let original = volumes.len();
        volumes.truncate(MAX_VOLUMES);
        snapshot.omitted_volumes = snapshot
            .omitted_volumes
            .saturating_add(original - volumes.len());
        let retained_before_deduplication = volumes.len();
        let mut seen = std::collections::HashSet::new();
        volumes.retain_mut(|volume| {
            volume.id = clean_text(&volume.id);
            volume.name = clean_text(&volume.name);
            volume.detail = volume.detail.as_deref().map(clean_text);
            !volume.id.is_empty() && seen.insert(volume.id.clone())
        });
        snapshot.omitted_volumes = snapshot
            .omitted_volumes
            .saturating_add(retained_before_deduplication - volumes.len());
    } else if let Err(detail) = &mut snapshot.volumes {
        *detail = clean_text(detail);
    }
    if let Ok(filesystems) = &mut snapshot.filesystems {
        let original = filesystems.len();
        filesystems.truncate(MAX_FILESYSTEMS);
        snapshot.omitted_filesystems = snapshot
            .omitted_filesystems
            .saturating_add(original - filesystems.len());
        let retained_before_deduplication = filesystems.len();
        let mut seen = std::collections::HashSet::new();
        filesystems.retain_mut(|filesystem| {
            filesystem.id = clean_text(&filesystem.id);
            filesystem.name = clean_text(&filesystem.name);
            filesystem.available_bytes = filesystem.available_bytes.min(filesystem.capacity_bytes);
            !filesystem.id.is_empty() && seen.insert(filesystem.id.clone())
        });
        snapshot.omitted_filesystems = snapshot
            .omitted_filesystems
            .saturating_add(retained_before_deduplication - filesystems.len());
    } else if let Err(detail) = &mut snapshot.filesystems {
        *detail = clean_text(detail);
    }
    snapshot
}

fn validate_action(action: &PeripheralAction) -> Result<(), PeripheralError> {
    let valid = match action {
        PeripheralAction::SetDefaultPrinter(id)
        | PeripheralAction::RemovePrinter(id)
        | PeripheralAction::PrintTestPage(id)
        | PeripheralAction::MountVolume(id)
        | PeripheralAction::UnmountVolume(id)
        | PeripheralAction::EjectVolume(id) => !id.trim().is_empty() && id.len() <= MAX_TEXT,
        PeripheralAction::CancelPrintJob { printer_id, job_id } => {
            !printer_id.trim().is_empty()
                && !job_id.trim().is_empty()
                && printer_id.len() <= MAX_TEXT
                && job_id.len() <= MAX_TEXT
        }
        PeripheralAction::AddPrinter { address } => {
            !address.trim().is_empty() && address.len() <= MAX_TEXT
        }
        PeripheralAction::OpenCleanupLocation(path) => path.is_absolute(),
    };
    valid.then_some(()).ok_or_else(|| PeripheralError {
        class: PeripheralFailureClass::InvalidTarget,
        detail: "Invalid or unbounded peripheral target".into(),
    })
}

fn clean_text(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_TEXT)
        .collect()
}
fn sanitize_error(mut error: PeripheralError) -> PeripheralError {
    error.detail = clean_text(&error.detail);
    error
}
fn sanitize_outcome(outcome: PeripheralOutcome) -> PeripheralOutcome {
    match outcome {
        PeripheralOutcome::Busy { detail } => PeripheralOutcome::Busy {
            detail: clean_text(&detail),
        },
        PeripheralOutcome::AuthorizationRequired { detail } => {
            PeripheralOutcome::AuthorizationRequired {
                detail: clean_text(&detail),
            }
        }
        PeripheralOutcome::Unsupported { detail } => PeripheralOutcome::Unsupported {
            detail: clean_text(&detail),
        },
        PeripheralOutcome::Rejected { detail } => PeripheralOutcome::Rejected {
            detail: clean_text(&detail),
        },
        accepted => accepted,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct Fixture {
        snapshot: Mutex<PeripheralSnapshot>,
    }
    impl PeripheralBackend for Fixture {
        fn inspect(&self) -> Result<PeripheralSnapshot, PeripheralError> {
            Ok(self.snapshot.lock().unwrap().clone())
        }
        fn request(&self, action: PeripheralAction) -> Result<PeripheralOutcome, PeripheralError> {
            let mut snapshot = self.snapshot.lock().unwrap();
            match action {
                PeripheralAction::SetDefaultPrinter(id) => {
                    if let Ok(printers) = &mut snapshot.printers {
                        for printer in printers {
                            printer.is_default = printer.id == id;
                        }
                    }
                }
                PeripheralAction::EjectVolume(id) => {
                    if let Ok(volumes) = &mut snapshot.volumes {
                        volumes.retain(|volume| volume.id != id);
                    }
                }
                _ => {}
            }
            Ok(PeripheralOutcome::Accepted)
        }
    }

    fn snapshot() -> PeripheralSnapshot {
        PeripheralSnapshot {
            provider: PeripheralProvider::Unsupported {
                platform: "fixture".into(),
            },
            printers: Ok(vec![
                Printer {
                    id: "printer".into(),
                    name: "Office\nPrinter".into(),
                    is_default: true,
                    state: PrinterState::Ready,
                    jobs: Vec::new(),
                },
                Printer {
                    id: "printer".into(),
                    name: "duplicate".into(),
                    is_default: true,
                    state: PrinterState::Busy,
                    jobs: Vec::new(),
                },
                Printer {
                    id: "other".into(),
                    name: "Other".into(),
                    is_default: true,
                    state: PrinterState::Ready,
                    jobs: Vec::new(),
                },
            ]),
            volumes: Ok(vec![RemovableVolume {
                id: "usb".into(),
                name: "USB".into(),
                capacity_bytes: Some(100),
                available_bytes: Some(20),
                mount_path: Some("/media/usb".into()),
                state: VolumeState::Mounted,
                ejectable: true,
                detail: None,
            }]),
            filesystems: Ok(vec![FilesystemUsage {
                id: "root".into(),
                name: "Root".into(),
                mount_path: "/".into(),
                capacity_bytes: 100,
                available_bytes: 200,
            }]),
            omitted_printers: 0,
            omitted_jobs: 0,
            omitted_volumes: 0,
            omitted_filesystems: 0,
        }
    }

    #[test]
    fn discovery_is_bounded_deduplicated_and_sanitized() {
        let service = PeripheralService::new(Box::new(Fixture {
            snapshot: Mutex::new(snapshot()),
        }));
        let snapshot = service.inspect().unwrap();
        let printers = snapshot.printers.unwrap();
        assert_eq!(printers.len(), 2);
        assert_eq!(
            printers.iter().filter(|printer| printer.is_default).count(),
            1
        );
        assert_eq!(printers[0].name, "OfficePrinter");
        assert_eq!(snapshot.filesystems.unwrap()[0].available_bytes, 100);
    }

    #[test]
    fn accepted_actions_are_refreshed_against_provider_state() {
        let service = PeripheralService::new(Box::new(Fixture {
            snapshot: Mutex::new(snapshot()),
        }));
        let (_, changed) = service
            .request_and_refresh(PeripheralAction::SetDefaultPrinter("other".into()))
            .unwrap();
        assert_eq!(
            changed
                .printers
                .unwrap()
                .iter()
                .find(|printer| printer.is_default)
                .unwrap()
                .id,
            "other"
        );
        let (_, changed) = service
            .request_and_refresh(PeripheralAction::EjectVolume("usb".into()))
            .unwrap();
        assert!(changed.volumes.unwrap().is_empty());
    }

    #[test]
    fn cleanup_contract_has_no_delete_action_and_rejects_relative_paths() {
        let service = PeripheralService::new(Box::new(Fixture {
            snapshot: Mutex::new(snapshot()),
        }));
        let error = service
            .request_and_refresh(PeripheralAction::OpenCleanupLocation("relative".into()))
            .unwrap_err();
        assert_eq!(error.class, PeripheralFailureClass::InvalidTarget);
    }
}
