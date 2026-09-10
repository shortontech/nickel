//! Bounded, task-resource permission requests. This module never grants authority by itself.

use std::{
    collections::{BTreeMap, VecDeque},
    time::{Duration, Instant},
};

use crate::leases::ResourceScope;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaseRequest {
    pub renewal: Option<nickel_session_protocol::RemoteLeaseRenewal>,
    pub scope: ResourceScope,
    /// None means until logout.
    pub duration: Option<Duration>,
    pub allow_resumption: bool,
    pub full_debug: bool,
}

impl From<nickel_session_protocol::RemoteLeaseRequest> for LeaseRequest {
    fn from(request: nickel_session_protocol::RemoteLeaseRequest) -> Self {
        Self {
            renewal: request.renewal,
            scope: request.scope,
            duration: request.duration_seconds.map(Duration::from_secs),
            allow_resumption: request.allow_resumption,
            full_debug: request.full_debug,
        }
    }
}

impl From<&LeaseRequest> for nickel_session_protocol::RemoteLeaseRequest {
    fn from(request: &LeaseRequest) -> Self {
        Self {
            renewal: request.renewal.clone(),
            scope: request.scope.clone(),
            duration_seconds: request.duration.map(|value| value.as_secs()),
            allow_resumption: request.allow_resumption,
            full_debug: request.full_debug,
        }
    }
}

#[derive(Debug, Eq, PartialEq, thiserror::Error)]
pub enum RequestError {
    #[error("client is not authenticated")]
    Unauthorized,
    #[error("client is blocked")]
    Blocked,
    #[error("permission request cooldown")]
    Cooldown { retry_after: Duration },
    #[error("permission request capacity reached")]
    Capacity,
    #[error("invalid lease request")]
    Invalid,
}

impl RequestError {
    fn audit_outcome(&self) -> Outcome {
        match self {
            Self::Unauthorized => Outcome::Unauthorized,
            Self::Blocked => Outcome::BlockedRequest,
            Self::Cooldown { .. } => Outcome::Cooldown,
            Self::Capacity => Outcome::Capacity,
            Self::Invalid => Outcome::Invalid,
        }
    }
}

#[derive(Default)]
struct ClientRequests {
    audit_id: u64,
    pending: Option<LeaseRequest>,
    pending_generation: u64,
    pending_changes: nickel_session_protocol::RemoteLeaseRequestChanges,
    blocked: bool,
    denials: u8,
    retry_at: Option<Instant>,
}

pub use nickel_session_protocol::RemotePermissionOutcome as Outcome;

pub const MAX_PERMISSION_AUDIT_EVENTS: usize = 128;

/// Payload-free local history. Client IDs are assigned locally, never caller labels.
#[derive(Clone, Copy, Debug)]
pub struct PermissionAuditEvent {
    pub generation: u64,
    pub observed_at: Instant,
    pub client_id: Option<u64>,
    pub outcome: Outcome,
}

#[derive(Default)]
pub struct LeaseRequests {
    clients: BTreeMap<String, ClientRequests>,
    outcomes: [u64; 11],
    audit: VecDeque<PermissionAuditEvent>,
    audit_generation: u64,
    audit_evicted: u64,
    request_generation: u64,
}

impl LeaseRequests {
    pub(crate) fn audit_client_id(&self, client: &str) -> Option<u64> {
        self.clients.get(client).map(|state| state.audit_id)
    }
    /// Returns true for a new or changed request, false for an equivalent pending request.
    pub fn request(
        &mut self,
        authenticated_client: &str,
        request: LeaseRequest,
        now: Instant,
    ) -> Result<bool, RequestError> {
        let result = self.request_inner(authenticated_client, request, now);
        let outcome = match &result {
            Ok(true) => Outcome::Submitted,
            Ok(false) => Outcome::Coalesced,
            Err(error) => error.audit_outcome(),
        };
        self.record(
            outcome,
            self.clients
                .get(authenticated_client)
                .map(|state| state.audit_id),
        );
        result
    }

    fn record(&mut self, outcome: Outcome, client_id: Option<u64>) {
        self.outcomes[outcome as usize] = self.outcomes[outcome as usize].saturating_add(1);
        self.audit_generation = self.audit_generation.saturating_add(1);
        if self.audit.len() == MAX_PERMISSION_AUDIT_EVENTS {
            self.audit.pop_front();
            self.audit_evicted = self.audit_evicted.saturating_add(1);
        }
        self.audit.push_back(PermissionAuditEvent {
            generation: self.audit_generation,
            observed_at: Instant::now(),
            client_id,
            outcome,
        });
    }

