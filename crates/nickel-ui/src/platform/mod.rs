use crate::model::{TrayItem, WindowId};

pub struct DesktopCapture {
    pub image: image::RgbaImage,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct NetworkStatus {
    pub available: bool,
    pub connected: bool,
    pub name: String,
    pub signal_percent: u32,
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

pub trait TraySource {
    fn snapshot(&self) -> Vec<TrayItem>;
    fn activate(&self, id: &str);
    fn context_menu(&self, id: &str);
}

#[derive(Clone, Copy)]
pub enum WindowAction {
    Activate,
    Close,
    Maximize,
    Minimize,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(target_os = "windows"), allow(dead_code))]
pub enum GlobalShortcut {
    ShowLauncher,
    HideLauncher,
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
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{
    TrayFeed, WindowFeed, application_icon, applications, audio_status, capture_pointer,
    configure_volume_osd_window, execute_run_command, launcher_has_foreground_focus,
    launcher_hotkey_receiver, launcher_visibility_applied, network_status, release_pointer,
    select_audio_device, send_shell_command, set_audio_volume, update_panel_fullscreen_state,
    wallpaper,
};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{
    TrayFeed, WindowFeed, application_icon, applications, audio_status, capture_active_window,
    capture_active_window_to_file, capture_desktop, capture_pointer, configure_context_menu_window,
    configure_desktop_window, configure_launcher_window, configure_panel_window,
    configure_volume_osd_window, copy_image_to_clipboard, copy_temp_image_path,
    execute_run_command, launcher_has_foreground_focus, launcher_hotkey_receiver,
    launcher_visibility_applied, network_status, paste_text_if_requested, release_panel_window,
    release_pointer, select_audio_device, send_shell_command, set_audio_volume,
    update_panel_fullscreen_state, wallpaper,
};

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use unsupported::{
    TrayFeed, WindowFeed, application_icon, applications, audio_status, capture_pointer,
    configure_volume_osd_window, execute_run_command, launcher_has_foreground_focus,
    launcher_hotkey_receiver, launcher_visibility_applied, network_status, release_pointer,
    select_audio_device, send_shell_command, set_audio_volume, update_panel_fullscreen_state,
    wallpaper,
};
