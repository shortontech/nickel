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

#[cfg(target_os = "linux")]
pub fn peripheral_backend() -> Box<dyn PeripheralBackend> {
    Box::new(LinuxPeripherals)
}

#[cfg(not(target_os = "linux"))]
pub fn peripheral_backend() -> Box<dyn PeripheralBackend> {
    Box::new(UnsupportedPeripherals)
}

#[cfg(target_os = "linux")]
struct LinuxPeripherals;

#[cfg(target_os = "linux")]
impl PeripheralBackend for LinuxPeripherals {
    fn inspect(&self) -> Result<PeripheralSnapshot, PeripheralError> {
        let cups_available =
            command_output("lpstat", &["-r"]).is_ok_and(|output| output.status.success());
        let udisks2_available =
            command_output("udisksctl", &["status"]).is_ok_and(|output| output.status.success());
        Ok(PeripheralSnapshot {
            provider: PeripheralProvider::LinuxCupsAndUDisks2 {
                cups_available,
                udisks2_available,
            },
            printers: discover_linux_printers(),
            volumes: discover_linux_volumes(),
            filesystems: discover_linux_filesystems(),
            omitted_printers: 0,
            omitted_jobs: 0,
            omitted_volumes: 0,
            omitted_filesystems: 0,
        })
    }

    fn request(&self, action: PeripheralAction) -> Result<PeripheralOutcome, PeripheralError> {
        match action {
            PeripheralAction::SetDefaultPrinter(id) => run_linux_action("lpoptions", &["-d", &id]),
            PeripheralAction::AddPrinter { address } => {
                use std::hash::{DefaultHasher, Hash, Hasher};
                let mut hasher = DefaultHasher::new();
                address.hash(&mut hasher);
                let id = format!("nickel-{:016x}", hasher.finish());
                run_linux_action("lpadmin", &["-p", &id, "-E", "-v", &address])
            }
            PeripheralAction::RemovePrinter(id) => run_linux_action("lpadmin", &["-x", &id]),
            PeripheralAction::CancelPrintJob { job_id, .. } => {
                run_linux_action("cancel", &[&job_id])
            }
            PeripheralAction::PrintTestPage(id) => {
                const TEST_PAGE: &str = "/usr/share/cups/data/testprint";
                if PathBuf::from(TEST_PAGE).is_file() {
                    run_linux_action("lp", &["-d", &id, TEST_PAGE])
                } else {
                    Ok(PeripheralOutcome::Unsupported {
                        detail: "The CUPS test-page fixture is not installed".into(),
                    })
                }
            }
            PeripheralAction::MountVolume(id) => {
                run_linux_action("udisksctl", &["mount", "-b", &id])
            }
            PeripheralAction::UnmountVolume(id) => {
                run_linux_action("udisksctl", &["unmount", "-b", &id])
            }
            PeripheralAction::EjectVolume(id) => {
                run_linux_action("udisksctl", &["power-off", "-b", &id])
            }
            PeripheralAction::OpenCleanupLocation(path) => crate::open_directory(&path)
                .map(|()| PeripheralOutcome::Accepted)
                .map_err(|detail| PeripheralError {
                    class: PeripheralFailureClass::ProviderUnavailable,
                    detail,
                }),
        }
    }
}

#[cfg(target_os = "linux")]
fn command_output(program: &str, arguments: &[&str]) -> std::io::Result<std::process::Output> {
    std::process::Command::new(program).args(arguments).output()
}

#[cfg(target_os = "linux")]
fn output_text(program: &str, arguments: &[&str]) -> Result<String, String> {
    let output = command_output(program, arguments)
        .map_err(|error| format!("{program} is unavailable: {error}"))?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        return Err(if detail.is_empty() {
            format!("{program} exited with {}", output.status)
        } else {
            detail
        });
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(target_os = "linux")]
fn run_linux_action(
    program: &str,
    arguments: &[&str],
) -> Result<PeripheralOutcome, PeripheralError> {
    let output = command_output(program, arguments).map_err(|error| PeripheralError {
        class: PeripheralFailureClass::ProviderUnavailable,
        detail: format!("{program} is unavailable: {error}"),
    })?;
    if output.status.success() {
        return Ok(PeripheralOutcome::Accepted);
    }
    let detail = clean_text(String::from_utf8_lossy(&output.stderr).trim());
    let lowercase = detail.to_lowercase();
    Ok(
        if lowercase.contains("busy") || lowercase.contains("in use") {
            PeripheralOutcome::Busy { detail }
        } else if lowercase.contains("not authorized")
            || lowercase.contains("permission")
            || lowercase.contains("authentication")
        {
            PeripheralOutcome::AuthorizationRequired { detail }
        } else {
            PeripheralOutcome::Rejected { detail }
        },
    )
}

