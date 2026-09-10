//! Trusted local trace lifecycle history; no trace samples or client strings.
pub use nickel_session_protocol::RemoteTraceTransition as Transition;
use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
    time::Instant,
};
pub const MAX_TRACE_AUDIT_EVENTS: usize = 128;
#[derive(Clone, Copy)]
pub struct Event {
    pub generation: u64,
    pub observed_at: Instant,
    pub client_id: u64,
    pub lease_id: u64,
    pub trace_id: u64,
    pub category: nickel_session_protocol::RemoteTraceCategory,
    pub transition: Transition,
    pub duration_limit_seconds: u16,
    pub elapsed_us: u64,
}
#[derive(Default)]
struct History {
    events: VecDeque<Event>,
    generation: u64,
    evicted: u64,
}
#[derive(Default)]
pub struct TraceAudit(Mutex<History>);
impl TraceAudit {
    pub fn snapshot(&self) -> Option<(Vec<Event>, u64)> {
        let history = self.0.lock().ok()?;
        Some((history.events.iter().copied().collect(), history.evicted))
    }
    fn record(&self, mut event: Event) {
        if let Ok(mut history) = self.0.lock() {
            history.generation = history.generation.saturating_add(1);
            event.generation = history.generation;
            if history.events.len() == MAX_TRACE_AUDIT_EVENTS {
                history.events.pop_front();
                history.evicted = history.evicted.saturating_add(1);
            }
            history.events.push_back(event);
        }
    }
}
pub(crate) struct TraceAuditHandle {
    audit: Arc<TraceAudit>,
    initial: Event,
    finished: bool,
}
impl TraceAuditHandle {
    pub(crate) fn start(
        audit: Arc<TraceAudit>,
        client_id: u64,
        lease_id: u64,
        trace_id: u64,
        seconds: u16,
        now: Instant,
        category: nickel_session_protocol::RemoteTraceCategory,
    ) -> Self {
        let initial = Event {
            generation: 0,
            observed_at: now,
            client_id,
            lease_id,
            trace_id,
            category,
            transition: Transition::Started,
            duration_limit_seconds: seconds,
            elapsed_us: 0,
        };
        audit.record(initial);
        Self {
            audit,
            initial,
            finished: false,
        }
    }
    pub(crate) fn finish(&mut self, transition: Transition, now: Instant) {
        if self.finished {
            return;
        }
        self.finished = true;
        self.audit.record(Event {
            observed_at: now,
            transition,
            elapsed_us: now
                .saturating_duration_since(self.initial.observed_at)
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            ..self.initial
        });
    }
}
impl Drop for TraceAuditHandle {
    fn drop(&mut self) {
        self.finish(Transition::Cancelled, Instant::now());
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_audit_correlates_once_and_retains_bounded_payload_free_history() {
        let audit = Arc::new(TraceAudit::default());
        let now = Instant::now();
        for id in 0..100 {
            let mut trace = TraceAuditHandle::start(
                audit.clone(),
                7,
                11,
                id,
                1,
                now,
                nickel_session_protocol::RemoteTraceCategory::NestedFrameDispatch,
            );
            trace.finish(
                Transition::TimedOut,
                now + std::time::Duration::from_secs(1),
            );
            trace.finish(Transition::Stopped, now);
            drop(trace);
        }
        let (events, evicted) = audit.snapshot().unwrap();
        assert_eq!(events.len(), 128);
        assert_eq!(evicted, 72);
        assert_eq!(events[0].generation, 73);
        for pair in events.chunks_exact(2) {
            assert_eq!(pair[0].trace_id, pair[1].trace_id);
            assert_eq!(pair[0].client_id, 7);
            assert_eq!(pair[1].lease_id, 11);
            assert_eq!(pair[0].transition, Transition::Started);
            assert_eq!(pair[1].transition, Transition::TimedOut);
            assert_eq!(pair[1].elapsed_us, 1_000_000);
        }
        drop(TraceAuditHandle::start(
            audit.clone(),
            8,
            12,
            101,
            60,
            now,
            nickel_session_protocol::RemoteTraceCategory::NestedFrameDispatch,
        ));
        assert_eq!(
            audit.snapshot().unwrap().0.last().unwrap().transition,
            Transition::Cancelled
        );
    }
}
