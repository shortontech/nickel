//! Executor-neutral native tree patch and scheduling protocol.
//! This crate contains serialized values only; it grants no host authority.
use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, PartialEq)]
pub enum ScheduledRender<T> {
    Unchanged,
    Rendered {
        value: T,
        dirty_components: Vec<String>,
        reconciliation_requested: bool,
    },
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct NativePatchEnvelope {
    pub version: u8,
    pub operations: Vec<NativePatchOperation>,
    pub counters: NativePatchCounters,
}

#[derive(Clone, Debug, Deserialize, serde::Serialize, PartialEq)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum NativePatchOperation {
    SetPrimitive {
        target: String,
        property: String,
        value: Value,
    },
    ReplaceHandlerSlot {
        slot: String,
        action: usize,
    },
    InsertChild {
        parent: String,
        key: String,
        child_id: String,
        index: usize,
        node: Value,
    },
    RemoveChild {
        parent: String,
        key: String,
        child_id: String,
        index: usize,
    },
    MoveChild {
        parent: String,
        key: String,
        child_id: String,
        from: usize,
        to: usize,
    },
    ReplaceSubtree {
        target: String,
        node: Value,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, serde::Serialize, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct NativePatchCounters {
    pub nodes_visited: u64,
    pub nodes_mutated: u64,
    /// Complete package-local trees materialized for compatibility/cold paths.
    #[serde(default)]
    pub local_materializations: u64,
    /// Nodes traversed by cross-package composition expansion.
    #[serde(default)]
    pub expansion_nodes: u64,
    /// Encoded complete-tree bytes crossing the package boundary.
    #[serde(default)]
    pub tree_bytes: u64,
}

#[derive(Debug, PartialEq)]
pub enum ScheduledPatch {
    Unchanged,
    Patched {
        patch: NativePatchEnvelope,
        dirty_components: Vec<String>,
        transport_bytes: usize,
        reconciliation_requested: bool,
    },
}

mod surface;
pub use surface::{
    OutputScope, SurfaceAnchor, SurfaceBounds, SurfaceBoundsProvider, SurfaceDefinition,
    SurfaceKind,
};
