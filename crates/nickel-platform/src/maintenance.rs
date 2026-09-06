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
    #[cfg(not(target_os = "linux"))]
    Box::new(UnsupportedMaintenance::detect())
}

#[cfg(not(target_os = "linux"))]
struct UnsupportedMaintenance {
    provider: MaintenanceProvider,
}

#[cfg(not(target_os = "linux"))]
impl UnsupportedMaintenance {
    fn detect() -> Self {
        Self {
            provider: MaintenanceProvider::Unsupported {
                platform: std::env::consts::OS.into(),
            },
        }
    }
}

#[cfg(not(target_os = "linux"))]
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
    let status = std::process::Command::new("pkcon")
        .args(arguments)
        .status()
        .map_err(|error| MaintenanceError {
            class: if error.kind() == std::io::ErrorKind::PermissionDenied {
                MaintenanceFailureClass::Authorization
            } else {
                MaintenanceFailureClass::ProviderUnavailable
            },
            detail: format!("PackageKit could not start: {error}"),
        })?;
    Ok(if status.success() {
        MaintenanceOutcome::Accepted
    } else {
        MaintenanceOutcome::Rejected {
            detail: format!("PackageKit rejected the request with {status}"),
        }
    })
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
