use std::{collections::VecDeque, time::Duration};

pub const DEFAULT_FOCUS_REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const TERMINAL_HISTORY_LIMIT: usize = 32;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FocusTransaction(pub u64);

pub type FocusRequestId = FocusTransaction;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FocusTargetLifetime {
    /// The target identity itself is generation-bearing (for example, a Wayland object ID).
    EmbeddedInTarget,
    Generation(u64),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct FocusSecurityEpoch(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum FocusScope {
    Ordinary,
    Launcher,
    Lock,
    Other(u64),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusRejectionReason {
    NativeDenied,
    TargetRetired,
    ScopeWithdrawn,
    AuthorityLost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FocusRequestPhase {
    Pending,
    Realized,
    Rejected(FocusRejectionReason),
    TimedOut,
    Superseded,
}

impl FocusRequestPhase {
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Pending)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusRequest<Surface> {
    pub transaction: FocusTransaction,
    pub surface: Surface,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FocusRequestRecord<Surface> {
    pub request: FocusRequest<Surface>,
    pub target_lifetime: FocusTargetLifetime,
    pub prior_eligible: Option<Surface>,
    pub scope: FocusScope,
    pub security_epoch: FocusSecurityEpoch,
    pub deadline: Duration,
    pub phase: FocusRequestPhase,
}

#[derive(Debug)]
pub struct FocusTransactions<Surface> {
    next: u64,
    current: Option<FocusRequestRecord<Surface>>,
    terminal: VecDeque<FocusRequestRecord<Surface>>,
    now: Duration,
}

impl<Surface> Default for FocusTransactions<Surface> {
    fn default() -> Self {
        Self {
            next: 0,
            current: None,
            terminal: VecDeque::new(),
            now: Duration::ZERO,
        }
    }
}

impl<Surface: Clone + Eq> FocusTransactions<Surface> {
    /// Compatibility entry point for callers without an external monotonic clock.
    /// New production consumers should use [`Self::request_at`].
    pub fn request(&mut self, surface: Surface) -> FocusRequest<Surface> {
        self.request_at(
            surface,
            FocusTargetLifetime::EmbeddedInTarget,
            None,
            FocusScope::Ordinary,
            FocusSecurityEpoch(0),
            self.now,
            DEFAULT_FOCUS_REQUEST_TIMEOUT,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn request_at(
        &mut self,
        surface: Surface,
        target_lifetime: FocusTargetLifetime,
        prior_eligible: Option<Surface>,
        scope: FocusScope,
        security_epoch: FocusSecurityEpoch,
        now: Duration,
        timeout: Duration,
    ) -> FocusRequest<Surface> {
        self.advance_to(now);
        if let Some(current) = self.current.take() {
            let terminal = if current.phase == FocusRequestPhase::Pending {
                FocusRequestPhase::Superseded
            } else {
                current.phase
            };
            self.record_terminal(FocusRequestRecord {
                phase: terminal,
                ..current
            });
        }
        self.next = self.next.wrapping_add(1).max(1);
        let request = FocusRequest {
            transaction: FocusTransaction(self.next),
            surface,
        };
        self.current = Some(FocusRequestRecord {
            request: request.clone(),
            target_lifetime,
            prior_eligible,
            scope,
            security_epoch,
            deadline: now.saturating_add(timeout),
            phase: FocusRequestPhase::Pending,
        });
        request
    }

    pub fn acknowledge(&mut self, request: &FocusRequest<Surface>) -> bool {
        self.acknowledge_at(request, self.now)
    }

    pub fn acknowledge_at(&mut self, request: &FocusRequest<Surface>, now: Duration) -> bool {
        self.advance_to(now);
        let Some(current) = self.current.as_mut() else {
            return false;
        };
        if current.request != *request || current.phase != FocusRequestPhase::Pending {
            return false;
        }
        current.phase = FocusRequestPhase::Realized;
        true
    }

    pub fn reject(
        &mut self,
        request: &FocusRequest<Surface>,
        reason: FocusRejectionReason,
        now: Duration,
    ) -> bool {
        self.advance_to(now);
        if !self.current.as_ref().is_some_and(|current| {
            current.request == *request && current.phase == FocusRequestPhase::Pending
        }) {
            return false;
        }
        self.finish_current(FocusRequestPhase::Rejected(reason));
        true
    }

    pub fn advance_to(&mut self, now: Duration) -> bool {
        self.now = self.now.max(now);
        let timed_out = self.current.as_ref().is_some_and(|current| {
            current.phase == FocusRequestPhase::Pending && self.now >= current.deadline
        });
        if timed_out {
            self.finish_current(FocusRequestPhase::TimedOut);
        }
        timed_out
    }

    /// Advance one scheduled request without allowing a delayed timer for an
    /// older request to expire newer intent.
    pub fn advance_request_to(&mut self, request: FocusRequestId, now: Duration) -> bool {
        if !self
            .current
            .as_ref()
            .is_some_and(|current| current.request.transaction == request)
        {
            return false;
        }
        self.advance_to(now)
    }

    pub fn loses_current(&mut self, request: &FocusRequest<Surface>) -> bool {
        if !self.current.as_ref().is_some_and(|current| {
            current.request == *request && current.phase == FocusRequestPhase::Realized
        }) {
            return false;
        }
        self.finish_current(FocusRequestPhase::Realized);
        true
    }

    pub fn requested(&self) -> Option<&FocusRequest<Surface>> {
        self.current.as_ref().map(|current| &current.request)
    }

    pub fn acknowledged(&self) -> Option<&FocusRequest<Surface>> {
        self.current
            .as_ref()
            .filter(|current| current.phase == FocusRequestPhase::Realized)
            .map(|current| &current.request)
    }

    pub fn phase(&self, request: &FocusRequest<Surface>) -> Option<FocusRequestPhase> {
        self.record(request).map(|record| record.phase)
    }

    pub fn record(&self, request: &FocusRequest<Surface>) -> Option<&FocusRequestRecord<Surface>> {
        self.current
            .as_ref()
            .filter(|current| current.request == *request)
            .or_else(|| {
                self.terminal
                    .iter()
                    .rev()
                    .find(|record| record.request.transaction == request.transaction)
            })
    }

    fn finish_current(&mut self, phase: FocusRequestPhase) {
        if let Some(mut current) = self.current.take() {
            current.phase = phase;
            self.record_terminal(current);
        }
    }

    fn record_terminal(&mut self, record: FocusRequestRecord<Surface>) {
        debug_assert!(record.phase.is_terminal());
        if self.terminal.len() == TERMINAL_HISTORY_LIMIT {
            self.terminal.pop_front();
        }
        self.terminal.push_back(record);
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FocusRejectionReason, FocusRequestPhase, FocusScope, FocusSecurityEpoch,
        FocusTargetLifetime, FocusTransactions,
    };
    use std::time::Duration;

    fn request_at(
        focus: &mut FocusTransactions<&'static str>,
        surface: &'static str,
        now_ms: u64,
    ) -> super::FocusRequest<&'static str> {
        focus.request_at(
            surface,
            FocusTargetLifetime::Generation(7),
            Some("prior"),
            FocusScope::Launcher,
            FocusSecurityEpoch(3),
            Duration::from_millis(now_ms),
            Duration::from_millis(100),
        )
    }

    #[test]
    fn matching_native_realization_completes_the_pending_request() {
        let mut focus = FocusTransactions::default();
        let request = request_at(&mut focus, "launcher", 10);
        assert_eq!(focus.phase(&request), Some(FocusRequestPhase::Pending));
        assert!(focus.acknowledge_at(&request, Duration::from_millis(20)));
        assert_eq!(focus.phase(&request), Some(FocusRequestPhase::Realized));
    }

    #[test]
    fn native_rejection_is_terminal() {
        let mut focus = FocusTransactions::default();
        let request = request_at(&mut focus, "launcher", 10);
        assert!(focus.reject(
            &request,
            FocusRejectionReason::NativeDenied,
            Duration::from_millis(20)
        ));
        assert_eq!(
            focus.phase(&request),
            Some(FocusRequestPhase::Rejected(
                FocusRejectionReason::NativeDenied
            ))
        );
        assert!(focus.requested().is_none());
    }

    #[test]
    fn synthetic_clock_expires_a_pending_request_at_its_absolute_deadline() {
        let mut focus = FocusTransactions::default();
        let request = request_at(&mut focus, "launcher", 10);
        assert!(!focus.advance_to(Duration::from_millis(109)));
        assert!(focus.advance_to(Duration::from_millis(110)));
        assert_eq!(focus.phase(&request), Some(FocusRequestPhase::TimedOut));
        assert!(focus.requested().is_none());
    }

    #[test]
    fn newer_request_supersedes_pending_but_does_not_rewrite_realized_history() {
        let mut focus = FocusTransactions::default();
        let first = request_at(&mut focus, "launcher", 10);
        assert!(focus.acknowledge_at(&first, Duration::from_millis(20)));
        let second = request_at(&mut focus, "launcher", 30);
        assert_eq!(focus.phase(&first), Some(FocusRequestPhase::Realized));
        assert_eq!(focus.phase(&second), Some(FocusRequestPhase::Pending));
        assert!(!focus.loses_current(&first));
    }

    #[test]
    fn late_acknowledgement_cannot_resurrect_timed_out_or_superseded_request() {
        let mut focus = FocusTransactions::default();
        let timed_out = request_at(&mut focus, "a", 0);
        focus.advance_to(Duration::from_millis(100));
        assert!(!focus.acknowledge_at(&timed_out, Duration::from_millis(101)));
        assert_eq!(focus.phase(&timed_out), Some(FocusRequestPhase::TimedOut));

        let superseded = request_at(&mut focus, "b", 110);
        let current = request_at(&mut focus, "c", 120);
        assert!(!focus.acknowledge_at(&superseded, Duration::from_millis(121)));
        assert_eq!(
            focus.phase(&superseded),
            Some(FocusRequestPhase::Superseded)
        );
        assert_eq!(focus.phase(&current), Some(FocusRequestPhase::Pending));
    }

    #[test]
    fn delayed_timer_for_superseded_request_cannot_expire_newer_intent() {
        let mut focus = FocusTransactions::default();
        let old = request_at(&mut focus, "a", 0);
        let current = request_at(&mut focus, "b", 50);

        assert!(!focus.advance_request_to(old.transaction, Duration::from_millis(200)));
        assert_eq!(focus.phase(&old), Some(FocusRequestPhase::Superseded));
        assert_eq!(focus.phase(&current), Some(FocusRequestPhase::Pending));
    }
}
