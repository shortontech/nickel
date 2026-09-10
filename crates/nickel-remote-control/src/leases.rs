//! Resource authorization. Resource evidence is supplied by production platform owners, never
//! accepted from a remote caller's claims. All deadlines use the session's monotonic clock.

use std::{
    collections::{BTreeMap, BTreeSet},
    time::Instant,
};

pub use nickel_session_protocol::{
    RemoteResourceId as ResourceId, RemoteResourceScope as ResourceScope,
};

/// Bound caller-owned identifiers before retaining or projecting approval state.
/// This validates representation only; platform owners still verify identity.
pub(crate) fn valid_resource_scope(scope: &ResourceScope) -> bool {
    let valid_id =
        |id: &str| !id.is_empty() && id.len() <= 512 && !id.chars().any(char::is_control);
    match scope {
        ResourceScope::Application(id) => valid_id(id),
        ResourceScope::Surface(resource)
        | ResourceScope::Window(resource)
        | ResourceScope::Output(resource) => resource.generation != 0 && valid_id(&resource.id),
        ResourceScope::FullSession => true,
    }
}

/// Resolved immediately before each effect or observation by Nickel's resource owner.
pub struct ResourceEvidence<'a> {
    pub surface: Option<&'a ResourceId>,
    pub window: Option<&'a ResourceId>,
    pub verified_application: Option<&'a str>,
    pub output: Option<&'a ResourceId>,
    pub authorized_surface_ancestors: &'a [ResourceId],
    pub protected: bool,
}