    pub fn audit(&self) -> impl Iterator<Item = &PermissionAuditEvent> {
        self.audit.iter()
    }

    pub fn audit_evicted(&self) -> u64 {
        self.audit_evicted
    }

    /// Record authority validation failures before a request reaches the pending queue.
    pub(crate) fn record_rejected(&mut self, authenticated_client: &str, error: &RequestError) {
        self.record(
            error.audit_outcome(),
            self.clients
                .get(authenticated_client)
                .map(|state| state.audit_id),
        );
    }

    pub(crate) fn record_unauthorized(&mut self) {
        self.record(Outcome::Unauthorized, None);
    }

    pub(crate) fn metrics(&self) -> String {
        use std::fmt::Write;
        let mut text = String::from(
            "# HELP nickel_mcp_permission_requests_total Coarse request submissions and local transitions; submitted includes changed requests.\n# TYPE nickel_mcp_permission_requests_total counter\n",
        );
        for (outcome, count) in [
            "submitted",
            "coalesced",
            "approved",
            "denied",
            "cancelled",
            "blocked",
            "invalid",
            "capacity",
            "cooldown",
            "blocked_request",
            "unauthorized",
        ]
        .iter()
        .zip(self.outcomes)
        {
            let _ = writeln!(
                text,
                "nickel_mcp_permission_requests_total{{outcome=\"{outcome}\"}} {count}"
            );
        }
        text
    }

    fn request_inner(
        &mut self,
        authenticated_client: &str,
        request: LeaseRequest,
        now: Instant,
    ) -> Result<bool, RequestError> {
        if authenticated_client.is_empty()
            || !crate::leases::valid_resource_scope(&request.scope)
            || authenticated_client.len() > 128
            || request
                .duration
                .is_some_and(|duration| duration.is_zero() || now.checked_add(duration).is_none())
            || (request.full_debug && request.scope != ResourceScope::FullSession)
        {
            return Err(RequestError::Invalid);
        }
        if !self.clients.contains_key(authenticated_client) && self.clients.len() >= 128 {
            return Err(RequestError::Capacity);
        }
        let pending_count = self
            .clients
            .values()
            .filter(|client| client.pending.is_some())
            .count();
        let audit_id = self.clients.len() as u64 + 1;
        let client = self
            .clients
            .entry(authenticated_client.to_owned())
            .or_insert_with(|| ClientRequests {
                audit_id,
                ..Default::default()
            });
        if client.blocked {
            return Err(RequestError::Blocked);
        }
        if let Some(retry_at) = client.retry_at.filter(|deadline| now < *deadline) {
            return Err(RequestError::Cooldown {
                retry_after: retry_at.duration_since(now),
            });
        }
        if client.pending.as_ref() == Some(&request) {
            return Ok(false);
        }
        if client.pending.is_none() && pending_count >= 16 {
            return Err(RequestError::Capacity);
        }
        let generation = self
            .request_generation
            .checked_add(1)
            .ok_or(RequestError::Capacity)?;
        if let Some(previous) = &client.pending {
            client.pending_changes.access_changed |= previous.scope != request.scope
                || previous.full_debug != request.full_debug
                || previous.allow_resumption != request.allow_resumption;
            client.pending_changes.duration_increased |= match (previous.duration, request.duration)
            {
                (Some(before), Some(after)) => after > before,
                (Some(_), None) => true,
                _ => false,
            };
        } else {
            client.pending_changes = Default::default();
        }
        client.pending = Some(request);
        client.pending_generation = generation;
        self.request_generation = generation;
        Ok(true)
    }

    pub fn pending(&self) -> impl Iterator<Item = (&str, &LeaseRequest)> {
        self.clients.iter().filter_map(|(id, client)| {
            client
                .pending
                .as_ref()
                .map(|request| (id.as_str(), request))
        })
    }

    pub fn pending_changes(
        &self,
        client: &str,
    ) -> nickel_session_protocol::RemoteLeaseRequestChanges {
        self.clients
            .get(client)
            .filter(|client| client.pending.is_some())
            .map(|client| client.pending_changes)
            .unwrap_or_default()
    }

