use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::theme::{Appearance, ThemeMode, accent_from_hue, accent_hue};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileIconPreference {
    Nickel,
    System,
}

impl FileIconPreference {
    pub fn default_for_os(os: &str) -> Self {
        if os == "windows" {
            Self::System
        } else {
            Self::Nickel
        }
    }
}

impl Default for FileIconPreference {
    fn default() -> Self {
        Self::default_for_os(std::env::consts::OS)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AnimationLevel {
    Off,
    Reduced,
    #[default]
    Normal,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShellSettings {
    pub bar_on_all_displays: bool,
    pub all_windows_on_every_bar: bool,
    pub desktop_count: u8,
    pub active_desktop: u8,
    pub theme: ThemePreference,
    pub accent_hue: Option<u16>,
    pub accent_intensity: Option<u8>,
    pub reduce_transparency: bool,
    pub animations: AnimationLevel,
    pub file_icon_provider: FileIconPreference,
    /// An explicitly selected Linux icon-theme name. Retained even when the
    /// theme is temporarily unavailable so the platform adapter can recover
    /// automatically when it returns.
    pub file_icon_theme: Option<String>,
    pub idle_dim_seconds: Option<u32>,
    pub idle_lock_seconds: Option<u32>,
    pub idle_suspend_seconds: Option<u32>,
}

impl Default for ShellSettings {
    fn default() -> Self {
        Self {
            bar_on_all_displays: true,
            all_windows_on_every_bar: true,
            desktop_count: 4,
            active_desktop: 0,
            theme: ThemePreference::System,
            accent_hue: None,
            accent_intensity: None,
            reduce_transparency: false,
            animations: AnimationLevel::Normal,
            file_icon_provider: FileIconPreference::default(),
            file_icon_theme: None,
            idle_dim_seconds: Some(300),
            idle_lock_seconds: Some(900),
            idle_suspend_seconds: None,
        }
    }
}

impl ShellSettings {
    pub fn load_default() -> Self {
        settings_path().and_then(Self::load).unwrap_or_default()
    }

    pub fn save_default(&self) -> io::Result<()> {
        self.save(settings_path()?)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        let mut settings = Self::default();
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "bar_on_all_displays" => settings.bar_on_all_displays = parse_bool(value),
                "all_windows_on_every_bar" => settings.all_windows_on_every_bar = parse_bool(value),
                "desktop_count" => {
                    settings.desktop_count = value.trim().parse().unwrap_or(4).clamp(1, 8)
                }
                "active_desktop" => settings.active_desktop = value.trim().parse().unwrap_or(0),
                "theme" => {
                    settings.theme = match value.trim() {
                        "light" => ThemePreference::Light,
                        "dark" => ThemePreference::Dark,
                        _ => ThemePreference::System,
                    }
                }
                "accent_hue" => {
                    settings.accent_hue = value.trim().parse::<u16>().ok().map(|hue| hue.min(359))
                }
                "accent_intensity" => {
                    settings.accent_intensity =
                        value.trim().parse::<u8>().ok().map(|value| value.min(100))
                }
                "reduce_transparency" => settings.reduce_transparency = parse_bool(value),
                "animations" => {
                    settings.animations = match value.trim() {
                        "off" => AnimationLevel::Off,
                        "reduced" => AnimationLevel::Reduced,
                        _ => AnimationLevel::Normal,
                    }
                }
                "file_icon_provider" => {
                    settings.file_icon_provider = match value.trim() {
                        "system" => FileIconPreference::System,
                        _ => FileIconPreference::Nickel,
                    }
                }
                "file_icon_theme" => {
                    settings.file_icon_theme = match value.trim() {
                        "" | "system" => None,
                        theme => Some(theme.to_owned()),
                    }
                }
                "idle_dim_seconds" => settings.idle_dim_seconds = parse_timeout(value),
                "idle_lock_seconds" => settings.idle_lock_seconds = parse_timeout(value),
                "idle_suspend_seconds" => settings.idle_suspend_seconds = parse_timeout(value),
                _ => {}
            }
        }
        settings.active_desktop = settings
            .active_desktop
            .min(settings.desktop_count.saturating_sub(1));
        Ok(settings)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            format!(
                "bar_on_all_displays={}\nall_windows_on_every_bar={}\ndesktop_count={}\nactive_desktop={}\ntheme={}\naccent_hue={}\naccent_intensity={}\nreduce_transparency={}\nanimations={}\nfile_icon_provider={}\nfile_icon_theme={}\nidle_dim_seconds={}\nidle_lock_seconds={}\nidle_suspend_seconds={}\n",
                self.bar_on_all_displays,
                self.all_windows_on_every_bar,
                self.desktop_count,
                self.active_desktop,
                match self.theme {
                    ThemePreference::System => "system",
                    ThemePreference::Light => "light",
                    ThemePreference::Dark => "dark",
                },
                self.accent_hue
                    .map(|hue| hue.to_string())
                    .unwrap_or_else(|| "system".to_owned()),
                self.accent_intensity
                    .map(|value| value.to_string())
                    .unwrap_or_else(|| "system".to_owned()),
                self.reduce_transparency,
                match self.animations {
                    AnimationLevel::Off => "off",
                    AnimationLevel::Reduced => "reduced",
                    AnimationLevel::Normal => "normal",
                },
                match self.file_icon_provider {
                    FileIconPreference::Nickel => "nickel",
                    FileIconPreference::System => "system",
                },
                self.file_icon_theme.as_deref().unwrap_or("system"),
                format_timeout(self.idle_dim_seconds),
                format_timeout(self.idle_lock_seconds),
                format_timeout(self.idle_suspend_seconds),
            ),
        )
    }

