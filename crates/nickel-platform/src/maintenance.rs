//! Typed platform authority for system maintenance, protection, privacy, and secure storage.
//!
//! Absence is represented explicitly. Consumers must never infer health from a missing provider or
//! a failed/stale observation.

use std::{
    fmt,
    sync::{Arc, OnceLock},
    time::SystemTime,
};

#[cfg(target_os = "linux")]
use std::path::Path;

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
        let script = "$ErrorActionPreference='Stop';$s=New-Object -ComObject Microsoft.Update.Session;$q=$s.CreateUpdateSearcher().Search('IsInstalled=0 and IsHidden=0');[Console]::Out.WriteLine($q.Updates.Count)";
        match windows_powershell(script) {
            Ok(output) => match output.trim().parse::<u32>() {
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
                Err(_) => failed_observation(observed_at, "Windows Update returned invalid data"),
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
            MaintenanceAction::CheckForUpdates => run_windows_update_action(
                "$ErrorActionPreference='Stop';$s=New-Object -ComObject Microsoft.Update.Session;$null=$s.CreateUpdateSearcher().Search('IsInstalled=0 and IsHidden=0')",
            ),
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
        match std::process::Command::new("pkcon")
            .args(["get-updates", "--plain", "--noninteractive"])
            .output()
        {
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
        let result = (|| {
            let connection = zbus::blocking::Connection::session().ok()?;
            let proxy = zbus::blocking::fdo::DBusProxy::new(&connection).ok()?;
            let name = zbus::names::BusName::try_from("org.freedesktop.secrets").ok()?;
            proxy.name_has_owner(name).ok()
        })();
        match result {
            Some(true) => Observation {
                state: ObservationState::Current,
                value: Some(SecureStorageReadiness::Ready),
                observed_at: Some(observed_at),
                detail: Some("Secret Service is available for this session".into()),
            },
            Some(false) => Observation {
                state: ObservationState::Current,
                value: Some(SecureStorageReadiness::Unavailable),
                observed_at: Some(observed_at),
                detail: Some("Secret Service has no session owner".into()),
            },
            None => Observation {
                state: ObservationState::Failed,
                value: None,
                observed_at: Some(observed_at),
                detail: Some("Secret Service readiness could not be queried".into()),
            },
        }
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
    let output = std::process::Command::new("powershell.exe")
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            script,
        ])
        .output()
        .map_err(|error| MaintenanceError {
            class: if error.kind() == std::io::ErrorKind::PermissionDenied {
                MaintenanceFailureClass::Authorization
            } else {
                MaintenanceFailureClass::ProviderUnavailable
            },
            detail: format!("Windows authority could not start: {error}"),
        })?;
    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).into_owned())
    } else {
        let diagnostic = format!(
            "{}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let class = classify_command_failure(output.status.code(), &diagnostic);
        Err(MaintenanceError {
            class,
            detail: format!(
                "Windows authority reported a {class:?} failure ({})",
                output.status
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
fn run_windows_update_action(script: &str) -> Result<MaintenanceOutcome, MaintenanceError> {
    windows_powershell(script).map(|_| MaintenanceOutcome::Accepted)
}

#[cfg(target_os = "windows")]
fn run_windows_install_updates() -> Result<MaintenanceOutcome, MaintenanceError> {
    let output = windows_powershell(
        "$ErrorActionPreference='Stop';$s=New-Object -ComObject Microsoft.Update.Session;$q=$s.CreateUpdateSearcher().Search('IsInstalled=0 and IsHidden=0');$u=New-Object -ComObject Microsoft.Update.UpdateColl;foreach($i in $q.Updates){if(-not $i.EulaAccepted){[Console]::Out.WriteLine('CONSENT_REQUIRED');return};$null=$u.Add($i)};if($u.Count -gt 0){$d=$s.CreateUpdateDownloader();$d.Updates=$u;$null=$d.Download();$ready=New-Object -ComObject Microsoft.Update.UpdateColl;foreach($i in $u){if($i.IsDownloaded){$null=$ready.Add($i)}};if($ready.Count -gt 0){$installer=$s.CreateUpdateInstaller();$installer.Updates=$ready;$null=$installer.Install()}}",
    )?;
    if output.lines().any(|line| line.trim() == "CONSENT_REQUIRED") {
        windows_native_consent(
            "ms-settings:windowsupdate",
            "Windows Update requires license consent",
        )
    } else {
        Ok(MaintenanceOutcome::Accepted)
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
    match std::process::Command::new(program).args(arguments).output() {
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
    let output = std::process::Command::new("pkcon")
        .args(arguments)
        .output()
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
