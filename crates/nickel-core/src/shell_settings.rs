use nickel_storage::{StagedWrite, config_path, read_regular_file, stage_write};
use std::{
    io,
    path::{Path, PathBuf},
};

use crate::theme::{Appearance, ThemeMode, accent_from_hue, accent_hue};

pub const SHELL_SETTINGS_VERSION: u8 = 1;
const MAX_SETTINGS_BYTES: usize = 64 * 1024;
pub const MAX_CONFIGURED_WORKSPACES: u8 = 10;

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
    /// Product-owned launch preferences, distinct from OS associations.
    pub preferred_terminal: Option<String>,
    pub preferred_file_manager: Option<String>,
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
            preferred_terminal: None,
            preferred_file_manager: None,
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
        let contents = Self::read_contents(path.as_ref())?.ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "shell settings file is missing")
        })?;
        Ok(Self::parse(&contents))
    }

    /// Transactions must not replace unreadable or newer configuration with
    /// defaults. A missing file is the only default allowed on the write path.
    pub fn load_for_update(path: impl AsRef<Path>) -> io::Result<Self> {
        let Some(contents) = Self::read_contents(path.as_ref())? else {
            return Ok(Self::default());
        };
        if contents
            .lines()
            .filter_map(|line| line.split_once('='))
            .any(|(key, value)| {
                key.trim() == "version"
                    && value.trim().parse::<u8>().ok() != Some(SHELL_SETTINGS_VERSION)
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "unsupported shell settings version",
            ));
        }
        Ok(Self::parse(&contents))
    }

    // Both ordinary reloads and transaction preparation use the same bounded
    // regular-file reader. Disk I/O remains OS-bound; FIFOs and final symlinks
    // are rejected without entering a blocking content read.
    fn read_contents(path: &Path) -> io::Result<Option<String>> {
        read_regular_file(path, MAX_SETTINGS_BYTES)?
            .map(|bytes| {
                String::from_utf8(bytes).map_err(|_| {
                    io::Error::new(io::ErrorKind::InvalidData, "invalid shell settings UTF-8")
                })
            })
            .transpose()
    }

    fn parse(contents: &str) -> Self {
        if let Some(version) = contents
            .lines()
            .find_map(|line| line.strip_prefix("version="))
            && version.trim().parse::<u8>().ok() != Some(SHELL_SETTINGS_VERSION)
        {
            return Self::default();
        }
        let mut settings = Self::default();
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "bar_on_all_displays" => settings.bar_on_all_displays = parse_bool(value),
                "all_windows_on_every_bar" => settings.all_windows_on_every_bar = parse_bool(value),
                "desktop_count" => {
                    settings.desktop_count = value
                        .trim()
                        .parse()
                        .unwrap_or(4)
                        .clamp(1, MAX_CONFIGURED_WORKSPACES)
                }
                // Legacy files may contain this transient value. Workspace selection is not
                // restored across sessions, so it is intentionally ignored.
                "active_desktop" => {}
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
                "preferred_terminal" => settings.preferred_terminal = parse_optional(value),
                "preferred_file_manager" => settings.preferred_file_manager = parse_optional(value),
                "idle_dim_seconds" => settings.idle_dim_seconds = parse_timeout(value),
                "idle_lock_seconds" => settings.idle_lock_seconds = parse_timeout(value),
                "idle_suspend_seconds" => settings.idle_suspend_seconds = parse_timeout(value),
                _ => {}
            }
        }
        settings
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        self.save_checked(path, || Ok(()))
    }

    pub fn save_checked(
        &self,
        path: impl AsRef<Path>,
        check_commit: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        let path = path.as_ref();
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        self.stage(path)?.commit(check_commit)
    }

    /// Prepare serialized settings without changing the destination. The caller
    /// owns admission, current-state validation and authority at commit time.
    pub fn stage(&self, path: impl AsRef<Path>) -> io::Result<StagedWrite> {
        let path = path.as_ref();
        stage_write(
            path,
            format!(
                "version={}\nbar_on_all_displays={}\nall_windows_on_every_bar={}\ndesktop_count={}\ntheme={}\naccent_hue={}\naccent_intensity={}\nreduce_transparency={}\nanimations={}\nfile_icon_provider={}\nfile_icon_theme={}\npreferred_terminal={}\npreferred_file_manager={}\nidle_dim_seconds={}\nidle_lock_seconds={}\nidle_suspend_seconds={}\n",
                SHELL_SETTINGS_VERSION,
                self.bar_on_all_displays,
                self.all_windows_on_every_bar,
                self.desktop_count,
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
                self.preferred_terminal.as_deref().unwrap_or("system"),
                self.preferred_file_manager.as_deref().unwrap_or("system"),
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

fn parse_optional(value: &str) -> Option<String> {
    match value.trim() {
        "" | "system" => None,
        value => Some(value.to_owned()),
    }
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

/// Canonical path watched by the compositor-owned shell for live settings reloads.
pub fn settings_path() -> io::Result<PathBuf> {
    config_path("shell-settings")
}

#[cfg(test)]
mod tests {
    use super::{
        AnimationLevel, FileIconPreference, MAX_CONFIGURED_WORKSPACES, SHELL_SETTINGS_VERSION,
        ShellSettings, ThemePreference,
    };

    #[test]
    fn bounded_settings_load_preserves_missing_and_version_semantics() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("settings");
        assert_eq!(
            ShellSettings::load(&path).unwrap_err().kind(),
            std::io::ErrorKind::NotFound
        );
        assert_eq!(
            ShellSettings::load_for_update(&path).unwrap(),
            ShellSettings::default()
        );
        std::fs::write(&path, "version=99\n").unwrap();
        assert_eq!(
            ShellSettings::load(&path).unwrap(),
            ShellSettings::default()
        );
        assert!(ShellSettings::load_for_update(&path).is_err());
        for contents in [vec![0xff], vec![b'x'; 64 * 1024 + 1]] {
            std::fs::write(&path, contents).unwrap();
            assert!(ShellSettings::load(&path).is_err());
            assert!(ShellSettings::load_for_update(&path).is_err());
        }
    }

    #[test]
    fn nonregular_settings_child_probe() {
        let Some(path) = std::env::var_os("NICKEL_SHELL_SETTINGS_TEST_PATH") else {
            return;
        };
        let result = if std::env::var_os("NICKEL_SHELL_SETTINGS_TEST_UPDATE").is_some() {
            ShellSettings::load_for_update(path)
        } else {
            ShellSettings::load(path)
        };
        assert!(result.is_err());
    }

    #[cfg(unix)]
    #[test]
    fn both_settings_loaders_reject_nonregular_inputs_without_waiting() {
        let root = tempfile::tempdir().unwrap();
        let fifo = root.path().join("fifo");
        use std::os::unix::ffi::OsStrExt;
        let name = std::ffi::CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: the owned terminated path remains live during mkfifo, and
        // creation is restricted to the fixture's private temporary directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let ordinary = root.path().join("ordinary");
        std::fs::write(&ordinary, "version=1\n").unwrap();
        let link = root.path().join("link");
        std::os::unix::fs::symlink(&ordinary, &link).unwrap();
        let dangling = root.path().join("dangling");
        std::os::unix::fs::symlink(root.path().join("missing"), &dangling).unwrap();
        for update in [false, true] {
            for path in [
                &fifo,
                std::path::Path::new("/dev/null"),
                root.path(),
                &link,
                &dangling,
            ] {
                let mut command = std::process::Command::new(std::env::current_exe().unwrap());
                command
                    .args([
                        "--exact",
                        "shell_settings::tests::nonregular_settings_child_probe",
                    ])
                    .env("NICKEL_SHELL_SETTINGS_TEST_PATH", path)
                    .env_remove("NICKEL_SHELL_SETTINGS_TEST_UPDATE")
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                if update {
                    command.env("NICKEL_SHELL_SETTINGS_TEST_UPDATE", "1");
                }
                let mut child = command.spawn().unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
                loop {
                    if let Some(status) = child.try_wait().unwrap() {
                        assert!(status.success(), "loader update={update} accepted {path:?}");
                        break;
                    }
                    if std::time::Instant::now() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        panic!("loader update={update} blocked on {path:?}");
                    }
                    std::thread::sleep(std::time::Duration::from_millis(10));
                }
            }
        }
    }

    #[test]
    fn transaction_load_preserves_preferences_and_rejects_unreadable_or_newer_files() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell-settings");
        assert_eq!(
            ShellSettings::load_for_update(&path).unwrap(),
            ShellSettings::default()
        );
        let settings = ShellSettings {
            desktop_count: 6,
            idle_lock_seconds: Some(71),
            preferred_terminal: Some("preserve-this-terminal".into()),
            ..ShellSettings::default()
        };
        settings.save(&path).unwrap();
        assert_eq!(ShellSettings::load_for_update(&path).unwrap(), settings);
        for contents in [
            b"version=99\nfuture_security_policy=retain\n".to_vec(),
            b" version = unknown\n".to_vec(),
            vec![0xff],
            vec![b'x'; 64 * 1024 + 1],
        ] {
            std::fs::write(&path, &contents).unwrap();
            assert!(ShellSettings::load_for_update(&path).is_err());
            assert_eq!(std::fs::read(&path).unwrap(), contents);
        }
        assert!(ShellSettings::load_for_update(root.path()).is_err());
    }

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
        assert_eq!(settings.preferred_terminal, None);
        assert_eq!(settings.preferred_file_manager, None);
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
            preferred_terminal: Some("foot".to_owned()),
            preferred_file_manager: Some("dolphin".to_owned()),
            idle_dim_seconds: Some(90),
            idle_lock_seconds: Some(240),
            idle_suspend_seconds: Some(1_800),
        };

        settings.save(&path).expect("save settings");
        let loaded = ShellSettings::load(&path).expect("load settings");
        std::fs::remove_file(path).expect("remove settings fixture");

        assert_eq!(loaded.active_desktop, 0);
        assert_eq!(
            loaded,
            ShellSettings {
                active_desktop: 0,
                ..settings
            }
        );
    }

    #[test]
    fn malformed_workspace_count_repairs_only_that_setting() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shell-settings");
        std::fs::write(
            &path,
            format!(
                "version={SHELL_SETTINGS_VERSION}\ndesktop_count=broken\nbar_on_all_displays=false\ntheme=dark\n"
            ),
        )
        .unwrap();
        let loaded = ShellSettings::load(path).unwrap();
        assert_eq!(loaded.desktop_count, 4);
        assert!(!loaded.bar_on_all_displays);
        assert_eq!(loaded.theme, ThemePreference::Dark);
    }

    #[test]
    fn workspace_count_is_validated_through_the_tenth_numeric_binding() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shell-settings");
        std::fs::write(&path, "desktop_count=255\nactive_desktop=8\n").unwrap();
        let loaded = ShellSettings::load(path).unwrap();
        assert_eq!(loaded.desktop_count, MAX_CONFIGURED_WORKSPACES);
        assert_eq!(loaded.active_desktop, 0);
    }

    #[test]
    fn unsupported_settings_versions_fall_back_without_rewriting_the_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shell-settings");
        let contents = "version=255\ndesktop_count=9\ntheme=dark\n";
        std::fs::write(&path, contents).unwrap();
        assert_eq!(
            ShellSettings::load(&path).unwrap(),
            ShellSettings::default()
        );
        assert_eq!(std::fs::read_to_string(path).unwrap(), contents);
    }

    #[test]
    fn repeated_saves_replace_the_complete_settings_atomically() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shell-settings");
        let mut settings = ShellSettings::default();
        settings.save(&path).unwrap();
        settings.desktop_count = 10;
        settings.theme = ThemePreference::Dark;
        settings.save(&path).unwrap();
        assert_eq!(ShellSettings::load(path).unwrap(), settings);
    }
}
