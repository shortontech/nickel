//! Typed, bounded display layout transactions owned by the production compositor.

use crate::leases::ResourceId;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_LAYOUT_OUTPUTS: usize = nickel_session_protocol::MAX_OUTPUTS;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// Exact production output incarnation. Connector labels alone are not authority.
    pub output: ResourceId,
    pub x: i32,
    pub y: i32,
    pub enabled: bool,
    /// Wayland fractional-scale units (120 == 100%).
    pub scale_120: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Layout {
    pub primary: ResourceId,
    /// Complete connected-output membership, including disabled outputs.
    pub outputs: Vec<Placement>,
}

impl Layout {
    pub fn valid_representation(&self) -> bool {
        if self.outputs.is_empty()
            || self.outputs.len() > MAX_LAYOUT_OUTPUTS
            || self.primary.id.is_empty()
            || self.primary.id.len() > 512
            || self.primary.generation == 0
        {
            return false;
        }
        let mut identities = BTreeSet::new();
        self.outputs.iter().all(|placement| {
            !placement.output.id.is_empty()
                && placement.output.id.len() <= 512
                && placement.output.generation != 0
                && (60..=480).contains(&placement.scale_120)
                && identities.insert((placement.output.id.as_str(), placement.output.generation))
        }) && self
            .outputs
            .iter()
            .any(|placement| placement.enabled && placement.output == self.primary)
    }

    /// Require the caller's complete prior/output set to match the owner snapshot exactly.
    pub fn same_output_incarnations(&self, current: &Self) -> bool {
        self.outputs.len() == current.outputs.len()
            && self.outputs.iter().all(|placement| {
                current
                    .outputs
                    .iter()
                    .any(|candidate| candidate.output == placement.output)
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub enum Transaction {
    Apply {
        topology_generation: u64,
        prior: Layout,
        requested: Layout,
    },
    Keep {
        topology_generation: u64,
        recovery_generation: u64,
    },
    Revert {
        topology_generation: u64,
        recovery_generation: u64,
    },
}

impl Transaction {
    pub fn topology_generation(&self) -> u64 {
        match self {
            Self::Apply {
                topology_generation,
                ..
            }
            | Self::Keep {
                topology_generation,
                ..
            }
            | Self::Revert {
                topology_generation,
                ..
            } => *topology_generation,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
    Confirmed,
    AwaitingConfirmation,
    RevertFailed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Recovery {
    pub state: RecoveryState,
    /// Owner generation required by Keep/Revert. Zero means no pending recovery.
    pub generation: u64,
    /// Session-uptime deadline. None after confirmation or a failed automatic revert.
    pub deadline_uptime_us: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct Snapshot {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    pub topology_generation: u64,
    /// False means the platform can report layout but has no truthful production mutation path.
    pub transaction_supported: bool,
    pub transaction_unavailable_reason: Option<String>,
    /// Current production owner state after any accepted request.
    pub requested: Layout,
    /// Last layout accepted as safe. It differs while Keep/Revert is pending.
    pub confirmed: Layout,
    pub recovery: Recovery,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(id: &str, generation: u64, x: i32) -> Placement {
        Placement {
            output: ResourceId {
                id: id.into(),
                generation,
            },
            x,
            y: 0,
            enabled: true,
            scale_120: 120,
        }
    }

    fn layout() -> Layout {
        Layout {
            primary: ResourceId {
                id: "left".into(),
                generation: 4,
            },
            outputs: vec![output("left", 4, -1920), output("right", 7, 0)],
        }
    }

    #[test]
    fn layout_requires_bounded_unique_exact_output_incarnations() {
        let current = layout();
        assert!(current.valid_representation());
        let mut reused = current.clone();
        reused.outputs[1].output.generation += 1;
        assert!(!reused.same_output_incarnations(&current));
        let mut incomplete = current.clone();
        incomplete.outputs.pop();
        assert!(!incomplete.same_output_incarnations(&current));
        let mut duplicate = current.clone();
        duplicate.outputs[1].output = duplicate.outputs[0].output.clone();
        assert!(!duplicate.valid_representation());
    }

    #[test]
    fn disabled_primary_and_invalid_scale_are_rejected() {
        let mut requested = layout();
        requested.outputs[0].enabled = false;
        assert!(!requested.valid_representation());
        requested.outputs[0].enabled = true;
        requested.outputs[0].scale_120 = 0;
        assert!(!requested.valid_representation());
    }
}
