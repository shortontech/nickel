//! Read-only external accessibility structure. Each platform reports unavailable
//! fields explicitly; provider text is excluded unless its protected-state and
//! boundedness can be proved at the native authority boundary.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MAX_NATIVE_SEMANTIC_NODES: usize = 128;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeSemanticAction {
    Invoke,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct NativeSemanticNode {
    /// Ordinal scoped to this observation. An advertised action is identified by
    /// this ordinal plus the snapshot's resource and observation generations;
    /// this is never a native provider object or handle.
    pub id: u32,
    pub parent: Option<u32>,
    /// Numeric native-provider role/control type; unknown future roles are retained.
    pub role: u32,
    /// Logical coordinates relative to the associated native window.
    pub bounds: Option<[i32; 4]>,
    pub enabled: bool,
    pub focused: bool,
    /// Safe actions observed without requesting provider strings or values.
    pub actions: Vec<NativeSemanticAction>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeSemanticScope {
    Window,
    /// Platform application projection anchored to the verified window. Linux means
    /// its one authenticated AT-SPI connection. Windows means the current bounded set
    /// of ordinary, unprotected windows with its exact owner-verified application
    /// identity; those windows may span processes and every HWND/process incarnation
    /// is verified.
    ApplicationConnection,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct NativeSemanticSnapshot {
    pub scope: NativeSemanticScope,
    /// Native owner identity anchor; scope specifies the extent of the result.
    pub window: String,
    pub window_generation: u64,
    pub association_generation: Option<u64>,
    pub observation_generation: u64,
    /// Provider traversal completion, relative to session start; never refreshed at delivery.
    pub observed_at_us: u64,
    /// Start of the non-atomic provider traversal, relative to session start.
    pub observation_started_at_us: u64,
    /// Final compositor-owner authority validation time, separate from provider data age.
    pub owner_validated_at_us: u64,
    /// False for current native providers, which do not provide atomic tree snapshots.
    pub atomic: bool,
    /// Provider fields deliberately omitted by this platform observation.
    pub unavailable_fields: Vec<String>,
    pub nodes: Vec<NativeSemanticNode>,
    pub truncated: bool,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NativeSemanticActionRequest {
    pub lease_id: u64,
    pub scope: NativeSemanticScope,
    pub window_id: String,
    pub window_generation: u64,
    pub observation_generation: u64,
    pub node: u32,
    pub action: NativeSemanticAction,
}

impl std::fmt::Debug for NativeSemanticActionRequest {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("NativeSemanticActionRequest(<redacted>)")
    }
}

impl NativeSemanticActionRequest {
    pub fn validate(&self) -> Result<(), &'static str> {
        if self.window_id.is_empty()
            || self.window_id.len() > 128
            || self.observation_generation == 0
        {
            return Err("invalid native semantic action identity");
        }
        if self.node as usize >= MAX_NATIVE_SEMANTIC_NODES {
            return Err("native semantic node exceeds limit");
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
pub struct NativeSemanticActionOutcome {
    /// True once the owner committed dispatch to the native provider.
    pub requested: bool,
    /// UI Automation Invoke has no state-result contract, so Nickel does not
    /// infer confirmation from a successful method return.
    pub confirmed: bool,
    /// True when committed dispatch did not return a success result before the
    /// bounded request ended. Never automatically retry this outcome.
    pub uncertain: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_identity_is_generation_bound_and_strict() {
        let mut value = serde_json::json!({
            "lease_id": 7,
            "scope": "window",
            "window_id": "12",
            "window_generation": 12,
            "observation_generation": 3,
            "node": 2,
            "action": "invoke"
        });
        let request: NativeSemanticActionRequest = serde_json::from_value(value.clone()).unwrap();
        assert!(request.validate().is_ok());
        assert_eq!(
            format!("{request:?}"),
            "NativeSemanticActionRequest(<redacted>)"
        );
        value["observation_generation"] = 0.into();
        assert!(
            serde_json::from_value::<NativeSemanticActionRequest>(value.clone())
                .unwrap()
                .validate()
                .is_err()
        );
        value["observation_generation"] = 3.into();
        value["extra"] = true.into();
        assert!(serde_json::from_value::<NativeSemanticActionRequest>(value).is_err());
    }
}