pub trait ResourceScopeAuthority {
    fn covers(&self, resource: &ResourceEvidence<'_>) -> bool;
}

impl ResourceScopeAuthority for ResourceScope {
    fn covers(&self, resource: &ResourceEvidence<'_>) -> bool {
        if resource.protected {
            return false;
        }
        match self {
            Self::Surface(id) => {
                resource.surface == Some(id) || resource.authorized_surface_ancestors.contains(id)
            }
            Self::Window(id) => resource.window == Some(id),
            Self::Application(id) => resource.verified_application == Some(id.as_str()),
            Self::Output(id) => resource.output == Some(id),
            Self::FullSession => true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Lease {
    pub id: u64,
    pub client_identity: String,
    pub scope: ResourceScope,
    pub issued_at: Instant,
    /// None means until logout; leases are never persisted across login sessions.
    pub expires_at: Option<Instant>,
    pub allow_resumption: bool,
    pub full_debug: bool,
    pub suspended: bool,
    /// Invalidates queued operations when authority is interrupted, even if later resumed.
    pub operation_generation: u64,
    pub renewal_generation: u64,
    connected: bool,
}

impl Lease {
    pub fn is_connected(&self) -> bool {
        self.connected
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum LeaseError {
    #[error("lease is missing, expired, suspended, or outside the resource boundary")]
    Unauthorized,
    #[error("another remote operation owns shared input")]
    Busy,
    #[error("lease deadline or scope is invalid")]
    Invalid,
    #[error("lease capacity reached")]
    Capacity,
}

#[derive(Default)]
pub struct LeaseAuthority {
    audit: crate::lease_audit::LeaseAudit,
    next_id: u64,
    leases: BTreeMap<u64, Lease>,
    input_owner: Option<(u64, u64)>,
    cancelled: BTreeSet<u64>,
}

impl LeaseAuthority {
    pub fn audit(&self) -> &crate::lease_audit::LeaseAudit {
        &self.audit
    }
    /// Called only after trusted local approval, never by an MCP operation directly.
    pub fn approve_local(
        &mut self,
        client_identity: String,
        scope: ResourceScope,
        now: Instant,
        expires_at: Option<Instant>,
        allow_resumption: bool,
        full_debug: bool,
    ) -> Result<u64, LeaseError> {
        if client_identity.is_empty()
            || !valid_resource_scope(&scope)
            || expires_at.is_some_and(|deadline| deadline <= now)
            || (full_debug && scope != ResourceScope::FullSession)
        {
            return Err(LeaseError::Invalid);
        }
        if self.leases.len() + self.cancelled.len() >= 128 {
            return Err(LeaseError::Capacity);
        }
        self.next_id = self.next_id.checked_add(1).ok_or(LeaseError::Capacity)?;
        let id = self.next_id;
        self.leases.insert(
            id,
            Lease {
                id,
                client_identity,
                scope,
                issued_at: now,
                expires_at,
                allow_resumption,
                full_debug,
                suspended: false,
                operation_generation: 0,
                renewal_generation: 0,
                connected: true,
            },
        );
        self.audit.record(
            &self.leases[&id],
            crate::lease_audit::LeaseTransition::Approved,
        );
        Ok(id)
    }

    /// Extend an existing lease only after a trusted local renewal decision.
    /// The displayed deadline is a compare-and-set token: replaying approval
    /// cannot extend the lease again. Scope, diagnostic authority, resumption
    /// policy and ownership of an ongoing gesture are deliberately unchanged.
    pub fn renew_local(
        &mut self,
        id: u64,
        authenticated_client: &str,
        displayed_deadline: Instant,
        expires_at: Option<Instant>,
        now: Instant,
    ) -> Result<(), LeaseError> {
        let lease = self.active_lease(id, authenticated_client, now)?;
        if lease.expires_at != Some(displayed_deadline)
            || expires_at.is_some_and(|deadline| deadline <= displayed_deadline)
        {
            return Err(LeaseError::Invalid);
        }
        let generation = lease
            .renewal_generation
            .checked_add(1)
            .ok_or(LeaseError::Capacity)?;
        let lease = self.leases.get_mut(&id).expect("active lease exists");
        lease.expires_at = expires_at;
        lease.renewal_generation = generation;
        self.audit
            .record(lease, crate::lease_audit::LeaseTransition::Renewed);
        Ok(())
    }

    pub fn authorize(
        &self,
        id: u64,
        authenticated_client: &str,
        resource: &ResourceEvidence<'_>,
        now: Instant,
        debug: bool,
    ) -> Result<&Lease, LeaseError> {
        let lease = self.active_lease(id, authenticated_client, now)?;
        if (!debug || lease.full_debug) && lease.scope.covers(resource) {
            Ok(lease)
        } else {
            Err(LeaseError::Unauthorized)
        }
    }

    pub fn active_lease(
        &self,
        id: u64,
        authenticated_client: &str,
        now: Instant,
    ) -> Result<&Lease, LeaseError> {
        self.leases
            .get(&id)
            .filter(|lease| {
                lease.client_identity == authenticated_client
                    && !lease.suspended
                    && lease.connected
                    && lease.expires_at.is_none_or(|deadline| now < deadline)
            })
            .ok_or(LeaseError::Unauthorized)
    }

    /// Reserve shared focus/input for an already authorized operation. Call authorize again at
    /// every dispatch boundary; reservation alone conveys no permission to deliver an event.
    pub fn reserve_input(&mut self, lease: u64, operation: u64) -> Result<(), LeaseError> {
        if !self.leases.contains_key(&lease) {
            return Err(LeaseError::Unauthorized);
        }
        let owner = (lease, operation);
        if self.input_owner.is_some() {
            return Err(LeaseError::Busy);
        }
        self.input_owner = Some(owner);
        Ok(())
    }

    pub fn release_input(&mut self, lease: u64, operation: u64) {
        if self.input_owner == Some((lease, operation)) {
            self.input_owner = None;
        }
    }

    pub(crate) fn owns_input(&self, lease: u64, operation: u64) -> bool {
        self.input_owner == Some((lease, operation))
    }

    /// Returns whether this lease owned shared input. The caller must cancel production effects
    /// and release its synthesized input before returning to event dispatch.
    pub fn revoke(&mut self, id: u64) -> bool {
        self.retire(id, crate::lease_audit::LeaseTransition::Revoked)
    }

    fn retire(&mut self, id: u64, transition: crate::lease_audit::LeaseTransition) -> bool {
        if let Some(lease) = self.leases.remove(&id) {
            self.audit.record(&lease, transition);
            self.cancelled.insert(id);
        }
        if self.input_owner.is_some_and(|(lease, _)| lease == id) {
            self.input_owner = None;
            true
        } else {
            false
        }
    }

    pub fn revoke_client(&mut self, client: &str) -> Vec<u64> {
        let ids: Vec<_> = self
            .leases
            .values()
            .filter(|lease| lease.client_identity == client)
            .map(|lease| lease.id)
            .collect();
        for id in &ids {
            self.revoke(*id);
        }
        ids
    }

    pub fn expire(&mut self, now: Instant) -> Vec<u64> {
        let ids: Vec<_> = self
            .leases
            .values()
            .filter(|lease| lease.expires_at.is_some_and(|deadline| now >= deadline))
            .map(|lease| lease.id)
            .collect();
        for id in &ids {
            self.retire(*id, crate::lease_audit::LeaseTransition::Expired);
        }
        ids
    }

    pub fn clear(&mut self) {
        for lease in self.leases.values() {
            self.audit
                .record(lease, crate::lease_audit::LeaseTransition::Revoked);
        }
        self.cancelled.extend(self.leases.keys().copied());
        self.leases.clear();
        self.input_owner = None;
    }

    /// Drain at the production dispatch boundary before allowing further remote effects.
    pub fn take_cancellations(&mut self) -> BTreeSet<u64> {
        std::mem::take(&mut self.cancelled)
    }

    pub fn iter(&self) -> impl Iterator<Item = &Lease> {
        self.leases.values()
    }

    pub fn suspend_local(&mut self, id: u64) -> Result<(), LeaseError> {
        let lease = self.leases.get_mut(&id).ok_or(LeaseError::Unauthorized)?;
        let changed = !lease.suspended;
        lease.suspended = true;
        lease.operation_generation = lease
            .operation_generation
            .checked_add(1)
            .ok_or(LeaseError::Capacity)?;
        self.cancelled.insert(id);
        if changed {
            self.audit
                .record(lease, crate::lease_audit::LeaseTransition::Paused);
        }
        if self.input_owner.is_some_and(|(lease, _)| lease == id) {
            self.input_owner = None;
        }
        Ok(())
    }

    pub fn resume_local(&mut self, id: u64, now: Instant) -> Result<(), LeaseError> {
        let lease = self.leases.get_mut(&id).ok_or(LeaseError::Unauthorized)?;
        if lease.expires_at.is_some_and(|deadline| now >= deadline) {
            return Err(LeaseError::Unauthorized);
        }
        let changed = lease.suspended;
        lease.suspended = false;
        if changed {
            self.audit
                .record(lease, crate::lease_audit::LeaseTransition::Resumed);
        }
        Ok(())
    }

    /// A reconnect never resumes locally paused authority and never recreates revoked leases.
    /// The caller must have authenticated the same identity before invoking this method.
    pub fn reconnect_authenticated(&mut self, client: &str, now: Instant) {
        self.expire(now);
        for lease in self
            .leases
            .values_mut()
            .filter(|lease| lease.client_identity == client)
        {
            if lease.allow_resumption && !lease.connected {
                lease.connected = true;
                self.audit
                    .record(lease, crate::lease_audit::LeaseTransition::Reconnected);
            }
        }
    }

    pub fn disconnect(&mut self, client: &str) {
        let ids: Vec<_> = self
            .leases
            .values()
            .filter(|lease| lease.client_identity == client)
            .map(|lease| (lease.id, lease.allow_resumption))
            .collect();
        for (id, resumable) in ids {
            let lease = &self.leases[&id];
            if lease.connected {
                self.audit
                    .record(lease, crate::lease_audit::LeaseTransition::Disconnected);
            }
            self.cancelled.insert(id);
            if resumable {
                let lease = self.leases.get_mut(&id).unwrap();
                lease.connected = false;
                if let Some(next) = lease.operation_generation.checked_add(1) {
                    lease.operation_generation = next;
                } else {
                    self.revoke(id);
                }
                if self.input_owner.is_some_and(|(lease, _)| lease == id) {
                    self.input_owner = None;
                }
            } else {
                self.revoke(id);
            }
        }
    }

    /// Destruction ends resource-specific authority immediately, even before an ID is reused.
    pub fn retire_resource(&mut self, resource: &ResourceId) {
        let ids: Vec<_> = self
            .leases
            .values()
            .filter(|lease| {
                matches!(&lease.scope,
            ResourceScope::Surface(id) | ResourceScope::Window(id) | ResourceScope::Output(id)
                if id == resource)
            })
            .map(|lease| lease.id)
            .collect();
        for id in ids {
            self.revoke(id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn lifecycle_audit_records_production_transitions_and_bounds_retention() {
        use crate::lease_audit::{LeaseTransition as Event, MAX_LEASE_AUDIT_EVENTS};
        let now = Instant::now();
        let mut authority = LeaseAuthority::default();
        let id = authority
            .approve_local(
                "private-client-canary".into(),
                ResourceScope::Application("private-app-canary".into()),
                now,
                Some(now + Duration::from_secs(30)),
                false,
                false,
            )
            .unwrap();
        authority.suspend_local(id).unwrap();
        authority.suspend_local(id).unwrap();
        authority.resume_local(id, now).unwrap();
        authority.resume_local(id, now).unwrap();
        authority
            .renew_local(
                id,
                "private-client-canary",
                now + Duration::from_secs(30),
                Some(now + Duration::from_secs(60)),
                now,
            )
            .unwrap();
        authority.expire(now + Duration::from_secs(60));
        authority.revoke(id);
        let events = authority.audit().events().copied().collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .map(|event| event.transition)
                .collect::<Vec<_>>(),
            vec![
                Event::Approved,
                Event::Paused,
                Event::Resumed,
                Event::Renewed,
                Event::Expired
            ]
        );
        assert!(events.iter().all(|event| event.lease_id == id));
        assert!(events.iter().all(|event| event.scope
            == crate::lease_audit::LeaseScopeKind::Application
            && !event.full_debug
            && !event.allow_resumption));
        assert_eq!(events[0].lifetime_limit, Some(Duration::from_secs(30)));
        assert_eq!(events[3].lifetime_limit, Some(Duration::from_secs(60)));
        assert!(!format!("{events:?}").contains("canary"));
        authority.take_cancellations();
        for _ in 0..100 {
            let id = authority
                .approve_local(
                    "a".into(),
                    ResourceScope::FullSession,
                    now,
                    None,
                    false,
                    false,
                )
                .unwrap();
            authority.clear();
            authority.revoke(id);
            authority.take_cancellations();
        }
        let retained = authority.audit().events().collect::<Vec<_>>();
        assert_eq!(retained.len(), MAX_LEASE_AUDIT_EVENTS);
        assert_eq!(
            authority.audit().evicted(),
            205 - MAX_LEASE_AUDIT_EVENTS as u64
        );
        assert!(
            retained
                .windows(2)
                .all(|pair| pair[1].generation == pair[0].generation + 1
                    && pair[1].observed_at >= pair[0].observed_at)
        );
    }

    #[test]
    fn local_renewal_extends_only_the_displayed_live_lease_without_changing_input_owner() {
        let now = Instant::now();
        let original = now + Duration::from_secs(1200);
        let extended = original + Duration::from_secs(7200);
        let scope = ResourceScope::Window(ResourceId {
            id: "window".into(),
            generation: 4,
        });
        let mut authority = LeaseAuthority::default();
        let id = authority
            .approve_local("a".into(), scope.clone(), now, Some(original), true, false)
            .unwrap();
        authority.reserve_input(id, 7).unwrap();
        assert_eq!(
            authority.renew_local(id, "other", original, Some(extended), now),
            Err(LeaseError::Unauthorized)
        );
        for invalid in [now, original] {
            assert_eq!(
                authority.renew_local(id, "a", original, Some(invalid), now),
                Err(LeaseError::Invalid)
            );
        }
        assert_eq!(
            authority.active_lease(id, "a", now).unwrap().expires_at,
            Some(original)
        );
        authority
            .renew_local(id, "a", original, Some(extended), now)
            .unwrap();
        let lease = authority.active_lease(id, "a", original).unwrap();
        assert_eq!(lease.scope, scope);
        assert_eq!(lease.issued_at, now);
        assert_eq!(lease.operation_generation, 0);
        assert!(lease.allow_resumption);
        assert!(!lease.full_debug);
        assert!(authority.owns_input(id, 7));
        assert!(authority.take_cancellations().is_empty());
        assert_eq!(
            authority.renew_local(
                id,
                "a",
                original,
                Some(extended + Duration::from_secs(1)),
                now
            ),
            Err(LeaseError::Invalid)
        );
        authority.renew_local(id, "a", extended, None, now).unwrap();
        assert_eq!(
            authority
                .active_lease(id, "a", extended)
                .unwrap()
                .expires_at,
            None
        );
        assert_eq!(
            authority.renew_local(id, "a", extended, None, now),
            Err(LeaseError::Invalid)
        );
    }

    #[test]
    fn renewal_cannot_restore_expired_paused_disconnected_or_revoked_authority() {
        let now = Instant::now();
        let deadline = now + Duration::from_secs(60);
        for state in ["expired", "paused", "disconnected", "revoked"] {
            let mut authority = LeaseAuthority::default();
            let id = authority
                .approve_local(
                    "a".into(),
                    ResourceScope::FullSession,
                    now,
                    Some(deadline),
                    true,
                    true,
                )
                .unwrap();
            let at = match state {
                "expired" => deadline,
                "paused" => {
                    authority.suspend_local(id).unwrap();
                    now
                }
                "disconnected" => {
                    authority.disconnect("a");
                    now
                }
                "revoked" => {
                    authority.revoke(id);
                    now
                }
                _ => unreachable!(),
            };
            assert_eq!(
                authority.renew_local(id, "a", deadline, None, at),
                Err(LeaseError::Unauthorized),
                "{state}"
            );
            assert!(authority.active_lease(id, "a", at).is_err(), "{state}");
        }
    }

    #[test]
    fn connection_audit_distinguishes_resumption_from_revocation_without_duplicates() {
        use crate::lease_audit::LeaseTransition as Event;
        let now = Instant::now();
        let mut authority = LeaseAuthority::default();
        let resumable = authority
            .approve_local(
                "a".into(),
                ResourceScope::FullSession,
                now,
                None,
                true,
                true,
            )
            .unwrap();
        authority.disconnect("a");
        authority.disconnect("a");
        authority.reconnect_authenticated("a", now);
        authority.reconnect_authenticated("a", now);
        let events = authority.audit().events().collect::<Vec<_>>();
        assert_eq!(
            events
                .iter()
                .map(|event| event.transition)
                .collect::<Vec<_>>(),
            vec![Event::Approved, Event::Disconnected, Event::Reconnected]
        );
        assert!(events.iter().all(|event| event.lease_id == resumable
            && event.lifetime_limit.is_none()
            && event.full_debug
            && event.allow_resumption));
        let temporary = authority
            .approve_local(
                "b".into(),
                ResourceScope::FullSession,
                now,
                None,
                false,
                false,
            )
            .unwrap();
        authority.disconnect("b");
        authority.reconnect_authenticated("b", now);
        assert!(authority.active_lease(temporary, "b", now).is_err());
        assert_eq!(
            authority
                .audit()
                .events()
                .filter(|event| event.lease_id == temporary)
                .map(|event| event.transition)
                .collect::<Vec<_>>(),
            vec![Event::Approved, Event::Disconnected, Event::Revoked]
        );
    }

    #[test]
    fn reconnect_cannot_undo_local_pause_expiry_or_revocation() {
        let now = Instant::now();
        let mut authority = LeaseAuthority::default();
        let id = authority
            .approve_local(
                "a".into(),
                ResourceScope::FullSession,
                now,
                Some(now + Duration::from_secs(10)),
                true,
                false,
            )
            .unwrap();
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: None,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        authority.reserve_input(id, 1).unwrap();
        authority.disconnect("a");
        assert_eq!(authority.take_cancellations(), BTreeSet::from([id]));
        assert!(authority.authorize(id, "a", &evidence, now, false).is_err());
        authority.reconnect_authenticated("a", now);
        assert!(authority.authorize(id, "a", &evidence, now, false).is_ok());
        authority.suspend_local(id).unwrap();
        authority.disconnect("a");
        authority.reconnect_authenticated("a", now);
        assert!(authority.authorize(id, "a", &evidence, now, false).is_err());
        authority.resume_local(id, now).unwrap();
        authority.reconnect_authenticated("a", now + Duration::from_secs(10));
        assert!(authority.iter().next().is_none());
        authority.reconnect_authenticated("a", now);
        assert!(authority.authorize(id, "a", &evidence, now, false).is_err());
    }

    #[test]
    fn resource_moves_do_not_extend_identity_or_authorize_protected_surfaces() {
        let window = ResourceId {
            id: "window".into(),
            generation: 1,
        };
        let output = ResourceId {
            id: "other-monitor".into(),
            generation: 2,
        };
        let mut evidence = ResourceEvidence {
            window: Some(&window),
            surface: None,
            verified_application: Some("app"),
            output: Some(&output),
            authorized_surface_ancestors: &[],
            protected: false,
        };
        assert!(ResourceScope::Window(window.clone()).covers(&evidence));
        assert!(ResourceScope::Application("app".into()).covers(&evidence));
        assert!(!ResourceScope::Application("launched-app".into()).covers(&evidence));
        assert!(
            !ResourceScope::Window(ResourceId {
                generation: 2,
                ..window.clone()
            })
            .covers(&evidence)
        );
        evidence.protected = true;
        assert!(!ResourceScope::FullSession.covers(&evidence));
    }

    #[test]
    fn expiry_and_revocation_prevent_delayed_authority_and_input_takeover() {
        let now = Instant::now();
        let mut authority = LeaseAuthority::default();
        let a = authority
            .approve_local(
                "a".into(),
                ResourceScope::FullSession,
                now,
                Some(now + Duration::from_secs(1)),
                false,
                false,
            )
            .unwrap();
        let b = authority
            .approve_local(
                "b".into(),
                ResourceScope::FullSession,
                now,
                None,
                false,
                false,
            )
            .unwrap();
        let evidence = ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: None,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        assert!(authority.authorize(a, "b", &evidence, now, false).is_err());
        authority.reserve_input(a, 10).unwrap();
        assert_eq!(authority.reserve_input(b, 11), Err(LeaseError::Busy));
        authority.release_input(b, 10);
        assert_eq!(authority.reserve_input(b, 11), Err(LeaseError::Busy));
        let deadline = now + Duration::from_secs(1);
        assert!(
            authority
                .authorize(a, "a", &evidence, deadline, false)
                .is_err()
        );
        assert_eq!(authority.expire(deadline), vec![a]);
        authority.reserve_input(b, 11).unwrap();
        authority.clear();
        assert!(authority.authorize(b, "b", &evidence, now, false).is_err());
    }
}
