use nickel_storage::{atomic_write, config_path};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

pub const MAX_TERMINAL_SCROLLBACK_LINES: usize = 100_000;
const MAX_SETTING_TEXT: usize = 256;

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
            foreground: 0xffd8dee9,
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
        let mut settings = Self::default();
        for line in fs::read_to_string(path)?.lines() {
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
                "foreground" => settings.foreground = parse_color(value).unwrap_or(0xffd8dee9),
                "background" => settings.background = parse_color(value).unwrap_or(0xff111318),
                "close_on_successful_exit" => settings.close_on_successful_exit = parse_bool(value),
                _ => {}
            }
        }
        Ok(settings)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let shell = safe_value(self.default_shell.as_deref().unwrap_or(""));
        let cwd = safe_value(
            &self
                .initial_working_directory
                .as_deref()
                .map(Path::to_string_lossy)
                .unwrap_or_default(),
        );
        let family = safe_value(&self.font_family);
        atomic_write(
            path.as_ref(),
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
            ),
        )
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
        .take(MAX_SETTING_TEXT)
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

fn settings_path() -> io::Result<PathBuf> {
    config_path("terminal-settings")
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert_eq!(settings.font_family.len(), MAX_SETTING_TEXT);
        assert_eq!(settings.font_size_tenths, 60);
        assert_eq!(settings.scrollback_lines, MAX_TERMINAL_SCROLLBACK_LINES);
        assert_eq!(settings.foreground, TerminalSettings::default().foreground);
    }
}
