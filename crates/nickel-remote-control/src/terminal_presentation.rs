//! Terminal presentation preferences; executable and directory values stay opaque.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_FONT_FAMILY_BYTES: usize = 256;
pub const MAX_SCROLLBACK_LINES: usize = 100_000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CursorStyle {
    Block,
    Beam,
    Underline,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub font_family: String,
    /// Font size in tenths of a logical pixel, from 60 through 720.
    pub font_size_tenths: u16,
    pub scrollback_lines: usize,
    pub cursor_style: CursorStyle,
    /// Straight-alpha ARGB color.
    pub foreground: u32,
    /// Straight-alpha ARGB color.
    pub background: u32,
    pub close_on_successful_exit: bool,
}

impl Preferences {
    pub fn valid(&self) -> bool {
        !self.font_family.is_empty()
            && self.font_family.len() <= MAX_FONT_FAMILY_BYTES
            && !self.font_family.chars().any(char::is_control)
            && (60..=720).contains(&self.font_size_tenths)
            && self.scrollback_lines <= MAX_SCROLLBACK_LINES
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior: Preferences,
    pub requested: Preferences,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_us: u64,
    pub configured: Preferences,
    /// Only presence is exposed; executable text is excluded.
    pub custom_shell_configured: bool,
    /// Only presence is exposed; directory paths are excluded.
    pub initial_directory_configured: bool,
    /// Production reads these preferences when creating a new terminal.
    pub applies_to_new_terminals: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_and_bounds_exclude_launch_policy_and_hostile_font_values() {
        let value = serde_json::json!({
            "generation": 1,
            "prior": {
                "font_family": "monospace", "font_size_tenths": 140,
                "scrollback_lines": 10000, "cursor_style": "block",
                "foreground": 4294769916u32, "background": 4279317272u32,
                "close_on_successful_exit": false
            },
            "requested": {
                "font_family": "Iosevka", "font_size_tenths": 160,
                "scrollback_lines": 20000, "cursor_style": "beam",
                "foreground": 4294769916u32, "background": 4279317272u32,
                "close_on_successful_exit": true
            }
        });
        let transaction = serde_json::from_value::<Transaction>(value.clone()).unwrap();
        assert!(transaction.prior.valid() && transaction.requested.valid());
        for field in [
            "default_shell",
            "initial_working_directory",
            "command",
            "path",
        ] {
            let mut hostile = value.clone();
            hostile["requested"][field] = serde_json::json!("/private/value");
            assert!(serde_json::from_value::<Transaction>(hostile).is_err());
        }
        let mut invalid = transaction.requested;
        invalid.font_family = "x\nsecret".into();
        assert!(!invalid.valid());
        invalid.font_family = "x".repeat(MAX_FONT_FAMILY_BYTES + 1);
        assert!(!invalid.valid());
    }
}
