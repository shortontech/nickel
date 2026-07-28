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
