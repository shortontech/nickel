//! Codex enablement without executable paths, credentials, or backend payloads.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Policy {
    Editable,
    ForceEnabled,
    ForceDisabled,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior_enabled: bool,
    pub requested_enabled: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_us: u64,
    pub configured_enabled: bool,
    pub effective_enabled: bool,
    pub policy: Policy,
    /// True when a nonstandard executable is configured. Its path is excluded.
    pub custom_executable_configured: bool,
    pub runtime_generation: u64,
    pub runtime_enabled: bool,
    pub active_chat_windows: u32,
    pub pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_excludes_sources_paths_credentials_and_force_policy() {
        let value = serde_json::json!({
            "generation": 2,
            "prior_enabled": true,
            "requested_enabled": false
        });
        assert!(serde_json::from_value::<Transaction>(value.clone()).is_ok());
        for field in [
            "source",
            "path",
            "token",
            "policy",
            "force",
            "close_windows",
        ] {
            let mut hostile = value.clone();
            hostile[field] = serde_json::json!(true);
            assert!(serde_json::from_value::<Transaction>(hostile).is_err());
        }
    }
}
