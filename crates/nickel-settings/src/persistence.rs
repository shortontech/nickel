use nickel_core::{shell_settings::ShellSettings, wallpaper_settings::WallpaperSettings};

pub(super) fn load_shell_settings() -> ShellSettings {
    ShellSettings::load_default()
}

pub(super) fn save_shell_settings(settings: &ShellSettings) {
    let _ = settings.save_default();
}

pub(super) fn load_wallpaper_settings() -> WallpaperSettings {
    WallpaperSettings::load_default()
}

pub(super) fn save_wallpaper_settings(settings: &WallpaperSettings) {
    let _ = settings.save_default();
}
