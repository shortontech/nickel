use nickel_storage::{atomic_write, config_path, read_regular_file, stage_write};
use std::{
    io,
    path::{Path, PathBuf},
};

pub const MAX_TERMINAL_SCROLLBACK_LINES: usize = 100_000;
pub const MAX_TERMINAL_SETTING_TEXT: usize = 256;
const MAX_SETTINGS_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum TerminalCursorStyle {
    #[default]
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSettings {
    pub default_shell: Option<String>,
    pub initial_working_directory: Option<PathBuf>,
    pub font_family: String,
    /// Font size in tenths of a logical pixel.
    pub font_size_tenths: u16,
    pub scrollback_lines: usize,
    pub cursor_style: TerminalCursorStyle,
    pub foreground: u32,
    pub background: u32,
    pub close_on_successful_exit: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            default_shell: None,
            initial_working_directory: None,
            font_family: "monospace".into(),
            font_size_tenths: 140,
            scrollback_lines: 10_000,
            cursor_style: TerminalCursorStyle::Block,
            foreground: 0xfffcfcfc,
            background: 0xff111318,
            close_on_successful_exit: false,
        }
    }
}

impl TerminalSettings {
    pub fn load_default() -> Self {
        settings_path().and_then(Self::load).unwrap_or_default()
    }

    pub fn save_default(&self) -> io::Result<()> {
        self.save(settings_path()?)
    }

    pub fn font_size(&self) -> f32 {
        f32::from(self.font_size_tenths) / 10.0
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = read_regular_file(path.as_ref(), MAX_SETTINGS_BYTES)?
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let contents = std::str::from_utf8(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut settings = Self::default();
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "default_shell" => settings.default_shell = bounded_optional(value),
                "initial_working_directory" => {
                    settings.initial_working_directory = bounded_optional(value).map(PathBuf::from)
                }
                "font_family" => {
                    if let Some(value) = bounded_optional(value) {
                        settings.font_family = value;
                    }
                }
                "font_size_tenths" => {
                    settings.font_size_tenths = value.parse().unwrap_or(140).clamp(60, 720)
                }
                "scrollback_lines" => {
                    settings.scrollback_lines = value
                        .parse()
                        .unwrap_or(10_000)
                        .min(MAX_TERMINAL_SCROLLBACK_LINES)
                }
                "cursor_style" => {
                    settings.cursor_style = match value {
                        "beam" => TerminalCursorStyle::Beam,
                        "underline" => TerminalCursorStyle::Underline,
                        _ => TerminalCursorStyle::Block,
                    }
                }
                "foreground" => settings.foreground = parse_color(value).unwrap_or(0xfffcfcfc),
                "background" => settings.background = parse_color(value).unwrap_or(0xff111318),
                "close_on_successful_exit" => settings.close_on_successful_exit = parse_bool(value),
                _ => {}
            }
        }
        Ok(settings)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        atomic_write(path, self.encode())
    }

    fn encode(&self) -> String {
        let shell = safe_value(self.default_shell.as_deref().unwrap_or(""));
        let cwd = safe_value(
            &self
                .initial_working_directory
                .as_deref()
                .map(Path::to_string_lossy)
                .unwrap_or_default(),
        );
        let family = safe_value(&self.font_family);
        format!(
            "default_shell={shell}\ninitial_working_directory={cwd}\nfont_family={family}\nfont_size_tenths={}\nscrollback_lines={}\ncursor_style={}\nforeground=#{:08x}\nbackground=#{:08x}\nclose_on_successful_exit={}\n",
            self.font_size_tenths.clamp(60, 720),
            self.scrollback_lines.min(MAX_TERMINAL_SCROLLBACK_LINES),
            match self.cursor_style {
                TerminalCursorStyle::Block => "block",
                TerminalCursorStyle::Beam => "beam",
                TerminalCursorStyle::Underline => "underline",
            },
            self.foreground,
            self.background,
            self.close_on_successful_exit,
        )
    }
}

/// One locked, staged terminal preference replacement. Remote callers can
/// construct a requested whole value that preserves launch fields, while this
/// owner prevents stale local state from reaching the rename boundary.
pub struct PreparedTerminalSettings {
    path: PathBuf,
    revision: Option<nickel_storage::RegularFileRevision>,
    requested: TerminalSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedTerminalSettings {
    pub fn prepare(
        path: PathBuf,
        prior: &TerminalSettings,
        requested: TerminalSettings,
    ) -> io::Result<Self> {
        let lock = nickel_storage::TransactionLock::try_acquire(&path)?;
        let revision = nickel_storage::regular_file_revision(&path)?;
        let current = match TerminalSettings::load(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                TerminalSettings::default()
            }
            Err(error) => return Err(error),
        };
        if nickel_storage::regular_file_revision(&path)? != revision || &current != prior {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "terminal settings changed",
            ));
        }
        let staged = stage_write(&path, requested.encode())?;
        Ok(Self {
            path,
            revision,
            requested,
            staged,
            _lock: lock,
        })
    }

    pub fn commit(self, check: impl FnOnce() -> io::Result<()>) -> io::Result<TerminalSettings> {
        self.staged.commit(|| {
            if nickel_storage::regular_file_revision(&self.path)? != self.revision {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "terminal settings changed",
                ));
            }
            check()
        })?;
        Ok(self.requested)
    }
}