    pub fn pending_generation(&self, client: &str) -> Option<u64> {
        self.clients
            .get(client)
            .filter(|state| state.pending.is_some())
            .map(|state| state.pending_generation)
    }

    /// Local UI must compare the displayed incarnation and payload before approval.
    /// A client broadening its pending scope must not inherit approval of an older card.
    pub fn take_approved_local(
        &mut self,
        client: &str,
        displayed: &LeaseRequest,
        pending_generation: u64,
    ) -> Option<LeaseRequest> {
        let state = self.clients.get_mut(client)?;
        if state.blocked
            || state.pending_generation != pending_generation
            || state.pending.as_ref() != Some(displayed)
        {
            return None;
        }
        state.denials = 0;
        state.retry_at = None;
        let request = state.pending.take();
        let audit_id = state.audit_id;
        self.record(Outcome::Approved, Some(audit_id));
        request
    }

    pub fn deny_displayed_local(
        &mut self,
        client: &str,
        displayed: &LeaseRequest,
        pending_generation: u64,
        now: Instant,
    ) -> bool {
        if self.pending_generation(client) != Some(pending_generation)
            || !self
                .clients
                .get(client)
                .is_some_and(|state| state.pending.as_ref() == Some(displayed))
        {
            return false;
        }
        self.deny_local(client, now);
        true
    }

    pub fn deny_local(&mut self, client: &str, now: Instant) {
        if let Some(state) = self.clients.get_mut(client) {
            if state.pending.take().is_none() {
                return;
            }
            state.denials = state.denials.saturating_add(1);
            let seconds = (5_u64 << state.denials.saturating_sub(1).min(6)).min(300);
            state.retry_at = now.checked_add(Duration::from_secs(seconds));
            let audit_id = state.audit_id;
            self.record(Outcome::Denied, Some(audit_id));
        }
    }

    pub fn is_blocked(&self, client: &str) -> bool {
        self.clients.get(client).is_some_and(|state| state.blocked)
    }

    pub fn block_local(&mut self, client: &str, blocked: bool) -> bool {
        if client.is_empty()
            || client.len() > 128
            || (!self.clients.contains_key(client) && self.clients.len() >= 128)
        {
            return false;
        }
        {
            let audit_id = self.clients.len() as u64 + 1;
            let state = self
                .clients
                .entry(client.to_owned())
                .or_insert_with(|| ClientRequests {
                    audit_id,
                    ..Default::default()
                });
            state.blocked = blocked;
            if state.pending.take().is_some() {
                let audit_id = state.audit_id;
                self.record(
                    if blocked {
                        Outcome::Blocked
                    } else {
                        Outcome::Cancelled
                    },
                    Some(audit_id),
                );
            }
        }
        true
    }

    pub fn cancel_pending(&mut self) {
        let mut cancelled = Vec::new();
        for state in self.clients.values_mut() {
            if state.pending.take().is_some() {
                cancelled.push(state.audit_id);
            }
        }
        for audit_id in cancelled {
            self.record(Outcome::Cancelled, Some(audit_id));
        }
    }

