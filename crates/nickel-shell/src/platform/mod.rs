use crate::model::{TrayItem, WindowId};

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecureStorageState {
    Starting,
    Locked,
    PromptRequired,
    Ready,
    Unavailable,
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
}

#[derive(Clone, Copy)]
pub enum WindowAction {
    Activate,
    Close,
    Maximize,
    Minimize,
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
pub enum GlobalShortcut {
    ShowLauncher,
    HideLauncher,
    LockState { locked: bool },
    ShowRun,
    SwitchNext,
    SwitchPrevious,
    SwitchGroupNext,
    SwitchGroupPrevious,
    CommitSwitch,
    CaptureActiveWindow,
    CaptureActiveWindowToFile,
    ShowScreenshotTool,
    AudioChanged { volume_percent: u8, muted: bool },
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
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, application_icon, applications,
    audio_status, bluetooth_status, capture_active_window, capture_active_window_to_file,
    capture_desktop, capture_pointer, copy_image_to_clipboard, copy_temp_image_path,
    execute_run_command, handle_focused_shortcut, launch_application,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    network_status, paste_text_if_requested, register_session_shell, release_pointer,
    request_secure_storage_retry, secure_storage_state, select_audio_device, send_shell_command,
    set_audio_volume, set_bluetooth_discovery, set_bluetooth_powered, set_wifi_enabled,
    show_window_system_menu, toggle_bluetooth_device, update_panel_fullscreen_state, wallpaper,
};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, application_icon, applications,
    audio_status, bluetooth_status, capture_active_window, capture_active_window_to_file,
    capture_desktop, capture_pointer, configure_context_menu_window, configure_desktop_window,
    configure_launcher_window, configure_panel_window, configure_volume_osd_window,
    copy_image_to_clipboard, copy_temp_image_path, execute_run_command, handle_focused_shortcut,
    launch_application, launcher_has_foreground_focus, launcher_hotkey_receiver,
    launcher_visibility_applied, network_status, paste_text_if_requested, register_session_shell,
    release_panel_window, release_pointer, select_audio_device, send_shell_command,
    set_audio_volume, set_bluetooth_discovery, set_bluetooth_powered, set_wifi_enabled,
    show_window_system_menu, toggle_bluetooth_device, update_panel_fullscreen_state, wallpaper,
};

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
mod unsupported;
#[cfg(target_os = "macos")]
pub use macos::{
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, application_icon, applications,
    audio_status, bluetooth_status, capture_pointer, configure_volume_osd_window,
    execute_run_command, handle_focused_shortcut, launch_application,
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
    NotificationFeed, TrayFeed, WindowFeed, activate_wifi_network, application_icon, applications,
    audio_status, bluetooth_status, capture_pointer, configure_volume_osd_window,
    execute_run_command, handle_focused_shortcut, launch_application,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    network_status, register_session_shell, release_pointer, select_audio_device,
    send_shell_command, set_audio_volume, set_bluetooth_discovery, set_bluetooth_powered,
    set_wifi_enabled, show_window_system_menu, toggle_bluetooth_device,
    update_panel_fullscreen_state, wallpaper,
};
