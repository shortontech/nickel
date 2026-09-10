//! Read-only external accessibility structure. Provider strings and values are
//! deliberately unavailable: AT-SPI offers no atomic protected-text projection.
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct NativeSemanticNode {
    /// Ordinal scoped to this observation, never an actionable object handle.
    pub id: u32,
    pub parent: Option<u32>,
    /// Numeric AT-SPI role from the provider; unknown future roles are retained.
    pub role: u32,
    /// Logical coordinates relative to the associated native window.
    pub bounds: Option<[i32; 4]>,
    pub enabled: bool,
    pub focused: bool,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NativeSemanticScope {
    Window,
    /// One authenticated accessibility connection of the anchor window's OS process.
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
    /// Always false: AT-SPI does not provide atomic tree snapshots.
    pub atomic: bool,
    /// Names, descriptions, text, values and actions are not collected.
    pub unavailable_fields: Vec<String>,
    pub nodes: Vec<NativeSemanticNode>,
    pub truncated: bool,
}
