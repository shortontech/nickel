//! Bounded semantic observation of an authorized window's current UI generation.
use schemars::JsonSchema;
use serde::Serialize;

pub const MAX_RESOLVED_NODES: usize = 1024;
pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SemanticValue {
    Boolean(bool),
    Number {
        value: f64,
        minimum: f64,
        maximum: f64,
        step: f64,
    },
    Text(String),
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SemanticNode {
    /// Snapshot-local ordinal; meaningful only with this surface and tree generation.
    pub id: u32,
    pub role: Option<String>,
    /// Logical coordinates relative to the hosted client surface.
    pub bounds: [f32; 4],
    pub name: Option<String>,
    pub description: Option<String>,
    pub enabled: bool,
    pub focused: bool,
    pub actions: Vec<String>,
    pub value: Option<SemanticValue>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SemanticSnapshot {
    pub window: String,
    pub window_generation: u64,
    pub surface: String,
    pub surface_generation: u64,
    pub tree_generation: u64,
    pub observed_at_us: u64,
    /// Flat semantic projection. Layout-only ancestors are not exposed.
    pub nodes: Vec<SemanticNode>,
}

/// Ordinary shell surface semantics, with no invented application-window identity.
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct SurfaceSemanticSnapshot {
    pub surface: String,
    pub surface_generation: u64,
    pub tree_generation: u64,
    pub observed_at_us: u64,
    pub nodes: Vec<SemanticNode>,
}

/// Matches the fixed action names returned by inspect_window.
#[derive(Clone, Copy, serde::Deserialize, JsonSchema)]
pub enum SemanticInvocation {
    Activate,
    Cancel,
    ContextMenu,
    Increment,
    Decrement,
    Expand,
    Collapse,
    Select,
    Dismiss,
    Scroll,
    EnterNavigation,
    ExitNavigation,
}

#[derive(serde::Deserialize, JsonSchema)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SemanticMutation {
    Invoke(SemanticInvocation),
    SetBoolean(bool),
    SetNumber(f64),
    SetText(String),
}

pub const MAX_MUTATION_TEXT_BYTES: usize = 2048;

#[derive(serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SemanticActionRequest {
    pub lease_id: u64,
    pub window_id: String,
    pub window_generation: u64,
    pub surface_id: String,
    pub surface_generation: u64,
    pub tree_generation: u64,
    pub node: u32,
    pub action: SemanticMutation,
}

impl std::fmt::Debug for SemanticActionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SemanticActionRequest(<redacted>)")
    }
}

impl SemanticActionRequest {
    /// Structural bounds only. The owner must still validate current identities,
    /// protection, lease authority and complete input ownership before dispatch.
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.window_id.len() > 20
            || self.window_id != self.window_generation.to_string()
            || self.surface_id.len() > 29
            || self.surface_id.strip_prefix("internal:")
                != Some(self.surface_generation.to_string().as_str())
        {
            return Err("invalid semantic resource identity");
        }
        if self.node as usize >= MAX_RESOLVED_NODES {
            return Err("semantic node exceeds limit");
        }
        self.action.validate()
    }
}

impl SemanticMutation {
    fn validate(&self) -> Result<(), &'static str> {
        match self {
            SemanticMutation::SetText(text) if text.len() > MAX_MUTATION_TEXT_BYTES => {
                Err("semantic text exceeds limit")
            }
            SemanticMutation::SetNumber(value) if !value.is_finite() => {
                Err("semantic number must be finite")
            }
            _ => Ok(()),
        }
    }
}

/// Observed completion of a shell action; never a promise of presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SurfaceSemanticCompletion {
    UiUpdated,
    Confirmed,
    Requested,
    Cancelled,
    Unavailable,
    Uncertain,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, serde::Serialize, JsonSchema)]
pub struct SurfaceSemanticActionOutcome {
    /// Preserves the immediate UI-change result, including before native delivery.
    pub changed: bool,
    pub completion: SurfaceSemanticCompletion,
    /// Earlier effects completed before a later step remained incomplete.
    pub partial: bool,
}

