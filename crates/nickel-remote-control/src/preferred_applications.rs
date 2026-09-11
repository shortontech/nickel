//! Path-free selection of Nickel's preferred terminal and file manager.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_CATALOG_ENTRIES: usize = 512;
pub const MAX_APPLICATION_ID_BYTES: usize = 512;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Choice {
    /// None selects Nickel's system default.
    pub application_id: Option<String>,
    /// True when a private stored command no longer maps to this catalog.
    pub unavailable: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub catalog_generation: u64,
    pub observed_at_us: u64,
    pub terminal: Choice,
    pub file_manager: Choice,
    /// Opaque IDs from the exact production catalog generation.
    pub available_applications: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    pub terminal_application_id: Option<String>,
    pub file_manager_application_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub catalog_generation: u64,
    pub prior: Selection,
    pub requested: Selection,
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_APPLICATION_ID_BYTES && !id.chars().any(char::is_control)
}

impl Selection {
    pub fn valid(&self) -> bool {
        self.terminal_application_id.as_deref().is_none_or(valid_id)
            && self
                .file_manager_application_id
                .as_deref()
                .is_none_or(valid_id)
    }
}

impl Transaction {
    pub fn valid(&self) -> bool {
        self.generation != 0
            && self.catalog_generation != 0
            && self.prior.valid()
            && self.requested.valid()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_is_path_free_bounded_and_supports_system_defaults() {
        let value = serde_json::json!({
            "generation": 3, "catalog_generation": 4,
            "prior": {"terminal_application_id": null, "file_manager_application_id": "files"},
            "requested": {"terminal_application_id": "terminal", "file_manager_application_id": null}
        });
        assert!(
            serde_json::from_value::<Transaction>(value.clone())
                .unwrap()
                .valid()
        );
        for field in ["path", "command", "arguments", "environment"] {
            let mut hostile = value.clone();
            hostile["requested"][field] = "/private/value".into();
            assert!(serde_json::from_value::<Transaction>(hostile).is_err());
        }
        assert!(!valid_id("bad\nidentity"));
        assert!(!valid_id(&"x".repeat(MAX_APPLICATION_ID_BYTES + 1)));
    }
}
