//! Shared native platform adapters used by Nickel applications.

mod media;
mod platform_contract;

pub use media::{DecodedPreview, PreviewDecodeError, decode_image_preview};
pub use platform_contract::{
    AdapterCapability, ContractEvidence, PLATFORM_CONTRACTS, PlatformContract, PlatformFamily,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDialogOutcome {
    Selected(std::path::PathBuf),
    Cancelled,
    Failed(String),
}

/// Opens the platform-native image chooser.
///
/// Linux uses the XDG portal directly. Windows and macOS report that the
/// capability is unavailable until their native adapters are implemented.
#[cfg(any(target_os = "windows", target_os = "macos"))]
pub fn choose_image_file(
    _callback: Box<dyn Fn(FileDialogOutcome) + Send + 'static>,
) -> Result<(), String> {
    Err(format!(
        "the native image file chooser is not implemented for {}",
        std::env::consts::OS
    ))
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub fn choose_image_file(
    _callback: Box<dyn Fn(FileDialogOutcome) + Send + 'static>,
) -> Result<(), String> {
    Err("the native image file chooser is unsupported on this platform".into())
}

#[cfg(target_os = "linux")]
pub fn choose_image_file(
    callback: Box<dyn Fn(FileDialogOutcome) + Send + 'static>,
) -> Result<(), String> {
    linux::choose_image_file(callback)
}

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "linux")]
mod linux;

#[cfg(target_os = "macos")]
mod macos;

#[cfg(target_os = "windows")]
pub use windows::{appearance, apply_window_appearance, path_icon, show_hidden_files};

#[cfg(target_os = "linux")]
pub use linux::{
    installed_icon_themes, path_icon, path_icon_theme_revision, path_icon_with_theme,
    path_icon_with_theme_at_size,
};

#[cfg(not(target_os = "linux"))]
pub fn installed_icon_themes() -> Vec<String> {
    Vec::new()
}

#[cfg(not(target_os = "linux"))]
pub fn path_icon_theme_revision(_theme: Option<&str>) -> u64 {
    1
}

#[cfg(target_os = "windows")]
pub fn path_icon_with_theme(
    path: &std::path::Path,
    _theme: Option<&str>,
) -> Option<image::RgbaImage> {
    path_icon(path)
}

#[cfg(target_os = "windows")]
pub fn path_icon_with_theme_at_size(
    path: &std::path::Path,
    _theme: Option<&str>,
    physical_size: u32,
) -> Option<image::RgbaImage> {
    windows::path_icon_at_size(path, physical_size)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn path_icon(_path: &std::path::Path) -> Option<image::RgbaImage> {
    None
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn path_icon_with_theme(
    _path: &std::path::Path,
    _theme: Option<&str>,
) -> Option<image::RgbaImage> {
    None
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn path_icon_with_theme_at_size(
    path: &std::path::Path,
    theme: Option<&str>,
    _physical_size: u32,
) -> Option<image::RgbaImage> {
    path_icon_with_theme(path, theme)
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
    linux::open_external_url(url)
}

#[cfg(target_os = "macos")]
pub fn open_external_url(url: &str) -> Result<(), String> {
    macos::open_external_url(url)
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub fn open_external_url(_url: &str) -> Result<(), String> {
    Err("opening external URLs is unsupported on this platform".into())
}

#[cfg(test)]
mod external_url_tests {
    use std::path::Path;

    use super::nickel_file_command;

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
