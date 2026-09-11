//! Typed platform authority for system maintenance, protection, privacy, and secure storage.
//!
//! Absence is represented explicitly. Consumers must never infer health from a missing provider or
//! a failed/stale observation.

use std::{
    fmt,
    sync::{Arc, OnceLock},
    time::SystemTime,
};

#[cfg(target_os = "windows")]
use std::{
    cell::RefCell,
    io::Read,
    os::windows::{io::AsRawHandle, process::CommandExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use windows::Win32::{
    Foundation::{CloseHandle, HANDLE},
    System::{
        Diagnostics::ToolHelp::{
            CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
        },
        JobObjects::{
            AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
            SetInformationJobObject, TerminateJobObject,
        },
        Threading::{
            CREATE_NO_WINDOW, CREATE_SUSPENDED, OpenThread, ResumeThread, THREAD_SUSPEND_RESUME,
        },
    },
};

#[cfg(target_os = "linux")]
use std::{path::Path, time::Duration};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaintenanceProvider {
    LinuxPackageKit { distribution: String },
    LinuxUnsupported { distribution: String },
    WindowsUpdateAndSecurity,
    Unsupported { platform: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ObservationState {
    Current,
    Stale,
    Unsupported,
    PermissionDenied,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Observation<T> {
    pub state: ObservationState,
    pub value: Option<T>,
    pub observed_at: Option<SystemTime>,
    pub detail: Option<String>,
}

impl<T> Observation<T> {
    pub fn unsupported(detail: impl Into<String>) -> Self {
        Self {
            state: ObservationState::Unsupported,
            value: None,
            observed_at: None,
            detail: Some(detail.into()),
        }
    }

    fn sanitize(mut self) -> Self {
        self.detail = self.detail.as_deref().map(redact_detail);
        if matches!(
            self.state,
            ObservationState::Unsupported
                | ObservationState::PermissionDenied
                | ObservationState::Failed
        ) {
            self.value = None;
        }
        self
    }
}

#[cfg(not(target_os = "windows"))]
fn unavailable_observation<T>() -> Observation<T> {
    Observation::unsupported("No supported authoritative provider is connected")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UpdatePhase {
    Idle,
    Checking,
    Downloading,
    Installing,
    AwaitingRestart,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpdateStatus {
    pub available: u32,
    pub phase: UpdatePhase,
    pub restart_required: bool,
    pub last_successful_check: Option<SystemTime>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectionHealth {
    Healthy,
    AttentionRequired,
    Unhealthy,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectionStatus {
    pub firewall: Observation<ProtectionHealth>,
    pub malware_protection: Observation<ProtectionHealth>,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PermissionKind {
    Camera,
    Microphone,
    Location,
    Notifications,
    ScreenCapture,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PermissionMutation {
    Direct,
    NativeConsent,
    ReadOnly,
    Unsupported,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PermissionStatus {
    pub kind: PermissionKind,
    pub global_enabled: Observation<bool>,
    pub per_application_consent: bool,
    pub mutation: PermissionMutation,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SecureStorageReadiness {
    Ready,
    Locked,
    RecoveryRequired,
    Unavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaintenanceSnapshot {
    pub provider: MaintenanceProvider,
    pub updates: Observation<UpdateStatus>,
    pub protection: ProtectionStatus,
    pub permissions: Vec<PermissionStatus>,
    pub secure_storage: Observation<SecureStorageReadiness>,
}

impl MaintenanceSnapshot {
    fn sanitize(mut self) -> Self {
        self.updates = self.updates.sanitize();
        self.protection.firewall = self.protection.firewall.sanitize();
        self.protection.malware_protection = self.protection.malware_protection.sanitize();
        for protection in [
            &mut self.protection.firewall,
            &mut self.protection.malware_protection,
        ] {
            if protection.state != ObservationState::Current
                && protection.value == Some(ProtectionHealth::Healthy)
            {
                protection.value = None;
            }
        }
        for permission in &mut self.permissions {
            permission.global_enabled = permission.global_enabled.clone().sanitize();
        }
        self.secure_storage = self.secure_storage.sanitize();
        self
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaintenanceAction {
    CheckForUpdates,
    InstallUpdates,
    ScheduleRestart,
    SetPermission(PermissionKind, bool),
    OpenNativePermissionSettings(PermissionKind),
    RecoverSecureStorage,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MaintenanceOutcome {
    Accepted,
    NativeConsentRequired { detail: String },
    Unsupported { detail: String },
    Rejected { detail: String },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MaintenanceFailureClass {
    Network,
    Authorization,
    ProviderUnavailable,
    Cancelled,
    Policy,
    Unknown,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MaintenanceError {
    pub class: MaintenanceFailureClass,
    pub detail: String,
}

impl fmt::Display for MaintenanceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}: {}", self.class, self.detail)
    }
}

impl std::error::Error for MaintenanceError {}

pub trait MaintenanceBackend: Send + Sync {
    fn inspect(&self) -> Result<MaintenanceSnapshot, MaintenanceError>;
    fn request(&self, action: MaintenanceAction) -> Result<MaintenanceOutcome, MaintenanceError>;
}

pub struct MaintenanceService {
    backend: Box<dyn MaintenanceBackend>,
}

impl MaintenanceService {
    pub fn new(backend: Box<dyn MaintenanceBackend>) -> Self {
        Self { backend }
    }

    pub fn inspect(&self) -> Result<MaintenanceSnapshot, MaintenanceError> {
        self.backend
            .inspect()
            .map(MaintenanceSnapshot::sanitize)
            .map_err(sanitize_error)
    }

    pub fn request(
        &self,
        action: MaintenanceAction,
    ) -> Result<MaintenanceOutcome, MaintenanceError> {
        self.backend
            .request(action)
            .map(sanitize_outcome)
            .map_err(sanitize_error)
    }
}

pub fn maintenance_service() -> Arc<MaintenanceService> {
    static SERVICE: OnceLock<Arc<MaintenanceService>> = OnceLock::new();
    Arc::clone(SERVICE.get_or_init(|| Arc::new(MaintenanceService::new(maintenance_backend()))))
}

pub fn maintenance_backend() -> Box<dyn MaintenanceBackend> {
    #[cfg(target_os = "linux")]
    return Box::new(LinuxMaintenance::detect());
    #[cfg(target_os = "windows")]
    return Box::new(WindowsMaintenance);
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    Box::new(UnsupportedMaintenance::detect())
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
struct UnsupportedMaintenance {
    provider: MaintenanceProvider,
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl UnsupportedMaintenance {
    fn detect() -> Self {
        Self {
            provider: MaintenanceProvider::Unsupported {
                platform: std::env::consts::OS.into(),
            },
        }
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
impl MaintenanceBackend for UnsupportedMaintenance {
    fn inspect(&self) -> Result<MaintenanceSnapshot, MaintenanceError> {
        Ok(MaintenanceSnapshot {
            provider: self.provider.clone(),
            updates: unavailable_observation(),
            protection: ProtectionStatus {
                firewall: unavailable_observation(),
                malware_protection: unavailable_observation(),
            },
            permissions: required_unsupported_permissions(),
            secure_storage: unavailable_observation(),
        })
    }

    fn request(&self, _: MaintenanceAction) -> Result<MaintenanceOutcome, MaintenanceError> {
        Ok(MaintenanceOutcome::Unsupported {
            detail: "No supported authoritative mutation provider is connected".into(),
        })
    }
}

#[cfg(target_os = "windows")]
struct WindowsMaintenance;

#[cfg(target_os = "windows")]
impl WindowsMaintenance {
    fn updates(&self) -> Observation<UpdateStatus> {
        let observed_at = SystemTime::now();
        let script = "$ErrorActionPreference='Stop';$s=New-Object -ComObject Microsoft.Update.Session;$q=$s.CreateUpdateSearcher().Search('IsInstalled=0 and IsHidden=0');[Console]::Out.WriteLine('SEARCH_RESULT:'+([int]$q.ResultCode));[Console]::Out.WriteLine('UPDATE_COUNT:'+([int]$q.Updates.Count))";
        match windows_powershell(script) {
            Ok(output) => match parse_windows_update_snapshot(&output) {
                Ok(available) => {
                    let Ok(restart_required) = windows_restart_required() else {
                        return failed_observation(
                            observed_at,
                            "Windows restart requirement could not be queried",
                        );
                    };
                    Observation {
                        state: ObservationState::Current,
                        value: Some(UpdateStatus {
                            available,
                            phase: UpdatePhase::Idle,
                            restart_required,
                            last_successful_check: Some(observed_at),
                        }),
                        observed_at: Some(observed_at),
                        detail: Some(format!(
                            "Windows Update Agent reported {available} available update(s)"
                        )),
                    }
                }
                Err(error) => command_failure_observation(observed_at, "Windows Update", error),
            },
            Err(error) => command_failure_observation(observed_at, "Windows Update", error),
        }
    }

    fn protection(&self) -> ProtectionStatus {
        let observed_at = SystemTime::now();
        let firewall = windows_powershell("$ErrorActionPreference='Stop';Get-NetFirewallProfile | ForEach-Object {[Console]::Out.WriteLine($_.Enabled)}")
            .map(|output| {
                let values = output.lines().filter_map(parse_powershell_bool).collect::<Vec<_>>();
                if !values.is_empty() && values.iter().all(|enabled| *enabled) {
                    ProtectionHealth::Healthy
                } else {
                    ProtectionHealth::AttentionRequired
                }
            });
        let malware = windows_powershell("$ErrorActionPreference='Stop';$s=Get-MpComputerStatus;[Console]::Out.WriteLine($s.AntivirusEnabled);[Console]::Out.WriteLine($s.RealTimeProtectionEnabled)")
            .map(|output| {
                let values = output.lines().filter_map(parse_powershell_bool).collect::<Vec<_>>();
                if values.len() == 2 && values.iter().all(|enabled| *enabled) {
                    ProtectionHealth::Healthy
                } else {
                    ProtectionHealth::AttentionRequired
                }
            });
        ProtectionStatus {
            firewall: protection_observation(observed_at, "Windows Firewall", firewall),
            malware_protection: protection_observation(observed_at, "Microsoft Defender", malware),
        }
    }

    fn permissions(&self) -> Vec<PermissionStatus> {
        [
            PermissionKind::Camera,
            PermissionKind::Microphone,
            PermissionKind::Location,
            PermissionKind::Notifications,
            PermissionKind::ScreenCapture,
        ]
        .into_iter()
        .map(|kind| PermissionStatus {
            kind,
            global_enabled: windows_permission_state(kind),
            per_application_consent: true,
            mutation: PermissionMutation::NativeConsent,
        })
        .collect()
    }
}

#[cfg(target_os = "windows")]
impl MaintenanceBackend for WindowsMaintenance {
    fn inspect(&self) -> Result<MaintenanceSnapshot, MaintenanceError> {
        Ok(MaintenanceSnapshot {
            provider: MaintenanceProvider::WindowsUpdateAndSecurity,
            updates: self.updates(),
            protection: self.protection(),
            permissions: self.permissions(),
            secure_storage: windows_secure_storage_readiness(),
        })
    }

    fn request(&self, action: MaintenanceAction) -> Result<MaintenanceOutcome, MaintenanceError> {
        match action {
            MaintenanceAction::CheckForUpdates => run_windows_check_updates(),
            MaintenanceAction::InstallUpdates => run_windows_install_updates(),
            MaintenanceAction::ScheduleRestart => windows_native_consent(
                "ms-settings:windowsupdate-restartoptions",
                "Windows owns restart scheduling",
            ),
            MaintenanceAction::SetPermission(kind, _)
            | MaintenanceAction::OpenNativePermissionSettings(kind) => windows_native_consent(
                windows_permission_uri(kind),
                "Windows owns per-application consent",
            ),
            MaintenanceAction::RecoverSecureStorage => windows_native_consent(
                "ms-settings:signinoptions",
                "Windows owns account and credential recovery",
            ),
        }
    }
}

#[cfg(target_os = "linux")]
struct LinuxMaintenance {
    distribution: String,
    packagekit: bool,
}

#[cfg(target_os = "linux")]
impl LinuxMaintenance {
    fn detect() -> Self {
        Self {
            distribution: linux_distribution_id(),
            packagekit: executable_on_path("pkcon"),
        }
    }

    fn updates(&self) -> Observation<UpdateStatus> {
        if !self.packagekit {
            return Observation::unsupported(format!(
                "{} has no PackageKit command provider",
                self.distribution
            ));
        }
        let observed_at = SystemTime::now();
        match super::peripherals::bounded_command_output(
            "pkcon",
            &["get-updates", "--plain", "--noninteractive"],
            Duration::from_secs(2),
            64 * 1024,
        ) {
            Ok(output) if output.status.success() => {
                let available = packagekit_update_count(&String::from_utf8_lossy(&output.stdout));
                Observation {
                    state: ObservationState::Current,
                    value: Some(UpdateStatus {
                        available,
                        phase: UpdatePhase::Idle,
                        restart_required: linux_restart_required(),
                        last_successful_check: Some(observed_at),
                    }),
                    observed_at: Some(observed_at),
                    detail: Some(format!(
                        "PackageKit reported {available} available update(s)"
                    )),
                }
            }
            Ok(output) => Observation {
                state: ObservationState::Failed,
                value: None,
                observed_at: Some(observed_at),
                detail: Some(format!("PackageKit query failed with {}", output.status)),
            },
            Err(error) => Observation {
                state: if error.kind() == std::io::ErrorKind::PermissionDenied {
                    ObservationState::PermissionDenied
                } else {
                    ObservationState::Failed
                },
                value: None,
                observed_at: Some(observed_at),
                detail: Some(format!("PackageKit could not start: {error}")),
            },
        }
    }

    fn firewall(&self) -> Observation<ProtectionHealth> {
        let observed_at = SystemTime::now();
        if executable_on_path("firewall-cmd") {
            return command_health("firewall-cmd", &["--state"], observed_at, |output| {
                if output.trim() == "running" {
                    ProtectionHealth::Healthy
                } else {
                    ProtectionHealth::AttentionRequired
                }
            });
        }
        if executable_on_path("ufw") {
            return command_health("ufw", &["status"], observed_at, |output| {
                if output
                    .lines()
                    .any(|line| line.trim().eq_ignore_ascii_case("Status: active"))
                {
                    ProtectionHealth::Healthy
                } else {
                    ProtectionHealth::AttentionRequired
                }
            });
        }
        Observation::unsupported("No supported firewall status provider is installed")
    }

    fn secure_storage(&self) -> Observation<SecureStorageReadiness> {
        let observed_at = SystemTime::now();
        match linux_secure_storage_readiness() {
            Ok(Some(readiness)) => Observation {
                state: ObservationState::Current,
                detail: Some(linux_secure_storage_detail(&readiness).into()),
                value: Some(readiness),
                observed_at: Some(observed_at),
            },
            Ok(None) => Observation {
                state: ObservationState::Current,
                value: Some(SecureStorageReadiness::Unavailable),
                observed_at: Some(observed_at),
                detail: Some("Secret Service has no session owner".into()),
            },
            Err(error) => Observation {
                state: ObservationState::Failed,
                value: None,
                observed_at: Some(observed_at),
                detail: Some(format!(
                    "Secret Service readiness could not be queried: {error}"
                )),
            },
        }
    }
}

#[cfg(target_os = "linux")]
fn linux_secure_storage_readiness()
-> Result<Option<SecureStorageReadiness>, Box<dyn std::error::Error>> {
    const SERVICE: &str = "org.freedesktop.secrets";
    const SERVICE_PATH: &str = "/org/freedesktop/secrets";
    const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
    const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";

    let address = zbus::Address::session()?;
    let connection = crate::bounded_dbus::connect_blocking(
        address,
        crate::bounded_dbus::Limits::ACCESSIBILITY,
        Duration::from_millis(300),
    )
    .map_err(std::io::Error::other)?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&connection)?;
    let name = zbus::names::BusName::try_from(SERVICE)?;
    if !dbus.name_has_owner(name)? {
        return Ok(None);
    }

    let service =
        zbus::blocking::Proxy::new(&connection, SERVICE, SERVICE_PATH, SERVICE_INTERFACE)?;
    let collection: zbus::zvariant::OwnedObjectPath = service.call("ReadAlias", &("default",))?;
    if collection.as_str() == "/" {
        return Ok(Some(SecureStorageReadiness::RecoveryRequired));
    }

    let collection = zbus::blocking::Proxy::new(
        &connection,
        SERVICE,
        collection.as_str(),
        COLLECTION_INTERFACE,
    )?;
    let locked: bool = collection.get_property("Locked")?;
    Ok(Some(if locked {
        SecureStorageReadiness::Locked
    } else {
        SecureStorageReadiness::Ready
    }))
}

#[cfg(target_os = "linux")]
const fn linux_secure_storage_detail(readiness: &SecureStorageReadiness) -> &'static str {
    match readiness {
        SecureStorageReadiness::Ready => "Secret Service default collection is ready",
        SecureStorageReadiness::Locked => "Secret Service default collection is locked",
        SecureStorageReadiness::RecoveryRequired => {
            "Secret Service has no default collection; provider recovery is required"
        }
        SecureStorageReadiness::Unavailable => "Secret Service is unavailable",
    }
}

#[cfg(target_os = "linux")]
impl MaintenanceBackend for LinuxMaintenance {
    fn inspect(&self) -> Result<MaintenanceSnapshot, MaintenanceError> {
        Ok(MaintenanceSnapshot {
            provider: if self.packagekit {
                MaintenanceProvider::LinuxPackageKit {
                    distribution: self.distribution.clone(),
                }
            } else {
                MaintenanceProvider::LinuxUnsupported {
                    distribution: self.distribution.clone(),
                }
            },
            updates: self.updates(),
            protection: ProtectionStatus {
                firewall: self.firewall(),
                malware_protection: Observation::unsupported(
                    "No supported Linux malware-protection authority is connected",
                ),
            },
            permissions: required_unsupported_permissions(),
            secure_storage: self.secure_storage(),
        })
    }

    fn request(&self, action: MaintenanceAction) -> Result<MaintenanceOutcome, MaintenanceError> {
        match action {
            MaintenanceAction::CheckForUpdates if self.packagekit => {
                run_packagekit(&["refresh", "force", "--noninteractive"])
            }
            MaintenanceAction::InstallUpdates if self.packagekit => {
                run_packagekit(&["update", "--noninteractive"])
            }
            MaintenanceAction::CheckForUpdates | MaintenanceAction::InstallUpdates => {
                Ok(MaintenanceOutcome::Unsupported {
                    detail: format!("{} has no PackageKit provider", self.distribution),
                })
            }
            MaintenanceAction::ScheduleRestart => Ok(MaintenanceOutcome::Unsupported {
                detail: "Linux restart scheduling has no connected provider".into(),
            }),
            MaintenanceAction::SetPermission(_, _)
            | MaintenanceAction::OpenNativePermissionSettings(_) => {
                Ok(MaintenanceOutcome::Unsupported {
                    detail:
                        "This Linux desktop exposes no supported global permission mutation API"
                            .into(),
                })
            }
            MaintenanceAction::RecoverSecureStorage => Ok(MaintenanceOutcome::Unsupported {
                detail: "Secret Service recovery remains owned by the configured provider".into(),
            }),
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_powershell(script: &str) -> Result<String, MaintenanceError> {
    const DEFAULT_DEADLINE: Duration = Duration::from_secs(30);
    let context = WINDOWS_MAINTENANCE_BOUND.with(|slot| slot.borrow().clone());
    let deadline = context
        .as_ref()
        .map_or_else(|| Instant::now() + DEFAULT_DEADLINE, |value| value.deadline);
    let cancelled = context.map(|value| value.cancelled);
    windows_powershell_contained(script, deadline, cancelled.as_deref())
}

#[cfg(target_os = "windows")]
const WINDOWS_POWERSHELL_OUTPUT_LIMIT: usize = 64 * 1024;

#[cfg(target_os = "windows")]
#[derive(Clone)]
struct WindowsMaintenanceBound {
    deadline: Instant,
    cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
}

#[cfg(target_os = "windows")]
thread_local! {
    static WINDOWS_MAINTENANCE_BOUND: RefCell<Option<WindowsMaintenanceBound>> = const { RefCell::new(None) };
}

/// Run the production Windows maintenance inspection under one absolute
/// deadline and a live authority check. Every PowerShell process is suspended
/// until it belongs to a kill-on-close job, so timeout and revocation cannot
/// leave a helper or one of its descendants running.
#[cfg(target_os = "windows")]
pub fn inspect_windows_maintenance_bounded(
    deadline: Instant,
    cancelled: Arc<dyn Fn() -> bool + Send + Sync>,
) -> Result<MaintenanceSnapshot, MaintenanceError> {
    if Instant::now() >= deadline || cancelled() {
        return Err(MaintenanceError {
            class: MaintenanceFailureClass::Cancelled,
            detail: "Windows maintenance inspection was cancelled".into(),
        });
    }
    let previous = WINDOWS_MAINTENANCE_BOUND.with(|slot| {
        slot.replace(Some(WindowsMaintenanceBound {
            deadline,
            cancelled,
        }))
    });
    struct Restore(Option<WindowsMaintenanceBound>);
    impl Drop for Restore {
        fn drop(&mut self) {
            WINDOWS_MAINTENANCE_BOUND.with(|slot| {
                slot.replace(self.0.take());
            });
        }
    }
    let _restore = Restore(previous);
    maintenance_service().inspect()
}

#[cfg(target_os = "windows")]
struct WindowsJob(HANDLE);

#[cfg(target_os = "windows")]
impl WindowsJob {
    fn new() -> Result<Self, MaintenanceError> {
        let handle = unsafe { CreateJobObjectW(None, None) }.map_err(windows_provider_error)?;
        let job = Self(handle);
        let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                (&raw const limits).cast(),
                std::mem::size_of_val(&limits) as u32,
            )
        }
        .map_err(windows_provider_error)?;
        Ok(job)
    }
}

#[cfg(target_os = "windows")]
impl Drop for WindowsJob {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_provider_error(error: windows::core::Error) -> MaintenanceError {
    MaintenanceError {
        class: MaintenanceFailureClass::ProviderUnavailable,
        detail: format!("Windows authority is unavailable: {error}"),
    }
}

#[cfg(target_os = "windows")]
fn resume_suspended_process(process_id: u32) -> Result<(), MaintenanceError> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
        .map_err(windows_provider_error)?;
    struct Snapshot(HANDLE);
    impl Drop for Snapshot {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
    let snapshot = Snapshot(snapshot);
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    let mut present = unsafe { Thread32First(snapshot.0, &mut entry).is_ok() };
    while present {
        if entry.th32OwnerProcessID == process_id {
            let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, false, entry.th32ThreadID) }
                .map_err(windows_provider_error)?;
            let resumed = unsafe { ResumeThread(thread) };
            unsafe {
                let _ = CloseHandle(thread);
            }
            if resumed == u32::MAX {
                return Err(windows_provider_error(windows::core::Error::from_thread()));
            }
            return Ok(());
        }
        present = unsafe { Thread32Next(snapshot.0, &mut entry).is_ok() };
    }
    Err(MaintenanceError {
        class: MaintenanceFailureClass::ProviderUnavailable,
        detail: "Windows authority process has no resumable thread".into(),
    })
}

#[cfg(target_os = "windows")]
fn drain_bounded(mut pipe: impl Read, limit: usize) -> Vec<u8> {
    let mut retained = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) | Err(_) => return retained,
            Ok(read) => {
                let remaining = limit.saturating_sub(retained.len());
                retained.extend_from_slice(&buffer[..read.min(remaining)]);
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn windows_powershell_contained(
    script: &str,
    deadline: Instant,
    cancelled: Option<&(dyn Fn() -> bool + Send + Sync)>,
) -> Result<String, MaintenanceError> {
    if Instant::now() >= deadline || cancelled.is_some_and(|check| check()) {
        return Err(MaintenanceError {
            class: MaintenanceFailureClass::Cancelled,
            detail: "Windows authority request expired or was revoked".into(),
        });
    }
    let job = WindowsJob::new()?;
    let mut child = Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .creation_flags(CREATE_SUSPENDED.0 | CREATE_NO_WINDOW.0)
        .spawn()
        .map_err(|error| MaintenanceError {
            class: if error.kind() == std::io::ErrorKind::PermissionDenied {
                MaintenanceFailureClass::Authorization
            } else {
                MaintenanceFailureClass::ProviderUnavailable
            },
            detail: format!("Windows authority could not start: {error}"),
        })?;
    let process = HANDLE(child.as_raw_handle());
    if let Err(error) = unsafe { AssignProcessToJobObject(job.0, process) } {
        let _ = child.kill();
        let _ = child.wait();
        return Err(windows_provider_error(error));
    }
    if let Err(error) = resume_suspended_process(child.id()) {
        let _ = unsafe { TerminateJobObject(job.0, 70) };
        let _ = child.wait();
        return Err(error);
    }
    let stdout = child.stdout.take().expect("piped stdout");
    let stderr = child.stderr.take().expect("piped stderr");
    let stdout_reader =
        std::thread::spawn(move || drain_bounded(stdout, WINDOWS_POWERSHELL_OUTPUT_LIMIT));
    let stderr_reader =
        std::thread::spawn(move || drain_bounded(stderr, WINDOWS_POWERSHELL_OUTPUT_LIMIT));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) if Instant::now() < deadline && !cancelled.is_some_and(|check| check()) => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(None) => {
                let _ = unsafe { TerminateJobObject(job.0, 70) };
                break child.wait().map_err(|_| MaintenanceError {
                    class: MaintenanceFailureClass::Cancelled,
                    detail: "Windows authority request expired or was revoked".into(),
                });
            }
            Err(error) => {
                let _ = unsafe { TerminateJobObject(job.0, 70) };
                let _ = child.wait();
                break Err(MaintenanceError {
                    class: MaintenanceFailureClass::ProviderUnavailable,
                    detail: format!("Windows authority process could not be observed: {error}"),
                });
            }
        }
    }?;
    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    if Instant::now() >= deadline || cancelled.is_some_and(|check| check()) {
        return Err(MaintenanceError {
            class: MaintenanceFailureClass::Cancelled,
            detail: "Windows authority request expired or was revoked".into(),
        });
    }
    if status.success() {
        Ok(String::from_utf8_lossy(&stdout).into_owned())
    } else {
        let diagnostic = format!(
            "{}\n{}",
            String::from_utf8_lossy(&stdout),
            String::from_utf8_lossy(&stderr)
        );
        let class = classify_command_failure(status.code(), &diagnostic);
        Err(MaintenanceError {
            class,
            detail: format!(
                "Windows authority reported a {class:?} failure ({})",
                status
            ),
        })
    }
}

#[cfg(target_os = "windows")]
fn parse_powershell_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

#[cfg(target_os = "windows")]
fn failed_observation<T>(observed_at: SystemTime, detail: &str) -> Observation<T> {
    Observation {
        state: ObservationState::Failed,
        value: None,
        observed_at: Some(observed_at),
        detail: Some(detail.into()),
    }
}

#[cfg(target_os = "windows")]
fn command_failure_observation<T>(
    observed_at: SystemTime,
    authority: &str,
    error: MaintenanceError,
) -> Observation<T> {
    Observation {
        state: if error.class == MaintenanceFailureClass::Authorization {
            ObservationState::PermissionDenied
        } else {
            ObservationState::Failed
        },
        value: None,
        observed_at: Some(observed_at),
        detail: Some(format!("{authority}: {}", error.detail)),
    }
}

#[cfg(target_os = "windows")]
fn protection_observation(
    observed_at: SystemTime,
    authority: &str,
    result: Result<ProtectionHealth, MaintenanceError>,
) -> Observation<ProtectionHealth> {
    match result {
        Ok(health) => Observation {
            state: ObservationState::Current,
            value: Some(health),
            observed_at: Some(observed_at),
            detail: Some(format!("{authority} reported current state")),
        },
        Err(error) => command_failure_observation(observed_at, authority, error),
    }
}

#[cfg(target_os = "windows")]
fn windows_restart_required() -> Result<bool, MaintenanceError> {
    windows_powershell("[Console]::Out.WriteLine((Test-Path 'HKLM:\\SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\WindowsUpdate\\Auto Update\\RebootRequired'))")
        .and_then(|output| parse_powershell_bool(&output).ok_or(MaintenanceError {
            class: MaintenanceFailureClass::Unknown,
            detail: "Windows returned an invalid restart state".into(),
        }))
}

#[cfg(target_os = "windows")]
fn windows_permission_state(kind: PermissionKind) -> Observation<bool> {
    let observed_at = SystemTime::now();
    let capability = match kind {
        PermissionKind::Camera => "webcam",
        PermissionKind::Microphone => "microphone",
        PermissionKind::Location => "location",
        PermissionKind::Notifications => "notifications",
        PermissionKind::ScreenCapture => "graphicsCaptureProgrammatic",
    };
    let script = format!(
        "$ErrorActionPreference='Stop';$p='HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\CapabilityAccessManager\\ConsentStore\\{capability}';if(Test-Path $p){{[Console]::Out.WriteLine((Get-ItemPropertyValue -Path $p -Name Value))}}else{{[Console]::Out.WriteLine('Unavailable')}}"
    );
    match windows_powershell(&script) {
        Ok(value) if value.trim().eq_ignore_ascii_case("Allow") => Observation {
            state: ObservationState::Current,
            value: Some(true),
            observed_at: Some(observed_at),
            detail: Some(format!("Windows {capability} consent is enabled")),
        },
        Ok(value) if value.trim().eq_ignore_ascii_case("Deny") => Observation {
            state: ObservationState::Current,
            value: Some(false),
            observed_at: Some(observed_at),
            detail: Some(format!("Windows {capability} consent is disabled")),
        },
        Ok(_) => Observation::unsupported(format!(
            "Windows does not expose a global {capability} consent value"
        )),
        Err(error) => command_failure_observation(observed_at, "Windows privacy consent", error),
    }
}

#[cfg(target_os = "windows")]
const fn windows_permission_uri(kind: PermissionKind) -> &'static str {
    match kind {
        PermissionKind::Camera => "ms-settings:privacy-webcam",
        PermissionKind::Microphone => "ms-settings:privacy-microphone",
        PermissionKind::Location => "ms-settings:privacy-location",
        PermissionKind::Notifications => "ms-settings:notifications",
        PermissionKind::ScreenCapture => "ms-settings:privacy-screencapture",
    }
}

#[cfg(target_os = "windows")]
fn windows_native_consent(uri: &str, detail: &str) -> Result<MaintenanceOutcome, MaintenanceError> {
    crate::open_external_url(uri).map_err(|error| MaintenanceError {
        class: MaintenanceFailureClass::ProviderUnavailable,
        detail: error,
    })?;
    Ok(MaintenanceOutcome::NativeConsentRequired {
        detail: detail.into(),
    })
}

#[cfg(target_os = "windows")]
fn run_windows_check_updates() -> Result<MaintenanceOutcome, MaintenanceError> {
    let output = windows_powershell(
        "$ErrorActionPreference='Stop';$s=New-Object -ComObject Microsoft.Update.Session;$r=$s.CreateUpdateSearcher().Search('IsInstalled=0 and IsHidden=0');[Console]::Out.WriteLine('SEARCH_RESULT:'+([int]$r.ResultCode))",
    )?;
    parse_windows_check_result(&output)
}

#[cfg(any(target_os = "windows", test))]
fn parse_windows_check_result(output: &str) -> Result<MaintenanceOutcome, MaintenanceError> {
    match output
        .lines()
        .find_map(|line| line.trim().strip_prefix("SEARCH_RESULT:"))
        .and_then(|value| value.parse::<u8>().ok())
    {
        Some(2) => Ok(MaintenanceOutcome::Accepted),
        Some(5) => Err(MaintenanceError {
            class: MaintenanceFailureClass::Cancelled,
            detail: "Windows Update search was aborted".into(),
        }),
        Some(code) => Err(MaintenanceError {
            class: MaintenanceFailureClass::Unknown,
            detail: format!("Windows Update search returned operation result {code}"),
        }),
        None => Err(MaintenanceError {
            class: MaintenanceFailureClass::Unknown,
            detail: "Windows Update returned no terminal search result".into(),
        }),
    }
}

#[cfg(any(target_os = "windows", test))]
fn parse_windows_update_snapshot(output: &str) -> Result<u32, MaintenanceError> {
    parse_windows_check_result(output)?;
    output
        .lines()
        .find_map(|line| line.trim().strip_prefix("UPDATE_COUNT:"))
        .and_then(|value| value.parse::<u32>().ok())
        .ok_or(MaintenanceError {
            class: MaintenanceFailureClass::Unknown,
            detail: "Windows Update returned no valid update count".into(),
        })
}

#[cfg(target_os = "windows")]
fn run_windows_install_updates() -> Result<MaintenanceOutcome, MaintenanceError> {
    let output = windows_powershell(
        "$ErrorActionPreference='Stop';$s=New-Object -ComObject Microsoft.Update.Session;$q=$s.CreateUpdateSearcher().Search('IsInstalled=0 and IsHidden=0');$u=New-Object -ComObject Microsoft.Update.UpdateColl;foreach($i in $q.Updates){if(-not $i.EulaAccepted){[Console]::Out.WriteLine('CONSENT_REQUIRED');return};$null=$u.Add($i)};if($u.Count -eq 0){[Console]::Out.WriteLine('NO_UPDATES');return};$d=$s.CreateUpdateDownloader();$d.Updates=$u;$dr=$d.Download();[Console]::Out.WriteLine('DOWNLOAD_RESULT:'+([int]$dr.ResultCode));if(([int]$dr.ResultCode)-notin 2,3){return};$ready=New-Object -ComObject Microsoft.Update.UpdateColl;foreach($i in $u){if($i.IsDownloaded){$null=$ready.Add($i)}};if($ready.Count -eq 0){[Console]::Out.WriteLine('NO_READY_UPDATES');return};$installer=$s.CreateUpdateInstaller();$installer.Updates=$ready;$ir=$installer.Install();[Console]::Out.WriteLine('INSTALL_RESULT:'+([int]$ir.ResultCode))",
    )?;
    match parse_windows_install_result(&output)? {
        MaintenanceOutcome::NativeConsentRequired { .. } => windows_native_consent(
            "ms-settings:windowsupdate",
            "Windows Update requires license consent",
        ),
        outcome => Ok(outcome),
    }
}

#[cfg(any(target_os = "windows", test))]
fn parse_windows_install_result(output: &str) -> Result<MaintenanceOutcome, MaintenanceError> {
    if output.lines().any(|line| line.trim() == "CONSENT_REQUIRED") {
        return Ok(MaintenanceOutcome::NativeConsentRequired {
            detail: "Windows Update requires license consent".into(),
        });
    }
    if output.lines().any(|line| line.trim() == "NO_UPDATES") {
        return Ok(MaintenanceOutcome::Accepted);
    }
    for (phase, marker) in [
        ("download", "DOWNLOAD_RESULT:"),
        ("installation", "INSTALL_RESULT:"),
    ] {
        if let Some(code) = output
            .lines()
            .find_map(|line| line.trim().strip_prefix(marker))
            .and_then(|value| value.parse::<u8>().ok())
            && code != 2
        {
            return Err(MaintenanceError {
                class: if code == 5 {
                    MaintenanceFailureClass::Cancelled
                } else {
                    MaintenanceFailureClass::Unknown
                },
                detail: format!("Windows Update {phase} returned operation result {code}"),
            });
        }
    }
    if output.lines().any(|line| line.trim() == "NO_READY_UPDATES") {
        return Err(MaintenanceError {
            class: MaintenanceFailureClass::Unknown,
            detail: "Windows Update downloaded no installable updates".into(),
        });
    }
    if output.lines().any(|line| line.trim() == "INSTALL_RESULT:2") {
        Ok(MaintenanceOutcome::Accepted)
    } else {
        Err(MaintenanceError {
            class: MaintenanceFailureClass::Unknown,
            detail: "Windows Update returned no terminal installation result".into(),
        })
    }
}

#[cfg(target_os = "windows")]
fn windows_secure_storage_readiness() -> Observation<SecureStorageReadiness> {
    use windows::Win32::{
        Foundation::{HLOCAL, LocalFree},
        Security::Cryptography::{CRYPT_INTEGER_BLOB, CryptProtectData},
    };
    let observed_at = SystemTime::now();
    let mut marker = *b"Nickel-DPAPI-probe";
    let input = CRYPT_INTEGER_BLOB {
        cbData: marker.len() as u32,
        pbData: marker.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    let result =
        unsafe { CryptProtectData(&raw const input, None, None, None, None, 0, &raw mut output) };
    if !output.pbData.is_null() {
        let _ = unsafe { LocalFree(Some(HLOCAL(output.pbData.cast()))) };
    }
    match result {
        Ok(()) => Observation {
            state: ObservationState::Current,
            value: Some(SecureStorageReadiness::Ready),
            observed_at: Some(observed_at),
            detail: Some("Windows DPAPI protected a non-secret readiness marker".into()),
        },
        Err(error) => failed_observation(
            observed_at,
            &format!("Windows DPAPI readiness probe failed: {error}"),
        ),
    }
}

#[cfg(not(target_os = "windows"))]
fn required_unsupported_permissions() -> Vec<PermissionStatus> {
    [
        PermissionKind::Camera,
        PermissionKind::Microphone,
        PermissionKind::Location,
        PermissionKind::Notifications,
        PermissionKind::ScreenCapture,
    ]
    .into_iter()
    .map(|kind| PermissionStatus {
        kind,
        global_enabled: unavailable_observation(),
        per_application_consent: false,
        mutation: PermissionMutation::Unsupported,
    })
    .collect()
}

#[cfg(target_os = "linux")]
fn packagekit_update_count(output: &str) -> u32 {
    output
        .lines()
        .map(str::trim)
        .filter(|line| {
            !line.is_empty()
                && !line.starts_with("Getting")
                && !line.starts_with("Finished")
                && !line.starts_with("Available")
                && !line.starts_with("There are no")
                && (line.contains(';') || line.contains('\t'))
        })
        .count()
        .min(u32::MAX as usize) as u32
}

#[cfg(target_os = "linux")]
fn linux_restart_required() -> bool {
    Path::new("/run/reboot-required").is_file() || Path::new("/var/run/reboot-required").is_file()
}

#[cfg(target_os = "linux")]
fn command_health(
    program: &str,
    arguments: &[&str],
    observed_at: SystemTime,
    classify: impl FnOnce(&str) -> ProtectionHealth,
) -> Observation<ProtectionHealth> {
    match super::peripherals::bounded_command_output(
        program,
        arguments,
        Duration::from_secs(2),
        64 * 1024,
    ) {
        Ok(output) if output.status.success() => Observation {
            state: ObservationState::Current,
            value: Some(classify(&String::from_utf8_lossy(&output.stdout))),
            observed_at: Some(observed_at),
            detail: Some(format!("{program} reported firewall state")),
        },
        Ok(output) => Observation {
            state: ObservationState::PermissionDenied,
            value: None,
            observed_at: Some(observed_at),
            detail: Some(format!("{program} status failed with {}", output.status)),
        },
        Err(error) => Observation {
            state: ObservationState::Failed,
            value: None,
            observed_at: Some(observed_at),
            detail: Some(format!("{program} status could not start: {error}")),
        },
    }
}

#[cfg(target_os = "linux")]
fn run_packagekit(arguments: &[&str]) -> Result<MaintenanceOutcome, MaintenanceError> {
    let output = super::peripherals::bounded_command_output(
        "pkcon",
        arguments,
        Duration::from_secs(2),
        64 * 1024,
    )
    .map_err(|error| MaintenanceError {
        class: if error.kind() == std::io::ErrorKind::PermissionDenied {
            MaintenanceFailureClass::Authorization
        } else {
            MaintenanceFailureClass::ProviderUnavailable
        },
        detail: format!("PackageKit could not start: {error}"),
    })?;
    if output.status.success() {
        return Ok(MaintenanceOutcome::Accepted);
    }
    let diagnostic = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let class = classify_command_failure(output.status.code(), &diagnostic);
    Err(MaintenanceError {
        class,
        detail: format!(
            "PackageKit reported a {class:?} failure ({})",
            output.status
        ),
    })
}

fn classify_command_failure(exit_code: Option<i32>, diagnostic: &str) -> MaintenanceFailureClass {
    if exit_code == Some(5) {
        return MaintenanceFailureClass::Authorization;
    }
    let diagnostic = diagnostic.to_ascii_lowercase();
    if diagnostic.contains("cancelled") || diagnostic.contains("canceled") {
        MaintenanceFailureClass::Cancelled
    } else if diagnostic.contains("permission denied")
        || diagnostic.contains("access denied")
        || diagnostic.contains("unauthorized")
        || diagnostic.contains("not authorized")
        || diagnostic.contains("authentication")
    {
        MaintenanceFailureClass::Authorization
    } else if diagnostic.contains("network")
        || diagnostic.contains("offline")
        || diagnostic.contains("timed out")
        || diagnostic.contains("timeout")
        || diagnostic.contains("could not resolve")
        || diagnostic.contains("name resolution")
    {
        MaintenanceFailureClass::Network
    } else if diagnostic.contains("policy") || diagnostic.contains("managed by") {
        MaintenanceFailureClass::Policy
    } else if diagnostic.contains("service unavailable")
        || diagnostic.contains("service is not running")
        || diagnostic.contains("provider unavailable")
        || diagnostic.contains("no such service")
    {
        MaintenanceFailureClass::ProviderUnavailable
    } else {
        MaintenanceFailureClass::Unknown
    }
}

#[cfg(target_os = "linux")]
fn linux_distribution_id() -> String {
    std::fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|contents| {
            contents.lines().find_map(|line| {
                line.strip_prefix("ID=")
                    .map(|value| value.trim_matches('"').to_owned())
            })
        })
        .filter(|id| !id.is_empty())
        .unwrap_or_else(|| "unknown-linux".into())
}

#[cfg(target_os = "linux")]
fn executable_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|root| root.join(name).is_file()))
}

fn sanitize_error(mut error: MaintenanceError) -> MaintenanceError {
    error.detail = redact_detail(&error.detail);
    error
}

fn sanitize_outcome(outcome: MaintenanceOutcome) -> MaintenanceOutcome {
    match outcome {
        MaintenanceOutcome::NativeConsentRequired { detail } => {
            MaintenanceOutcome::NativeConsentRequired {
                detail: redact_detail(&detail),
            }
        }
        MaintenanceOutcome::Unsupported { detail } => MaintenanceOutcome::Unsupported {
            detail: redact_detail(&detail),
        },
        MaintenanceOutcome::Rejected { detail } => MaintenanceOutcome::Rejected {
            detail: redact_detail(&detail),
        },
        accepted => accepted,
    }
}

fn redact_detail(detail: &str) -> String {
    detail
        .split_whitespace()
        .take(64)
        .map(|word| {
            let lower = word.to_ascii_lowercase();
            if ["password=", "token=", "secret=", "key="]
                .iter()
                .any(|prefix| lower.starts_with(prefix))
            {
                "<redacted>".to_owned()
            } else {
                word.chars().take(128).collect()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    struct HostileFixture;

    impl MaintenanceBackend for HostileFixture {
        fn inspect(&self) -> Result<MaintenanceSnapshot, MaintenanceError> {
            Ok(MaintenanceSnapshot {
                provider: MaintenanceProvider::Unsupported {
                    platform: "fixture".into(),
                },
                updates: Observation {
                    state: ObservationState::Stale,
                    value: Some(UpdateStatus {
                        available: 0,
                        phase: UpdatePhase::Idle,
                        restart_required: false,
                        last_successful_check: None,
                    }),
                    observed_at: Some(SystemTime::now()),
                    detail: Some("token=private stale cache".into()),
                },
                protection: ProtectionStatus {
                    firewall: Observation {
                        state: ObservationState::Stale,
                        value: Some(ProtectionHealth::Healthy),
                        observed_at: Some(SystemTime::now()),
                        detail: Some("stale provider".into()),
                    },
                    malware_protection: Observation::unsupported("missing"),
                },
                permissions: Vec::new(),
                secure_storage: Observation::unsupported("password=hunter2 unavailable"),
            })
        }

        fn request(&self, _: MaintenanceAction) -> Result<MaintenanceOutcome, MaintenanceError> {
            Err(MaintenanceError {
                class: MaintenanceFailureClass::Authorization,
                detail: "secret=value permission denied".into(),
            })
        }
    }

    struct StatefulFixture {
        camera_enabled: Mutex<bool>,
    }

    impl MaintenanceBackend for StatefulFixture {
        fn inspect(&self) -> Result<MaintenanceSnapshot, MaintenanceError> {
            let camera_enabled = *self.camera_enabled.lock().unwrap();
            Ok(MaintenanceSnapshot {
                provider: MaintenanceProvider::Unsupported {
                    platform: "deterministic-fixture".into(),
                },
                updates: Observation {
                    state: ObservationState::Current,
                    value: Some(UpdateStatus {
                        available: 3,
                        phase: UpdatePhase::Downloading,
                        restart_required: true,
                        last_successful_check: Some(SystemTime::UNIX_EPOCH),
                    }),
                    observed_at: Some(SystemTime::UNIX_EPOCH),
                    detail: Some("fixture progress".into()),
                },
                protection: ProtectionStatus {
                    firewall: Observation {
                        state: ObservationState::Current,
                        value: Some(ProtectionHealth::Healthy),
                        observed_at: Some(SystemTime::UNIX_EPOCH),
                        detail: None,
                    },
                    malware_protection: Observation::unsupported("partial provider"),
                },
                permissions: vec![PermissionStatus {
                    kind: PermissionKind::Camera,
                    global_enabled: Observation {
                        state: ObservationState::Current,
                        value: Some(camera_enabled),
                        observed_at: Some(SystemTime::UNIX_EPOCH),
                        detail: None,
                    },
                    per_application_consent: true,
                    mutation: PermissionMutation::Direct,
                }],
                secure_storage: Observation::unsupported("partial provider"),
            })
        }

        fn request(
            &self,
            action: MaintenanceAction,
        ) -> Result<MaintenanceOutcome, MaintenanceError> {
            match action {
                MaintenanceAction::SetPermission(PermissionKind::Camera, enabled) => {
                    *self.camera_enabled.lock().unwrap() = enabled;
                    Ok(MaintenanceOutcome::Accepted)
                }
                MaintenanceAction::InstallUpdates => Err(MaintenanceError {
                    class: MaintenanceFailureClass::Cancelled,
                    detail: "provider cancelled the transfer".into(),
                }),
                _ => Ok(MaintenanceOutcome::Unsupported {
                    detail: "fixture unsupported action".into(),
                }),
            }
        }
    }

    #[test]
    fn stale_or_absent_providers_cannot_report_healthy_values() {
        let snapshot = MaintenanceService::new(Box::new(HostileFixture))
            .inspect()
            .unwrap();
        assert_eq!(snapshot.updates.state, ObservationState::Stale);
        assert!(snapshot.updates.value.is_some());
        assert_eq!(
            snapshot.updates.detail.as_deref(),
            Some("<redacted> stale cache")
        );
        assert!(
            snapshot
                .secure_storage
                .detail
                .unwrap()
                .starts_with("<redacted>")
        );
        assert_eq!(snapshot.protection.firewall.state, ObservationState::Stale);
        assert_eq!(snapshot.protection.firewall.value, None);
    }

    #[test]
    fn classified_failures_are_redacted_at_the_service_boundary() {
        let error = MaintenanceService::new(Box::new(HostileFixture))
            .request(MaintenanceAction::CheckForUpdates)
            .unwrap_err();
        assert_eq!(error.class, MaintenanceFailureClass::Authorization);
        assert_eq!(error.detail, "<redacted> permission denied");
    }

    #[test]
    fn deterministic_adapter_preserves_progress_restart_partial_and_permission_state() {
        let service = MaintenanceService::new(Box::new(StatefulFixture {
            camera_enabled: Mutex::new(false),
        }));
        let before = service.inspect().unwrap();
        let updates = before.updates.value.unwrap();
        assert_eq!(updates.phase, UpdatePhase::Downloading);
        assert_eq!(updates.available, 3);
        assert!(updates.restart_required);
        assert_eq!(
            before.protection.malware_protection.state,
            ObservationState::Unsupported
        );
        assert_eq!(before.permissions[0].global_enabled.value, Some(false));
        assert_eq!(
            service
                .request(MaintenanceAction::SetPermission(
                    PermissionKind::Camera,
                    true,
                ))
                .unwrap(),
            MaintenanceOutcome::Accepted
        );
        assert_eq!(
            service.inspect().unwrap().permissions[0]
                .global_enabled
                .value,
            Some(true)
        );
    }

    #[test]
    fn deterministic_adapter_keeps_cancellation_classified() {
        let service = MaintenanceService::new(Box::new(StatefulFixture {
            camera_enabled: Mutex::new(false),
        }));
        let error = service
            .request(MaintenanceAction::InstallUpdates)
            .unwrap_err();
        assert_eq!(error.class, MaintenanceFailureClass::Cancelled);
        assert_eq!(error.detail, "provider cancelled the transfer");
    }

    #[test]
    fn deterministic_progress_and_failure_classification_matrix_is_complete() {
        for phase in [
            UpdatePhase::Idle,
            UpdatePhase::Checking,
            UpdatePhase::Downloading,
            UpdatePhase::Installing,
            UpdatePhase::AwaitingRestart,
        ] {
            let mut snapshot = StatefulFixture {
                camera_enabled: Mutex::new(false),
            }
            .inspect()
            .unwrap();
            snapshot.updates.value.as_mut().unwrap().phase = phase;
            assert_eq!(snapshot.sanitize().updates.value.unwrap().phase, phase);
        }

        for class in [
            MaintenanceFailureClass::Network,
            MaintenanceFailureClass::Authorization,
            MaintenanceFailureClass::ProviderUnavailable,
            MaintenanceFailureClass::Cancelled,
            MaintenanceFailureClass::Policy,
            MaintenanceFailureClass::Unknown,
        ] {
            let sanitized = sanitize_error(MaintenanceError {
                class,
                detail: "token=private classified failure".into(),
            });
            assert_eq!(sanitized.class, class);
            assert_eq!(sanitized.detail, "<redacted> classified failure");
        }
    }

    #[test]
    fn native_command_diagnostics_map_to_actionable_failure_classes() {
        for (code, diagnostic, expected) in [
            (Some(5), "", MaintenanceFailureClass::Authorization),
            (
                Some(1),
                "The transaction was cancelled by the user",
                MaintenanceFailureClass::Cancelled,
            ),
            (
                Some(1),
                "Could not resolve update.example.test",
                MaintenanceFailureClass::Network,
            ),
            (
                Some(1),
                "Updates are managed by organization policy",
                MaintenanceFailureClass::Policy,
            ),
            (
                Some(1),
                "Package service is not running",
                MaintenanceFailureClass::ProviderUnavailable,
            ),
            (
                Some(1),
                "unclassified provider error",
                MaintenanceFailureClass::Unknown,
            ),
        ] {
            assert_eq!(classify_command_failure(code, diagnostic), expected);
        }
    }

    #[test]
    fn windows_update_operation_results_cannot_masquerade_as_acceptance() {
        assert_eq!(
            parse_windows_update_snapshot("SEARCH_RESULT:2\nUPDATE_COUNT:17\n").unwrap(),
            17
        );
        for output in [
            "SEARCH_RESULT:3\nUPDATE_COUNT:17\n",
            "SEARCH_RESULT:2\n",
            "SEARCH_RESULT:2\nUPDATE_COUNT:-1\n",
        ] {
            assert_eq!(
                parse_windows_update_snapshot(output).unwrap_err().class,
                MaintenanceFailureClass::Unknown
            );
        }
        assert_eq!(
            parse_windows_check_result("SEARCH_RESULT:2\n").unwrap(),
            MaintenanceOutcome::Accepted
        );
        assert_eq!(
            parse_windows_check_result("SEARCH_RESULT:5\n")
                .unwrap_err()
                .class,
            MaintenanceFailureClass::Cancelled
        );
        for output in [
            "SEARCH_RESULT:3\n",
            "SEARCH_RESULT:4\n",
            "",
            "SEARCH_RESULT:x\n",
        ] {
            assert_eq!(
                parse_windows_check_result(output).unwrap_err().class,
                MaintenanceFailureClass::Unknown
            );
        }
        assert_eq!(
            parse_windows_install_result("NO_UPDATES\n").unwrap(),
            MaintenanceOutcome::Accepted
        );
        assert_eq!(
            parse_windows_install_result("DOWNLOAD_RESULT:2\nINSTALL_RESULT:2\n").unwrap(),
            MaintenanceOutcome::Accepted
        );
        assert!(matches!(
            parse_windows_install_result("CONSENT_REQUIRED\n").unwrap(),
            MaintenanceOutcome::NativeConsentRequired { .. }
        ));

        for (output, class) in [
            ("DOWNLOAD_RESULT:5\n", MaintenanceFailureClass::Cancelled),
            (
                "DOWNLOAD_RESULT:3\nINSTALL_RESULT:2\n",
                MaintenanceFailureClass::Unknown,
            ),
            (
                "DOWNLOAD_RESULT:2\nINSTALL_RESULT:4\n",
                MaintenanceFailureClass::Unknown,
            ),
            ("NO_READY_UPDATES\n", MaintenanceFailureClass::Unknown),
            ("", MaintenanceFailureClass::Unknown),
        ] {
            assert_eq!(
                parse_windows_install_result(output).unwrap_err().class,
                class
            );
        }
    }

    #[test]
    fn default_backend_exposes_every_required_permission_without_implied_consent() {
        let snapshot = maintenance_backend().inspect().unwrap();
        assert_eq!(snapshot.permissions.len(), 5);
        for kind in [
            PermissionKind::Camera,
            PermissionKind::Microphone,
            PermissionKind::Location,
            PermissionKind::Notifications,
            PermissionKind::ScreenCapture,
        ] {
            let permission = snapshot
                .permissions
                .iter()
                .find(|permission| permission.kind == kind)
                .expect("required permission category");
            assert_eq!(
                permission.global_enabled.state,
                ObservationState::Unsupported
            );
            assert_eq!(permission.global_enabled.value, None);
            assert!(!permission.per_application_consent);
            assert_eq!(permission.mutation, PermissionMutation::Unsupported);
        }
        for observation in [
            &snapshot.protection.firewall,
            &snapshot.protection.malware_protection,
        ] {
            if observation.state != ObservationState::Current {
                assert_ne!(observation.value, Some(ProtectionHealth::Healthy));
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn packagekit_plain_output_counts_only_package_records() {
        let output = "Getting updates\nAvailable updates\nnormal;pkg-one;1.2;x86_64;repo\nsecurity\tpkg-two\t3.4\nFinished\n";
        assert_eq!(packagekit_update_count(output), 2);
        assert_eq!(
            packagekit_update_count("Getting updates\nThere are no updates available\nFinished\n"),
            0
        );
    }
}