    /// Retire requests whose production authority no longer exists. This is a
    /// cancellation, not a user denial, and must not impose a denial cooldown.
    pub(crate) fn retain_pending(
        &mut self,
        mut keep: impl FnMut(&str, &LeaseRequest) -> bool,
    ) -> u64 {
        let mut cancelled = Vec::new();
        for (client, state) in &mut self.clients {
            if state
                .pending
                .as_ref()
                .is_some_and(|request| !keep(client, request))
            {
                state.pending = None;
                cancelled.push(state.audit_id);
            }
        }
        let count = cancelled.len() as u64;
        for audit_id in cancelled {
            self.record(Outcome::Cancelled, Some(audit_id));
        }
        count
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pending_incarnation_survives_coalescing_but_never_reappears() {
        let now = Instant::now();
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::FullSession,
            duration: None,
            allow_resumption: false,
            full_debug: false,
        };
        let mut requests = LeaseRequests::default();
        requests.request("client", request.clone(), now).unwrap();
        let old = requests.pending_generation("client").unwrap();
        assert!(!requests.request("client", request.clone(), now).unwrap());
        assert_eq!(requests.pending_generation("client"), Some(old));
        requests.cancel_pending();
        assert_eq!(requests.pending_generation("client"), None);
        requests.request("client", request.clone(), now).unwrap();
        let fresh = requests.pending_generation("client").unwrap();
        assert!(fresh > old);
        assert!(
            requests
                .take_approved_local("client", &request, old)
                .is_none()
        );
        assert!(!requests.deny_displayed_local("client", &request, old, now));
        assert_eq!(requests.pending_generation("client"), Some(fresh));
        assert!(
            requests
                .take_approved_local("client", &request, fresh)
                .is_some()
        );
        requests.request("client", request.clone(), now).unwrap();
        let next = requests.pending_generation("client").unwrap();
        assert!(next > fresh);
        requests.request_generation = u64::MAX;
        let changed = LeaseRequest {
            full_debug: true,
            ..request.clone()
        };
        assert_eq!(
            requests.request("client", changed, now),
            Err(RequestError::Capacity)
        );
        assert_eq!(requests.pending_generation("client"), Some(next));
        assert!(requests.deny_displayed_local("client", &request, next, now));
    }

    #[test]
    fn invalid_resource_identifiers_cannot_replace_pending_approval_or_create_leases() {
        use crate::leases::{LeaseAuthority, LeaseError, ResourceId};
        let now = Instant::now();
        let original = LeaseRequest {
            renewal: None,
            scope: ResourceScope::Application("verified-application".into()),
            duration: Some(Duration::from_secs(1200)),
            allow_resumption: false,
            full_debug: false,
        };
        let mut requests = LeaseRequests::default();
        requests.request("client", original.clone(), now).unwrap();
        let mut leases = LeaseAuthority::default();
        let mut invalid = Vec::new();
        for id in [
            String::new(),
            "x".repeat(513),
            "é".repeat(257),
            "app\nFull desktop".into(),
            "app\0".into(),
        ] {
            invalid.push(ResourceScope::Application(id.clone()));
            let resource = ResourceId { id, generation: 1 };
            invalid.extend([
                ResourceScope::Surface(resource.clone()),
                ResourceScope::Window(resource.clone()),
                ResourceScope::Output(resource),
            ]);
        }
        let zero = ResourceId {
            id: "output".into(),
            generation: 0,
        };
        invalid.extend([
            ResourceScope::Surface(zero.clone()),
            ResourceScope::Window(zero.clone()),
            ResourceScope::Output(zero),
        ]);
        for scope in invalid {
            assert_eq!(
                requests.request(
                    "client",
                    LeaseRequest {
                        scope: scope.clone(),
                        ..original.clone()
                    },
                    now
                ),
                Err(RequestError::Invalid)
            );
            assert_eq!(
                leases.approve_local("client".into(), scope, now, None, false, false),
                Err(LeaseError::Invalid)
            );
            assert_eq!(requests.pending_changes("client"), Default::default());
        }
        // Invalid replacements leave the exact displayed request approvable.
        requests
            .take_approved_local(
                "client",
                &original,
                requests.pending_generation("client").unwrap_or(0),
            )
            .unwrap();
        let boundary = ResourceScope::Application("é".repeat(256));
        requests
            .request(
                "client",
                LeaseRequest {
                    scope: boundary.clone(),
                    ..original
                },
                now,
            )
            .unwrap();
        assert!(
            leases
                .approve_local("client".into(), boundary, now, None, false, false)
                .is_ok()
        );
    }

    #[test]
    fn pending_change_warnings_accumulate_until_a_local_decision() {
        let now = Instant::now();
        let original = LeaseRequest {
            renewal: None,
            scope: ResourceScope::Application("app".into()),
            duration: Some(Duration::from_secs(1200)),
            allow_resumption: false,
            full_debug: false,
        };
        let mut requests = LeaseRequests::default();
        requests.request("client", original.clone(), now).unwrap();
        assert_eq!(requests.pending_changes("client"), Default::default());
        let changed = LeaseRequest {
            scope: ResourceScope::FullSession,
            duration: Some(Duration::from_secs(7200)),
            ..original.clone()
        };
        requests.request("client", changed.clone(), now).unwrap();
        let flags = requests.pending_changes("client");
        assert!(flags.access_changed && flags.duration_increased);
        assert!(!requests.request("client", changed, now).unwrap());
        requests.request("client", original.clone(), now).unwrap();
        assert_eq!(requests.pending_changes("client"), flags);
        assert_eq!(requests.pending().count(), 1);
        requests
            .take_approved_local(
                "client",
                &original,
                requests.pending_generation("client").unwrap_or(0),
            )
            .unwrap();
        assert_eq!(requests.pending_changes("client"), Default::default());
        requests.request("client", original, now).unwrap();
        assert_eq!(requests.pending_changes("client"), Default::default());
        assert_eq!(requests.pending_changes("other-client"), Default::default());
    }

