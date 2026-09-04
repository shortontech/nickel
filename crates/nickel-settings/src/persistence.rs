use nickel_core::{
    optional_features::OptionalFeatureSettings, shell_settings::ShellSettings,
    wallpaper_settings::WallpaperSettings,
};

pub(super) fn load_optional_feature_settings() -> OptionalFeatureSettings {
    OptionalFeatureSettings::load_default()
}

pub(super) fn try_update_optional_feature_settings(
    update: impl FnOnce(&mut OptionalFeatureSettings),
) -> Result<OptionalFeatureSettings, String> {
    OptionalFeatureSettings::update_default(update).map_err(|error| error.to_string())
}

pub(super) fn load_shell_settings() -> ShellSettings {
    ShellSettings::load_default()
}

pub(super) fn try_save_shell_settings(settings: &ShellSettings) -> Result<(), String> {
    settings.save_default().map_err(|error| error.to_string())
}

pub(super) fn load_wallpaper_settings() -> WallpaperSettings {
    WallpaperSettings::load_default()
}

pub(super) fn try_save_wallpaper_settings(settings: &WallpaperSettings) -> Result<(), String> {
    settings.save_default().map_err(|error| error.to_string())
}
