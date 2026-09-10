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
    publish_application_discovery, run_signature_diagnostics, save_temp_image,
    shell_command_payload,
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
    publish_application_discovery, verify_trusted_control_window,
};

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) use unsupported::{prepare_application_discovery, publish_application_discovery};
