//! Trusted local operation history. Request and result payloads cannot enter this collector.

use std::{
    collections::VecDeque,
    sync::Mutex,
    time::{Duration, Instant},
};

pub const MAX_OPERATION_AUDIT_EVENTS: usize = 128;

pub use nickel_session_protocol::RemoteOperationOutcome as Outcome;

#[derive(Clone, Copy)]
pub struct Event {
    pub generation: u64,
    pub observed_at: Instant,
    /// Fixed server-owned MCP method name, never supplied by a caller.
    pub method: &'static str,
    /// First lease that authorized this operation. Absent when authorization never succeeded.
    pub matched_lease_id: Option<u64>,
    /// Duration through the method result; cancellation does not imply rollback.
    pub duration: Duration,
    pub outcome: Outcome,
}

#[derive(Default)]
struct History {
    events: VecDeque<Event>,
    generation: u64,
    evicted: u64,
}

/// This type is deliberately not serializable. Only the trusted local session projection may read
/// it; MCP diagnostics and tool responses do not expose an accessor.
#[derive(Default)]
pub struct OperationAudit(Mutex<History>);

impl OperationAudit {
    pub fn snapshot(&self) -> Option<(Vec<Event>, u64)> {
        let history = self.0.lock().ok()?;
        Some((history.events.iter().copied().collect(), history.evicted))
    }

    pub(crate) fn record(
        &self,
        method: &'static str,
        matched_lease_id: Option<u64>,
        duration: Duration,
        outcome: Outcome,
    ) {
        if let Ok(mut history) = self.0.lock() {
            history.generation = history.generation.saturating_add(1);
            let generation = history.generation;
            if history.events.len() == MAX_OPERATION_AUDIT_EVENTS {
                history.events.pop_front();
                history.evicted = history.evicted.saturating_add(1);
            }
            history.events.push_back(Event {
                generation,
                observed_at: Instant::now(),
                method,
                matched_lease_id,
                duration,
                outcome,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn history_is_bounded_and_contains_only_fixed_metadata() {
        let audit = OperationAudit::default();
        for id in 0..(MAX_OPERATION_AUDIT_EVENTS as u64 + 17) {
            audit.record(
                "pointer_action",
                (id % 2 == 0).then_some(id),
                Duration::from_micros(id),
                if id % 3 == 0 {
                    Outcome::Error
                } else {
                    Outcome::Success
                },
            );
        }

        let (events, evicted) = audit.snapshot().unwrap();
        assert_eq!(events.len(), MAX_OPERATION_AUDIT_EVENTS);
        assert_eq!(evicted, 17);
        assert_eq!(events[0].generation, 18);
        assert_eq!(events.last().unwrap().generation, 145);
        assert!(events.iter().all(|event| event.method == "pointer_action"));
        assert_eq!(events.last().unwrap().duration, Duration::from_micros(144));
        assert_eq!(events.last().unwrap().matched_lease_id, Some(144));
        assert_eq!(events.last().unwrap().outcome, Outcome::Error);
    }
}