#[cfg(target_os = "linux")]
fn discover_linux_printers() -> Result<Vec<Printer>, String> {
    let listing = output_text("lpstat", &["-p"])?;
    let default = output_text("lpstat", &["-d"])
        .ok()
        .and_then(|line| line.split_once(':').map(|(_, id)| id.trim().to_owned()));
    let jobs = output_text("lpstat", &["-W", "not-completed", "-o"]).unwrap_or_default();
    let mut printers = listing
        .lines()
        .filter_map(|line| parse_lpstat_printer(line, default.as_deref()))
        .collect::<Vec<_>>();
    for job in jobs.lines().filter_map(parse_lpstat_job) {
        if let Some(printer) = printers.iter_mut().find(|printer| {
            job.id
                .strip_prefix(&printer.id)
                .is_some_and(|suffix| suffix.starts_with('-'))
        }) {
            printer.jobs.push(job);
        }
    }
    Ok(printers)
}

#[cfg(target_os = "linux")]
fn parse_lpstat_printer(line: &str, default: Option<&str>) -> Option<Printer> {
    let remainder = line.strip_prefix("printer ")?;
    let id = remainder.split_whitespace().next()?.to_owned();
    let lowercase = remainder.to_lowercase();
    let state = if lowercase.contains("fault") || lowercase.contains("error") {
        PrinterState::Error
    } else if lowercase.contains("disabled") || lowercase.contains("offline") {
        PrinterState::Offline
    } else if lowercase.contains("printing") || lowercase.contains("processing") {
        PrinterState::Busy
    } else {
        PrinterState::Ready
    };
    Some(Printer {
        name: id.clone(),
        is_default: default == Some(id.as_str()),
        id,
        state,
        jobs: Vec::new(),
    })
}

#[cfg(target_os = "linux")]
fn parse_lpstat_job(line: &str) -> Option<PrintJob> {
    let id = line.split_whitespace().next()?.to_owned();
    Some(PrintJob {
        name: id.clone(),
        id,
        state: PrintJobState::Pending,
    })
}

#[cfg(target_os = "linux")]
fn discover_linux_volumes() -> Result<Vec<RemovableVolume>, String> {
    let listing = output_text(
        "lsblk",
        &[
            "-P",
            "-b",
            "-o",
            "PATH,LABEL,SIZE,FSAVAIL,MOUNTPOINT,RM,HOTPLUG,TYPE",
        ],
    )?;
    Ok(listing.lines().filter_map(parse_lsblk_volume).collect())
}

#[cfg(target_os = "linux")]
fn parse_lsblk_volume(line: &str) -> Option<RemovableVolume> {
    let fields = parse_key_value_fields(line);
    let id = fields.get("PATH")?.clone();
    let removable = fields.get("RM").is_some_and(|value| value == "1")
        || fields.get("HOTPLUG").is_some_and(|value| value == "1");
    let kind = fields.get("TYPE").map(String::as_str).unwrap_or_default();
    if !removable || matches!(kind, "loop" | "rom") {
        return None;
    }
    let mount = fields
        .get("MOUNTPOINT")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    Some(RemovableVolume {
        name: fields
            .get("LABEL")
            .filter(|value| !value.is_empty())
            .cloned()
            .unwrap_or_else(|| id.clone()),
        capacity_bytes: fields.get("SIZE").and_then(|value| value.parse().ok()),
        available_bytes: fields.get("FSAVAIL").and_then(|value| value.parse().ok()),
        state: if mount.is_some() {
            VolumeState::Mounted
        } else {
            VolumeState::Unmounted
        },
        mount_path: mount,
        ejectable: true,
        detail: None,
        id,
    })
}

#[cfg(target_os = "linux")]
fn parse_key_value_fields(line: &str) -> std::collections::HashMap<String, String> {
    let mut fields = std::collections::HashMap::new();
    let bytes = line.as_bytes();
    let mut cursor = 0;
    while cursor < bytes.len() {
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let key_start = cursor;
        while bytes.get(cursor).is_some_and(|byte| *byte != b'=') {
            cursor += 1;
        }
        if cursor >= bytes.len() {
            break;
        }
        let key = &line[key_start..cursor];
        cursor += 1;
        if bytes.get(cursor) != Some(&b'"') {
            break;
        }
        cursor += 1;
        let value_start = cursor;
        while bytes.get(cursor) != Some(&b'"') && cursor < bytes.len() {
            cursor += 1;
        }
        fields.insert(
            key.to_owned(),
            decode_lsblk_value(&line[value_start..cursor]),
        );
        cursor = cursor.saturating_add(1);
    }
    fields
}

