use crate::model::{TrayItem, WindowId};

pub trait TraySource {
    fn snapshot(&self) -> Vec<TrayItem>;
    fn activate(&self, id: &str);
}

#[derive(Clone, Copy)]
pub enum WindowAction {
    Activate,
    Close,
    Maximize,
    Minimize,
}

#[derive(Clone, Copy)]
pub enum ShellCommand {
    Toggle,
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
pub use linux::{TrayFeed, WindowFeed, applications, send_shell_command};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{TrayFeed, WindowFeed, applications, send_shell_command};

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use unsupported::{TrayFeed, WindowFeed, applications, send_shell_command};
