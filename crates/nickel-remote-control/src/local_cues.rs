//! Trusted local lifecycle indications. No remote text or input payload enters a cue.
use crate::{lease_audit::LeaseTransition, leases::LeaseAuthority};
use std::{
    collections::BTreeMap,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleCue {
    Start,
    Pause,
    ApproachingExpiry,
    Expired,
    Stop,
}

/// One cursor per local session owner, independent of output count or repaint rate.
#[derive(Default)]
pub struct LifecycleCues {
    audit_generation: u64,
    warned: BTreeMap<u64, Instant>,
}
impl LifecycleCues {
    /// At most one cue of each fixed kind per batch. A renewed deadline gets a
    /// fresh warning; until-logout authority has no invented expiration warning.
    pub fn collect(&mut self, leases: &LeaseAuthority, now: Instant) -> Vec<LifecycleCue> {
        let mut cues = Vec::with_capacity(5);
        let mut add = |cue| {
            if !cues.contains(&cue) {
                cues.push(cue);
            }
        };
        for event in leases
            .audit()
            .events()
            .filter(|event| event.generation > self.audit_generation)
        {
            match event.transition {
                LeaseTransition::Approved
                | LeaseTransition::Resumed
                | LeaseTransition::Reconnected => {
                    if leases.iter().any(|lease| {
                        lease.id == event.lease_id
                            && !lease.suspended
                            && lease.is_connected()
                            && lease.expires_at.is_none_or(|deadline| now < deadline)
                    }) {
                        add(LifecycleCue::Start);
                    }
                }
                LeaseTransition::Paused => add(LifecycleCue::Pause),
                LeaseTransition::Disconnected if event.allow_resumption => add(LifecycleCue::Pause),
                LeaseTransition::Disconnected | LeaseTransition::Revoked => add(LifecycleCue::Stop),
                LeaseTransition::Expired => add(LifecycleCue::Expired),
                LeaseTransition::Renewed => (),
            }
        }
        if let Some(last) = leases.audit().events().last() {
            self.audit_generation = last.generation;
        }
        self.warned
            .retain(|id, _| leases.iter().any(|lease| lease.id == *id));
        for lease in leases.iter() {
            let Some(deadline) = lease.expires_at else {
                continue;
            };
            if lease.suspended || !lease.is_connected() || now >= deadline {
                continue;
            }
            let warning = (deadline.saturating_duration_since(lease.issued_at) / 4)
                .min(Duration::from_secs(60));
            if deadline.saturating_duration_since(now) <= warning
                && self.warned.get(&lease.id) != Some(&deadline)
            {
                self.warned.insert(lease.id, deadline);
                add(LifecycleCue::ApproachingExpiry);
            }
        }
        cues
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::leases::ResourceScope;
    #[test]
    fn production_lifecycle_emits_once_and_renewal_rearms_warning() {
        let now = Instant::now();
        let mut leases = LeaseAuthority::default();
        let mut cues = LifecycleCues::default();
        let deadline = now + Duration::from_secs(120);
        let id = leases
            .approve_local(
                "client".into(),
                ResourceScope::FullSession,
                now,
                Some(deadline),
                true,
                false,
            )
            .unwrap();
        assert_eq!(cues.collect(&leases, now), [LifecycleCue::Start]);
        assert!(cues.collect(&leases, now).is_empty());
        leases.suspend_local(id).unwrap();
        assert_eq!(cues.collect(&leases, now), [LifecycleCue::Pause]);
        leases.resume_local(id, now).unwrap();
        assert_eq!(cues.collect(&leases, now), [LifecycleCue::Start]);
        assert_eq!(
            cues.collect(&leases, now + Duration::from_secs(90)),
            [LifecycleCue::ApproachingExpiry]
        );
        assert!(
            cues.collect(&leases, now + Duration::from_secs(91))
                .is_empty()
        );
        let renewed = now + Duration::from_secs(240);
        leases
            .renew_local(
                id,
                "client",
                deadline,
                Some(renewed),
                now + Duration::from_secs(91),
            )
            .unwrap();
        assert!(
            cues.collect(&leases, now + Duration::from_secs(91))
                .is_empty()
        );
        assert_eq!(
            cues.collect(&leases, now + Duration::from_secs(180)),
            [LifecycleCue::ApproachingExpiry]
        );
        leases.expire(renewed);
        assert_eq!(cues.collect(&leases, renewed), [LifecycleCue::Expired]);
        assert!(cues.collect(&leases, renewed).is_empty());
    }
    #[test]
    fn disconnect_and_stop_are_payload_free_bounded_local_events() {
        let now = Instant::now();
        let mut leases = LeaseAuthority::default();
        let mut cues = LifecycleCues::default();
        for resumable in [true, false] {
            leases
                .approve_local(
                    "private label".into(),
                    ResourceScope::FullSession,
                    now,
                    None,
                    resumable,
                    false,
                )
                .unwrap();
        }
        assert_eq!(cues.collect(&leases, now), [LifecycleCue::Start]);
        leases.disconnect("private label");
        let batch = cues.collect(&leases, now);
        assert!(batch.contains(&LifecycleCue::Pause));
        assert!(batch.contains(&LifecycleCue::Stop));
        assert!(batch.len() <= 5);
        assert!(
            cues.collect(&leases, now + Duration::from_secs(3600))
                .is_empty()
        );
    }
}
