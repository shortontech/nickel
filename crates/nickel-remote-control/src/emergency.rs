//! Nonblocking invalidation shared with native emergency-key hooks.
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

/// Triggering this handle invalidates current and subsequently created permits
/// without acquiring the desktop authority mutex. Native input release and
/// listener shutdown still belong to the owner. Re-enabling requires owner
/// cleanup first; an earlier permit can never acquire the new epoch.
#[derive(Clone)]
pub struct EmergencyStopHandle(Arc<AtomicU64>);

impl Default for EmergencyStopHandle {
    fn default() -> Self {
        Self(Arc::new(AtomicU64::new(1)))
    }
}
impl EmergencyStopHandle {
    /// Latch Disabled without allocation, system calls, or authority locks.
    /// Repeated stops advance the epoch so an overlapping re-enable cannot
    /// acknowledge an emergency it has not actually cleaned up.
    pub fn trigger(&self) {
        self.latch();
    }
    pub(crate) fn latch(&self) -> u64 {
        let previous = self
            .0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |epoch| {
                Some(epoch.saturating_add(2) | 1)
            })
            .unwrap();
        previous.saturating_add(2) | 1
    }
    pub(crate) fn epoch(&self) -> u64 {
        self.0.load(Ordering::SeqCst)
    }
    pub(crate) fn stopped(&self) -> bool {
        self.epoch() & 1 != 0
    }
    /// Re-enable only the exact stop epoch acknowledged by owner cleanup.
    /// A newer hook stop wins, even when the listener was already disabled.
    pub(crate) fn rearm(&self, acknowledged: u64) -> bool {
        let Some(next) = acknowledged.checked_add(1) else {
            return false;
        };
        acknowledged & 1 != 0
            && self
                .0
                .compare_exchange(acknowledged, next, Ordering::SeqCst, Ordering::SeqCst)
                .is_ok()
    }
    pub(crate) fn ticket(&self) -> EmergencyTicket {
        EmergencyTicket {
            handle: self.clone(),
            epoch: self.0.load(Ordering::SeqCst),
        }
    }
}

#[derive(Clone)]
pub(crate) struct EmergencyTicket {
    handle: EmergencyStopHandle,
    epoch: u64,
}
impl EmergencyTicket {
    pub(crate) fn check(&self) -> Result<(), String> {
        if self.epoch & 1 == 0 && self.handle.0.load(Ordering::SeqCst) == self.epoch {
            Ok(())
        } else {
            Err("remote authority was stopped".into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stopped_and_old_epoch_tickets_never_revive() {
        let handle = EmergencyStopHandle::default();
        let disabled = handle.ticket();
        assert!(disabled.check().is_err());
        assert!(handle.rearm(handle.epoch()));
        let running = handle.ticket();
        assert!(running.check().is_ok());
        handle.trigger();
        let stopped = handle.ticket();
        assert!(running.check().is_err());
        assert!(stopped.check().is_err());
        let acknowledged = handle.epoch();
        handle.trigger();
        assert!(!handle.rearm(acknowledged));
        assert!(handle.rearm(handle.epoch()));
        assert!(handle.ticket().check().is_ok());
        assert!(running.check().is_err());
        assert!(stopped.check().is_err());
        assert!(disabled.check().is_err());
    }
    #[test]
    fn epoch_exhaustion_stays_stopped() {
        let handle = EmergencyStopHandle(Arc::new(AtomicU64::new(u64::MAX)));
        assert!(!handle.rearm(handle.epoch()));
        assert!(handle.stopped());
        assert!(handle.ticket().check().is_err());
    }
    fn authorized() -> (
        Arc<std::sync::Mutex<crate::ControlPlane>>,
        crate::DesktopPermit,
        EmergencyStopHandle,
    ) {
        let mut control = crate::ControlPlane::default();
        control.set_enabled(true);
        let identity = control.connect_identity("emergency test").unwrap();
        crate::ready_connection(&mut control, &identity, std::time::Instant::now());
        let lease = control
            .leases_mut()
            .approve_local(
                identity.client_id.clone(),
                crate::leases::ResourceScope::FullSession,
                std::time::Instant::now(),
                None,
                false,
                false,
            )
            .unwrap();
        let emergency = control.emergency.clone();
        let control = Arc::new(std::sync::Mutex::new(control));
        let permit =
            crate::DesktopPermit::new(control.clone(), identity.client_id, identity.token, lease);
        (control, permit, emergency)
    }
    #[test]
    fn emergency_invalidates_permits_while_authority_mutex_is_busy() {
        use std::{
            sync::mpsc,
            time::{Duration, Instant},
        };
        let (control, permit, emergency) = authorized();
        assert!(permit.check_live().is_ok());
        let worker_permit = permit.clone();
        let (entered, observing) = mpsc::sync_channel(1);
        let (release, waiting) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            worker_permit.with_input(
                &crate::leases::ResourceEvidence {
                    surface: None,
                    window: None,
                    verified_application: None,
                    output: None,
                    authorized_surface_ancestors: &[],
                    protected: false,
                },
                || {
                    entered.send(()).unwrap();
                    // Recovery bounds a failed early-rejection regression: the owner
                    // mutex is released even if the observer accidentally waits on it.
                    let _ = waiting.recv_timeout(Duration::from_secs(1));
                    Ok("result accepted before emergency")
                },
            )
        });
        observing.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(control.try_lock().is_err());
        let started = Instant::now();
        emergency.trigger();
        let revoked = permit.check_live();
        let mut invoked = false;
        let denied = permit.with_debug(false, || {
            invoked = true;
            Ok(())
        });
        let elapsed = started.elapsed();
        let _ = release.send(());
        let completed = worker.join().unwrap();
        assert!(
            elapsed < Duration::from_millis(500),
            "emergency waited for authority: {elapsed:?}"
        );
        assert!(revoked.is_err());
        assert!(denied.is_err());
        assert!(!invoked);
        assert!(
            completed.is_err(),
            "in-flight result escaped after emergency"
        );
        let control = control.lock().unwrap();
        assert!(!control.enabled());
        assert!(
            !control
                .leases
                .owns_input(permit.lease, permit.operation_id.unwrap())
        );
    }
    #[test]
    fn owner_must_cleanup_before_rearming_emergency_authority() {
        let (control, permit, emergency) = authorized();
        emergency.trigger();
        let mut control = control.lock().unwrap();
        control.set_enabled(true);
        assert!(!control.enabled(), "enable bypassed emergency cleanup");
        assert!(permit.check_emergency().is_err());
        control.set_enabled(false);
        assert_eq!(control.leases.iter().count(), 0);
        emergency.trigger();
        control.set_enabled(true);
        assert!(!control.enabled(), "new stop was rearmed without cleanup");
        control.set_enabled(false);
        control.set_enabled(true);
        assert!(control.enabled());
        assert!(permit.check_emergency().is_err());
    }
}
