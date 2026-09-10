//! Required logical connection lifetime, independent of ordinary HTTP responses.
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{ClientConnectionPermit, ControlPlane};

/// Coalesced, payload-free transport-loss notification to the native owner.
/// The platform callback must not wait for a mutex, queue capacity, or owner.
/// A failed wake retains pending cleanup for the owner's periodic fallback.
#[derive(Clone)]
pub struct ConnectionCleanupWake {
    state: Arc<CleanupWakeState>,
}
struct CleanupWakeState {
    pending: AtomicBool,
    failed: AtomicBool,
    wake: Box<dyn Fn() -> bool + Send + Sync>,
}
impl ConnectionCleanupWake {
    pub fn new(wake: impl Fn() -> bool + Send + Sync + 'static) -> Self {
        Self {
            state: Arc::new(CleanupWakeState {
                pending: AtomicBool::new(false),
                failed: AtomicBool::new(false),
                wake: Box::new(wake),
            }),
        }
    }
    pub fn notify(&self) {
        if !self.state.pending.swap(true, Ordering::AcqRel) && !(self.state.wake)() {
            self.state.failed.store(true, Ordering::Release);
        }
    }
    /// Consume before doing owner work: a signal during cleanup must remain pending.
    pub fn take_pending(&self) -> bool {
        self.state.pending.swap(false, Ordering::AcqRel)
    }
    /// Fixed diagnostic only, consumed on the owner; notification performs no logging.
    pub fn take_wake_failure(&self) -> bool {
        self.state.failed.swap(false, Ordering::AcqRel)
    }
}

pub(crate) const MAX_WATCHES: usize = 16;
pub(crate) const MAX_CLIENT_WATCHES: usize = 2;
pub(crate) const WATCH_SECONDS: u64 = 60;

pub(crate) struct WatchEntry {
    pub client: String,
    pub ready: Arc<AtomicBool>,
    pub transport_alive: Arc<AtomicBool>,
    request_lifetime: Option<Arc<crate::operation_metrics::RequestLifetime>>,
    pub expires_at: std::time::Instant,
}

#[derive(Clone)]
pub(crate) struct WatchPresence {
    transport_alive: Arc<AtomicBool>,
    request_lifetime: Option<Arc<crate::operation_metrics::RequestLifetime>>,
    expires_at: std::time::Instant,
}

impl WatchPresence {
    pub(crate) fn is_live(&self, now: std::time::Instant) -> bool {
        now < self.expires_at
            && self.transport_alive.load(Ordering::SeqCst)
            && !self
                .request_lifetime
                .as_ref()
                .is_some_and(|lifetime| lifetime.is_cancelled())
    }
}

impl WatchEntry {
    pub(crate) fn transport_is_alive(&self) -> bool {
        self.transport_alive.load(Ordering::SeqCst)
            && !self
                .request_lifetime
                .as_ref()
                .is_some_and(|lifetime| lifetime.is_cancelled())
    }

    pub(crate) fn presence(&self) -> WatchPresence {
        WatchPresence {
            transport_alive: self.transport_alive.clone(),
            request_lifetime: self.request_lifetime.clone(),
            expires_at: self.expires_at,
        }
    }
}

impl Drop for WatchEntry {
    fn drop(&mut self) {
        // Every authoritative removal path (including retain/clear) retires the
        // exact transport incarnation; moving a reservation preserves it.
        self.transport_alive.store(false, Ordering::SeqCst);
    }
}

/// Dropping the last ready watch invalidates authority synchronously. Native
/// owners already reconcile cancellation on their dispatch loop; normal watch
/// completion additionally waits for that reconciliation before replying.
pub(crate) struct ConnectionWatch {
    owner_wake: Option<ConnectionCleanupWake>,
    control: Arc<Mutex<ControlPlane>>,
    id: u64,
    transport_alive: Arc<AtomicBool>,
    ready: Arc<AtomicBool>,
    presence: WatchPresence,
    emergency: Option<crate::emergency::EmergencyTicket>,
}

impl ControlPlane {
    /// Close a transport-owned watch. Native owners reconcile cancelled input
    /// before acknowledging a graceful close.
    pub fn close_connection_watch(&mut self, id: u64, now: std::time::Instant) {
        if let Some(entry) = self.connection_watches.remove(&id) {
            entry.transport_alive.store(false, Ordering::SeqCst);
            if entry.ready.load(Ordering::SeqCst) && !self.has_ready_connection(&entry.client, now)
            {
                self.disconnect_client(&entry.client);
            }
        }
    }

