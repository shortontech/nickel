//! Read-only external accessibility structure. Each platform reports unavailable
//! fields explicitly; provider text is excluded unless its protected-state and
//! boundedness can be proved at the native authority boundary.
use schemars::JsonSchema;
use serde::Serialize;

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct NativeSemanticNode {
    /// Ordinal scoped to this observation, never an actionable object handle.
    pub id: u32,
    pub parent: Option<u32>,
    /// Numeric native-provider role/control type; unknown future roles are retained.
    pub role: u32,
    /// Logical coordinates relative to the associated native window.
    pub bounds: Option<[i32; 4]>,
    pub enabled: bool,
    pub focused: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, JsonSchema)]
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
    /// False for current native providers, which do not provide atomic tree snapshots.
    pub atomic: bool,
    /// Provider fields deliberately omitted by this platform observation.
    pub unavailable_fields: Vec<String>,
    pub nodes: Vec<NativeSemanticNode>,
    pub truncated: bool,
}
