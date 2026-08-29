//! Shared native platform adapters used by Nickel applications.

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "windows")]
pub use windows::{appearance, apply_window_appearance, path_icon, show_hidden_files};

#[cfg(target_os = "linux")]
pub use linux::path_icon;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn path_icon(_path: &std::path::Path) -> Option<image::RgbaImage> {
    None
}

#[cfg(not(target_os = "windows"))]
pub fn show_hidden_files() -> bool {
    false
}

#[cfg(not(target_os = "windows"))]
pub fn appearance() -> nickel_core::theme::Appearance {
    nickel_core::theme::Appearance::default()
}

#[cfg(not(target_os = "windows"))]
pub fn apply_window_appearance(
    _window: &winit::window::Window,
    _appearance: nickel_core::theme::Appearance,
) {
}

/// Open a directory in the Nickel file browser installed beside the current application.
pub fn open_directory(path: &std::path::Path) -> Result<(), String> {
    nickel_file_command(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start Nickel File: {error}"))
}

fn nickel_file_command(path: &std::path::Path) -> std::process::Command {
    let executable = std::env::current_exe().unwrap_or_else(|_| "nickel".into());
    #[cfg(target_os = "windows")]
    let executable = executable.with_file_name("nickel-file.exe");
    #[cfg(not(target_os = "windows"))]
    let executable = executable.with_file_name("nickel-file");
    let mut command = std::process::Command::new(executable);
    command.arg(path);
    command
}

#[cfg(target_os = "windows")]
pub fn open_external_url(url: &str) -> Result<(), String> {
    use ::windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::PCWSTR,
    };
    use std::os::windows::ffi::OsStrExt;

    let operation = std::ffi::OsStr::new("open")
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let target = std::ffi::OsStr::new(url)
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    // SAFETY: both UTF-16 strings are terminated and remain alive for this
    // synchronous ShellExecuteW call. No returned handle is dereferenced.
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(operation.as_ptr()),
            PCWSTR(target.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!(
            "the system URL handler failed with code {}",
            result.0 as isize
        ))
    }
}

#[cfg(target_os = "linux")]
pub fn open_external_url(url: &str) -> Result<(), String> {
    external_url_command(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start the system URL handler: {error}"))
}

#[cfg(target_os = "linux")]
fn external_url_command(url: &str) -> std::process::Command {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(url);
    command
}

#[cfg(target_os = "macos")]
pub fn open_external_url(url: &str) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(url)
        .spawn()
        .map(|_| ())
        .map_err(|error| format!("could not start the system URL handler: {error}"))
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub fn open_external_url(_url: &str) -> Result<(), String> {
    Err("opening external URLs is unsupported on this platform".into())
}

#[cfg(all(test, target_os = "linux"))]
mod external_url_tests {
    use std::path::Path;

    use super::{external_url_command, nickel_file_command};

    #[test]
    fn delegates_the_exact_url_to_the_desktop_default_handler() {
        let command = external_url_command("https://example.com/a?b=c#d");
        assert_eq!(command.get_program(), "xdg-open");
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["https://example.com/a?b=c#d"]
        );
    }

    #[test]
    fn directories_delegate_to_the_sibling_nickel_file() {
        let command = nickel_file_command(Path::new("/tmp/example folder"));
        assert_eq!(
            Path::new(command.get_program())
                .file_name()
                .and_then(|name| name.to_str()),
            Some("nickel-file")
        );
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            [Path::new("/tmp/example folder").as_os_str()]
        );
    }
}
