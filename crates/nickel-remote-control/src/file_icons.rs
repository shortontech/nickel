//! Typed file-icon preferences and a bounded platform-owned theme catalog.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Provider {
    Nickel,
    System,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub provider: Provider,
    /// A platform theme identifier, never a path. An unavailable configured ID
    /// remains visible so temporary removal is not mistaken for user intent.
    pub theme: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Theme {
    pub id: String,
    pub configured: bool,
    pub available: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    SetNickelProvider {},
    UseSystemDefault {},
    SetInstalledSystemTheme { theme_id: String },
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior: Preferences,
    pub change: Change,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_us: u64,
    pub configured: Preferences,
    pub themes: Vec<Theme>,
    /// The shell/file-manager settings reload was requested. This does not
    /// claim that every icon has completed decoding or presentation.
    pub cache_refresh_requested: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_schema_rejects_paths_and_untyped_provider_values() {
        let value = serde_json::json!({
            "generation": 1,
            "prior": {"provider": "system", "theme": null},
            "change": {"kind": "set_installed_system_theme", "theme_id": "Papirus"}
        });
        assert!(serde_json::from_value::<Transaction>(value.clone()).is_ok());
        let mut path = value.clone();
        path["change"]["path"] = serde_json::json!("/usr/share/icons/Papirus");
        assert!(serde_json::from_value::<Transaction>(path).is_err());
        let mut provider = value;
        provider["prior"]["provider"] = serde_json::json!("custom");
        assert!(serde_json::from_value::<Transaction>(provider).is_err());
    }
}
