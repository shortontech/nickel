use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ShellSettings {
    pub bar_on_all_displays: bool,
    pub all_windows_on_every_bar: bool,
    pub desktop_count: u8,
    pub active_desktop: u8,
}

impl Default for ShellSettings {
    fn default() -> Self {
        Self {
            bar_on_all_displays: true,
            all_windows_on_every_bar: true,
            desktop_count: 4,
            active_desktop: 0,
        }
    }
}

impl ShellSettings {
    pub fn load_default() -> Self {
        Self::load(settings_path()).unwrap_or_default()
    }

    pub fn save_default(self) -> io::Result<()> {
        self.save(settings_path())
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
                _ => {}
            }
        }
        settings.active_desktop = settings
            .active_desktop
            .min(settings.desktop_count.saturating_sub(1));
        Ok(settings)
    }

    pub fn save(self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            format!(
                "bar_on_all_displays={}\nall_windows_on_every_bar={}\ndesktop_count={}\nactive_desktop={}\n",
                self.bar_on_all_displays,
                self.all_windows_on_every_bar,
                self.desktop_count,
                self.active_desktop
            ),
        )
    }
}

fn parse_bool(value: &str) -> bool {
    matches!(value.trim(), "1" | "true" | "yes" | "on")
}

fn settings_path() -> PathBuf {
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return PathBuf::from(local).join("Nickel").join("shell-settings");
    }
    #[cfg(not(target_os = "windows"))]
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME") {
        return PathBuf::from(config).join("nickel").join("shell-settings");
    }
    std::env::temp_dir().join("nickel-shell-settings")
}

#[cfg(test)]
mod tests {
    use super::ShellSettings;

    #[test]
    fn defaults_to_two_display_friendly_bar_and_four_desktops() {
        let settings = ShellSettings::default();
        assert!(settings.bar_on_all_displays);
        assert!(settings.all_windows_on_every_bar);
        assert_eq!(settings.desktop_count, 4);
    }
}
