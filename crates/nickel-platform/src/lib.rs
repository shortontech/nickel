//! Shared native platform adapters used by Nickel applications.

mod media;

pub use media::{DecodedPreview, PreviewDecodeError, decode_image_preview};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDialogOutcome {
    Selected(std::path::PathBuf),
    Cancelled,
    Failed(String),
}

/// Opens the platform-native image chooser. Linux uses the XDG portal
/// directly; other platforms use SDL's native dialog adapter.
#[cfg(not(target_os = "linux"))]
pub fn choose_image_file(
    callback: Box<dyn Fn(FileDialogOutcome) + Send + 'static>,
) -> Result<(), String> {
    use sdl3::dialog::{DialogError, DialogFileFilter, show_open_file_dialog};

    let filters = [DialogFileFilter {
        name: "Images",
        pattern: "png;jpg;jpeg;webp;bmp",
    }];
    show_open_file_dialog(
        &filters,
        None::<&std::path::Path>,
        false,
        None,
        Box::new(move |result, _| {
            callback(match result {
                Ok(paths) => paths
                    .into_iter()
                    .next()
                    .map(FileDialogOutcome::Selected)
                    .unwrap_or(FileDialogOutcome::Cancelled),
                Err(DialogError::Canceled) => FileDialogOutcome::Cancelled,
                Err(error) => FileDialogOutcome::Failed(error.to_string()),
            });
        }),
    )
    .map_err(|error| error.to_string())
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
    linux::open_external_url(url)
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
