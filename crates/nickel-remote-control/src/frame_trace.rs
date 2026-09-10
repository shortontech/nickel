//! Bounded CPU frame-dispatch traces. No pixels, window identity, or input payloads.
use crate::DesktopPermit;
use schemars::JsonSchema;
use serde::Serialize;
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub use nickel_session_protocol::RemoteTraceCategory as FrameTraceCategory;

pub const MAX_FRAME_TRACE_RECORDS: usize = 256;
pub const MAX_FRAME_TRACE_SECONDS: u16 = 60;
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FrameTraceRecord {
    pub generation: u64,
    pub observed_at_us: u64,
    pub output_generation: u64,
    /// Wall time spent dispatching a redraw, not GPU presentation latency.
    pub cpu_dispatch_us: u64,
}
#[derive(Clone, Debug, Serialize, JsonSchema)]
pub struct FrameTraceSnapshot {
    pub category: FrameTraceCategory,
    pub active: bool,
    pub duration_seconds: u16,
    pub observed_at_us: u64,
    pub generation: u64,
    pub evicted: u64,
    pub records: Vec<FrameTraceRecord>,
}
struct History {
    category: FrameTraceCategory,
    started: Instant,
    deadline: Instant,
    duration_seconds: u16,
    stopped: bool,
    generation: u64,
    evicted: u64,
    records: VecDeque<FrameTraceRecord>,
}
impl History {
    fn new(seconds: u16, now: Instant, category: FrameTraceCategory) -> Result<Self, String> {
        if !(1..=MAX_FRAME_TRACE_SECONDS).contains(&seconds) {
            return Err("frame trace duration must be 1..=60 seconds".into());
        }
        Ok(Self {
            category,
            started: now,
            deadline: now + Duration::from_secs(u64::from(seconds)),
            duration_seconds: seconds,
            stopped: false,
            generation: 0,
            evicted: 0,
            records: VecDeque::new(),
        })
    }
    fn active(&self, now: Instant) -> bool {
        !self.stopped && now < self.deadline
    }
    fn record(&mut self, now: Instant, output_generation: u64, duration: Duration) {
        if !self.active(now) {
            return;
        }
        self.generation = self.generation.saturating_add(1);
        if self.records.len() == MAX_FRAME_TRACE_RECORDS {
            self.records.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        self.records.push_back(FrameTraceRecord {
            generation: self.generation,
            observed_at_us: micros(now.saturating_duration_since(self.started)),
            output_generation,
            cpu_dispatch_us: micros(duration),
        });
    }
    fn snapshot(&self, now: Instant) -> FrameTraceSnapshot {
        FrameTraceSnapshot {
            category: self.category,
            active: self.active(now),
            duration_seconds: self.duration_seconds,
            observed_at_us: micros(now.saturating_duration_since(self.started)),
            generation: self.generation,
            evicted: self.evicted,
            records: self.records.iter().cloned().collect(),
        }
    }
}
fn micros(duration: Duration) -> u64 {
    duration.as_micros().min(u128::from(u64::MAX)) as u64
}
/// Session owner holds at most one trace. Construction occurs inside its fresh
/// full-debug authorization transaction; collection rechecks that standing lease.
pub struct FrameTrace {
    permit: DesktopPermit,
    history: History,
    audit: crate::trace_audit::TraceAuditHandle,
}
impl FrameTrace {
    pub fn new_authorized(
        permit: DesktopPermit,
        seconds: u16,
        category: FrameTraceCategory,
    ) -> Result<Self, String> {
        let now = Instant::now();
        let history = History::new(seconds, now, category)?;
        let audit = crate::trace_audit::TraceAuditHandle::start(
            permit
                .trace_audit
                .clone()
                .ok_or("trace audit unavailable")?,
            permit
                .audit_client_id
                .ok_or("trace client identity unavailable")?,
            permit.lease,
            permit.operation_id.ok_or("trace identity unavailable")?,
            seconds,
            now,
            category,
        );
        Ok(Self {
            permit,
            history,
            audit,
        })
    }

    pub fn owned_by(&self, request: &DesktopPermit) -> bool {
        std::sync::Arc::ptr_eq(&self.permit.control, &request.control)
            && self.permit.client == request.client
            && self.permit.lease == request.lease
            && self.permit.operation_generation == request.operation_generation
    }
    pub fn revalidate(&mut self, protected: bool) -> bool {
        let valid = self
            .permit
            .continued_observation()
            .and_then(|permit| permit.with_debug(protected, || Ok(())))
            .is_ok();
        if !valid {
            self.audit
                .finish(crate::trace_audit::Transition::Cancelled, Instant::now());
        } else if Instant::now() >= self.history.deadline {
            self.audit.finish(
                crate::trace_audit::Transition::TimedOut,
                self.history.deadline,
            );
        }
        valid
    }
    pub fn active(&self) -> bool {
        self.history.active(Instant::now())
    }
    pub fn stop(&mut self) {
        let now = Instant::now();
        if now >= self.history.deadline {
            self.audit.finish(
                crate::trace_audit::Transition::TimedOut,
                self.history.deadline,
            );
        } else {
            self.audit
                .finish(crate::trace_audit::Transition::Stopped, now);
        }
        self.history.stopped = true;
    }
    pub fn snapshot(&self) -> FrameTraceSnapshot {
        self.history.snapshot(Instant::now())
    }
    pub fn record(&mut self, protected: bool, output_generation: u64, duration: Duration) -> bool {
        if !self.revalidate(protected) {
            return false;
        }
        let Ok(permit) = self.permit.continued_observation() else {
            return false;
        };
        permit
            .with_debug(protected, || {
                self.history
                    .record(Instant::now(), output_generation, duration);
                Ok(())
            })
            .is_ok()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn trace_deadline_stop_and_retention_bound_actual_recording() {
        let now = Instant::now();
        assert!(History::new(0, now, FrameTraceCategory::NestedFrameDispatch).is_err());
        assert!(History::new(61, now, FrameTraceCategory::NestedFrameDispatch).is_err());
        let mut history = History::new(1, now, FrameTraceCategory::NestedFrameDispatch).unwrap();
        for i in 0..300 {
            history.record(now + Duration::from_micros(i), 7, Duration::from_micros(42));
        }
        history.record(now + Duration::from_secs(1), 7, Duration::ZERO);
        let snapshot = history.snapshot(now + Duration::from_secs(1));
        assert!(!snapshot.active);
        assert_eq!(snapshot.generation, 300);
        assert_eq!(snapshot.evicted, 44);
        assert_eq!(snapshot.records.len(), 256);
        assert_eq!(snapshot.records[0].generation, 45);
        assert_eq!(snapshot.records[0].cpu_dispatch_us, 42);
        let mut stopped = History::new(60, now, FrameTraceCategory::NestedFrameDispatch).unwrap();
        stopped.stopped = true;
        stopped.record(now, 7, Duration::ZERO);
        assert_eq!(stopped.generation, 0);
    }
}