#[cfg(target_os = "linux")]
fn decode_lsblk_value(value: &str) -> String {
    let mut decoded = String::new();
    let mut chars = value.chars().peekable();
    while let Some(character) = chars.next() {
        if character == '\\' && chars.peek() == Some(&'x') {
            chars.next();
            let digits = chars.by_ref().take(2).collect::<String>();
            if let Ok(byte) = u8::from_str_radix(&digits, 16) {
                decoded.push(char::from(byte));
                continue;
            }
            decoded.push_str("\\x");
            decoded.push_str(&digits);
        } else {
            decoded.push(character);
        }
    }
    decoded
}

#[cfg(target_os = "linux")]
fn discover_linux_filesystems() -> Result<Vec<FilesystemUsage>, String> {
    let listing = output_text("df", &["--output=source,size,avail,target", "-B1"])?;
    Ok(listing
        .lines()
        .skip(1)
        .filter_map(parse_df_filesystem)
        .collect())
}

#[cfg(target_os = "linux")]
fn parse_df_filesystem(line: &str) -> Option<FilesystemUsage> {
    let fields = line.split_whitespace().collect::<Vec<_>>();
    if fields.len() < 4 {
        return None;
    }
    let id = fields[0].to_owned();
    let capacity_bytes = fields[1].parse().ok()?;
    let available_bytes = fields[2].parse().ok()?;
    let mount_path = PathBuf::from(fields[3..].join(" "));
    Some(FilesystemUsage {
        name: mount_path.display().to_string(),
        mount_path,
        capacity_bytes,
        available_bytes,
        id,
    })
}

#[cfg(not(target_os = "linux"))]
struct UnsupportedPeripherals;
#[cfg(not(target_os = "linux"))]
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
                PeripheralAction::MountVolume(id) => {
                    if let Ok(volumes) = &mut snapshot.volumes
                        && let Some(volume) = volumes.iter_mut().find(|volume| volume.id == id)
                    {
                        volume.state = VolumeState::Mounted;
                        volume.mount_path = Some("/media/fixture".into());
                    }
                }
                PeripheralAction::UnmountVolume(id) => {
                    if let Ok(volumes) = &mut snapshot.volumes
                        && let Some(volume) = volumes.iter_mut().find(|volume| volume.id == id)
                    {
                        volume.state = VolumeState::Unmounted;
                        volume.mount_path = None;
                    }
                }
                PeripheralAction::CancelPrintJob { printer_id, job_id } => {
                    if let Ok(printers) = &mut snapshot.printers
                        && let Some(printer) =
                            printers.iter_mut().find(|printer| printer.id == printer_id)
                    {
                        printer.jobs.retain(|job| job.id != job_id);
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
    fn queue_and_mount_transitions_are_refreshed() {
        let mut initial = snapshot();
        initial.printers.as_mut().unwrap()[0].jobs.push(PrintJob {
            id: "printer-42".into(),
            name: "Document".into(),
            state: PrintJobState::Pending,
        });
        let service = PeripheralService::new(Box::new(Fixture {
            snapshot: Mutex::new(initial),
        }));
        let (_, changed) = service
            .request_and_refresh(PeripheralAction::CancelPrintJob {
                printer_id: "printer".into(),
                job_id: "printer-42".into(),
            })
            .unwrap();
        assert!(changed.printers.unwrap()[0].jobs.is_empty());
        let (_, changed) = service
            .request_and_refresh(PeripheralAction::UnmountVolume("usb".into()))
            .unwrap();
        assert_eq!(
            changed.volumes.as_ref().unwrap()[0].state,
            VolumeState::Unmounted
        );
        let (_, changed) = service
            .request_and_refresh(PeripheralAction::MountVolume("usb".into()))
            .unwrap();
        assert_eq!(
            changed.volumes.as_ref().unwrap()[0].state,
            VolumeState::Mounted
        );
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

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_service_output_parsers_preserve_stable_identities() {
        let printer = parse_lpstat_printer(
            "printer office is idle. enabled since Monday",
            Some("office"),
        )
        .unwrap();
        assert_eq!(printer.id, "office");
        assert!(printer.is_default);

        let volume = parse_lsblk_volume(
            r#"PATH="/dev/sdb1" LABEL="Backup\x20Disk" SIZE="4096" FSAVAIL="2048" MOUNTPOINT="/media/Backup\x20Disk" RM="1" HOTPLUG="1" TYPE="part""#,
        )
        .unwrap();
        assert_eq!(volume.id, "/dev/sdb1");
        assert_eq!(volume.name, "Backup Disk");
        assert_eq!(volume.state, VolumeState::Mounted);

        let filesystem =
            parse_df_filesystem("/dev/root      1000       250 /media/My Drive").unwrap();
        assert_eq!(filesystem.capacity_bytes, 1000);
        assert_eq!(filesystem.available_bytes, 250);
        assert_eq!(filesystem.mount_path, PathBuf::from("/media/My Drive"));
    }
}
