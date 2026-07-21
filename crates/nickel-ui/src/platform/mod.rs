use crate::model::WindowId;

#[derive(Clone, Copy)]
pub enum WindowAction {
    Close,
}

#[derive(Clone, Copy)]
pub enum ShellCommand {
    Toggle,
    Show,
    Hide,
    ShowContextMenu {
        x: i32,
    },
    HideContextMenu,
    WindowAction {
        window: WindowId,
        action: WindowAction,
    },
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::{WindowFeed, applications, send_shell_command};

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::{WindowFeed, applications, send_shell_command};

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
mod unsupported;
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub use unsupported::{WindowFeed, applications, send_shell_command};
