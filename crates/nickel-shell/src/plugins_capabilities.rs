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
            "settings": plugin.settings,
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

#[derive(Clone, Debug, PartialEq)]
pub struct PluginsSettingEffect {
    pub id: String,
    pub key: String,
    pub revision: u64,
    pub prior_value: Value,
    pub value: Value,
}
impl PluginsSettingEffect {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        if value["type"] != "plugins.setSetting" {
            return Err("unknown plugin setting operation".into());
        }
        let identity = |name: &str| {
            value[name]
                .as_str()
                .filter(|id| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control))
                .map(str::to_owned)
                .ok_or_else(|| format!("invalid plugin setting {name}"))
        };
        let scalar = |name: &str| {
            value
                .get(name)
                .filter(|value| {
                    value.is_boolean()
                        || value.is_number()
                        || value.as_str().is_some_and(|text| text.len() <= 65535)
                })
                .cloned()
                .ok_or_else(|| format!("invalid plugin setting {name}"))
        };
        Ok(Self {
            id: identity("id")?,
            key: identity("key")?,
            revision: value["revision"]
                .as_str()
                .and_then(|value| value.parse().ok())
                .ok_or("invalid plugin revision")?,
            prior_value: scalar("priorValue")?,
            value: scalar("value")?,
        })
    }
    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot["available"] != true || snapshot["writable"] != true {
            return Err("plugin setting management is unavailable".into());
        }
        if snapshot["revision"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            != Some(self.revision)
        {
            return Err("plugin inventory is stale".into());
        }
        let setting = snapshot["plugins"]
            .as_array()
            .and_then(|plugins| plugins.iter().find(|plugin| plugin["id"] == self.id))
            .and_then(|plugin| plugin["settings"].as_array())
            .and_then(|settings| settings.iter().find(|setting| setting["id"] == self.key))
            .ok_or("plugin setting is unavailable")?;
        if setting["value"] != self.prior_value {
            return Err("plugin setting has changed".into());
        }
        let kind: nickel_session_protocol::PluginSettingKind =
            serde_json::from_value(setting["kind"].clone())
                .map_err(|_| "invalid plugin setting metadata")?;
        if !kind.accepts(&self.value) {
            return Err("plugin setting value is outside its bounds".into());
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn metadata_settings_require_current_revision_prior_value_and_declared_bounds() {
        let mut snapshot = json!({"available":true,"writable":true,"revision":"7","plugins":[{"id":"example","settings":[{"id":"count","kind":{"kind":"integer","min":1,"max":4},"value":2}]}]});
        let effect=PluginsSettingEffect::parse(&json!({"type":"plugins.setSetting","id":"example","key":"count","revision":"7","priorValue":2,"value":3})).unwrap();
        assert!(effect.validate(&snapshot).is_ok());
        let mut outside = effect.clone();
        outside.value = json!(5);
        assert!(outside.validate(&snapshot).is_err());
        outside.value = json!(1.5);
        assert!(outside.validate(&snapshot).is_err());
        snapshot["plugins"][0]["settings"][0]["value"] = json!(3);
        assert!(effect.validate(&snapshot).is_err());
        snapshot["plugins"][0]["settings"][0]["value"] = json!(2);
        snapshot["revision"] = json!("8");
        assert!(effect.validate(&snapshot).is_err());
        snapshot["revision"] = json!("7");
        snapshot["writable"] = json!(false);
        assert!(effect.validate(&snapshot).is_err());
    }

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellSelectionEffect {
    pub id: String,
    pub revision: u64,
}
impl ShellSelectionEffect {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let id = value["id"]
            .as_str()
            .filter(|id| !id.is_empty() && id.len() <= 128 && !id.chars().any(char::is_control))
            .ok_or("invalid shell identity")?
            .to_owned();
        let revision = value["revision"]
            .as_str()
            .and_then(|value| value.parse().ok())
            .ok_or("invalid plugin inventory revision")?;
        Ok(Self { id, revision })
    }
    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot
            .get("shellPreview")
            .is_some_and(|preview| !preview.is_null())
        {
            return Err("a shell preview is already pending".into());
        }
        if snapshot["available"] != true || snapshot["writable"] != true {
            return Err("shell selection is unavailable".into());
        }
        if snapshot["revision"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            != Some(self.revision)
        {
            return Err("plugin inventory is stale".into());
        }
        if !snapshot["plugins"].as_array().is_some_and(|plugins| {
            plugins
                .iter()
                .any(|plugin| plugin["id"] == self.id && plugin["shell"] == true)
        }) {
            return Err("shell package is unavailable".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShellPreviewDecision {
    pub token: u64,
    pub revision: u64,
    pub confirm: bool,
}
impl ShellPreviewDecision {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let confirm = match value["type"].as_str() {
            Some("plugins.confirmShell") => true,
            Some("plugins.revertShell") => false,
            _ => return Err("invalid shell preview decision".into()),
        };
        let token = value["token"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|token| *token > 0)
            .ok_or("invalid shell preview token")?;
        let revision = value["revision"]
            .as_str()
            .and_then(|value| value.parse().ok())
            .ok_or("invalid plugin inventory revision")?;
        Ok(Self {
            token,
            revision,
            confirm,
        })
    }
    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot["available"] != true || snapshot["writable"] != true {
            return Err("shell preview decision is unavailable".into());
        }
        if snapshot["revision"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            != Some(self.revision)
        {
            return Err("plugin inventory is stale".into());
        }
        let preview = &snapshot["shellPreview"];
        if preview["token"]
            .as_str()
            .and_then(|value| value.parse::<u64>().ok())
            != Some(self.token)
        {
            return Err("shell preview token is stale".into());
        }
        if preview[if self.confirm {
            "canConfirm"
        } else {
            "canRevert"
        }] != true
        {
            return Err("shell preview decision is unavailable".into());
        }
        Ok(())
    }
}
