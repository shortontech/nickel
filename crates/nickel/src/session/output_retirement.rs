//! Bounded, two-phase retirement for output protocol globals.
//!
//! Output removal cannot immediately destroy a global: clients need a short
//! window to observe its disabled state before the server removes it. This
//! module owns that lifecycle policy independently of compositor operations.

use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(crate) const BIND_SETTLE_GRACE: Duration = Duration::from_secs(3);
pub(crate) const DISABLED_GRACE: Duration = Duration::from_secs(3);
pub(crate) const MAX_PENDING: usize = nickel_session_protocol::MAX_OUTPUTS;

#[derive(Debug, Eq, PartialEq)]
pub(crate) enum RetirementAction<T> {
    Disable { identity: String, value: T },
    Remove { identity: String, value: T },
}

pub(crate) struct DeferredRetirements<T> {
    pending: VecDeque<PendingRetirement<T>>,
}

struct PendingRetirement<T> {
    identity: String,
    deadline: Instant,
    disabled: bool,
    value: T,
}

impl<T> Default for DeferredRetirements<T> {
    fn default() -> Self {
        Self {
            pending: VecDeque::new(),
        }
    }
}

impl<T> DeferredRetirements<T> {
    pub(crate) fn has_capacity(&self) -> bool {
        self.pending.len() < MAX_PENDING
    }

    pub(crate) fn defer(&mut self, now: Instant, identity: String, value: T) -> Result<(), T> {
        if !self.has_capacity() {
            return Err(value);
        }
        self.pending.push_back(PendingRetirement {
            identity,
            deadline: now + BIND_SETTLE_GRACE,
            disabled: false,
            value,
        });
        Ok(())
    }

    pub(crate) fn advance(&mut self, now: Instant) -> Vec<RetirementAction<T>>
    where
        T: Clone,
    {
        let mut actions = Vec::new();
        let mut retained = VecDeque::with_capacity(self.pending.len());
        while let Some(mut pending) = self.pending.pop_front() {
            if pending.deadline > now {
                retained.push_back(pending);
            } else if pending.disabled {
                actions.push(RetirementAction::Remove {
                    identity: pending.identity,
                    value: pending.value,
                });
            } else {
                pending.disabled = true;
                pending.deadline = now + DISABLED_GRACE;
                actions.push(RetirementAction::Disable {
                    identity: pending.identity.clone(),
                    value: pending.value.clone(),
                });
                retained.push_back(pending);
            }
        }
        self.pending = retained;
        actions
    }

    pub(crate) fn len(&self) -> usize {
        self.pending.len()
    }

    pub(crate) fn has_enabled_identity(&self, identity: &str) -> bool {
        self.pending
            .iter()
            .any(|pending| !pending.disabled && pending.identity == identity)
    }
}

pub(crate) fn capacity_available(pending: usize, live: usize) -> bool {
    pending.saturating_add(live) < MAX_PENDING
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_is_bounded_and_advances_through_both_grace_periods() {
        let started = Instant::now();
        let mut retirements = DeferredRetirements::default();
        for value in 0..MAX_PENDING {
            assert_eq!(
                retirements.defer(started, format!("output-{value}"), value),
                Ok(())
            );
        }
        assert_eq!(retirements.len(), MAX_PENDING);
        assert!(!capacity_available(MAX_PENDING - 1, 1));
        assert_eq!(
            retirements.defer(started, "overflow".into(), MAX_PENDING),
            Err(MAX_PENDING)
        );
        assert!(
            retirements
                .advance(started + BIND_SETTLE_GRACE - Duration::from_millis(1))
                .is_empty()
        );
        assert_eq!(
            retirements.advance(started + BIND_SETTLE_GRACE),
            (0..MAX_PENDING)
                .map(|value| RetirementAction::Disable {
                    identity: format!("output-{value}"),
                    value,
                })
                .collect::<Vec<_>>()
        );
        assert!(
            retirements
                .advance(started + BIND_SETTLE_GRACE + DISABLED_GRACE - Duration::from_millis(1))
                .is_empty()
        );
        assert_eq!(
            retirements.advance(started + BIND_SETTLE_GRACE + DISABLED_GRACE),
            (0..MAX_PENDING)
                .map(|value| RetirementAction::Remove {
                    identity: format!("output-{value}"),
                    value,
                })
                .collect::<Vec<_>>()
        );
        assert_eq!(retirements.len(), 0);
    }

    #[test]
    fn repeated_churn_returns_to_baseline_each_grace_window() {
        let started = Instant::now();
        let mut retirements = DeferredRetirements::default();
        for generation in 0..64 {
            let cycle = started + (BIND_SETTLE_GRACE + DISABLED_GRACE) * generation;
            for output in 0..MAX_PENDING {
                retirements
                    .defer(cycle, format!("output-{output}"), (generation, output))
                    .unwrap();
            }
            assert!(!retirements.has_capacity());
            assert_eq!(
                retirements.advance(cycle + BIND_SETTLE_GRACE).len(),
                MAX_PENDING
            );
            assert_eq!(
                retirements
                    .advance(cycle + BIND_SETTLE_GRACE + DISABLED_GRACE)
                    .len(),
                MAX_PENDING
            );
            assert_eq!(retirements.len(), 0);
        }
    }
}
