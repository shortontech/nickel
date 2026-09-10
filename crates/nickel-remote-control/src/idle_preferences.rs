//! Non-security idle preferences; lock policy is deliberately excluded.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Timeout {
    Disabled,
    AfterSeconds(u32),
}

impl Timeout {
    pub fn valid_request(self) -> bool {
        match self {
            Self::Disabled => true,
            Self::AfterSeconds(seconds) => (30..=604_800).contains(&seconds),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Preferences {
    pub dim: Timeout,
    pub suspend: Timeout,
}

impl Preferences {
    pub fn valid_request(self) -> bool {
        self.dim.valid_request() && self.suspend.valid_request()
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior: Preferences,
    pub requested: Preferences,
}

#[derive(Clone, Debug, JsonSchema, Serialize)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_us: u64,
    pub configured: Preferences,
    pub applied: Preferences,
    pub applied_generation: u64,
    pub pending: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_excludes_lock_and_bounds_requested_timeouts() {
        let value = serde_json::json!({
            "generation": 1,
            "prior": { "dim": { "after_seconds": 300 }, "suspend": "disabled" },
            "requested": { "dim": { "after_seconds": 600 }, "suspend": "disabled" }
        });
        let transaction: Transaction = serde_json::from_value(value.clone()).unwrap();
        assert!(transaction.requested.valid_request());
        for field in [
            "lock",
            "idle_lock_seconds",
            "password",
            "inhibit",
            "shutdown",
        ] {
            let mut hostile = value.clone();
            hostile["requested"][field] = serde_json::json!(30);
            assert!(serde_json::from_value::<Transaction>(hostile).is_err());
        }
        assert!(
            !Preferences {
                dim: Timeout::AfterSeconds(29),
                suspend: Timeout::Disabled,
            }
            .valid_request()
        );
    }
}