    #[test]
    fn pending_duration_warning_treats_until_logout_as_an_increase() {
        for (before, after, increased) in [
            (Some(60), None, true),
            (None, Some(60), false),
            (Some(120), Some(60), false),
            (Some(60), Some(120), true),
        ] {
            let now = Instant::now();
            let mut requests = LeaseRequests::default();
            let original = LeaseRequest {
                renewal: None,
                scope: ResourceScope::FullSession,
                duration: before.map(Duration::from_secs),
                allow_resumption: false,
                full_debug: false,
            };
            requests.request("client", original.clone(), now).unwrap();
            requests
                .request(
                    "client",
                    LeaseRequest {
                        duration: after.map(Duration::from_secs),
                        ..original
                    },
                    now,
                )
                .unwrap();
            let changes = requests.pending_changes("client");
            assert_eq!(changes.duration_increased, increased);
            assert!(!changes.access_changed);
        }
    }

    #[test]
    fn pending_access_warning_includes_debug_and_resumption_changes() {
        for (full_debug, allow_resumption) in [(true, false), (false, true)] {
            let now = Instant::now();
            let mut requests = LeaseRequests::default();
            let original = LeaseRequest {
                renewal: None,
                scope: ResourceScope::FullSession,
                duration: None,
                full_debug: false,
                allow_resumption: false,
            };
            requests.request("client", original.clone(), now).unwrap();
            requests
                .request(
                    "client",
                    LeaseRequest {
                        full_debug,
                        allow_resumption,
                        ..original
                    },
                    now,
                )
                .unwrap();
            assert!(requests.pending_changes("client").access_changed);
            assert!(!requests.pending_changes("client").duration_increased);
        }
    }

