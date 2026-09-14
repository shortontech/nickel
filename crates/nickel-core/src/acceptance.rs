//! Deterministic, bounded support for interaction acceptance tests.
//!
//! These types are observation aids, not an alternate interaction reducer. Tests
//! still drive production entry points and record only the facts they observe.

use std::{collections::VecDeque, fmt, time::Duration};

/// Conservative defaults for in-process acceptance fixtures.
pub const DEFAULT_TRACE_ENTRY_LIMIT: usize = 128;
pub const DEFAULT_PENDING_DEADLINE: Duration = Duration::from_secs(2);

/// A generated fixture identity that cannot accidentally contain user content.
#[derive(Clone, Copy, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct OpaqueIdentity(u64);

impl OpaqueIdentity {
    pub const fn fixture(value: u64) -> Self {
        Self(value)
    }
}

impl fmt::Debug for OpaqueIdentity {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "opaque-{}", self.0)
    }
}

/// A monotonic clock advanced explicitly by a test.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct SyntheticClock {
    now: Duration,
}

impl SyntheticClock {
    pub const fn at(now: Duration) -> Self {
        Self { now }
    }

    pub const fn now(self) -> Duration {
        self.now
    }

    pub fn advance(&mut self, elapsed: Duration) -> Duration {
        self.now = self
            .now
            .checked_add(elapsed)
            .expect("synthetic clock overflowed");
        self.now
    }

    pub fn deadline_after(self, timeout: Duration) -> Deadline {
        Deadline(
            self.now
                .checked_add(timeout)
                .expect("synthetic deadline overflowed"),
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Deadline(Duration);

impl Deadline {
    pub fn elapsed(self, clock: SyntheticClock) -> bool {
        clock.now >= self.0
    }
}

/// A bounded trace that makes loss explicit rather than silently growing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BoundedTrace<T> {
    entries: VecDeque<T>,
    limit: usize,
    dropped: usize,
}

impl<T> BoundedTrace<T> {
    pub fn new(limit: usize) -> Self {
        assert!(limit > 0, "a trace must retain at least one entry");
        Self {
            entries: VecDeque::with_capacity(limit),
            limit,
            dropped: 0,
        }
    }

    pub fn record(&mut self, entry: T) {
        if self.entries.len() == self.limit {
            self.entries.pop_front();
            self.dropped += 1;
        }
        self.entries.push_back(entry);
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = &T> {
        self.entries.iter()
    }

    pub const fn limit(&self) -> usize {
        self.limit
    }

    pub const fn dropped(&self) -> usize {
        self.dropped
    }
}

impl<T> Default for BoundedTrace<T> {
    fn default() -> Self {
        Self::new(DEFAULT_TRACE_ENTRY_LIMIT)
    }
}

/// Independent assertions over an observed trace.
pub trait TraceAssertions<T> {
    fn assert_exact(&self, expected: &[T]);
    fn assert_count(&self, expected: &T, count: usize);
    fn assert_absent(&self, forbidden: &T);
}

impl<T> TraceAssertions<T> for BoundedTrace<T>
where
    T: fmt::Debug + PartialEq,
{
    #[track_caller]
    fn assert_exact(&self, expected: &[T]) {
        let actual = self.entries().collect::<Vec<_>>();
        let expected = expected.iter().collect::<Vec<_>>();
        assert_eq!(actual, expected, "observed effect order differed");
    }

    #[track_caller]
    fn assert_count(&self, expected: &T, count: usize) {
        let actual = self.entries().filter(|entry| *entry == expected).count();
        assert_eq!(actual, count, "observed effect count differed");
    }

    #[track_caller]
    fn assert_absent(&self, forbidden: &T) {
        assert!(
            self.entries().all(|entry| entry != forbidden),
            "forbidden trace entry observed: {forbidden:?}"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_boundary_requires_no_sleep() {
        let mut clock = SyntheticClock::default();
        let deadline = clock.deadline_after(DEFAULT_PENDING_DEADLINE);

        clock.advance(DEFAULT_PENDING_DEADLINE - Duration::from_nanos(1));
        assert!(!deadline.elapsed(clock));
        clock.advance(Duration::from_nanos(1));
        assert!(deadline.elapsed(clock));
    }

    #[test]
    fn overflow_is_bounded_and_observable() {
        let mut trace = BoundedTrace::new(2);
        trace.record(OpaqueIdentity::fixture(1));
        trace.record(OpaqueIdentity::fixture(2));
        trace.record(OpaqueIdentity::fixture(3));

        trace.assert_exact(&[OpaqueIdentity::fixture(2), OpaqueIdentity::fixture(3)]);
        assert_eq!(trace.limit(), 2);
        assert_eq!(trace.dropped(), 1);
    }

    #[test]
    fn ordered_count_and_forbidden_oracles_are_independent() {
        let mut trace = BoundedTrace::new(4);
        trace.record("begin");
        trace.record("cancel");

        trace.assert_exact(&["begin", "cancel"]);
        trace.assert_count(&"cancel", 1);
        trace.assert_absent(&"commit");
    }
}