    pub(super) fn expire_connection_watches(&mut self, now: std::time::Instant) {
        let expired: Vec<_> = self
            .connection_watches
            .iter()
            .filter(|(_, entry)| now >= entry.expires_at || !entry.transport_is_alive())
            .map(|(id, _)| *id)
            .collect();
        for id in expired {
            self.close_connection_watch(id, now);
        }
    }

    /// Trusted transport admission. The transport must close the resulting watch
    /// on request loss; reservation alone grants no authority.
    pub fn reserve_connection_watch(
        &mut self,
        client: &str,
        token: &str,
        now: std::time::Instant,
    ) -> Result<u64, String> {
        self.expire_connection_watches(now);
        if !self.enabled()
            || !self.authenticate(client, token)
            || self.lease_requests.is_blocked(client)
        {
            return Err("client is unauthorized".into());
        }
        if self.connection_watches.len() >= MAX_WATCHES
            || self
                .connection_watches
                .values()
                .filter(|entry| entry.client == client)
                .count()
                >= MAX_CLIENT_WATCHES
        {
            return Err("connection watch capacity reached".into());
        }
        let id = self
            .next_connection_watch
            .checked_add(1)
            .ok_or("connection identity exhausted")?;
        self.next_connection_watch = id;
        self.connection_watches.insert(
            id,
            WatchEntry {
                client: client.to_owned(),
                ready: Arc::new(AtomicBool::new(false)),
                transport_alive: Arc::new(AtomicBool::new(true)),
                request_lifetime: None,
                expires_at: now + std::time::Duration::from_secs(WATCH_SECONDS),
            },
        );
        Ok(id)
    }

    /// Called by the desktop owner only, before its cancellation reconciliation
    /// and readiness acknowledgement. Never extends a lease deadline.
    pub fn activate_connection_watch(
        &mut self,
        client: &str,
        token: &str,
        id: u64,
        locked: bool,
        now: std::time::Instant,
    ) -> Result<(), String> {
        if locked || !self.enabled() || !self.authenticate(client, token) {
            return Err("connection is unavailable".into());
        }
        let entry = self
            .connection_watches
            .get(&id)
            .filter(|entry| {
                entry.client == client && now < entry.expires_at && entry.transport_is_alive()
            })
            .ok_or("connection watch was cancelled or expired")?;
        if entry.ready.load(Ordering::SeqCst) {
            return Err("connection watch already started".into());
        }
        // Validate before taking the reservation: an explicit disconnect or
        // revocation that already removed it must never be undone. Retain this
        // valid reservation locally while expiry invalidates the old connection,
        // since disconnect intentionally removes all of that client's watches.
        // A transport can cancel atomically while authority is held. Readiness
        // always checks its incarnation flag, so restoration cannot revive it.
        let reservation = self.connection_watches.remove(&id).unwrap();
        self.expire_connection_watches(now);
        self.connection_watches.insert(id, reservation);
        let already_connected = self.has_ready_connection(client, now);
        self.connection_watches
            .get_mut(&id)
            .unwrap()
            .ready
            .store(true, Ordering::SeqCst);
        if !already_connected
            && let Err(error) = self.reconnect_client_authenticated(client, token, now)
        {
            self.connection_watches
                .get_mut(&id)
                .unwrap()
                .ready
                .store(false, Ordering::SeqCst);
            return Err(error.to_string());
        }
        Ok(())
    }
}

impl ConnectionWatch {
    pub fn reserve(permit: &mut ClientConnectionPermit) -> Result<Self, String> {
        let mut control = permit
            .control
            .lock()
            .map_err(|_| "control authority unavailable")?;
        let id = control.reserve_connection_watch(
            &permit.client,
            &permit.token,
            std::time::Instant::now(),
        )?;
        control
            .connection_watches
            .get_mut(&id)
            .unwrap()
            .request_lifetime = Some(permit.lifetime.clone());
        permit.watch_id = Some(id);
        Ok(Self {
            owner_wake: None,
            control: permit.control.clone(),
            id,
            ready: control.connection_watches.get(&id).unwrap().ready.clone(),
            presence: control.connection_watches.get(&id).unwrap().presence(),
            emergency: permit.emergency.clone(),
            transport_alive: control
                .connection_watches
                .get(&id)
                .unwrap()
                .transport_alive
                .clone(),
        })
    }