/// Mutation of an ordinary shell node observed by inspect_surface.
#[derive(serde::Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceSemanticActionRequest {
    pub lease_id: u64,
    pub surface_id: String,
    pub surface_generation: u64,
    pub tree_generation: u64,
    pub node: u32,
    pub action: SemanticMutation,
}
impl std::fmt::Debug for SurfaceSemanticActionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SurfaceSemanticActionRequest(<redacted>)")
    }
}
impl SurfaceSemanticActionRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.surface_id.len() > 29
            || self.surface_id.strip_prefix("internal:")
                != Some(self.surface_generation.to_string().as_str())
        {
            return Err("invalid semantic surface identity");
        }
        if self.node as usize >= MAX_RESOLVED_NODES {
            return Err("semantic node exceeds limit");
        }
        self.action.validate()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(action: SemanticMutation) -> SemanticActionRequest {
        SemanticActionRequest {
            lease_id: 1,
            window_id: "12".into(),
            window_generation: 12,
            surface_id: "internal:7".into(),
            surface_generation: 7,
            tree_generation: 9,
            node: 0,
            action,
        }
    }

    #[test]
    fn mutation_requests_bound_values_and_never_debug_payloads() {
        let mut value = request(SemanticMutation::SetText("private-input-canary".into()));
        assert!(value.validate().is_ok());
        let debug = format!("{value:?}");
        assert!(!debug.contains("private-input-canary"));
        assert!(!debug.contains("internal:7"));
        value.action = SemanticMutation::SetText("é".repeat(MAX_MUTATION_TEXT_BYTES / 2));
        assert!(value.validate().is_ok());
        value.action = SemanticMutation::SetText("é".repeat(MAX_MUTATION_TEXT_BYTES / 2 + 1));
        assert!(value.validate().is_err());
        value.action = SemanticMutation::SetNumber(f64::NAN);
        assert!(value.validate().is_err());
        value.action = SemanticMutation::Invoke(SemanticInvocation::Activate);
        value.surface_id = "internal:07".into();
        assert!(value.validate().is_err());
        value.surface_id = "internal:7".into();
        value.node = MAX_RESOLVED_NODES as u32;
        assert!(value.validate().is_err());
    }

    #[test]
    fn shell_mutation_has_no_invented_window_and_redacts_payloads() {
        let mut value: SurfaceSemanticActionRequest = serde_json::from_value(serde_json::json!({
            "lease_id": 1, "surface_id":"internal:9", "surface_generation":9, "tree_generation":3, "node":0,
            "action":{"kind":"set_text","value":"private query"}
        })).unwrap();
        assert!(value.validate().is_ok());
        assert!(!format!("{value:?}").contains("private query"));
        value.surface_id = "internal:09".into();
        assert!(value.validate().is_err());
        value.surface_id = "internal:9".into();
        value.action = SemanticMutation::SetText("x".repeat(MAX_MUTATION_TEXT_BYTES + 1));
        assert!(value.validate().is_err());
    }

    #[test]
    fn mutation_wire_format_rejects_unknown_fields_and_actions() {
        let mut value = serde_json::json!({
            "lease_id":1, "window_id":"12", "window_generation":12,
            "surface_id":"internal:7", "surface_generation":7, "tree_generation":9,
            "node":0, "action":{"kind":"invoke", "value":"Activate"}
        });
        assert!(
            serde_json::from_value::<SemanticActionRequest>(value.clone())
                .unwrap()
                .validate()
                .is_ok()
        );
        value["action"]["extra"] = true.into();
        assert!(serde_json::from_value::<SemanticActionRequest>(value.clone()).is_err());
        value["action"] = serde_json::json!({"kind":"invoke", "value":"RunShell"});
        assert!(serde_json::from_value::<SemanticActionRequest>(value.clone()).is_err());
        value["action"] = serde_json::json!({"kind":"set_text", "value":"ordinary"});
        value["extra"] = true.into();
        assert!(serde_json::from_value::<SemanticActionRequest>(value).is_err());
    }
}
