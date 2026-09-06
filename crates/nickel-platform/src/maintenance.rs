//! Typed platform authority for system maintenance, protection, privacy, and secure storage.
//!
//! Absence is represented explicitly. Consumers must never infer health from a missing provider or
//! a failed/stale observation.

use std::{
    fmt,
    sync::{Arc, OnceLock},
    time::SystemTime,
};

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
        if self.state != ObservationState::Current {
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
    Box::new(UnsupportedMaintenance::detect())
}

struct UnsupportedMaintenance {
    provider: MaintenanceProvider,
}

impl UnsupportedMaintenance {
    fn detect() -> Self {
        #[cfg(target_os = "linux")]
        {
            let distribution = linux_distribution_id();
            let packagekit = executable_on_path("pkcon");
            return Self {
                provider: if packagekit {
                    MaintenanceProvider::LinuxPackageKit { distribution }
                } else {
                    MaintenanceProvider::LinuxUnsupported { distribution }
                },
            };
        }
        #[cfg(target_os = "windows")]
        return Self {
            provider: MaintenanceProvider::WindowsUpdateAndSecurity,
        };
        #[allow(unreachable_code)]
        Self {
            provider: MaintenanceProvider::Unsupported {
                platform: std::env::consts::OS.into(),
            },
        }
    }
}

impl MaintenanceBackend for UnsupportedMaintenance {
    fn inspect(&self) -> Result<MaintenanceSnapshot, MaintenanceError> {
        Ok(MaintenanceSnapshot {
            provider: self.provider.clone(),
            updates: unavailable_observation(),
            protection: ProtectionStatus {
                firewall: unavailable_observation(),
                malware_protection: unavailable_observation(),
            },
            permissions: [
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
            .collect(),
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
                    firewall: Observation::unsupported("missing"),
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
        assert!(snapshot.updates.value.is_none());
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
        assert_ne!(
            snapshot.protection.firewall.state,
            ObservationState::Current
        );
        assert_ne!(
            snapshot.protection.malware_protection.state,
            ObservationState::Current
        );
    }
}
