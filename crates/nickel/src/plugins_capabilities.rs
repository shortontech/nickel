//! Public package inventory and guarded lifecycle requests, backed by the shell registry.
use nickel_session_protocol::PluginStatusSnapshot;
use serde_json::{Value, json};

pub(crate) fn snapshot(
    status: &PluginStatusSnapshot,
    writable: bool,
    result: Option<&Value>,
) -> Value {
    json!({
        "available": true, "writable": writable,
        "revision": status.activation_generation.to_string(),
        "truncated": status.plugins.len() > 256,
        "lastResult": result,
        "plugins": status.plugins.iter().take(256).map(|plugin| json!({
            "id": plugin.id, "name": plugin.name, "author": plugin.author,
            "version": plugin.version, "enabled": plugin.desired_enabled,
            "health": plugin.health, "grants": plugin.capabilities,
            "surfaces": plugin.surfaces, "composition": plugin.composition,
            "memory": {
                "jsHeapBytes": plugin.memory.js_heap_bytes,
                "nativeUiBytes": plugin.memory.native_ui_bytes,
                "textureBytes": plugin.memory.texture_bytes,
                "trackedPeakBytes": plugin.memory.tracked_peak_bytes,
                "timers": plugin.memory.timers, "subscriptions": plugin.memory.subscriptions,
                "componentBreakdownAvailable": false,
            }
        })).collect::<Vec<_>>()
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginsEffect {
    pub id: String,
    pub enabled: bool,
    pub revision: u64,
    pub prior_enabled: bool,
}

impl PluginsEffect {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let enabled = match value["type"].as_str() {
            Some("plugins.enable") => true,
            Some("plugins.disable") => false,
            _ => return Err("unknown plugin management operation".into()),
        };
        let id = value["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control))
            .ok_or("invalid plugin identity")?
            .to_owned();
        let revision = value["revision"]
            .as_str()
            .and_then(|value| value.parse().ok())
            .ok_or("invalid plugin inventory revision")?;
        let prior_enabled = value["priorEnabled"]
            .as_bool()
            .ok_or("invalid prior plugin state")?;
        Ok(Self {
            id,
            enabled,
            revision,
            prior_enabled,
        })
    }

    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot["available"] != true || snapshot["writable"] != true {
            return Err("plugin management is unavailable".into());
        }
        if snapshot["revision"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            != Some(self.revision)
        {
            return Err("plugin inventory is stale".into());
        }
        let target = snapshot["plugins"]
            .as_array()
            .and_then(|plugins| plugins.iter().find(|plugin| plugin["id"] == self.id))
            .ok_or("plugin is unavailable")?;
        if target["enabled"].as_bool() != Some(self.prior_enabled) {
            return Err("plugin state has changed".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn lifecycle_requests_reject_stale_unknown_and_readonly_inventory() {
        let effect = PluginsEffect::parse(
            &json!({"type":"plugins.disable","id":"example","revision":"7","priorEnabled":true}),
        )
        .unwrap();
        let mut inventory = json!({"available":true,"writable":true,"revision":"7","plugins":[{"id":"example","enabled":true}]});
        assert!(effect.validate(&inventory).is_ok());
        inventory["revision"] = json!("8");
        assert!(effect.validate(&inventory).is_err());
        inventory["revision"] = json!("7");
        inventory["plugins"][0]["enabled"] = json!(false);
        assert!(effect.validate(&inventory).is_err());
        inventory["plugins"] = json!([]);
        assert!(effect.validate(&inventory).is_err());
        inventory["writable"] = json!(false);
        assert!(effect.validate(&inventory).is_err());
        assert!(
            PluginsEffect::parse(
                &json!({"type":"plugins.disable","id":"example","revision":7,"priorEnabled":true})
            )
            .is_err()
        );
    }
    #[test]
    fn unavailable_memory_is_null_and_inventory_revision_is_lossless() {
        use nickel_session_protocol::{PluginMemorySnapshot, PluginRuntimeHealth, PluginStatus};
        let status = PluginStatusSnapshot {
            activation_generation: u64::MAX,
            plugins: vec![PluginStatus {
                id: "example".into(),
                name: "Example".into(),
                author: None,
                version: None,
                desired_enabled: true,
                health: PluginRuntimeHealth::Running,
                capabilities: vec!["windows-read".into()],
                surfaces: vec![],
                composition: vec![],
                settings: vec![],
                memory: PluginMemorySnapshot::default(),
            }],
        };
        let value = snapshot(&status, false, None);
        assert_eq!(value["revision"], u64::MAX.to_string());
        assert!(value["plugins"][0]["memory"]["jsHeapBytes"].is_null());
        assert_eq!(value["plugins"][0]["grants"][0], "windows-read");
        assert_eq!(
            value["plugins"][0]["memory"]["componentBreakdownAvailable"],
            false
        );
    }
}
