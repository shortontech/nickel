//! Bounded payload-free production transitions. Never accepts strings or input data.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

pub const MAX_DESKTOP_EVENTS: usize = 128;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductionEffectKind {
    ShellCommand,
    DeviceControl,
    ApplicationLaunch,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProductionEffectOutcome {
    Confirmed,
    UiUpdated,
    Requested,
    Cancelled,
    Unavailable,
    Uncertain,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum DesktopEventKind {
    /// Native process evidence made this ordinary window observable.
    WindowIdentityVerified { window_id: u64 },
    /// A previously verified ordinary native window was retired.
    WindowRetired { window_id: u64 },
    /// Compositor focus assignment to a verified ordinary window, not a native
    /// client acknowledgement. Window IDs match inventory generation values.
    KeyboardFocusChanged { window_id: u64 },
    OutputMembershipChanged {
        latest_output_identity_generation: u64,
        outputs: usize,
    },
    /// A production owner committed a typed shell continuation. Fixed enums
    /// deliberately exclude the command, target, client and application.
    ProductionEffectCompleted {
        effect: ProductionEffectKind,
        outcome: ProductionEffectOutcome,
    },
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DesktopEvent {
    pub generation: u64,
    pub observed_at_us: u64,
    pub event: DesktopEventKind,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DesktopEventSnapshot {
    pub generation: u64,
    pub evicted: u64,
    pub events: Vec<DesktopEvent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ReadDesktopEvents {
    pub lease_id: u64,
    pub after_generation: u64,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DesktopEventBatch {
    pub generation: u64,
    pub history_gap: bool,
    pub events: Vec<DesktopEvent>,
}

#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct DesktopEventObservation {
    pub observation_generation: u64,
    pub observed_at_us: u64,
    pub history: DesktopEventBatch,
}

#[derive(Default)]
pub struct DesktopEvents {
    generation: u64,
    evicted: u64,
    events: VecDeque<DesktopEvent>,
}

impl DesktopEvents {
    pub fn record(&mut self, event: DesktopEventKind, observed_at_us: u64) {
        self.generation = self.generation.saturating_add(1);
        if self.events.len() == MAX_DESKTOP_EVENTS {
            self.events.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        self.events.push_back(DesktopEvent {
            generation: self.generation,
            observed_at_us,
            event,
        });
    }
    pub fn since(&self, after: u64) -> Result<DesktopEventBatch, String> {
        if after > self.generation {
            return Err("event cursor is ahead of this session; take a fresh snapshot".into());
        }
        Ok(DesktopEventBatch {
            generation: self.generation,
            history_gap: self
                .events
                .front()
                .is_some_and(|event| after < event.generation.saturating_sub(1)),
            events: self
                .events
                .iter()
                .filter(|event| event.generation > after)
                .cloned()
                .collect(),
        })
    }

    pub fn snapshot(&self) -> DesktopEventSnapshot {
        DesktopEventSnapshot {
            generation: self.generation,
            evicted: self.evicted,
            events: self.events.iter().cloned().collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursor_reports_eviction_without_duplicates_or_silent_future_cursor_reset() {
        let mut events = DesktopEvents::default();
        assert!(events.since(0).unwrap().events.is_empty());
        assert!(events.since(1).is_err());
        for index in 1..=140 {
            events.record(
                DesktopEventKind::KeyboardFocusChanged { window_id: index },
                index,
            );
        }
        let gap = events.since(1).unwrap();
        assert!(gap.history_gap);
        assert_eq!(gap.events.len(), 128);
        assert_eq!(gap.events[0].generation, 13);
        assert!(!events.since(12).unwrap().history_gap);
        let tail = events.since(139).unwrap();
        assert_eq!(tail.events.len(), 1);
        assert_eq!(tail.generation, 140);
        let caught_up = events.since(tail.generation).unwrap();
        assert!(caught_up.events.is_empty() && !caught_up.history_gap);
        assert!(events.since(141).is_err());
    }

    #[test]
    fn transitions_keep_order_and_bound_retention_without_payload_fields() {
        let mut events = DesktopEvents::default();
        for index in 0..200 {
            events.record(
                DesktopEventKind::KeyboardFocusChanged {
                    window_id: index % 2 + 1,
                },
                index * 100,
            );
        }
        let snapshot = events.snapshot();
        assert_eq!(snapshot.generation, 200);
        assert_eq!(snapshot.evicted, 72);
        assert_eq!(snapshot.events.len(), MAX_DESKTOP_EVENTS);
        assert_eq!(snapshot.events[0].generation, 73);
        assert!(
            snapshot
                .events
                .windows(2)
                .all(|pair| pair[0].generation + 1 == pair[1].generation
                    && pair[0].observed_at_us < pair[1].observed_at_us)
        );
        assert_eq!(
            snapshot.events.last().unwrap().event,
            DesktopEventKind::KeyboardFocusChanged { window_id: 2 }
        );
    }

    #[test]
    fn production_effect_events_serialize_without_targets_or_payload_fields() {
        let mut events = DesktopEvents::default();
        events.record(
            DesktopEventKind::ProductionEffectCompleted {
                effect: ProductionEffectKind::ApplicationLaunch,
                outcome: ProductionEffectOutcome::Confirmed,
            },
            17,
        );
        let json = serde_json::to_string(&events.snapshot()).unwrap();
        assert!(json.contains("application_launch"));
        for excluded in ["command", "target", "client", "path", "application_id"] {
            assert!(!json.contains(excluded));
        }
    }
}
