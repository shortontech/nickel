//! Local-only lifecycle audit. No caller strings or input payloads can enter it.
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub const MAX_LEASE_AUDIT_EVENTS: usize = 128;

pub use nickel_session_protocol::{
    RemoteLeaseScopeKind as LeaseScopeKind, RemoteLeaseTransition as LeaseTransition,
};

#[derive(Clone, Copy, Debug)]
pub struct LeaseAuditEvent {
    pub generation: u64,
    pub observed_at: Instant,
    /// Session-local lease identity, never a token or client-supplied label.
    pub lease_id: u64,
    pub transition: LeaseTransition,
    pub scope: LeaseScopeKind,
    /// Current approved lifetime from initial issuance; None means until logout.
    pub lifetime_limit: Option<Duration>,
    pub full_debug: bool,
    pub allow_resumption: bool,
}

/// Not serializable: security-control history is for trusted local inspection.
#[derive(Default)]
pub struct LeaseAudit {
    events: VecDeque<LeaseAuditEvent>,
    generation: u64,
    evicted: u64,
}

impl LeaseAudit {
    pub(crate) fn record(&mut self, lease: &crate::leases::Lease, transition: LeaseTransition) {
        use crate::leases::ResourceScope;
        let scope = match &lease.scope {
            ResourceScope::Surface(_) => LeaseScopeKind::Surface,
            ResourceScope::Window(_) => LeaseScopeKind::Window,
            ResourceScope::Application(_) => LeaseScopeKind::Application,
            ResourceScope::Output(_) => LeaseScopeKind::Output,
            ResourceScope::FullSession => LeaseScopeKind::FullSession,
        };
        self.generation = self.generation.saturating_add(1);
        if self.events.len() == MAX_LEASE_AUDIT_EVENTS {
            self.events.pop_front();
            self.evicted = self.evicted.saturating_add(1);
        }
        self.events.push_back(LeaseAuditEvent {
            generation: self.generation,
            observed_at: Instant::now(),
            lease_id: lease.id,
            transition,
            scope,
            lifetime_limit: lease
                .expires_at
                .map(|deadline| deadline.saturating_duration_since(lease.issued_at)),
            full_debug: lease.full_debug,
            allow_resumption: lease.allow_resumption,
        });
    }

    pub fn events(&self) -> impl Iterator<Item = &LeaseAuditEvent> {
        self.events.iter()
    }

    pub fn evicted(&self) -> u64 {
        self.evicted
    }
}
