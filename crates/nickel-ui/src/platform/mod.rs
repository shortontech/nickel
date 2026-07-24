use crate::model::{TrayItem, WindowId};

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
    TrayFeed, WindowFeed, application_icon, applications, execute_run_command,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    send_shell_command, wallpaper,
};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{
    TrayFeed, WindowFeed, application_icon, applications, configure_context_menu_window,
    configure_desktop_window, configure_launcher_window, configure_panel_window,
    execute_run_command, launcher_has_foreground_focus, launcher_hotkey_receiver,
    launcher_visibility_applied, release_panel_window, send_shell_command, wallpaper,
};

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use unsupported::{
    TrayFeed, WindowFeed, application_icon, applications, execute_run_command,
    launcher_has_foreground_focus, launcher_hotkey_receiver, launcher_visibility_applied,
    send_shell_command, wallpaper,
};
