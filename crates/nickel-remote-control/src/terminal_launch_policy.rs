//! Typed terminal launch policy.
//!
//! The wire contract contains only fixed identities selected from a fresh
//! owner-produced catalog. Executable, command-line and filesystem path text
//! never enters a request or response.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ShellIdentity {
    BourneShell,
    Bash,
    Dash,
    Zsh,
    Fish,
    CommandPrompt,
    WindowsPowerShell,
    PowerShell,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DirectoryIdentity {
    Home,
    Desktop,
    Documents,
    Downloads,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    /// `None` selects the platform default shell.
    pub default_shell: Option<ShellIdentity>,
    /// `None` selects the terminal process's inherited directory.
    pub initial_directory: Option<DirectoryIdentity>,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
pub struct ObservedPolicy {
    pub configured: Policy,
    /// A private configured executable did not match the current catalog.
    pub shell_unavailable: bool,
    /// A private configured directory did not match the current catalog.
    pub directory_unavailable: bool,
}

impl ObservedPolicy {
    pub fn valid(&self) -> bool {
        !(self.shell_unavailable && self.configured.default_shell.is_some()
            || self.directory_unavailable && self.configured.initial_directory.is_some())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    pub observed_at_us: u64,
    pub available_shells: Vec<ShellIdentity>,
    pub available_directories: Vec<DirectoryIdentity>,
    pub policy: ObservedPolicy,
    /// The production terminal reads this policy when it creates a process.
    pub applies_to_new_terminals: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior: ObservedPolicy,
    pub requested: Policy,
}

impl Transaction {
    pub fn valid(&self) -> bool {
        self.generation != 0 && self.prior.valid()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct TransactionOutcome {
    pub snapshot: Snapshot,
    /// A confirmed replacement affects processes created after the commit.
    pub applies_to_new_terminals: bool,
    /// Existing terminal processes retain the policy they loaded at creation.
    pub existing_terminals_changed: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_accepts_only_typed_catalog_selections() {
        let value = serde_json::json!({
            "generation": 3,
            "prior": {
                "configured": {"default_shell": "bash", "initial_directory": "home"},
                "shell_unavailable": false,
                "directory_unavailable": false
            },
            "requested": {"default_shell": "zsh", "initial_directory": "downloads"}
        });
        assert!(
            serde_json::from_value::<Transaction>(value.clone())
                .unwrap()
                .valid()
        );
        for (field, hostile) in [
            ("executable", "/bin/sh"),
            ("command", "sh -c secret"),
            ("path", "/private/directory"),
        ] {
            let mut value = value.clone();
            value["requested"][field] = hostile.into();
            assert!(serde_json::from_value::<Transaction>(value).is_err());
        }
        let mut unknown = value;
        unknown["requested"]["default_shell"] = "private_shell".into();
        assert!(serde_json::from_value::<Transaction>(unknown).is_err());
    }

    #[test]
    fn unavailable_prior_cannot_also_claim_a_catalog_identity() {
        assert!(
            !ObservedPolicy {
                configured: Policy {
                    default_shell: Some(ShellIdentity::Bash),
                    initial_directory: None,
                },
                shell_unavailable: true,
                directory_unavailable: false,
            }
            .valid()
        );
    }
}
