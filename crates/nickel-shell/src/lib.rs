//! Reusable Nickel shell surfaces.

#[cfg(feature = "workbench-fixtures")]
mod allocation_counter;

#[cfg(feature = "workbench-fixtures")]
pub mod desktop {
    #[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
    pub enum WallpaperPosition {
        Center,
        Tile,
        Stretch,
        Fit,
        Span,
        #[default]
        Fill,
    }

    #[derive(Clone, Debug, Default)]
    pub struct Wallpaper {
        pub image: Option<image::RgbaImage>,
        pub color: [u8; 3],
        pub position: WallpaperPosition,
    }
}

#[cfg(feature = "workbench-fixtures")]
#[allow(clippy::needless_borrow, dead_code)]
mod icons;
#[cfg(feature = "workbench-fixtures")]
#[allow(clippy::manual_is_multiple_of, dead_code)]
mod launcher;
#[cfg(all(feature = "workbench-fixtures", target_os = "linux"))]
#[allow(dead_code)]
mod lock_auth;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod model;
#[cfg(feature = "workbench-fixtures")]
mod notification;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod places;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code, unused_imports)]
mod platform;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod sdl_control_view;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod sdl_gpu;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod sdl_launcher_view;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod sdl_live_shell;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod sdl_notification_view;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod sdl_screenshot;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod sdl_shell;
#[cfg(feature = "workbench-fixtures")]
#[allow(dead_code)]
mod sdl_window_preview;

#[cfg(feature = "workbench-fixtures")]
mod workbench_fixtures;

#[cfg(feature = "workbench-fixtures")]
pub use workbench_fixtures::ShellFixtureProvider;
