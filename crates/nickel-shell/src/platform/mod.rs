use crate::model::{TrayItem, WindowId};
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

pub fn surface_size(window: &sdl3::video::Window) -> (u32, u32) {
    #[cfg(target_os = "windows")]
    {
        return windows::surface_size(window);
    }
    #[cfg(not(target_os = "windows"))]
    {
        window.size_in_pixels()
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
    fn dismiss(&self, id: u32);
    fn invoke(&self, id: u32, action_key: &str);
}

#[derive(Clone, Copy)]
pub enum WindowAction {
    Activate,
    Close,
    Maximize,
    Minimize,
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub enum GlobalShortcut {
    ToggleLauncher,
    ShowLauncher,
    HideLauncher,
    LockState { locked: bool },
    ShowRun,
    SwitchNext,
    SwitchPrevious,
    SwitchGroupNext,
    SwitchGroupPrevious,
    CommitSwitch,
    Screenshot(ScreenshotAction),
    AudioChanged { volume_percent: u8, muted: bool },
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
    Hide,
    LogOut,
    SessionAction(SessionAction),
    Unlock,
    ShowContextMenu {
        x: i32,
        width: i32,
        height: i32,
    },
    ShowPreview {
        x: i32,
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
    HideContextMenu,
    HighlightWindow(WindowId),
    ClearWindowHighlight,
    WindowAction {
        window: WindowId,
        action: WindowAction,
    },
    CreateWorkspace,
    RemoveWorkspace(u64),
    SwitchWorkspace(u64),
    MoveWindowToWorkspace {
        window: WindowId,
        workspace: u64,
    },
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
            "org.kde.konsole.desktop",
            "Konsole"
        )));
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, application_discovery,
    application_icon, applications, audio_status, bluetooth_status, capture_active_window,
    capture_active_window_to_file, capture_desktop, capture_pointer, copy_image_to_clipboard,
    copy_temp_image_path, execute_run_command, handle_focused_shortcut, launch_application,
    launch_session_application, launcher_has_foreground_focus, launcher_hotkey_receiver,
    launcher_visibility_applied, network_status, paste_text_if_requested, register_session_shell,
    release_pointer, request_secure_storage_retry, respond_runtime_diagnostics,
    respond_semantic_action, respond_semantic_target, secure_storage_state, select_audio_device,
    semantic_target_receiver, send_shell_command, set_audio_volume, set_bluetooth_discovery,
    set_bluetooth_powered, set_wifi_enabled, shell_readiness, show_window_system_menu,
    toggle_bluetooth_device, update_panel_fullscreen_state, wallpaper,
};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, application_discovery,
    application_icon, applications, audio_status, bluetooth_status, capture_active_window,
    capture_active_window_to_file, capture_desktop, capture_pointer, configure_context_menu_window,
    configure_desktop_window, configure_launcher_window, configure_panel_window,
    configure_screenshot_window, configure_volume_osd_window, copy_image_to_clipboard,
    copy_temp_image_path, execute_run_command, handle_focused_shortcut, launch_application,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    network_status, paste_text_if_requested, register_session_shell, release_panel_window,
    release_pointer, select_audio_device, send_shell_command, set_audio_volume,
    set_bluetooth_discovery, set_bluetooth_powered, set_wifi_enabled, show_window_system_menu,
    toggle_bluetooth_device, update_panel_fullscreen_state, wallpaper,
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod unsupported;
#[cfg(target_os = "macos")]
pub use macos::{
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, application_discovery,
    application_icon, applications, audio_status, bluetooth_status, capture_pointer,
    configure_volume_osd_window, execute_run_command, handle_focused_shortcut, launch_application,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    network_status, register_session_shell, release_pointer, select_audio_device,
    send_shell_command, set_audio_volume, set_bluetooth_discovery, set_bluetooth_powered,
    set_wifi_enabled, show_window_system_menu, toggle_bluetooth_device,
    update_panel_fullscreen_state, wallpaper,
};

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
pub use unsupported::{
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, application_discovery,
    application_icon, applications, audio_status, bluetooth_status, capture_pointer,
    configure_volume_osd_window, execute_run_command, handle_focused_shortcut, launch_application,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    network_status, register_session_shell, release_pointer, select_audio_device,
    send_shell_command, set_audio_volume, set_bluetooth_discovery, set_bluetooth_powered,
    set_wifi_enabled, show_window_system_menu, toggle_bluetooth_device,
    update_panel_fullscreen_state, wallpaper,
};