    pub fn with_owner_wake(mut self, wake: Option<ConnectionCleanupWake>) -> Self {
        self.owner_wake = wake;
        self
    }

    pub fn is_live(&self) -> bool {
        self.ready.load(Ordering::SeqCst)
            && self.presence.is_live(std::time::Instant::now())
            && self
                .emergency
                .as_ref()
                .is_some_and(|ticket| ticket.check().is_ok())
    }

    pub fn close(&mut self) -> ClientConnectionPermit {
        if self.transport_alive.swap(false, Ordering::SeqCst)
            && let Some(wake) = &self.owner_wake
        {
            wake.notify();
        }
        // Never occupy an async transport worker behind an owner callback.
        // The returned owner reconciliation, or normal owner tick for Drop,
        // removes this already invalidated entry and releases native input.
        if let Ok(mut control) = self.control.try_lock() {
            control.close_connection_watch(self.id, std::time::Instant::now());
        }
        ClientConnectionPermit::reconciliation(self.control.clone())
    }
}

impl Drop for ConnectionWatch {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        ClientConnectionAction, IssuedCapability, lease_requests::LeaseRequest,
        leases::ResourceScope,
    };
    use std::time::{Duration, Instant};

    #[test]
    fn cleanup_wake_coalesces_and_rearms_after_owner_consumes() {
        use std::sync::atomic::AtomicUsize;
        let count = Arc::new(AtomicUsize::new(0));
        let calls = count.clone();
        let wake = ConnectionCleanupWake::new(move || {
            calls.fetch_add(1, Ordering::SeqCst);
            true
        });
        for _ in 0..100 {
            wake.notify();
        }
        assert_eq!(count.load(Ordering::SeqCst), 1);
        assert!(wake.take_pending());
        // A signal during the owner's cleanup must not be cleared at its end.
        wake.notify();
        assert!(wake.take_pending());
        assert!(!wake.take_pending());
        assert_eq!(count.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn cleanup_wake_failure_keeps_fallback_work_and_reports_once() {
        let wake = ConnectionCleanupWake::new(|| false);
        wake.notify();
        assert!(wake.take_pending());
        assert!(wake.take_wake_failure());
        assert!(!wake.take_wake_failure());
        wake.notify();
        assert!(wake.take_pending());
        assert!(wake.take_wake_failure());
    }

    #[test]
    fn abrupt_watch_drop_wakes_owner_without_waiting_for_authority() {
        let (control, client) = fixture();
        let wake = ConnectionCleanupWake::new(|| true);
        let watch = open(&control, &client).with_owner_wake(Some(wake.clone()));
        let guard = control.lock().unwrap();
        // This would deadlock if Drop or its wake acquired the authority mutex.
        drop(watch);
        assert!(wake.take_pending());
        assert!(!guard.has_ready_connection(&client.client_id, Instant::now()));
    }

    fn fixture() -> (Arc<Mutex<ControlPlane>>, IssuedCapability) {
        let mut control = ControlPlane::default();
        control.set_enabled(true);
        let client = control.connect_identity("Watch fixture").unwrap();
        (Arc::new(Mutex::new(control)), client)
    }
    fn reserve(
        control: &Arc<Mutex<ControlPlane>>,
        client: &IssuedCapability,
    ) -> (ConnectionWatch, ClientConnectionPermit) {
        let mut permit = ClientConnectionPermit {
            emergency: Some(control.lock().unwrap().emergency.ticket()),
            control: control.clone(),
            client: client.client_id.clone(),
            token: client.token.clone(),
            deadline: Instant::now() + Duration::from_secs(2),
            lifetime: Default::default(),
            watch_id: None,
            reconcile_only: false,
        };
        let watch = ConnectionWatch::reserve(&mut permit).unwrap();
        (watch, permit)
    }
    fn open(control: &Arc<Mutex<ControlPlane>>, client: &IssuedCapability) -> ConnectionWatch {
        let (watch, permit) = reserve(control, client);
        permit.apply(ClientConnectionAction::Watch, false).unwrap();
        watch
    }
    fn approve(
        control: &Arc<Mutex<ControlPlane>>,
        client: &IssuedCapability,
        resumable: bool,
    ) -> u64 {
        let mut state = control.lock().unwrap();
        let now = Instant::now();
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::FullSession,
            duration: Some(Duration::from_secs(120)),
            allow_resumption: resumable,
            full_debug: false,
        };
        state
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        let generation = state
            .lease_requests()
            .pending_generation(&client.client_id)
            .unwrap();
        state
            .approve_lease_local(&client.client_id, &request, generation, now)
            .unwrap()
    }
    #[test]
    fn watch_poll_close_and_drop_never_wait_for_owner_authority() {
        for graceful in [false, true] {
            let (control, client) = fixture();
            let watch = open(&control, &client);
            let lease = approve(&control, &client, false);
            let held = control.lock().unwrap();
            let (send, recv) = std::sync::mpsc::channel();
            let worker = std::thread::spawn(move || {
                let mut watch = watch;
                let live = watch.is_live();
                let reconciliation = graceful.then(|| watch.close());
                drop(watch);
                send.send((live, reconciliation)).unwrap();
            });
            let observed = recv.recv_timeout(Duration::from_secs(2));
            let remained_for_owner = held.connection_watches.len();
            drop(held);
            worker.join().unwrap();
            let (live, reconciliation) =
                observed.expect("transport must not wait for owner authority");
            assert!(live, "mutex contention is not loss of connection");
            assert_eq!(remained_for_owner, 1);
            assert!(
                !control
                    .lock()
                    .unwrap()
                    .has_ready_connection(&client.client_id, Instant::now())
            );
            if let Some(reconciliation) = reconciliation {
                // This is the owner-side graceful-close boundary, before native
                // reconciliation and the server's awaited desktop reply.
                reconciliation
                    .apply(ClientConnectionAction::Disconnect, false)
                    .unwrap();
            } else {
                control
                    .lock()
                    .unwrap()
                    .reconcile_pending_lease_requests(Instant::now());
            }
            let state = control.lock().unwrap();
            assert!(state.connection_watches.is_empty());
            assert!(
                state
                    .leases
                    .active_lease(lease, &client.client_id, Instant::now())
                    .is_err()
            );
        }
    }

    #[test]
    fn every_authoritative_removal_retires_guard_projection() {
        for removal in 0..4 {
            let (control, client) = fixture();
            let watch = open(&control, &client);
            assert!(watch.is_live());
            {
                let mut state = control.lock().unwrap();
                match removal {
                    0 => {
                        state.disconnect_client(&client.client_id);
                    }
                    1 => {
                        state.revoke(&client.client_id);
                    }
                    2 => state.lock(),
                    _ => state.emergency_stop(),
                }
            }
            assert!(!watch.is_live());
            drop(watch);
        }
    }

    #[test]
    fn watch_request_cancellation_reaches_commit_before_guard_cleanup() {
        let (control, client) = fixture();
        let (watch, connection) = reserve(&control, &client);
        let cancellation = tokio_util::sync::CancellationToken::new();
        connection.lifetime.attach(cancellation.clone());
        connection
            .apply(ClientConnectionAction::Watch, false)
            .unwrap();
        let lease = approve(&control, &client, false);
        let permit = crate::DesktopPermit::new(
            control.clone(),
            client.client_id.clone(),
            client.token.clone(),
            lease,
        );
        let evidence = crate::leases::ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: None,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        assert!(
            permit
                .with_input_boundary(&evidence, |boundary| {
                    // The watch guard remains owned and cannot run cleanup in this callback.
                    cancellation.cancel();
                    assert!(watch.transport_alive.load(Ordering::SeqCst));
                    assert!(permit.check_commit_boundary(boundary).is_err());
                    Ok(())
                })
                .is_err()
        );
        assert!(
            !control
                .lock()
                .unwrap()
                .has_ready_connection(&client.client_id, Instant::now())
        );
        assert!(
            !control
                .lock()
                .unwrap()
                .leases
                .owns_input(lease, permit.operation_id.unwrap())
        );
        // Admission reconciles the cancelled watch before opening another one.
        let replacement = open(&control, &client);
        drop(watch);
        assert!(replacement.is_live());
        assert!(permit.with_resource(&evidence, || Ok(())).is_err());
    }

    #[test]
    fn transport_loss_is_visible_inside_locked_effect_and_releases_input() {
        for overlap in [false, true] {
            let (control, client) = fixture();
            let watch = open(&control, &client);
            let other = overlap.then(|| open(&control, &client));
            let lease = approve(&control, &client, false);
            let mut permit = crate::DesktopPermit::new(
                control.clone(),
                client.client_id.clone(),
                client.token.clone(),
                lease,
            );
            // Keep request expiry beyond the contention watchdog: an old implementation
            // must fail because it publishes success, not pass by timing out the request.
            permit.deadline = Instant::now() + Duration::from_secs(10);
            let alive = watch.transport_alive.clone();
            let (start, started) = std::sync::mpsc::channel();
            let closer = std::thread::spawn(move || {
                started.recv().unwrap();
                drop(watch);
            });
            let evidence = crate::leases::ResourceEvidence {
                surface: None,
                window: None,
                verified_application: None,
                output: None,
                authorized_surface_ancestors: &[],
                protected: false,
            };
            let mut commit_allowed = None;
            let result = permit.with_authority_deadline(
                &evidence,
                false,
                crate::InputReservation::Begin,
                |boundary| {
                    start.send(()).unwrap();
                    let timeout = Instant::now() + Duration::from_secs(2);
                    while alive.load(Ordering::SeqCst) && Instant::now() < timeout {
                        std::thread::yield_now();
                    }
                    commit_allowed = Some(permit.check_commit_boundary(boundary).is_ok());
                    // A successful native callback must not publish a result after loss.
                    Ok(47)
                },
            );
            closer.join().unwrap();
            assert_eq!(result.is_ok(), overlap);
            assert_eq!(commit_allowed, Some(overlap));
            assert_eq!(
                control
                    .lock()
                    .unwrap()
                    .leases
                    .owns_input(lease, permit.operation_id.unwrap()),
                overlap
            );
            if overlap {
                control
                    .lock()
                    .unwrap()
                    .leases
                    .release_input(lease, permit.operation_id.unwrap());
            }
            drop(other);
        }
    }

    #[test]
    fn cancelled_pending_transport_cannot_activate_or_consume_capacity() {
        let (control, client) = fixture();
        let (mut watch, permit) = reserve(&control, &client);
        // Simulate the atomic half of close while owner cleanup is still pending.
        watch.transport_alive.store(false, Ordering::SeqCst);
        assert!(permit.apply(ClientConnectionAction::Watch, false).is_err());
        let replacement = open(&control, &client);
        watch.close();
        assert!(replacement.is_live());
        assert_eq!(control.lock().unwrap().connection_watches.len(), 1);
    }

    #[test]
    fn overlapping_watches_disconnect_only_on_last_loss_and_do_not_extend_leases() {
        let (control, client) = fixture();
        let first = open(&control, &client);
        let temporary = approve(&control, &client, false);
        let resumable = approve(&control, &client, true);
        let deadline = control
            .lock()
            .unwrap()
            .leases
            .iter()
            .find(|l| l.id == resumable)
            .unwrap()
            .expires_at;
        let second = open(&control, &client);
        drop(first);
        assert!(second.is_live());
        assert!(
            control
                .lock()
                .unwrap()
                .leases
                .active_lease(temporary, &client.client_id, Instant::now())
                .is_ok()
        );
        drop(second);
        assert!(
            control
                .lock()
                .unwrap()
                .leases
                .active_lease(resumable, &client.client_id, Instant::now())
                .is_err()
        );
        let replacement = open(&control, &client);
        let state = control.lock().unwrap();
        let lease = state
            .leases
            .active_lease(resumable, &client.client_id, Instant::now())
            .unwrap();
        assert_eq!(lease.expires_at, deadline);
        assert_eq!(lease.operation_generation, 1);
        assert!(!state.leases.iter().any(|l| l.id == temporary));
        drop(state);
        drop(replacement);
    }
    #[test]
    fn identity_and_unready_or_failed_watch_grant_no_permission_request_authority() {
        let (control, client) = fixture();
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::FullSession,
            duration: None,
            allow_resumption: false,
            full_debug: false,
        };
        let denied = || {
            assert!(
                control
                    .lock()
                    .unwrap()
                    .request_lease(
                        &client.client_id,
                        &client.token,
                        request.clone(),
                        Instant::now()
                    )
                    .is_err()
            )
        };
        denied();
        let (watch, permit) = reserve(&control, &client);
        denied();
        assert!(permit.apply(ClientConnectionAction::Watch, true).is_err());
        drop(watch);
        denied();
        let active = open(&control, &client);
        approve(&control, &client, false);
        drop(active);
        denied();
    }
    #[test]
    fn explicit_disconnect_invalidates_old_watch_and_stale_drop_cannot_cancel_reconnect() {
        let (control, client) = fixture();
        let old = open(&control, &client);
        let lease = approve(&control, &client, true);
        control.lock().unwrap().disconnect_client(&client.client_id);
        assert!(!old.is_live());
        assert!(
            control
                .lock()
                .unwrap()
                .reconnect_client_authenticated(&client.client_id, &client.token, Instant::now())
                .is_err()
        );
        let new = open(&control, &client);
        drop(old);
        assert!(new.is_live());
        assert!(
            control
                .lock()
                .unwrap()
                .leases
                .active_lease(lease, &client.client_id, Instant::now())
                .is_ok()
        );
    }
    #[test]
    fn watch_admission_is_bounded_and_released_even_before_owner_execution() {
        let (control, client) = fixture();
        let (first, _) = reserve(&control, &client);
        let (second, mut permit) = reserve(&control, &client);
        assert!(ConnectionWatch::reserve(&mut permit).is_err());
        drop(first);
        let third = ConnectionWatch::reserve(&mut permit).unwrap();
        drop(second);
        drop(third);
        assert!(control.lock().unwrap().connection_watches.is_empty());
    }
    #[test]
    fn expired_presence_denies_stale_local_approval_and_new_watch_cancels_old_generation() {
        let (control, client) = fixture();
        let old = open(&control, &client);
        let lease = approve(&control, &client, true);
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::FullSession,
            duration: None,
            allow_resumption: true,
            full_debug: false,
        };
        let mut state = control.lock().unwrap();
        let now = Instant::now();
        state
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        let generation = state
            .lease_requests()
            .pending_generation(&client.client_id)
            .unwrap();
        let later = now + Duration::from_secs(WATCH_SECONDS + 1);
        assert!(!state.has_ready_connection(&client.client_id, later));
        assert!(
            state
                .approve_lease_local(&client.client_id, &request, generation, later)
                .is_err()
        );
        let next = state
            .reserve_connection_watch(&client.client_id, &client.token, later)
            .unwrap();
        state
            .activate_connection_watch(&client.client_id, &client.token, next, false, later)
            .unwrap();
        assert!(
            state
                .lease_requests()
                .pending_generation(&client.client_id)
                .is_none()
        );
        assert_eq!(
            state
                .leases
                .active_lease(lease, &client.client_id, later)
                .unwrap()
                .operation_generation,
            1
        );
        drop(state);
        drop(old);
    }
    #[test]
    fn expired_presence_denies_owner_permits_even_before_transport_task_cleanup() {
        let (control, client) = fixture();
        let at = Instant::now() - Duration::from_secs(WATCH_SECONDS + 1);
        let mut state = control.lock().unwrap();
        let id = state
            .reserve_connection_watch(&client.client_id, &client.token, at)
            .unwrap();
        state
            .activate_connection_watch(&client.client_id, &client.token, id, false, at)
            .unwrap();
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::FullSession,
            duration: Some(Duration::from_secs(120)),
            allow_resumption: true,
            full_debug: true,
        };
        state
            .request_lease(&client.client_id, &client.token, request.clone(), at)
            .unwrap();
        let generation = state
            .lease_requests()
            .pending_generation(&client.client_id)
            .unwrap();
        let lease = state
            .approve_lease_local(&client.client_id, &request, generation, at)
            .unwrap();
        assert!(
            state
                .leases
                .active_lease(lease, &client.client_id, Instant::now())
                .is_ok()
        );
        drop(state);
        let permit = crate::DesktopPermit::new(control, client.client_id, client.token, lease);
        assert!(permit.check_live().is_err());
        assert!(
            permit
                .with_debug::<()>(false, || panic!("expired watch exposed diagnostics"))
                .is_err()
        );
        assert!(permit.continued_observation().is_err());
    }
    #[test]
    fn disconnected_card_cannot_approve_identical_request_on_new_connection() {
        let (control, client) = fixture();
        let old = open(&control, &client);
        let request = LeaseRequest {
            renewal: None,
            scope: ResourceScope::FullSession,
            duration: None,
            allow_resumption: true,
            full_debug: false,
        };
        let now = Instant::now();
        let old_generation = {
            let mut state = control.lock().unwrap();
            state
                .request_lease(&client.client_id, &client.token, request.clone(), now)
                .unwrap();
            state
                .lease_requests()
                .pending_generation(&client.client_id)
                .unwrap()
        };
        drop(old);
        let _new = open(&control, &client);
        let mut state = control.lock().unwrap();
        state
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        let current = state
            .lease_requests()
            .pending_generation(&client.client_id)
            .unwrap();
        assert_ne!(current, old_generation);
        assert!(
            state
                .approve_lease_local(&client.client_id, &request, old_generation, now)
                .is_err()
        );
        assert!(state.leases.iter().next().is_none());
        state
            .approve_lease_local(&client.client_id, &request, current, now)
            .unwrap();
    }
    #[test]
    fn replacement_reserved_before_expiry_cannot_skip_disconnect_when_activated_late() {
        let (control, client) = fixture();
        let old = open(&control, &client);
        let temporary = approve(&control, &client, false);
        let resumable = approve(&control, &client, true);
        let mut state = control.lock().unwrap();
        let deadline = state.connection_watches.get(&old.id).unwrap().expires_at;
        let pending = LeaseRequest {
            renewal: None,
            scope: ResourceScope::FullSession,
            duration: None,
            allow_resumption: true,
            full_debug: false,
        };
        state
            .request_lease(&client.client_id, &client.token, pending, Instant::now())
            .unwrap();
        state.leases.reserve_input(resumable, 42).unwrap();
        let next = state
            .reserve_connection_watch(
                &client.client_id,
                &client.token,
                deadline - Duration::from_millis(1),
            )
            .unwrap();
        let activated = deadline + Duration::from_millis(1);
        state
            .activate_connection_watch(&client.client_id, &client.token, next, false, activated)
            .unwrap();
        assert!(
            !state.leases.iter().any(|lease| lease.id == temporary),
            "expired old presence must revoke nonresumable authority"
        );
        assert!(
            state
                .lease_requests()
                .pending_generation(&client.client_id)
                .is_none()
        );
        assert!(!state.leases.owns_input(resumable, 42));
        let resumed = state
            .leases
            .active_lease(resumable, &client.client_id, activated)
            .unwrap();
        assert_eq!(resumed.operation_generation, 1);
        assert!(
            state
                .connection_watches
                .get(&next)
                .unwrap()
                .ready
                .load(Ordering::SeqCst),
            "valid replacement reservation must survive old-watch expiry"
        );
        drop(state);
        drop(old);
        assert!(
            control
                .lock()
                .unwrap()
                .has_ready_connection(&client.client_id, activated)
        );
    }

    #[test]
    fn explicit_disconnect_cannot_restore_a_previously_reserved_replacement() {
        let (control, client) = fixture();
        let old = open(&control, &client);
        let (replacement, permit) = reserve(&control, &client);
        control.lock().unwrap().disconnect_client(&client.client_id);
        assert!(permit.apply(ClientConnectionAction::Watch, false).is_err());
        assert!(!replacement.is_live());
        assert!(control.lock().unwrap().connection_watches.is_empty());
        drop(replacement);
        drop(old);
    }

    #[test]
    fn lock_and_revocation_invalidate_watches_without_restoring_authority() {
        for lock in [false, true] {
            let (control, client) = fixture();
            let watch = open(&control, &client);
            let lease = approve(&control, &client, true);
            if lock {
                control.lock().unwrap().lock();
            } else {
                control.lock().unwrap().revoke(&client.client_id);
            }
            assert!(!watch.is_live());
            assert!(
                control
                    .lock()
                    .unwrap()
                    .leases
                    .active_lease(lease, &client.client_id, Instant::now())
                    .is_err()
            );
        }
    }
}