fn bounded_optional(value: &str) -> Option<String> {
    let value = safe_value(value);
    (!value.is_empty()).then_some(value)
}

fn safe_value(value: &str) -> String {
    value
        .chars()
        .filter(|character| !character.is_control())
        .take(MAX_TERMINAL_SETTING_TEXT)
        .collect::<String>()
        .trim()
        .to_owned()
}

fn parse_color(value: &str) -> Option<u32> {
    let value = value.strip_prefix('#').unwrap_or(value);
    let color = u32::from_str_radix(value, 16).ok()?;
    Some(if value.len() <= 6 {
        0xff00_0000 | color
    } else {
        color
    })
}

fn parse_bool(value: &str) -> bool {
    matches!(value, "1" | "true" | "yes" | "on")
}

pub fn settings_path() -> io::Result<PathBuf> {
    config_path("terminal-settings")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn settings_round_trip_every_supported_terminal_preference() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("terminal-settings");
        let expected = TerminalSettings {
            default_shell: Some("/bin/fish".into()),
            initial_working_directory: Some("/tmp/space ü".into()),
            font_family: "Iosevka Term".into(),
            font_size_tenths: 175,
            scrollback_lines: 42_000,
            cursor_style: TerminalCursorStyle::Beam,
            foreground: 0xffabcdef,
            background: 0xff102030,
            close_on_successful_exit: true,
        };
        expected.save(&path).unwrap();
        assert_eq!(TerminalSettings::load(path).unwrap(), expected);
    }

    #[test]
    fn malformed_and_hostile_values_are_repaired_and_bounded() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("terminal-settings");
        fs::write(
            &path,
            format!(
                "font_family={}\nfont_size_tenths=1\nscrollback_lines=999999999\nforeground=oops\n",
                "x".repeat(1_000)
            ),
        )
        .unwrap();
        let settings = TerminalSettings::load(path).unwrap();
        assert_eq!(settings.font_family.len(), MAX_TERMINAL_SETTING_TEXT);
        assert_eq!(settings.font_size_tenths, 60);
        assert_eq!(settings.scrollback_lines, MAX_TERMINAL_SCROLLBACK_LINES);
        assert_eq!(settings.foreground, TerminalSettings::default().foreground);
    }

    #[test]
    fn settings_transport_rejects_oversized_and_non_utf8_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("terminal-settings");
        fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES + 1]).unwrap();
        assert_eq!(
            TerminalSettings::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(&path, b"font_family=valid\nbackground=\xff\n").unwrap();
        assert_eq!(
            TerminalSettings::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn staged_presentation_change_preserves_launch_fields_and_checks_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("terminal-settings");
        let prior = TerminalSettings {
            default_shell: Some("/bin/private-shell".into()),
            initial_working_directory: Some("/private/directory".into()),
            ..Default::default()
        };
        prior.save(&path).unwrap();
        let mut requested = prior.clone();
        requested.font_family = "Iosevka".into();
        requested.cursor_style = TerminalCursorStyle::Beam;

        let cancelled =
            PreparedTerminalSettings::prepare(path.clone(), &prior, requested.clone()).unwrap();
        assert!(
            cancelled
                .commit(|| Err(io::Error::other("cancelled")))
                .is_err()
        );
        assert_eq!(TerminalSettings::load(&path).unwrap(), prior);

        let stale =
            PreparedTerminalSettings::prepare(path.clone(), &prior, requested.clone()).unwrap();
        fs::write(
            &path,
            prior
                .encode()
                .replace("font_size_tenths=140", "font_size_tenths=150"),
        )
        .unwrap();
        assert!(stale.commit(|| Ok(())).is_err());

        let current = TerminalSettings::load(&path).unwrap();
        requested.font_size_tenths = current.font_size_tenths;
        let accepted = PreparedTerminalSettings::prepare(path.clone(), &current, requested)
            .unwrap()
            .commit(|| Ok(()))
            .unwrap();
        assert_eq!(accepted.default_shell, prior.default_shell);
        assert_eq!(
            accepted.initial_working_directory,
            prior.initial_working_directory
        );
        assert_eq!(accepted.font_family, "Iosevka");
    }
}
