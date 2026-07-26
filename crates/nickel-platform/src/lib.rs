//! Shared native platform adapters used by Nickel applications.

#[cfg(target_os = "windows")]
mod windows;

#[cfg(target_os = "windows")]
pub use windows::{appearance, path_icon, show_hidden_files};

#[cfg(not(target_os = "windows"))]
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