    pub fn resolve_appearance(&self, system: Appearance) -> Appearance {
        Appearance {
            mode: match self.theme {
                ThemePreference::System => system.mode,
                ThemePreference::Light => ThemeMode::Light,
                ThemePreference::Dark => ThemeMode::Dark,
            },
            accent: self
                .accent_hue
                .map(accent_from_hue)
                .unwrap_or(system.accent),
            intensity: self.accent_intensity.unwrap_or(system.intensity),
        }
    }

    pub fn displayed_hue(&self, system: Appearance) -> u16 {
        self.accent_hue.unwrap_or_else(|| accent_hue(system.accent))
    }

    pub fn displayed_intensity(&self, system: Appearance) -> u8 {
        self.accent_intensity.unwrap_or(system.intensity)
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.trim(), "1" | "true" | "yes" | "on")
}

fn parse_timeout(value: &str) -> Option<u32> {
    match value.trim() {
        "off" | "none" | "disabled" | "0" => None,
        value => value.parse::<u32>().ok().filter(|seconds| *seconds > 0),
    }
}

fn format_timeout(timeout: Option<u32>) -> String {
    timeout.map_or_else(|| "off".to_owned(), |seconds| seconds.to_string())
}

fn settings_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local).join("Nickel").join("shell-settings"));
    }
    #[cfg(target_os = "windows")]
    return Err(io::Error::new(
        io::ErrorKind::NotFound,
        "LOCALAPPDATA is not set",
    ));
    #[cfg(not(target_os = "windows"))]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_CONFIG_HOME and HOME are not set",
                )
            })?;
        Ok(config.join("nickel").join("shell-settings"))
    }
}

#[cfg(test)]
mod tests {
    use super::{AnimationLevel, FileIconPreference, ShellSettings, ThemePreference};

    #[test]
    fn defaults_to_two_display_friendly_bar_and_four_desktops() {
        let settings = ShellSettings::default();
        assert!(settings.bar_on_all_displays);
        assert!(settings.all_windows_on_every_bar);
        assert_eq!(settings.desktop_count, 4);
        assert_eq!(settings.active_desktop, 0);
        assert_eq!(settings.theme, ThemePreference::System);
        assert_eq!(settings.accent_hue, None);
        assert_eq!(settings.accent_intensity, None);
        assert!(!settings.reduce_transparency);
        assert_eq!(settings.animations, AnimationLevel::Normal);
        assert_eq!(
            settings.file_icon_provider,
            if cfg!(target_os = "windows") {
                FileIconPreference::System
            } else {
                FileIconPreference::Nickel
            }
        );
        assert_eq!(settings.file_icon_theme, None);
        assert_eq!(settings.idle_dim_seconds, Some(300));
        assert_eq!(settings.idle_lock_seconds, Some(900));
        assert_eq!(settings.idle_suspend_seconds, None);
    }

    #[test]
    fn file_icon_defaults_are_independent_of_the_build_host() {
        assert_eq!(
            FileIconPreference::default_for_os("windows"),
            FileIconPreference::System
        );
        assert_eq!(
            FileIconPreference::default_for_os("linux"),
            FileIconPreference::Nickel
        );
        assert_eq!(
            FileIconPreference::default_for_os("macos"),
            FileIconPreference::Nickel
        );
    }

    #[test]
    fn save_and_load_preserves_every_user_preference() {
        let path = std::env::temp_dir().join(format!(
            "nickel-shell-settings-appearance-{}",
            std::process::id()
        ));
        let settings = ShellSettings {
            bar_on_all_displays: false,
            all_windows_on_every_bar: false,
            desktop_count: 7,
            active_desktop: 5,
            theme: super::ThemePreference::Dark,
            accent_hue: Some(271),
            accent_intensity: Some(63),
            reduce_transparency: true,
            animations: AnimationLevel::Off,
            file_icon_provider: FileIconPreference::System,
            file_icon_theme: Some("Papirus-Dark".to_owned()),
            idle_dim_seconds: Some(90),
            idle_lock_seconds: Some(240),
            idle_suspend_seconds: Some(1_800),
        };

        settings.save(&path).expect("save settings");
        let loaded = ShellSettings::load(&path).expect("load settings");
        std::fs::remove_file(path).expect("remove settings fixture");

        assert_eq!(loaded, settings);
    }
}
