use crate::model::{TrayItem, WindowId};
#[cfg(target_os = "windows")]
pub(crate) use windows::remote_observation;
pub(crate) mod status_mailbox;
use nickel_input::global::{ShortcutCapability, ShortcutOwnership};

#[cfg(target_os = "linux")]
#[derive(Clone, Debug, PartialEq)]
pub enum ShellTestRequest {
    SemanticTarget {
        request_id: u64,
        target: nickel_session_protocol::ShellSemanticTarget,
        reply_path: std::path::PathBuf,
    },
    RuntimeDiagnostics {
        request_id: u64,
        reply_path: std::path::PathBuf,
    },
}

/// Returns the native client-area size when the platform can query it.
///
/// `fallback` is supplied by the window runtime so this boundary remains
/// independent of whichever crate owns the event loop and native window.
pub fn surface_size(
    _window: &impl raw_window_handle::HasWindowHandle,
    fallback: (u32, u32),
) -> (u32, u32) {
    #[cfg(target_os = "windows")]
    {
        windows::surface_size(_window, fallback)
    }
    #[cfg(not(target_os = "windows"))]
    {
        fallback
    }
}

pub fn renders_desktop_background() -> bool {
    cfg!(any(target_os = "linux", target_os = "windows"))
}

pub struct DesktopCapture {
    pub image: image::RgbaImage,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct WifiNetworkStatus {
    pub id: String,
    pub name: String,
    pub signal_percent: u32,
    pub connected: bool,
    pub saved: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkStatus {
    pub available: bool,
    pub enabled: bool,
    pub connected: bool,
    pub name: String,
    pub signal_percent: u32,
    pub networks: Vec<WifiNetworkStatus>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BluetoothDeviceStatus {
    pub id: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct BluetoothStatus {
    pub available: bool,
    pub powered: bool,
    pub discovering: bool,
    pub devices: Vec<BluetoothDeviceStatus>,
}

pub(crate) const CONNECTIVITY_DEVICE_LIMIT: usize = 256;
pub(crate) const CONNECTIVITY_TEXT_LIMIT: usize = 512;

#[derive(Clone, Debug)]
pub(crate) struct ConnectivityRefresh {
    pub network: NetworkStatus,
    pub bluetooth: BluetoothStatus,
    pub partial: bool,
}

pub(crate) fn bound_connectivity_refresh(
    mut network: NetworkStatus,
    mut bluetooth: BluetoothStatus,
) -> ConnectivityRefresh {
    let mut partial = network.networks.len() > CONNECTIVITY_DEVICE_LIMIT
        || bluetooth.devices.len() > CONNECTIVITY_DEVICE_LIMIT;
    network.networks.truncate(CONNECTIVITY_DEVICE_LIMIT);
    bluetooth.devices.truncate(CONNECTIVITY_DEVICE_LIMIT);
    let bounded = |value: &mut String, partial: &mut bool| {
        if value.chars().count() > CONNECTIVITY_TEXT_LIMIT {
            *value = value.chars().take(CONNECTIVITY_TEXT_LIMIT).collect();
            *partial = true;
        }
    };
    bounded(&mut network.name, &mut partial);
    for entry in &mut network.networks {
        bounded(&mut entry.id, &mut partial);
        bounded(&mut entry.name, &mut partial);
    }
    for entry in &mut bluetooth.devices {
        bounded(&mut entry.id, &mut partial);
        bounded(&mut entry.name, &mut partial);
    }
    ConnectivityRefresh {
        network,
        bluetooth,
        partial,
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AudioDeviceStatus {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct AudioStatus {
    pub available: bool,
    pub devices: Vec<AudioDeviceStatus>,
    pub volume_percent: u8,
    pub muted: bool,
}

pub(crate) const AUDIO_DEVICE_LIMIT: usize = 128;
pub(crate) const AUDIO_TEXT_LIMIT: usize = 512;

#[derive(Clone, Debug)]
pub(crate) struct AudioRefresh {
    pub audio: AudioStatus,
    pub partial: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PeripheralRefresh {
    pub printers_available: bool,
    pub volumes_available: bool,
    pub filesystems_available: bool,
    pub printer_count: u32,
    pub volume_count: u32,
    pub filesystem_count: u32,
    pub partial: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct MaintenanceRefresh {
    pub maintenance_available: bool,
    pub updates_available: Option<u32>,
    pub restart_required: Option<bool>,
    pub firewall_healthy: Option<bool>,
    pub malware_protection_healthy: Option<bool>,
    pub known_permission_states: u32,
    pub secure_storage_status_available: bool,
    pub partial: bool,
}

fn summarize_maintenance(snapshot: nickel_platform::MaintenanceSnapshot) -> MaintenanceRefresh {
    use nickel_platform::{ObservationState, ProtectionHealth};
    let current = |state| state == ObservationState::Current;
    let updates = current(snapshot.updates.state)
        .then_some(snapshot.updates.value.as_ref())
        .flatten();
    let firewall = current(snapshot.protection.firewall.state)
        .then_some(snapshot.protection.firewall.value.as_ref())
        .flatten();
    let malware = current(snapshot.protection.malware_protection.state)
        .then_some(snapshot.protection.malware_protection.value.as_ref())
        .flatten();
    let secure_storage_status_available =
        current(snapshot.secure_storage.state) && snapshot.secure_storage.value.is_some();
    let known_permission_states = snapshot
        .permissions
        .iter()
        .filter(|status| {
            status.global_enabled.state == ObservationState::Current
                && status.global_enabled.value.is_some()
        })
        .count()
        .min(u32::MAX as usize) as u32;
    let partial = updates.is_none()
        || firewall.is_none()
        || malware.is_none()
        || !secure_storage_status_available
        || known_permission_states as usize != snapshot.permissions.len();
    MaintenanceRefresh {
        maintenance_available: updates.is_some()
            || firewall.is_some()
            || malware.is_some()
            || secure_storage_status_available
            || known_permission_states != 0,
        updates_available: updates.map(|status| status.available),
        restart_required: updates.map(|status| status.restart_required),
        firewall_healthy: firewall.map(|health| *health == ProtectionHealth::Healthy),
        malware_protection_healthy: malware.map(|health| *health == ProtectionHealth::Healthy),
        known_permission_states,
        secure_storage_status_available,
        partial,
    }
}

pub(crate) fn refresh_maintenance_status() -> Result<MaintenanceRefresh, String> {
    #[cfg(target_os = "windows")]
    return Err(
        "bounded Windows maintenance diagnostics are unavailable until native job containment is installed"
            .into(),
    );

    #[cfg(not(target_os = "windows"))]
    nickel_platform::maintenance_service()
        .inspect()
        .map(summarize_maintenance)
        .map_err(|error| error.to_string())
}

fn summarize_peripherals(snapshot: nickel_platform::PeripheralSnapshot) -> PeripheralRefresh {
    let printers_available = snapshot.printers.is_ok();
    let volumes_available = snapshot.volumes.is_ok();
    let filesystems_available = snapshot.filesystems.is_ok();
    let printer_count = snapshot
        .printers
        .as_ref()
        .map_or(0, |values| values.len().min(u32::MAX as usize) as u32);
    let volume_count = snapshot
        .volumes
        .as_ref()
        .map_or(0, |values| values.len().min(u32::MAX as usize) as u32);
    let filesystem_count = snapshot
        .filesystems
        .as_ref()
        .map_or(0, |values| values.len().min(u32::MAX as usize) as u32);
    PeripheralRefresh {
        printers_available,
        volumes_available,
        filesystems_available,
        printer_count,
        volume_count,
        filesystem_count,
        partial: snapshot.omitted_printers != 0
            || snapshot.omitted_jobs != 0
            || snapshot.omitted_volumes != 0
            || snapshot.omitted_filesystems != 0,
    }
}

pub(crate) fn refresh_peripheral_status() -> Result<PeripheralRefresh, String> {
    nickel_platform::peripheral_service()
        .inspect()
        .map(summarize_peripherals)
        .map_err(|error| error.to_string())
}

pub(crate) fn bound_audio_refresh(mut audio: AudioStatus) -> AudioRefresh {
    let mut partial = audio.devices.len() > AUDIO_DEVICE_LIMIT;
    audio.devices.truncate(AUDIO_DEVICE_LIMIT);
    for device in &mut audio.devices {
        for value in [&mut device.id, &mut device.name] {
            if value.chars().count() > AUDIO_TEXT_LIMIT {
                *value = value.chars().take(AUDIO_TEXT_LIMIT).collect();
                partial = true;
            }
        }
    }
    AudioRefresh { audio, partial }
}

/// Replaceable platform state consumed by the in-process shell. Intermediate
/// snapshots may coalesce; ordered commands must never use this status channel.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SystemStatusUpdate {
    Network(NetworkStatus),
    Bluetooth(BluetoothStatus),
    Audio(AudioStatus),
    AudioWithActivity {
        status: AudioStatus,
        activity: AudioActivity,
    },
    ShellSettingsChanged,
}

/// Bounded feedback facts from snapshots replaced before the consumer drained.
/// Value activity is reset on availability changes so reconnect alone is silent.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AudioActivity {
    pub value_changed: bool,
    pub availability_changed: bool,
}

pub fn system_status_receiver() -> status_mailbox::StatusReceiver {
    #[cfg(target_os = "linux")]
    {
        linux::system_status_receiver()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (_sender, receiver) = status_mailbox::channel();
        receiver
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LaunchError {
    EmptyCommand,
    InvalidQuotes,
    MissingTarget(String),
    NotFound(String),
    PathNotFound(String),
    AccessDenied(String),
    NoAssociation(String),
    Platform(String),
}

fn spawn_deferred_terminal(arguments: &[String]) -> Result<std::process::Child, LaunchError> {
    let (program, rest) = arguments.split_first().ok_or(LaunchError::EmptyCommand)?;
    let current =
        std::env::current_exe().map_err(|error| LaunchError::Platform(error.to_string()))?;
    let terminal_name = if cfg!(target_os = "windows") {
        "nickel-terminal.exe"
    } else {
        "nickel-terminal"
    };
    let mut executable = current.with_file_name(terminal_name);
    if current
        .parent()
        .is_some_and(|parent| parent.file_name() == Some("deps".as_ref()))
    {
        executable = current
            .parent()
            .and_then(std::path::Path::parent)
            .unwrap_or_else(|| std::path::Path::new("."))
            .join(terminal_name);
    }
    if !executable.is_file() {
        return Err(LaunchError::MissingTarget("Nickel Terminal".into()));
    }
    let mut command = std::process::Command::new(executable);
    command
        .arg("--defer-window-for-child-ms")
        // The platform authority decides at 100 ms. This larger bound is only a fail-safe for
        // transporting its timestamped window or expiry watermark to the terminal process.
        .arg("250")
        .arg("--")
        .arg(program)
        .args(rest)
        .env_remove("NICKEL_SESSION_CONTROL")
        .env_remove("NICKEL_SESSION_TOKEN")
        .env_remove("NICKEL_SHELL_TEST_CONTROL");
    command.stdin(std::process::Stdio::piped());
    command
        .spawn()
        .map_err(|error| LaunchError::Platform(error.to_string()))
}

/// A failure while making a request over the shell/session control channel.
///
/// This intentionally contains categories rather than OS error strings.  The
/// latter can contain socket paths, usernames, or other machine-specific data
/// that should not be propagated into shell status or normal logs.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionRequestError {
    MissingControlSocket,
    MissingSessionToken,
    SocketBind,
    SocketConfiguration,
    Encoding,
    Send,
    Receive,
    ReceiveTimeout,
    Decoding,
    Authorization {
        message: String,
    },
    ServerRejection {
        code: nickel_session_protocol::ErrorCode,
        message: String,
    },
    UnexpectedResponse {
        expected: &'static str,
    },
}

impl std::fmt::Display for SessionRequestError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingControlSocket => formatter.write_str("control socket is not configured"),
            Self::MissingSessionToken => {
                formatter.write_str("session capability is not configured")
            }
            Self::SocketBind => formatter.write_str("could not bind a control reply socket"),
            Self::SocketConfiguration => {
                formatter.write_str("could not configure the control reply socket")
            }
            Self::Encoding => formatter.write_str("could not encode the control request"),
            Self::Send => formatter.write_str("could not send the control request"),
            Self::Receive => formatter.write_str("could not receive the control response"),
            Self::ReceiveTimeout => {
                formatter.write_str("timed out waiting for the control response")
            }
            Self::Decoding => formatter.write_str("could not decode the control response"),
            Self::Authorization { message } => {
                write!(formatter, "control authorization failed: {message}")
            }
            Self::ServerRejection { code, message } => {
                write!(formatter, "control request rejected ({code:?}): {message}")
            }
            Self::UnexpectedResponse { expected } => {
                write!(
                    formatter,
                    "unexpected control response (expected {expected})"
                )
            }
        }
    }
}

impl SessionRequestError {
    pub fn stage(&self) -> &'static str {
        match self {
            Self::MissingControlSocket | Self::MissingSessionToken => "configuration",
            Self::SocketBind => "socket-bind",
            Self::SocketConfiguration => "socket-configuration",
            Self::Encoding => "encoding",
            Self::Send => "send",
            Self::Receive => "receive",
            Self::ReceiveTimeout => "receive-timeout",
            Self::Decoding => "decoding",
            Self::Authorization { .. } => "authorization",
            Self::ServerRejection { .. } => "server-rejection",
            Self::UnexpectedResponse { .. } => "unexpected-response",
        }
    }
}

impl std::error::Error for SessionRequestError {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecureStorageState {
    Starting,
    Locked,
    PromptRequired,
    Ready,
    Unavailable,
    UnavailableReason(nickel_session_protocol::SecureStorageUnavailableReason),
    ControlUnavailable,
}

pub fn application_requires_secure_storage(application: &crate::model::Application) -> bool {
    let identity = format!("{} {}", application.id(), application.name()).to_ascii_lowercase();
    ["chrome", "chromium", "signal"]
        .iter()
        .any(|marker| identity.contains(marker))
}

pub trait TraySource {
    fn snapshot(&self) -> Vec<TrayItem>;
    fn activate(&self, id: &str);
    fn context_menu(&self, id: &str);
}

pub trait NotificationSource {
    fn snapshot(&self) -> Option<crate::notification::DesktopNotification>;
    fn history(&self) -> Vec<crate::notification::DesktopNotification> {
        self.snapshot().into_iter().collect()
    }
    fn dismiss(&self, id: u32);
    fn invoke(&self, id: u32, action_key: &str);
}

#[derive(Clone, Copy)]
pub enum WindowAction {
    Activate,
    Close,
    Maximize,
    Minimize,
    Fullscreen,
    SnapLeading,
    SnapTrailing,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeedStatus {
    Loading,
    Ready,
    Disconnected,
    Failed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeedState<T> {
    Loading,
    Ready(T),
    Disconnected,
    Failed,
}

impl<T> FeedState<T> {
    pub fn status(&self) -> FeedStatus {
        match self {
            Self::Loading => FeedStatus::Loading,
            Self::Ready(_) => FeedStatus::Ready,
            Self::Disconnected => FeedStatus::Disconnected,
            Self::Failed => FeedStatus::Failed,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SessionAction {
    RestartShell,
    Lock,
    Suspend,
    LogOut,
    Reboot,
    PowerOff,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct WorkspaceSummary {
    pub id: u64,
    pub active: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub enum ScreenshotAction {
    InteractiveRegion,
    ActiveWindow,
    InteractiveRegionToFile,
    ActiveWindowToFile,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub enum GlobalShortcut {
    ReloadShellSettings,
    ToggleLauncher,
    ShowLauncher,
    HideLauncher,
    LockState {
        locked: bool,
    },
    ShowRun,
    OpenFiles,
    OpenSettings,
    ShowControlCenter,
    ShowNotifications,
    ShowDesktop,
    ProjectDisplays,
    ShowWindowMenu,
    SwitchNext,
    SwitchPrevious,
    SwitchGroupNext,
    SwitchGroupPrevious,
    CommitSwitch,
    CancelSwitch,
    Screenshot(ScreenshotAction),
    AudioChanged {
        available: bool,
        volume_percent: u8,
        muted: bool,
        output_name: Option<String>,
    },
    ConsumerControl(nickel_session_protocol::ConsumerControl),
}

pub struct GlobalShortcutFeed {
    pub receiver: std::sync::mpsc::Receiver<GlobalShortcut>,
    pub ownership: ShortcutOwnership,
    pub capability: ShortcutCapability,
}

impl GlobalShortcutFeed {
    pub fn unavailable(reason: nickel_input::global::UnavailableReason) -> Self {
        let (_sender, receiver) = std::sync::mpsc::channel();
        Self {
            receiver,
            ownership: ShortcutOwnership::OperatingSystem,
            capability: ShortcutCapability::Unavailable(reason),
        }
    }
}

#[derive(Clone)]
pub enum ShellCommand {
    Show,
    ShowFromController,
    Hide,
    LogOut,
    SessionAction(SessionAction),
    Unlock,
    ShowContextMenu {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    },
    ShowPreview {
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        windows: Vec<WindowId>,
    },
    ShowTaskSwitcher {
        width: i32,
        height: i32,
        windows: Vec<WindowId>,
    },
    #[cfg(target_os = "linux")]
    FocusControlCenter,
    #[cfg(target_os = "linux")]
    FocusPreview,
    #[cfg(target_os = "linux")]
    FocusContextMenu,
    #[cfg(target_os = "linux")]
    FocusScreenshot,
    #[cfg(target_os = "linux")]
    RestoreApplicationFocus,
    #[cfg(target_os = "linux")]
    SetShellRoleVisible {
        role: nickel_session_protocol::ShellRole,
        visible: bool,
    },
    #[cfg(target_os = "linux")]
    ShowAnchoredShellRole {
        role: nickel_session_protocol::ShellRole,
        anchor: nickel_session_protocol::ShellPopoverAnchor,
    },
    HideContextMenu,
    HighlightWindow(WindowId),
    ClearWindowHighlight,
    WindowAction {
        window: WindowId,
        action: WindowAction,
    },
    CreateWorkspace,
    ToggleShowDesktop,
    RemoveWorkspace(u64),
    SwitchWorkspace(u64),
    MoveWindowToWorkspace {
        window: WindowId,
        workspace: u64,
    },
    MoveWindowToDisplay {
        window: WindowId,
        output: String,
    },
    ApplyOutputs(nickel_session_protocol::OutputLayout),
}

#[cfg(test)]
mod tests {
    use crate::model::Application;

    #[test]
    fn credential_dependent_applications_are_identified_for_launch_gating() {
        let application = |id: &str, name: &str| {
            Application::new(id.into(), name.into(), None, None, Some(vec![id.into()]))
        };
        assert!(super::application_requires_secure_storage(&application(
            "google-chrome.desktop",
            "Google Chrome"
        )));
        assert!(super::application_requires_secure_storage(&application(
            "org.signal.Signal.desktop",
            "Signal"
        )));
        assert!(!super::application_requires_secure_storage(&application(
            "org.nickel.Terminal.desktop",
            "Nickel Terminal"
        )));
    }

    #[test]
    fn connectivity_refresh_bounds_cardinality_and_text_without_exposing_payloads() {
        let long = "x".repeat(super::CONNECTIVITY_TEXT_LIMIT + 1);
        let network = super::NetworkStatus {
            name: long.clone(),
            networks: (0..super::CONNECTIVITY_DEVICE_LIMIT + 1)
                .map(|_| super::WifiNetworkStatus {
                    id: long.clone(),
                    name: long.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let bluetooth = super::BluetoothStatus {
            devices: (0..super::CONNECTIVITY_DEVICE_LIMIT + 1)
                .map(|_| super::BluetoothDeviceStatus {
                    id: long.clone(),
                    name: long.clone(),
                    ..Default::default()
                })
                .collect(),
            ..Default::default()
        };
        let refresh = super::bound_connectivity_refresh(network, bluetooth);
        assert!(refresh.partial);
        assert_eq!(
            refresh.network.networks.len(),
            super::CONNECTIVITY_DEVICE_LIMIT
        );
        assert_eq!(
            refresh.bluetooth.devices.len(),
            super::CONNECTIVITY_DEVICE_LIMIT
        );
        assert!(refresh.network.name.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT);
        assert!(refresh.network.networks.iter().all(|entry| {
            entry.id.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT
                && entry.name.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT
        }));
        assert!(refresh.bluetooth.devices.iter().all(|entry| {
            entry.id.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT
                && entry.name.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT
        }));
    }

    #[test]
    fn audio_refresh_bounds_cardinality_and_text() {
        let long = "x".repeat(super::AUDIO_TEXT_LIMIT + 1);
        let audio = super::AudioStatus {
            available: true,
            devices: (0..super::AUDIO_DEVICE_LIMIT + 1)
                .map(|_| super::AudioDeviceStatus {
                    id: long.clone(),
                    name: long.clone(),
                    is_default: false,
                })
                .collect(),
            ..Default::default()
        };
        let refresh = super::bound_audio_refresh(audio);
        assert!(refresh.partial);
        assert_eq!(refresh.audio.devices.len(), super::AUDIO_DEVICE_LIMIT);
        assert!(refresh.audio.devices.iter().all(|device| {
            device.id.chars().count() <= super::AUDIO_TEXT_LIMIT
                && device.name.chars().count() <= super::AUDIO_TEXT_LIMIT
        }));
    }

    #[test]
    fn peripheral_refresh_retains_only_coarse_counts_and_availability() {
        let refresh = super::summarize_peripherals(nickel_platform::PeripheralSnapshot {
            provider: nickel_platform::PeripheralProvider::Unsupported {
                platform: "sensitive provider detail".into(),
            },
            printers: Ok(vec![nickel_platform::Printer {
                id: "private-printer-id".into(),
                name: "Private printer".into(),
                is_default: true,
                state: nickel_platform::PrinterState::Ready,
                jobs: vec![nickel_platform::PrintJob {
                    id: "private-job-id".into(),
                    name: "Secret document".into(),
                    state: nickel_platform::PrintJobState::Pending,
                }],
            }]),
            volumes: Err("private volume path".into()),
            filesystems: Ok(vec![nickel_platform::FilesystemUsage {
                id: "private-filesystem-id".into(),
                name: "Private filesystem".into(),
                mount_path: "/private/path".into(),
                capacity_bytes: 10,
                available_bytes: 5,
            }]),
            omitted_printers: 0,
            omitted_jobs: 2,
            omitted_volumes: 0,
            omitted_filesystems: 0,
        });
        assert_eq!(
            refresh,
            super::PeripheralRefresh {
                printers_available: true,
                volumes_available: false,
                filesystems_available: true,
                printer_count: 1,
                volume_count: 0,
                filesystem_count: 1,
                partial: true,
            }
        );
    }

    #[test]
    fn maintenance_refresh_retains_health_without_provider_or_error_details() {
        use std::time::SystemTime;
        fn current<T>(value: T) -> nickel_platform::Observation<T> {
            nickel_platform::Observation {
                state: nickel_platform::ObservationState::Current,
                value: Some(value),
                observed_at: Some(SystemTime::now()),
                detail: Some("private provider detail".into()),
            }
        }
        let refresh = super::summarize_maintenance(nickel_platform::MaintenanceSnapshot {
            provider: nickel_platform::MaintenanceProvider::LinuxPackageKit {
                distribution: "private distribution".into(),
            },
            updates: current(nickel_platform::UpdateStatus {
                available: 3,
                phase: nickel_platform::UpdatePhase::Idle,
                restart_required: true,
                last_successful_check: Some(SystemTime::now()),
            }),
            protection: nickel_platform::ProtectionStatus {
                firewall: current(nickel_platform::ProtectionHealth::Healthy),
                malware_protection: nickel_platform::Observation {
                    state: nickel_platform::ObservationState::Failed,
                    value: None,
                    observed_at: Some(SystemTime::now()),
                    detail: Some("private failure".into()),
                },
            },
            permissions: vec![nickel_platform::PermissionStatus {
                kind: nickel_platform::PermissionKind::Camera,
                global_enabled: current(true),
                per_application_consent: true,
                mutation: nickel_platform::PermissionMutation::NativeConsent,
            }],
            secure_storage: current(nickel_platform::SecureStorageReadiness::Locked),
        });
        assert_eq!(
            refresh,
            super::MaintenanceRefresh {
                maintenance_available: true,
                updates_available: Some(3),
                restart_required: Some(true),
                firewall_healthy: Some(true),
                malware_protection_healthy: None,
                known_permission_states: 1,
                secure_storage_status_available: true,
                partial: true,
            }
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "queries live PackageKit, firewall, and Secret Service providers"]
    fn live_maintenance_refresh_returns_only_coarse_status() {
        let _ = super::refresh_maintenance_status().expect("live maintenance refresh");
    }

    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "queries live CUPS, UDisks2, and filesystem providers"]
    fn live_peripheral_refresh_returns_only_coarse_status() {
        let refresh = super::refresh_peripheral_status().expect("live peripheral refresh");
        assert!(refresh.printer_count <= 512);
        assert!(refresh.volume_count <= 512);
        assert!(refresh.filesystem_count <= 256);
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    GuardedControlOrigin, GuardedControlOriginOwner, GuardedControlOutcome, NotificationFeed,
    TrayFeed, WindowFeed, activate_wifi_network, application_discovery, application_icon,
    applications, audio_status, bluetooth_status, capture_active_window,
    capture_active_window_to_file, capture_desktop, capture_pointer, configure_on_screen_keyboard,
    configured_primary_output, copy_image_to_clipboard, copy_temp_image_path,
    deliver_on_screen_keyboard_input, execute_run_command, handle_consumer_control,
    handle_focused_shortcut, launch_application, launch_session_application,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    network_status, on_screen_keyboard_snapshot, prepare_audio_environment, projection_outputs,
    register_session_shell, register_shell_surface, release_pointer, request_secure_storage_retry,
    respond_runtime_diagnostics, respond_semantic_action, respond_semantic_target,
    secure_storage_state, select_audio_device, semantic_target_receiver, send_shell_command,
    set_audio_volume, set_bluetooth_discovery, set_bluetooth_powered, set_wifi_enabled,
    shell_readiness, show_window_system_menu, submit_guarded_control, toggle_bluetooth_device,
    update_panel_fullscreen_state, wallpaper,
};
#[cfg(target_os = "linux")]
pub(crate) use linux::{
    capture_output, installed_application_signatures, prepare_application_discovery,
    publish_application_discovery, refresh_audio_status, refresh_connectivity_status,
    run_signature_diagnostics, save_temp_image, shell_command_payload,
};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, active_display_point,
    application_discovery, application_icon, applications, audio_status, bluetooth_status,
    capture_active_window, capture_active_window_to_file, capture_desktop, capture_pointer,
    configure_context_menu_window, configure_desktop_window, configure_launcher_window,
    configure_panel_window, configure_preview_window, configure_screenshot_window,
    configure_volume_osd_window, configured_primary_output, copy_image_to_clipboard,
    copy_temp_image_path, execute_run_command, handle_consumer_control, handle_focused_shortcut,
    launch_application, launcher_has_foreground_focus, launcher_hotkey_receiver,
    launcher_visibility_applied, network_status, register_session_shell, release_panel_window,
    release_pointer, select_audio_device, send_shell_command, set_audio_volume,
    set_bluetooth_discovery, set_bluetooth_powered, set_wifi_enabled, show_window_system_menu,
    toggle_bluetooth_device, update_panel_fullscreen_state, wallpaper,
};

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use unsupported::{
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, active_display_point,
    application_discovery, application_icon, applications, audio_status, bluetooth_status,
    capture_pointer, configure_volume_osd_window, configured_primary_output, execute_run_command,
    handle_consumer_control, handle_focused_shortcut, launch_application,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    network_status, register_session_shell, release_pointer, select_audio_device,
    send_shell_command, set_audio_volume, set_bluetooth_discovery, set_bluetooth_powered,
    set_wifi_enabled, show_window_system_menu, toggle_bluetooth_device,
    update_panel_fullscreen_state, wallpaper,
};

#[cfg(target_os = "windows")]
pub(crate) use windows::{
    expose_trusted_control_window, prepare_application_discovery, prepare_trusted_control_window,
    publish_application_discovery, refresh_audio_status, refresh_connectivity_status,
    verify_trusted_control_window,
};

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) use unsupported::refresh_audio_status;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) use unsupported::refresh_connectivity_status;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) use unsupported::{prepare_application_discovery, publish_application_discovery};
