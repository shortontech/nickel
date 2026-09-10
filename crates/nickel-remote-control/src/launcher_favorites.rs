//! Installed-application favorites only; no recent-history contents or file paths.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_FAVORITES: usize = 64;
pub const MAX_APPLICATION_ID_BYTES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Change {
    Add { application_id: String },
    Remove { application_id: String },
    Reorder { application_ids: Vec<String> },
}
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub catalog_generation: u64,
    pub prior: Vec<String>,
    pub change: Change,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub catalog_generation: u64,
    pub observed_at_us: u64,
    pub favorites: Vec<String>,
    /// Unavailable/unsupported stored favorites are retained but never disclosed.
    pub unavailable_favorites: usize,
    /// Production launcher model matches this configuration, not pixel confirmation.
    pub runtime_applied: bool,
}
pub fn valid_ids(ids: &[String]) -> bool {
    ids.len() <= MAX_FAVORITES
        && ids.iter().enumerate().all(|(index, id)| {
            !id.is_empty()
                && id.len() <= MAX_APPLICATION_ID_BYTES
                && !id.chars().any(char::is_control)
                && !ids[..index].contains(id)
        })
}
impl Transaction {
    pub fn valid(&self) -> bool {
        self.generation != 0
            && self.catalog_generation != 0
            && valid_ids(&self.prior)
            && match &self.change {
                Change::Add { application_id } | Change::Remove { application_id } => {
                    valid_ids(std::slice::from_ref(application_id))
                }
                Change::Reorder { application_ids } => valid_ids(application_ids),
            }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn favorites_schema_rejects_history_paths_and_unbounded_lists() {
        assert!(
            serde_json::from_value::<Transaction>(serde_json::json!({
                "generation": 1, "catalog_generation": 1, "prior": [],
                "change": { "kind": "add", "application_id": "app.desktop", "path": "/tmp/x" }
            }))
            .is_err()
        );
        assert!(!valid_ids(&vec!["app.desktop".into(); 65]));
        assert!(!valid_ids(&["app\n.desktop".into()]));
        assert!(!valid_ids(&["a".repeat(513)]));
        assert!(valid_ids(&["org.example.App.desktop".into()]));
    }
}