    #[test]
    fn local_decisions_count_once_and_stale_denial_does_not_extend_cooldown() {
        let now = Instant::now();
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::Application("private-application-canary".into()),
            duration: None,
            allow_resumption: false,
            full_debug: false,
        };
        let mut requests = LeaseRequests::default();
        requests
            .request("private-client-canary", request.clone(), now)
            .unwrap();
        requests
            .request("private-client-canary", request.clone(), now)
            .unwrap();
        requests.deny_local("private-client-canary", now);
        requests.deny_local("private-client-canary", now + Duration::from_secs(4));
        assert!(matches!(
            requests.request("private-client-canary", request.clone(), now),
            Err(RequestError::Cooldown { .. })
        ));
        requests
            .request(
                "private-client-canary",
                request.clone(),
                now + Duration::from_secs(5),
            )
            .unwrap();
        assert!(
            requests
                .take_approved_local(
                    "private-client-canary",
                    &request,
                    requests
                        .pending_generation("private-client-canary")
                        .unwrap_or(0)
                )
                .is_some()
        );
        assert!(
            requests
                .take_approved_local(
                    "private-client-canary",
                    &request,
                    requests
                        .pending_generation("private-client-canary")
                        .unwrap_or(0)
                )
                .is_none()
        );
        requests
            .request(
                "private-client-canary",
                request.clone(),
                now + Duration::from_secs(6),
            )
            .unwrap();
        requests.cancel_pending();
        requests.cancel_pending();
        requests
            .request(
                "private-client-canary",
                request.clone(),
                now + Duration::from_secs(7),
            )
            .unwrap();
        requests.block_local("private-client-canary", true);
        requests.block_local("private-client-canary", true);
        assert_eq!(
            requests.request(
                "private-client-canary",
                request,
                now + Duration::from_secs(8)
            ),
            Err(RequestError::Blocked)
        );
        let text = requests.metrics();
        for (outcome, count) in [
            ("submitted", 4),
            ("coalesced", 1),
            ("approved", 1),
            ("denied", 1),
            ("cancelled", 1),
            ("blocked", 1),
            ("cooldown", 1),
            ("blocked_request", 1),
        ] {
            assert!(text.contains(&format!("outcome=\"{outcome}\"}} {count}\n")));
        }
        assert_eq!(
            text.lines().filter(|line| !line.starts_with('#')).count(),
            11
        );
        assert!(!text.contains("private-"));
    }
    #[test]
    fn permission_history_is_bounded_correlated_and_payload_free() {
        let now = Instant::now();
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::Application("secret-app-canary".into()),
            duration: None,
            allow_resumption: false,
            full_debug: false,
        };
        let mut requests = LeaseRequests::default();
        requests.record_unauthorized();
        requests
            .request("secret-client-canary", request.clone(), now)
            .unwrap();
        requests
            .request("secret-client-canary", request.clone(), now)
            .unwrap();
        requests.deny_local("secret-client-canary", now);
        requests.deny_local("secret-client-canary", now);
        requests
            .request("other-client-canary", request.clone(), now)
            .unwrap();
        requests
            .take_approved_local(
                "other-client-canary",
                &request,
                requests
                    .pending_generation("other-client-canary")
                    .unwrap_or(0),
            )
            .unwrap();
        requests
            .request("other-client-canary", request.clone(), now)
            .unwrap();
        requests.retain_pending(|_, _| false);
        requests.retain_pending(|_, _| false);
        let events: Vec<_> = requests.audit().copied().collect();
        assert_eq!(
            events.iter().map(|event| event.outcome).collect::<Vec<_>>(),
            vec![
                Outcome::Unauthorized,
                Outcome::Submitted,
                Outcome::Coalesced,
                Outcome::Denied,
                Outcome::Submitted,
                Outcome::Approved,
                Outcome::Submitted,
                Outcome::Cancelled,
            ]
        );
        assert_eq!(events[0].client_id, None);
        assert!(
            events[1..4]
                .iter()
                .all(|event| event.client_id == events[1].client_id)
        );
        assert!(
            events[4..]
                .iter()
                .all(|event| event.client_id == events[4].client_id)
        );
        assert_ne!(events[1].client_id, events[4].client_id);
        assert!(!format!("{events:?}").contains("canary"));
        for _ in 0..100 {
            requests
                .request("other-client-canary", request.clone(), now)
                .unwrap();
            requests.cancel_pending();
        }
        let retained: Vec<_> = requests.audit().collect();
        assert_eq!(retained.len(), MAX_PERMISSION_AUDIT_EVENTS);
        assert_eq!(requests.audit_evicted(), 80);
        assert_eq!(retained[0].generation, 81);
        assert_eq!(retained.last().unwrap().generation, 208);
        assert!(
            retained
                .windows(2)
                .all(|pair| pair[0].generation < pair[1].generation
                    && pair[0].observed_at <= pair[1].observed_at)
        );
        assert!(!requests.metrics().contains("canary"));
    }

    #[test]
    fn changed_request_cannot_reuse_old_approval_and_denials_have_bounded_retry() {
        let now = Instant::now();
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::Application("app".into()),
            duration: Some(Duration::from_secs(1200)),
            allow_resumption: false,
            full_debug: false,
        };
        let mut requests = LeaseRequests::default();
        assert_eq!(requests.request("client", request.clone(), now), Ok(true));
        assert_eq!(requests.request("client", request.clone(), now), Ok(false));
        let broader = LeaseRequest {
            renewal: None,
            scope: ResourceScope::FullSession,
            ..request.clone()
        };
        requests.request("client", broader.clone(), now).unwrap();
        assert!(
            requests
                .take_approved_local(
                    "client",
                    &request,
                    requests.pending_generation("client").unwrap_or(0)
                )
                .is_none()
        );
        requests.deny_local("client", now);
        assert_eq!(
            requests.request("client", broader.clone(), now),
            Err(RequestError::Cooldown {
                retry_after: Duration::from_secs(5)
            })
        );
        let later = now + Duration::from_secs(5);
        requests.request("client", broader, later).unwrap();
        requests.block_local("client", true);
        assert_eq!(
            requests.request("client", request, later),
            Err(RequestError::Blocked)
        );
        assert_eq!(requests.pending().count(), 0);
    }
}
