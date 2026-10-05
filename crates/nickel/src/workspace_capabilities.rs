//! Reusable workspace observations and revision-bound native requests.
use crate::platform::WorkspaceSummary;
use nickel_core::plugins::PluginCapability;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

pub(crate) fn snapshot(workspaces: &[WorkspaceSummary], writable: bool) -> Value {
    let entries = workspaces
        .iter()
        .take(32)
        .map(|workspace| json!({"id":workspace.id.to_string(),"active":workspace.active}))
        .collect::<Vec<_>>();
    let revision = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&entries).unwrap())
    );
    json!({"available":true,"revision":revision,"workspaces":entries,"activeWorkspace":workspaces.iter().find(|workspace|workspace.active).map(|workspace|workspace.id.to_string()),"operations":{"switch":writable,"create":writable&&workspaces.len()<32,"remove":writable&&workspaces.len()>1}})
}
#[derive(Clone, Debug, PartialEq)]
pub struct WorkspaceEffect {
    pub(crate) operation: String,
    pub(crate) revision: String,
    pub(crate) id: Option<u64>,
}
impl WorkspaceEffect {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let operation = value["type"]
            .as_str()
            .filter(|operation| {
                matches!(
                    *operation,
                    "workspaces.switch" | "workspaces.create" | "workspaces.remove"
                )
            })
            .ok_or("invalid workspace operation")?;
        let revision = value["revision"]
            .as_str()
            .filter(|revision| revision.len() == 64)
            .ok_or("invalid workspace observation")?;
        let id = if operation == "workspaces.create" {
            None
        } else {
            Some(
                value["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 20)
                    .and_then(|id| {
                        id.parse::<u64>()
                            .ok()
                            .filter(|parsed| parsed.to_string() == id)
                    })
                    .ok_or("invalid workspace identity")?,
            )
        };
        Ok(Self {
            operation: operation.into(),
            revision: revision.into(),
            id,
        })
    }
    pub(crate) fn capability(&self) -> PluginCapability {
        PluginCapability::WorkspacesSwitch
    }
    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        let operation = self.operation.split_once('.').unwrap().1;
        if snapshot["revision"].as_str() != Some(&self.revision)
            || snapshot["operations"][operation] != true
        {
            return Err("workspace observation is stale or operation unavailable".into());
        }
        if let Some(id) = self.id {
            let identity = id.to_string();
            if !snapshot["workspaces"].as_array().is_some_and(|entries| {
                entries
                    .iter()
                    .any(|entry| entry["id"].as_str() == Some(identity.as_str()))
            }) {
                return Err("workspace no longer exists".into());
            }
        }
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_identity_round_trips_without_numeric_coercion() {
        let observed = snapshot(
            &[WorkspaceSummary {
                id: u64::MAX,
                active: true,
            }],
            true,
        );
        let request = json!({"type":"workspaces.switch","id":u64::MAX.to_string(),"revision":observed["revision"]});
        let effect = WorkspaceEffect::parse(&request).unwrap();
        assert_eq!(effect.id, Some(u64::MAX));
        assert!(effect.validate(&observed).is_ok());
        let mut numeric = observed.clone();
        numeric["workspaces"][0]["id"] = json!(u64::MAX);
        assert!(effect.validate(&numeric).is_err());
        for id in ["044", "+44", " 44", "18446744073709551616"] {
            let mut invalid = request.clone();
            invalid["id"] = json!(id);
            assert!(WorkspaceEffect::parse(&invalid).is_err(), "{id}");
        }
    }
    #[test]
    fn workspace_requests_use_stable_ids_and_reject_changed_inventory_or_unsupported_operations() {
        let entries = [
            WorkspaceSummary {
                id: 9,
                active: true,
            },
            WorkspaceSummary {
                id: 44,
                active: false,
            },
        ];
        let observed = snapshot(&entries, true);
        let effect = WorkspaceEffect::parse(
            &json!({"type":"workspaces.switch","id":"44","revision":observed["revision"]}),
        )
        .unwrap();
        assert!(effect.validate(&observed).is_ok());
        assert!(effect.validate(&snapshot(&entries, false)).is_err());
        assert!(effect.validate(&snapshot(&entries[..1], true)).is_err());
        assert!(
            WorkspaceEffect::parse(
                &json!({"type":"workspaces.switch","id":44,"revision":observed["revision"]})
            )
            .is_err()
        );
    }
}
