#[derive(Clone, Copy)]
pub enum ShellCommand {
    Toggle,
    Show,
    Hide,
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
