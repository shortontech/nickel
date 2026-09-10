//! On-screen-keyboard preference and coarse runtime acknowledgement only.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Preference {
    Automatic,
    Enabled,
    Disabled,
}

#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior: Preference,
    pub requested: Preference,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_us: u64,
    pub configured: Preference,
    pub runtime_generation: u64,
    pub runtime_enabled: bool,
    pub touchscreen_present: bool,
    pub environment_override: bool,
    pub pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_excludes_controller_and_recipient_authority() {
        let value = serde_json::json!({
            "generation": 7, "prior": "automatic", "requested": "enabled"
        });
        assert!(serde_json::from_value::<Transaction>(value.clone()).is_ok());
        for field in [
            "environment_override",
            "visible",
            "dock_top",
            "height",
            "recipient",
            "epoch",
            "text",
        ] {
            let mut hostile = value.clone();
            hostile[field] = serde_json::json!(true);
            assert!(serde_json::from_value::<Transaction>(hostile).is_err());
        }
    }
}
