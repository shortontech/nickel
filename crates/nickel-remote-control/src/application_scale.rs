//! Typed application compatibility policy; no raw toolkit file or command API.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Policy {
    FollowNickel,
    Unchanged,
    Custom { scale_120: u32 },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Family {
    Gtk,
    Qt,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Toolkit {
    pub family: Family,
    pub available: bool,
    /// Bounded canonical numeric value, or follow-nickel for an absent Qt key.
    pub observed: Option<String>,
    pub owned: bool,
    pub pending: bool,
    pub restart_required: bool,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Snapshot {
    pub generation: u64,
    /// Worker observation start, measured from session start.
    pub observation_started_at_us: u64,
    /// Worker observation completion, not owner delivery time.
    pub observed_at_us: u64,
    /// Settings and toolkit reads are sequential, not one atomic snapshot.
    pub atomic: bool,
    pub policy: Policy,
    pub toolkits: Vec<Toolkit>,
}
#[derive(Clone, Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Transaction {
    pub generation: u64,
    pub prior: Policy,
    pub requested: Policy,
}
#[derive(Clone, Copy, Debug, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    Unchanged,
    Confirmed,
    ExternalConflict,
    Unavailable,
    Failed,
    Uncertain,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct Outcome {
    pub family: Family,
    pub kind: OutcomeKind,
    pub restart_required: bool,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct TransactionOutcome {
    pub snapshot: Snapshot,
    pub outcomes: Vec<Outcome>,
}
