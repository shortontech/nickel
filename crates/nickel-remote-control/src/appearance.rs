//! Typed Nickel shell appearance preferences. No paths or permission settings.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThemePreference {
    System,
    Light,
    Dark,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Animations {
    Off,
    Reduced,
    Normal,
}
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub theme: ThemePreference,
    /// None inherits the platform default; explicit hue is 0..=359.
    pub accent_hue: Option<u16>,
    /// None inherits the platform default; explicit intensity is 0..=100.
    pub accent_intensity: Option<u8>,
    pub reduce_transparency: bool,
    pub animations: Animations,
}
impl Preferences {
    pub fn valid(&self) -> bool {
        self.accent_hue.is_none_or(|value| value <= 359)
            && self.accent_intensity.is_none_or(|value| value <= 100)
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
    /// Owner-issued configuration observation, required by the next transaction.
    pub generation: u64,
    pub observed_at_us: u64,
    /// Configured Nickel preferences, not OS-wide publication or presented pixels.
    pub configured: Preferences,
}
