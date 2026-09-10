//! Security authority for Nickel's optional remote-control service.
//!
//! Transport adapters may present pairing challenges and MCP requests, but only this state
//! machine can turn a locally approved client into a scoped capability.

use std::collections::{BTreeMap, VecDeque};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use thiserror::Error;

mod admission;
pub mod appearance;
pub mod application_scale;
pub mod capture;
mod connection_watch;
pub mod launcher_favorites;
pub use connection_watch::ConnectionCleanupWake;
mod emergency;
pub use emergency::EmergencyStopHandle;
pub mod desktop_events;
pub mod diagnostics;
mod event_subscriptions;
pub mod frame_trace;
pub mod keyboard;
pub mod lease_audit;
pub mod lease_requests;
pub mod leases;
pub mod listener;
pub mod local_cues;
pub mod native_semantics;
mod operation_metrics;
pub mod pointer;
pub mod semantics;
mod server;
pub mod terminal_presentation;
pub mod trace_audit;
pub mod wallpaper;
pub mod window_actions;
pub use server::{DesktopAuthority, RemoteControlServer, ServerError, WindowSummary};
mod settings;
pub use settings::{RemoteAiControlSettings, SettingsError};

/// A graceful transition applies to all leases of the authenticated identity.
/// Completing an ordinary HTTP response is not a disconnect.
#[derive(Clone, Copy, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClientConnectionAction {
    Disconnect,
    Reconnect,
    /// Hold a bounded standard MCP request as an opt-in logical connection.
    Watch,
}

/// Authenticated self-service request, deliberately neither Debug nor serializable.
/// Only the desktop owner may apply it and acknowledge after draining cancelled input.
pub struct ClientConnectionPermit {
    emergency: Option<emergency::EmergencyTicket>,
    control: std::sync::Arc<std::sync::Mutex<ControlPlane>>,
    client: String,
    token: String,
    deadline: std::time::Instant,
    lifetime: std::sync::Arc<operation_metrics::RequestLifetime>,
    watch_id: Option<u64>,
    reconcile_only: bool,
}

impl ClientConnectionPermit {
    pub(crate) fn new(
        control: std::sync::Arc<std::sync::Mutex<ControlPlane>>,
        client: String,
        token: String,
        metrics: &std::sync::Arc<operation_metrics::OperationMetrics>,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> Result<Self, String> {
        let lifetime = operation_metrics::current_lifetime(metrics)
            .ok_or("request execution scope is unavailable")?;
        lifetime.attach(cancellation);
        let emergency = Some(
            control
                .lock()
                .map_err(|_| "control authority unavailable")?
                .emergency
                .ticket(),
        );
        Ok(Self {
            emergency,
            control,
            client,
            token,
            deadline: std::time::Instant::now() + Duration::from_secs(2),
            lifetime,
            watch_id: None,
            reconcile_only: false,
        })
    }

    /// Internal cancellation reconciliation carries no authority to reconnect.
    pub(crate) fn reconciliation(control: std::sync::Arc<std::sync::Mutex<ControlPlane>>) -> Self {
        Self {
            emergency: None,
            control,
            client: String::new(),
            token: String::new(),
            deadline: std::time::Instant::now() + Duration::from_secs(2),
            lifetime: Default::default(),
            watch_id: None,
            reconcile_only: true,
        }
    }

    /// Recheck credentials, request delivery and lock state at the native owner.
    /// Call the owner's cancellation reconciliation before sending the response.
    pub fn apply(self, action: ClientConnectionAction, locked: bool) -> Result<(), String> {
        if !self.reconcile_only {
            self.emergency
                .as_ref()
                .ok_or("emergency authority unavailable")?
                .check()?;
        }
        let mut control = self
            .control
            .lock()
            .map_err(|_| "control authority unavailable")?;
        let now = std::time::Instant::now();
        if self.lifetime.is_cancelled() || now >= self.deadline {
            return Err("connection request was cancelled or expired".into());
        }
        if self.reconcile_only {
            control.expire_connection_watches(now);
            return Ok(());
        }
        self.emergency
            .as_ref()
            .ok_or("emergency authority unavailable")?
            .check()?;
        if !control.enabled() || !control.authenticate(&self.client, &self.token) {
            return Err("client is unauthorized".into());
        }
        let result = match action {
            ClientConnectionAction::Disconnect => {
                control.disconnect_client(&self.client);
                Ok(())
            }
            ClientConnectionAction::Watch => control.activate_connection_watch(
                &self.client,
                &self.token,
                self.watch_id.ok_or("connection watch was not reserved")?,
                locked,
                now,
            ),
            ClientConnectionAction::Reconnect if locked => Err("session is locked".into()),
            ClientConnectionAction::Reconnect => control
                .reconnect_client_authenticated(&self.client, &self.token, now)
                .map_err(|error| error.to_string()),
        };
        self.emergency
            .as_ref()
            .ok_or("emergency authority unavailable")?
            .check()?;
        result
    }
}

/// Carries an authenticated request to the production owner. It deliberately has no serialization
/// or Debug implementation: client credentials must never enter protocol snapshots or logs.
#[derive(Clone)]
pub struct DesktopPermit {
    emergency: Option<emergency::EmergencyTicket>,
    control: std::sync::Arc<std::sync::Mutex<ControlPlane>>,
    client: String,
    token: String,
    lease: u64,
    operation_generation: Option<u64>,
    trace_audit: Option<std::sync::Arc<trace_audit::TraceAudit>>,
    audit_client_id: Option<u64>,
    operation_id: Option<u64>,
    deadline: std::time::Instant,
    operation_metrics: Option<std::sync::Arc<operation_metrics::OperationMetrics>>,
    operation_authorization:
        Option<std::sync::Arc<std::sync::OnceLock<diagnostics::OperationAuthorization>>>,
    request_lifetime: Option<std::sync::Arc<operation_metrics::RequestLifetime>>,
    admission: Option<std::sync::Arc<admission::AdmissionLimits>>,
}

/// One owner dispatch's bounded authority window. Original watch incarnations
/// are retained so transport loss remains visible while authority is locked.
/// A later replacement watch cannot revive this already accepted dispatch.
pub struct CommitBoundary {
    deadline: std::time::Instant,
    watches: Vec<connection_watch::WatchPresence>,
}

impl CommitBoundary {
    pub fn deadline(&self) -> std::time::Instant {
        self.deadline
    }

    fn connection_live(&self, now: std::time::Instant) -> bool {
        self.watches.iter().any(|watch| watch.is_live(now))
    }
}

/// Production-owned reservation for a gesture spanning requests. Never expose
/// this object or its credentials in protocol data. The seat owner must release
/// synthesized input before dropping it, including on cancellation.
pub struct HeldInput {
    permit: DesktopPermit,
}

impl HeldInput {
    pub fn expires_at(&self) -> Result<Option<std::time::Instant>, String> {
        self.permit.check_emergency()?;
        let control = self
            .permit
            .control
            .lock()
            .map_err(|_| "control authority unavailable")?;
        self.permit.check_emergency()?;
        control
            .leases
            .active_lease(
                self.permit.lease,
                &self.permit.client,
                std::time::Instant::now(),
            )
            .map(|lease| lease.expires_at)
            .map_err(|error| error.to_string())
    }
    pub fn owned_by(&self, request: &DesktopPermit) -> bool {
        std::sync::Arc::ptr_eq(&self.permit.control, &request.control)
            && self.permit.client == request.client
            && self.permit.lease == request.lease
            && self.permit.operation_generation == request.operation_generation
    }

    /// Revalidate a standing native effect independently of HTTP delivery.
    pub fn check_resource(&self, resource: &leases::ResourceEvidence<'_>) -> Result<(), String> {
        let mut permit = self.permit.clone();
        permit.deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        permit.with_authority(resource, false, InputReservation::Continue, || Ok(()))
    }
    /// Lease lifetime, not the initial HTTP request's delivery deadline, governs
    /// held input. Each continuation still needs its own unexpired request.
    pub fn check_live(&self) -> Result<(), String> {
        self.permit.check_lifetime(false)?;
        let control = self
            .permit
            .control
            .lock()
            .map_err(|_| "control authority unavailable")?;
        self.permit.check_emergency()?;
        if control
            .leases
            .owns_input(self.permit.lease, self.permit.operation_id.unwrap())
        {
            Ok(())
        } else {
            Err("remote input ownership was cancelled".into())
        }
    }
}

impl Drop for HeldInput {
    fn drop(&mut self) {
        if let Ok(mut control) = self.permit.control.lock() {
            control
                .leases
                .release_input(self.permit.lease, self.permit.operation_id.unwrap());
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum InputReservation {
    None,
    Transaction,
    Begin,
    Continue,
}

impl DesktopPermit {
    /// Compare standing effect ownership without exposing authority credentials.
    pub fn same_lease_as(&self, request: &Self) -> bool {
        std::sync::Arc::ptr_eq(&self.control, &request.control)
            && self.client == request.client
            && self.lease == request.lease
            && self.operation_generation == request.operation_generation
    }

    /// Refresh delivery time for a bounded standing observation, preserving the
    /// original lease and cancellation generation. Never extends authority.
    /// Callers must bound their standing observation or launch-placement lifetime.
    pub fn continued_observation(&self) -> Result<Self, String> {
        self.check_lifetime(false)?;
        let mut continued = self.clone();
        continued.deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        Ok(continued)
    }

    /// Inspect this request's approved scope for preparation only. Effects still
    /// require fresh resource evidence at the production owner boundary.
    pub fn resource_scope(&self) -> Result<leases::ResourceScope, String> {
        self.check_live()?;
        let control = self
            .control
            .lock()
            .map_err(|_| "control authority unavailable".to_owned())?;
        self.check_emergency()?;
        control
            .leases
            .active_lease(self.lease, &self.client, std::time::Instant::now())
            .map(|lease| lease.scope.clone())
            .map_err(|error| error.to_string())
    }

    pub fn check_live(&self) -> Result<(), String> {
        self.check_lifetime(true)
    }

    fn check_emergency(&self) -> Result<(), String> {
        self.emergency
            .as_ref()
            .ok_or("emergency authority unavailable")?
            .check()
    }

    /// Standing native cleanup must never wait behind another authority holder.
    /// Contention conservatively retires the owned effect; it cannot rearm it.
    pub fn check_standing_live(&self) -> Result<(), String> {
        self.check_lifetime_mode(false, true)
    }

    fn check_lifetime(&self, check_delivery_deadline: bool) -> Result<(), String> {
        self.check_lifetime_mode(check_delivery_deadline, false)
    }

    fn check_lifetime_mode(
        &self,
        check_delivery_deadline: bool,
        nonblocking: bool,
    ) -> Result<(), String> {
        self.check_emergency()?;
        if self
            .request_lifetime
            .as_ref()
            .is_some_and(|lifetime| lifetime.is_cancelled())
        {
            return Err("remote request was cancelled".into());
        }
        let control = if nonblocking {
            self.control
                .try_lock()
                .map_err(|_| "control authority busy or unavailable")?
        } else {
            self.control
                .lock()
                .map_err(|_| "control authority unavailable")?
        };
        self.check_emergency()?;
        let now = std::time::Instant::now();
        if (check_delivery_deadline && now >= self.deadline)
            || self
                .request_lifetime
                .as_ref()
                .is_some_and(|lifetime| lifetime.is_cancelled())
            || !control.enabled()
            || !control.authenticate(&self.client, &self.token)
            || !control.has_ready_connection(&self.client, now)
        {
            return Err("remote request expired or revoked".into());
        }
        control
            .leases
            .active_lease(self.lease, &self.client, now)
            .map_err(|error| error.to_string())
            .and_then(|lease| {
                if Some(lease.operation_generation) == self.operation_generation {
                    Ok(())
                } else {
                    Err("remote operation was cancelled".into())
                }
            })
    }
    pub(crate) fn new(
        control: std::sync::Arc<std::sync::Mutex<ControlPlane>>,
        client: String,
        token: String,
        lease: u64,
    ) -> Self {
        static NEXT_OPERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let operation_id = NEXT_OPERATION
            .fetch_update(
                std::sync::atomic::Ordering::Relaxed,
                std::sync::atomic::Ordering::Relaxed,
                |id| id.checked_add(1),
            )
            .ok();
        let (operation_generation, operation_metrics, trace_audit, audit_client_id, emergency) =
            control
                .lock()
                .ok()
                .map(|control| {
                    (
                        control
                            .leases
                            .active_lease(lease, &client, std::time::Instant::now())
                            .ok()
                            .map(|lease| lease.operation_generation),
                        Some(control.operation_metrics.clone()),
                        Some(control.trace_audit.clone()),
                        control.lease_requests.audit_client_id(&client),
                        Some(control.emergency.ticket()),
                    )
                })
                .unwrap_or_default();
        let operation_authorization = operation_metrics
            .as_ref()
            .and_then(operation_metrics::current_authorization);
        let request_lifetime = operation_metrics
            .as_ref()
            .and_then(operation_metrics::current_lifetime);
        Self {
            control,
            client,
            token,
            lease,
            admission: None,
            emergency,
            operation_generation,
            operation_id,
            operation_metrics,
            operation_authorization,
            request_lifetime,
            trace_audit,
            audit_client_id,
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(2),
        }
    }

    /// Bind transport cancellation to the required measured request scope.
    pub(crate) fn attach_request_cancellation(
        &self,
        token: tokio_util::sync::CancellationToken,
    ) -> Result<(), String> {
        let lifetime = self
            .request_lifetime
            .as_ref()
            .ok_or("request lifetime is unavailable")?;
        lifetime.attach(token);
        Ok(())
    }

    /// Read aggregate counters plus bounded, payload-free method completion history.
    /// The public metrics endpoint exposes only aggregate counters and timings.
    /// This independent collector can be sampled inside `with_debug` without
    /// recursively locking control authority. It grants no desktop observation.
    pub fn operation_metrics_snapshot(&self) -> Option<diagnostics::OperationMetricsSnapshot> {
        self.operation_metrics.as_ref()?.snapshot()
    }

    /// Payload-free listener bookkeeping, safe to sample inside owner authorization.
    pub fn admission_snapshot(&self) -> Option<diagnostics::AdmissionDiagnostic> {
        self.admission.as_ref()?.snapshot()
    }

    /// Public aggregate bookkeeping only; call outside `with_debug` to avoid
    /// recursively locking authority. A later debug check still gates delivery.
    pub fn lease_metrics_snapshot(
        &self,
        observed_at_us: u64,
    ) -> Option<diagnostics::LeaseMetricsDiagnostic> {
        let control = self.control.try_lock().ok()?;
        Some(control.lease_metrics(std::time::Instant::now(), observed_at_us))
    }

    /// Execute at the owner boundary with freshly resolved evidence. Keep authorization locked
    /// until the synchronous effect completes so revocation cannot race its acceptance.
    pub fn with_resource<T>(
        &self,
        resource: &leases::ResourceEvidence<'_>,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_authority(resource, false, InputReservation::None, effect)
    }

    /// Serialize complete input/focus transactions against continuous owners.
    /// Release reservation on either successful delivery or a rejected effect.
    pub fn with_input<T>(
        &self,
        resource: &leases::ResourceEvidence<'_>,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_input_boundary(resource, |_| effect())
    }

    /// A resource-scoped input transaction with an explicit staged commit check.
    /// Native owners must bound work and recheck immediately before dispatch.
    pub fn with_input_boundary<T>(
        &self,
        resource: &leases::ResourceEvidence<'_>,
        effect: impl FnOnce(&CommitBoundary) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_authority_deadline(resource, false, InputReservation::Transaction, effect)
    }

    /// Reserve the seat and deliver the initial gesture event atomically. A
    /// rejected effect does not leave a reservation behind.
    pub fn begin_input(
        &self,
        resource: &leases::ResourceEvidence<'_>,
        effect: impl FnOnce() -> Result<(), String>,
    ) -> Result<HeldInput, String> {
        self.with_authority(resource, false, InputReservation::Begin, effect)?;
        Ok(HeldInput {
            permit: self.clone(),
        })
    }

    /// Continue only the original client's unchanged lease and gesture. The
    /// resource is resolved afresh by the seat owner for every motion or key.
    pub fn continue_input<T>(
        &self,
        held: &HeldInput,
        resource: &leases::ResourceEvidence<'_>,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let original = &held.permit;
        if !std::sync::Arc::ptr_eq(&self.control, &original.control)
            || self.client != original.client
            || self.lease != original.lease
            || self.operation_generation != original.operation_generation
        {
            return Err("request does not own this input gesture".into());
        }
        held.check_live()?;
        let mut continuation = self.clone();
        continuation.operation_id = original.operation_id;
        continuation.with_authority(resource, false, InputReservation::Continue, effect)
    }

    /// Recheck cancellation at a staged effect's final commit boundary while
    /// `with_debug_input_deadline` already holds authority. This does not grant
    /// authority and deliberately never reacquires its non-reentrant mutex.
    pub fn check_commit_boundary(&self, boundary: &CommitBoundary) -> Result<(), String> {
        self.check_emergency()?;
        let now = std::time::Instant::now();
        if now >= boundary.deadline.min(self.deadline)
            || !boundary.connection_live(now)
            || self
                .request_lifetime
                .as_ref()
                .is_some_and(|lifetime| lifetime.is_cancelled())
        {
            return Err("remote commit expired or was cancelled".into());
        }
        Ok(())
    }

    pub fn with_debug<T>(
        &self,
        protected: bool,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_authority(
            &leases::ResourceEvidence {
                surface: None,
                window: None,
                verified_application: None,
                output: None,
                authorized_surface_ancestors: &[],
                protected,
            },
            true,
            InputReservation::None,
            effect,
        )
    }

    /// Commit a diagnostic mutation that can change shared focus or geometry.
    /// The owner must also reject physical input ownership before committing.
    pub fn with_debug_input<T>(
        &self,
        protected: bool,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_debug_input_deadline(protected, |_| effect())
    }

    /// Supplies this dispatch's request/lease/watch boundary while authority stays locked.
    /// Slow staging must recheck it immediately before committing state.
    pub fn with_debug_input_deadline<T>(
        &self,
        protected: bool,
        effect: impl FnOnce(&CommitBoundary) -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_authority_deadline(
            &leases::ResourceEvidence {
                surface: None,
                window: None,
                verified_application: None,
                output: None,
                authorized_surface_ancestors: &[],
                protected,
            },
            true,
            InputReservation::Transaction,
            effect,
        )
    }

    fn with_authority<T>(
        &self,
        resource: &leases::ResourceEvidence<'_>,
        debug: bool,
        input: InputReservation,
        effect: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        self.with_authority_deadline(resource, debug, input, |_| effect())
    }

    fn with_authority_deadline<T>(
        &self,
        resource: &leases::ResourceEvidence<'_>,
        debug: bool,
        input: InputReservation,
        effect: impl FnOnce(&CommitBoundary) -> Result<T, String>,
    ) -> Result<T, String> {
        self.check_emergency()?;
        if self
            .request_lifetime
            .as_ref()
            .is_some_and(|lifetime| lifetime.is_cancelled())
        {
            return Err("remote request was cancelled".into());
        }
        let mut control = self
            .control
            .lock()
            .map_err(|_| "control authority unavailable")?;
        self.check_emergency()?;
        let now = std::time::Instant::now();
        if now >= self.deadline
            || self
                .request_lifetime
                .as_ref()
                .is_some_and(|lifetime| lifetime.is_cancelled())
            || !control.enabled()
            || !control.authenticate(&self.client, &self.token)
            || !control.has_ready_connection(&self.client, now)
        {
            return Err("remote request expired or revoked".into());
        }
        let lease = control
            .leases
            .authorize(self.lease, &self.client, resource, now, debug)
            .map_err(|error| error.to_string())?;
        if Some(lease.operation_generation) != self.operation_generation {
            return Err("remote operation was cancelled".into());
        }
        let connection_deadline = control
            .ready_connection_deadline(&self.client, now)
            .ok_or("remote connection expired")?;
        let valid_until = lease
            .expires_at
            .map_or(self.deadline, |expires| expires.min(self.deadline))
            .min(connection_deadline);
        let boundary = CommitBoundary {
            deadline: valid_until,
            watches: control
                .connection_watches
                .values()
                .filter(|entry| {
                    entry.client == self.client
                        && entry.ready.load(std::sync::atomic::Ordering::SeqCst)
                        && now < entry.expires_at
                        && entry
                            .transport_alive
                            .load(std::sync::atomic::Ordering::SeqCst)
                })
                .take(connection_watch::MAX_CLIENT_WATCHES)
                .map(|entry| entry.presence())
                .collect(),
        };
        let operation = self.operation_id.ok_or("operation identity exhausted")?;
        if let Some(authorization) = &self.operation_authorization {
            let _ = authorization.set(diagnostics::OperationAuthorization {
                client_id: self.audit_client_id,
                lease_id: self.lease,
                operation_id: operation,
                lease_operation_generation: lease.operation_generation,
            });
        }
        if input == InputReservation::Continue && !control.leases.owns_input(self.lease, operation)
        {
            return Err("remote input ownership was cancelled".into());
        }
        if matches!(
            input,
            InputReservation::Transaction | InputReservation::Begin
        ) {
            control
                .leases
                .reserve_input(self.lease, operation)
                .map_err(|error| error.to_string())?;
        }
        // Acceptance is the last epoch check before invoking the native effect.
        // Stop may interrupt an already accepted effect; discard its result and
        // release our logical reservation even though native cleanup is deferred.
        let result = self
            .check_commit_boundary(&boundary)
            .and_then(|()| effect(&boundary))
            .and_then(|result| {
                self.check_commit_boundary(&boundary)?;
                if !control.has_ready_connection(&self.client, std::time::Instant::now()) {
                    return Err("remote connection expired during the operation".into());
                }
                Ok(result)
            });
        if input == InputReservation::Transaction
            || (input != InputReservation::None && result.is_err())
        {
            control.leases.release_input(self.lease, operation);
        }
        result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectiveState {
    Disabled,
    Enabled,
    Rejected,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeStatus {
    pub requested_enabled: bool,
    pub effective: EffectiveState,
    pub generation: u64,
    pub acknowledged_generation: u64,
    pub endpoint: String,
    pub host_fingerprint: Option<String>,
    pub environment_override: bool,
    pub diagnostic: Option<String>,
}

pub struct RemoteControlRuntime {
    emergency: EmergencyStopHandle,
    control: std::sync::Arc<std::sync::Mutex<ControlPlane>>,
    server: Option<RemoteControlServer>,
    status: RuntimeStatus,
}

impl Default for RemoteControlRuntime {
    fn default() -> Self {
        let control = ControlPlane::default();
        let emergency = control.emergency.clone();
        Self {
            emergency,
            control: std::sync::Arc::new(std::sync::Mutex::new(control)),
            server: None,
            status: RuntimeStatus {
                requested_enabled: false,
                effective: EffectiveState::Disabled,
                generation: 0,
                acknowledged_generation: 0,
                endpoint: MCP_ENDPOINT.into(),
                host_fingerprint: None,
                environment_override: false,
                diagnostic: None,
            },
        }
    }
}

impl RemoteControlRuntime {
    pub fn emergency_stop_handle(&self) -> EmergencyStopHandle {
        self.emergency.clone()
    }
    pub fn status(&self) -> &RuntimeStatus {
        &self.status
    }

    pub fn set_diagnostic(&mut self, diagnostic: impl Into<String>) {
        self.status.diagnostic = Some(diagnostic.into().chars().take(256).collect());
    }

    pub fn control(&self) -> std::sync::Arc<std::sync::Mutex<ControlPlane>> {
        self.control.clone()
    }

    pub fn apply(
        &mut self,
        settings: &RemoteAiControlSettings,
        desktop: std::sync::Arc<dyn DesktopAuthority>,
    ) {
        if settings.generation < self.status.acknowledged_generation {
            return;
        }
        self.status.requested_enabled = settings.requested_enabled;
        self.status.generation = settings.generation;
        self.status.acknowledged_generation = settings.generation;
        self.status.diagnostic = None;
        if !settings.requested_enabled {
            self.stop(EffectiveState::Disabled);
            return;
        }
        if self.server.is_some() {
            self.status.effective = EffectiveState::Enabled;
            return;
        }
        self.control.lock().unwrap().set_enabled(true);
        match RemoteControlServer::start(self.control.clone(), desktop) {
            Ok(server) => {
                self.status.endpoint = server.endpoint().to_owned();
                self.status.host_fingerprint = server.host_fingerprint.clone();
                self.status.environment_override = server.environment_override;
                self.server = Some(server);
                self.status.effective = EffectiveState::Enabled;
            }
            Err(error) => {
                self.control.lock().unwrap().set_enabled(false);
                self.status.effective = EffectiveState::Rejected;
                self.status.diagnostic = Some(error.to_string().chars().take(256).collect());
            }
        }
    }

    pub fn emergency_stop(&mut self) {
        let generation = self.status.generation.saturating_add(1);
        self.emergency_stop_at(generation);
    }

    pub fn emergency_stop_at(&mut self, generation: u64) {
        self.stop(EffectiveState::Disabled);
        self.status.requested_enabled = false;
        self.status.generation = generation;
        self.status.acknowledged_generation = generation;
    }

    /// End authority for this login session without changing the user's persisted preference.
    pub fn shutdown_session(&mut self) {
        self.stop(EffectiveState::Disabled);
    }

    pub fn lock(&mut self) {
        self.control.lock().unwrap().lock();
    }

    fn stop(&mut self, effective: EffectiveState) {
        self.emergency.trigger();
        // Detached blocking workers can outlive listener shutdown. Revoke their
        // authority before cancelling network tasks or joining the listener.
        self.control.lock().unwrap().set_enabled(false);
        if let Some(server) = self.server.take() {
            server.stop();
        }
        self.status.effective = effective;
    }
}

pub const MCP_ENDPOINT: &str = "http://127.0.0.1:42637/mcp";
pub const PAIRING_LIFETIME_SECS: u64 = 5 * 60;
pub const MAX_PAIRING_ATTEMPTS: u8 = 6;
const GLOBAL_PAIRING_ATTEMPTS: usize = 24;
const GLOBAL_PAIRING_WINDOW_SECS: u64 = 60;
pub const MAX_PENDING_CLIENTS: usize = 16;
const TOKEN_BYTES: usize = 32;
const CEREMONY_BYTES: usize = 16;
const CODE_SYMBOLS: &[u8; 32] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Capability {
    Observe,
    WindowManagement,
    SettingsRead,
    SettingsChange,
    ApplicationLaunch,
    PointerInput,
    KeyboardInput,
    ScreenCapture,
}

#[derive(Clone, Eq, PartialEq)]
pub struct PairingDisplay {
    pub ceremony_id: String,
    pub qr_payload: String,
    pub short_code: String,
    pub expires_at: u64,
}

impl std::fmt::Debug for PairingDisplay {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("PairingDisplay")
            .field("ceremony_id", &self.ceremony_id)
            .field("qr_payload", &"<redacted>")
            .field("short_code", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct PendingClient {
    pub id: String,
    pub label: String,
    pub requested: Vec<Capability>,
    pub connected_at: u64,
}

#[derive(Clone, Eq, PartialEq)]
pub struct GrantedClient {
    pub id: String,
    pub label: String,
    pub capabilities: Vec<Capability>,
    pub remembered: bool,
    token: [u8; TOKEN_BYTES],
    token_claimable: bool,
    origin: Option<ClientOrigin>,
}

/// Last authenticated socket peer. Not serialized into remote discovery or
/// metrics; the trusted local owner chooses its presentation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ClientOrigin {
    pub address: std::net::IpAddr,
    pub tls: bool,
}

pub const MAX_CONNECTION_AUDIT_EVENTS: usize = 128;

/// Trusted local history of changes to the authenticated socket origin.
/// Client identity is server-generated; labels, credentials and forwarded headers
/// never enter this record. Repeated requests from an unchanged peer coalesce.
pub struct ConnectionAuditEvent {
    pub generation: u64,
    pub observed_at: std::time::Instant,
    pub client_id: String,
    pub origin: ClientOrigin,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct GrantedClientSummary {
    pub id: String,
    pub label: String,
    pub capabilities: Vec<Capability>,
    pub remembered: bool,
}

#[derive(Clone, Eq, PartialEq)]
pub struct IssuedCapability {
    pub client_id: String,
    pub token: String,
    pub capabilities: Vec<Capability>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Approval {
    Deny,
    AllowOnce,
    Remember,
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum ControlError {
    #[error("remote control is disabled")]
    Disabled,
    #[error("pairing ceremony is missing or expired")]
    PairingExpired,
    #[error("pairing challenge was rejected")]
    PairingRejected,
    #[error("pairing attempt limit reached")]
    AttemptLimit,
    #[error("pairing attempts are temporarily rate limited")]
    RateLimited,
    #[error("pending-client capacity reached")]
    Capacity,
    #[error("client is no longer pending")]
    StaleClient,
    #[error("randomness is unavailable")]
    Randomness,
    #[error("capability is invalid or revoked")]
    Unauthorized,
    #[error("remembering a client requires secure credential storage, which is not available yet")]
    PersistentApprovalUnavailable,
}

#[derive(Clone)]
struct PairingCeremony {
    id: String,
    qr_secret: [u8; TOKEN_BYTES],
    code_digest: [u8; 32],
    expires_at: u64,
    attempts: u8,
}

#[derive(Default)]
pub struct ControlPlane {
    emergency: EmergencyStopHandle,
    emergency_acknowledged: Option<u64>,
    connection_watches: BTreeMap<u64, connection_watch::WatchEntry>,
    next_connection_watch: u64,
    connection_audit: VecDeque<ConnectionAuditEvent>,
    connection_audit_generation: u64,
    connection_audit_evicted: u64,
    trace_audit: std::sync::Arc<trace_audit::TraceAudit>,
    operation_metrics: std::sync::Arc<operation_metrics::OperationMetrics>,
    leases: leases::LeaseAuthority,
    lease_requests: lease_requests::LeaseRequests,
    enabled: bool,
    generation: u64,
    pairing: Option<PairingCeremony>,
    pending: BTreeMap<String, PendingClient>,
    granted: BTreeMap<String, GrantedClient>,
    pairing_attempts: VecDeque<u64>,
}

/// Stateful recognizer fed only by native physical-key adapters. Remote/synthetic reducers must
/// never call this recognizer, which makes the emergency chord unavailable to a controlled client.
#[derive(Default)]
pub struct EmergencyChord {
    left: bool,
    right: bool,
    fired: bool,
}

impl EmergencyChord {
    pub fn handle_physical(
        &mut self,
        key: nickel_input::KeyCode,
        edge: nickel_input::KeyEdge,
        remote_control_enabled: bool,
    ) -> bool {
        if !remote_control_enabled {
            self.left = false;
            self.right = false;
            self.fired = false;
            return false;
        }
        let pressed = edge == nickel_input::KeyEdge::Pressed;
        match key {
            nickel_input::KeyCode::ControlLeft => self.left = pressed,
            nickel_input::KeyCode::ControlRight => self.right = pressed,
            _ => return false,
        }
        if !self.left || !self.right {
            self.fired = false;
            return false;
        }
        if self.fired {
            return false;
        }
        self.fired = true;
        true
    }
}

impl ControlPlane {
    /// Logical presence is independent of individual HTTP requests and idle user time.
    /// Every authority-bearing boundary must check this under the authority lock.
    pub fn has_ready_connection(&self, client: &str, now: std::time::Instant) -> bool {
        self.ready_connection_deadline(client, now).is_some()
    }

    fn ready_connection_deadline(
        &self,
        client: &str,
        now: std::time::Instant,
    ) -> Option<std::time::Instant> {
        self.connection_watches
            .values()
            .filter(|entry| {
                entry.client == client
                    && entry.ready.load(std::sync::atomic::Ordering::SeqCst)
                    && entry.transport_is_alive()
                    && now < entry.expires_at
            })
            .map(|entry| entry.expires_at)
            .max()
    }

    pub fn trace_audit(&self) -> &trace_audit::TraceAudit {
        &self.trace_audit
    }
    /// Establish a session identity with no desktop authority. The random credential is issued
    /// once over the listener's protected transport and is distinct from any lease approval.
    pub fn connect_identity(&mut self, label: &str) -> Result<IssuedCapability, ControlError> {
        if !self.enabled() {
            return Err(ControlError::Disabled);
        }
        validate_label(label)?;
        if self.granted.len() >= 128 {
            return Err(ControlError::Capacity);
        }
        let id = hex(&random::<CEREMONY_BYTES>()?);
        let token = random::<TOKEN_BYTES>()?;
        self.granted.insert(
            id.clone(),
            GrantedClient {
                id: id.clone(),
                label: label.to_owned(),
                capabilities: Vec::new(),
                remembered: false,
                token,
                token_claimable: false,
                origin: None,
            },
        );
        self.generation = self.generation.saturating_add(1);
        Ok(IssuedCapability {
            client_id: id,
            token: hex(&token),
            capabilities: Vec::new(),
        })
    }
    pub fn request_lease(
        &mut self,
        client: &str,
        token: &str,
        request: lease_requests::LeaseRequest,
        now: std::time::Instant,
    ) -> Result<bool, lease_requests::RequestError> {
        if !self.enabled() || !self.authenticate(client, token) {
            self.lease_requests.record_unauthorized();
            return Err(lease_requests::RequestError::Unauthorized);
        }
        if self.lease_requests.is_blocked(client) {
            self.lease_requests
                .record_rejected(client, &lease_requests::RequestError::Blocked);
            return Err(lease_requests::RequestError::Blocked);
        }
        if !self.has_ready_connection(client, now) {
            self.lease_requests.record_unauthorized();
            return Err(lease_requests::RequestError::Unauthorized);
        }
        if let Err(error) = self.validate_renewal_request(client, &request, now) {
            self.lease_requests.record_rejected(client, &error);
            return Err(error);
        }
        let changed = self.lease_requests.request(client, request, now)?;
        if changed {
            self.generation = self.generation.saturating_add(1);
        }
        Ok(changed)
    }

    fn validate_renewal_request(
        &self,
        client: &str,
        request: &lease_requests::LeaseRequest,
        now: std::time::Instant,
    ) -> Result<(), lease_requests::RequestError> {
        if let Some(renewal) = &request.renewal {
            let lease = self
                .leases
                .active_lease(renewal.lease_id, client, now)
                .map_err(|_| lease_requests::RequestError::Unauthorized)?;
            if lease.renewal_generation != renewal.generation
                || lease.scope != request.scope
                || lease.full_debug != request.full_debug
                || lease.allow_resumption != request.allow_resumption
            {
                return Err(lease_requests::RequestError::Invalid);
            }
            let deadline = lease
                .expires_at
                .ok_or(lease_requests::RequestError::Invalid)?;
            let remaining = deadline.duration_since(now);
            if remaining > Duration::from_secs(300) {
                return Err(lease_requests::RequestError::Cooldown {
                    retry_after: remaining - Duration::from_secs(300),
                });
            }
            if request
                .duration
                .is_some_and(|duration| now.checked_add(duration).is_none_or(|new| new <= deadline))
            {
                return Err(lease_requests::RequestError::Invalid);
            }
        }
        Ok(())
    }

    pub fn approve_lease_local(
        &mut self,
        client: &str,
        displayed: &lease_requests::LeaseRequest,
        pending_generation: u64,
        now: std::time::Instant,
    ) -> Result<u64, leases::LeaseError> {
        self.approve_lease_with_duration_local(
            client,
            displayed,
            pending_generation,
            displayed.duration,
            now,
        )
    }

    /// The trusted local user may choose a different duration while approving
    /// the exact displayed request. Scope, client, debug access, resumption, and
    /// renewal incarnation must still match the pending card without alteration.
    pub fn approve_lease_with_duration_local(
        &mut self,
        client: &str,
        displayed: &lease_requests::LeaseRequest,
        pending_generation: u64,
        duration: Option<std::time::Duration>,
        now: std::time::Instant,
    ) -> Result<u64, leases::LeaseError> {
        if !self.enabled
            || !self.granted.contains_key(client)
            || !self.has_ready_connection(client, now)
            || self.lease_requests.pending_generation(client) != Some(pending_generation)
            || !self
                .lease_requests
                .pending()
                .any(|(id, request)| id == client && request == displayed)
        {
            return Err(leases::LeaseError::Unauthorized);
        }
        if duration.is_some_and(|duration| duration.is_zero()) {
            return Err(leases::LeaseError::Invalid);
        }
        let expires_at = duration
            .map(|duration| now.checked_add(duration).ok_or(leases::LeaseError::Invalid))
            .transpose()?;
        let id = if let Some(renewal) = &displayed.renewal {
            let lease = self.leases.active_lease(renewal.lease_id, client, now)?;
            if lease.renewal_generation != renewal.generation
                || lease.scope != displayed.scope
                || lease.full_debug != displayed.full_debug
                || lease.allow_resumption != displayed.allow_resumption
            {
                return Err(leases::LeaseError::Invalid);
            }
            let deadline = lease.expires_at.ok_or(leases::LeaseError::Invalid)?;
            self.leases
                .renew_local(renewal.lease_id, client, deadline, expires_at, now)?;
            renewal.lease_id
        } else {
            self.leases.approve_local(
                client.to_owned(),
                displayed.scope.clone(),
                now,
                expires_at,
                displayed.allow_resumption,
                displayed.full_debug,
            )?
        };
        self.lease_requests
            .take_approved_local(client, displayed, pending_generation);
        self.generation = self.generation.saturating_add(1);
        Ok(id)
    }

    pub fn lease_requests(&self) -> &lease_requests::LeaseRequests {
        &self.lease_requests
    }

    /// Keep local approval cards tied to the current client and lease incarnation.
    pub fn reconcile_pending_lease_requests(&mut self, now: std::time::Instant) {
        self.expire_connection_watches(now);
        let leases = &self.leases;
        let clients = &self.granted;
        let cancelled = self.lease_requests.retain_pending(|client, request| {
            clients.contains_key(client)
                && request.renewal.as_ref().is_none_or(|renewal| {
                    leases
                        .active_lease(renewal.lease_id, client, now)
                        .is_ok_and(|lease| {
                            lease.renewal_generation == renewal.generation
                                && lease.scope == request.scope
                                && lease.full_debug == request.full_debug
                                && lease.allow_resumption == request.allow_resumption
                                && lease.expires_at.is_some()
                        })
                })
        });
        if cancelled != 0 {
            self.generation = self.generation.saturating_add(1);
        }
    }

    /// Handle a confirmed logical client disconnect.
    ///
    /// Called by the transport owner after loss of the client's logical control
    /// connection, not after an individual stateless HTTP response. The desktop
    /// owner must drain cancelled operations and release native input before
    /// acknowledging the transition. Graceful MCP requests use this path;
    /// required connection watches use this path on their last stream loss.
    pub fn disconnect_client(&mut self, client: &str) -> bool {
        self.connection_watches
            .retain(|_, entry| entry.client != client);
        if !self.granted.contains_key(client) {
            return false;
        }
        let connected = self
            .leases
            .iter()
            .any(|lease| lease.client_identity == client && lease.is_connected());
        let pending = self.lease_requests.retain_pending(|id, _| id != client) != 0;
        if connected {
            self.leases.disconnect(client);
        }
        if connected || pending {
            self.generation = self.generation.saturating_add(1);
        }
        connected || pending
    }

    /// Re-establish identity without reviving revoked/expired leases, locally
    /// paused authority, cancelled input, or an old pending approval card.
    pub fn reconnect_client_authenticated(
        &mut self,
        client: &str,
        token: &str,
        now: std::time::Instant,
    ) -> Result<(), ControlError> {
        if !self.enabled
            || !self.authenticate(client, token)
            || !self.has_ready_connection(client, now)
        {
            return Err(ControlError::Unauthorized);
        }
        self.leases.reconnect_authenticated(client, now);
        self.reconcile_pending_lease_requests(now);
        self.generation = self.generation.saturating_add(1);
        Ok(())
    }

    /// Trusted local policy. Reversing a block never recreates revoked leases.
    pub fn block_client_local(&mut self, client: &str, blocked: bool) -> Result<(), ControlError> {
        if !self.granted.contains_key(client) {
            return Err(ControlError::Unauthorized);
        }
        if !self.lease_requests.block_local(client, blocked) {
            return Err(ControlError::Capacity);
        }
        if blocked {
            self.connection_watches
                .retain(|_, entry| entry.client != client);
            self.leases.revoke_client(client);
        }
        self.generation = self.generation.saturating_add(1);
        Ok(())
    }

    pub fn lease_requests_mut(&mut self) -> &mut lease_requests::LeaseRequests {
        &mut self.lease_requests
    }
    pub fn leases(&self) -> &leases::LeaseAuthority {
        &self.leases
    }

    /// Local approval and production dispatch share this authority under the control-plane lock.
    pub fn leases_mut(&mut self) -> &mut leases::LeaseAuthority {
        &mut self.leases
    }
    pub fn enabled(&self) -> bool {
        self.enabled && !self.emergency.stopped()
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if enabled {
            if self.enabled
                || !self
                    .emergency
                    .rearm(self.emergency_acknowledged.unwrap_or(1))
            {
                return;
            }
            self.enabled = true;
        } else {
            if !self.enabled && self.emergency.epoch() == self.emergency_acknowledged.unwrap_or(1) {
                return;
            }
            let acknowledged = self.emergency.latch();
            self.enabled = false;
            self.revoke_runtime_authority();
            self.emergency_acknowledged = Some(acknowledged);
        }
        self.generation = self.generation.saturating_add(1);
    }

    pub fn start_pairing(&mut self, now: u64) -> Result<PairingDisplay, ControlError> {
        if !self.enabled() {
            return Err(ControlError::Disabled);
        }
        let ceremony_random = random::<CEREMONY_BYTES>()?;
        let qr_secret = random::<TOKEN_BYTES>()?;
        let code_random = random::<5>()?;
        let ceremony_id = hex(&ceremony_random);
        let short_code = encode_code(code_random);
        let expires_at = now.saturating_add(PAIRING_LIFETIME_SECS);
        let qr_payload = format!(
            "nickel://pair/v1?ceremony={ceremony_id}&secret={}",
            hex(&qr_secret)
        );
        self.pairing = Some(PairingCeremony {
            id: ceremony_id.clone(),
            qr_secret,
            code_digest: digest(short_code.as_bytes()),
            expires_at,
            attempts: 0,
        });
        self.generation = self.generation.saturating_add(1);
        Ok(PairingDisplay {
            ceremony_id,
            qr_payload,
            short_code,
            expires_at,
        })
    }

    pub fn cancel_pairing(&mut self) {
        if self.pairing.take().is_some() {
            self.generation = self.generation.saturating_add(1);
        }
    }

    pub fn exchange_short_code(
        &mut self,
        ceremony_id: &str,
        short_code: &str,
        label: &str,
        requested: Vec<Capability>,
        now: u64,
    ) -> Result<PendingClient, ControlError> {
        let candidate = digest(short_code.trim().to_ascii_uppercase().as_bytes());
        self.exchange(ceremony_id, &candidate, false, label, requested, now)
    }

    pub fn exchange_qr_secret(
        &mut self,
        ceremony_id: &str,
        qr_secret_hex: &str,
        label: &str,
        requested: Vec<Capability>,
        now: u64,
    ) -> Result<PendingClient, ControlError> {
        let secret = decode_hex::<TOKEN_BYTES>(qr_secret_hex).unwrap_or([0; TOKEN_BYTES]);
        self.exchange(ceremony_id, &secret, true, label, requested, now)
    }

    fn exchange(
        &mut self,
        ceremony_id: &str,
        candidate: &[u8; 32],
        qr: bool,
        label: &str,
        mut requested: Vec<Capability>,
        now: u64,
    ) -> Result<PendingClient, ControlError> {
        if !self.enabled() {
            return Err(ControlError::Disabled);
        }
        while self
            .pairing_attempts
            .front()
            .is_some_and(|attempt| now.saturating_sub(*attempt) >= GLOBAL_PAIRING_WINDOW_SECS)
        {
            self.pairing_attempts.pop_front();
        }
        if self.pairing_attempts.len() >= GLOBAL_PAIRING_ATTEMPTS {
            return Err(ControlError::RateLimited);
        }
        self.pairing_attempts.push_back(now);
        let ceremony = self.pairing.as_mut().ok_or(ControlError::PairingExpired)?;
        if now > ceremony.expires_at {
            self.pairing = None;
            return Err(ControlError::PairingExpired);
        }
        if ceremony.attempts >= MAX_PAIRING_ATTEMPTS {
            self.pairing = None;
            return Err(ControlError::AttemptLimit);
        }
        ceremony.attempts += 1;
        let expected = if qr {
            &ceremony.qr_secret
        } else {
            &ceremony.code_digest
        };
        let valid_id = ceremony.id.as_bytes().ct_eq(ceremony_id.as_bytes()).into();
        let valid_secret: bool = expected.ct_eq(candidate).into();
        if !(valid_id && valid_secret) {
            if ceremony.attempts >= MAX_PAIRING_ATTEMPTS {
                self.pairing = None;
                return Err(ControlError::AttemptLimit);
            }
            return Err(ControlError::PairingRejected);
        }
        if self.pending.len() >= MAX_PENDING_CLIENTS {
            return Err(ControlError::Capacity);
        }
        validate_label(label)?;
        requested.sort();
        requested.dedup();
        let client = PendingClient {
            id: hex(&random::<CEREMONY_BYTES>()?),
            label: label.to_owned(),
            requested,
            connected_at: now,
        };
        self.pairing = None;
        self.pending.insert(client.id.clone(), client.clone());
        self.generation = self.generation.saturating_add(1);
        Ok(client)
    }

    pub fn pending_clients(&self) -> impl Iterator<Item = &PendingClient> {
        self.pending.values()
    }

    pub fn granted_clients(&self) -> impl Iterator<Item = GrantedClientSummary> + '_ {
        self.granted.values().map(|client| GrantedClientSummary {
            id: client.id.clone(),
            label: client.label.clone(),
            capabilities: client.capabilities.clone(),
            remembered: client.remembered,
        })
    }

    pub fn client_origin(&self, client: &str) -> Option<ClientOrigin> {
        self.granted.get(client).and_then(|client| client.origin)
    }

    pub(crate) fn lease_metrics(
        &self,
        now: std::time::Instant,
        observed_at_us: u64,
    ) -> diagnostics::LeaseMetricsDiagnostic {
        let mut active_by_scope = [0; 5];
        for lease in self.leases.iter().filter(|lease| {
            self.has_ready_connection(&lease.client_identity, now)
                && self
                    .leases
                    .active_lease(lease.id, &lease.client_identity, now)
                    .is_ok()
        }) {
            use leases::ResourceScope;
            active_by_scope[match lease.scope {
                ResourceScope::Surface(_) => 0,
                ResourceScope::Window(_) => 1,
                ResourceScope::Application(_) => 2,
                ResourceScope::Output(_) => 3,
                ResourceScope::FullSession => 4,
            }] += 1;
        }
        diagnostics::LeaseMetricsDiagnostic {
            observed_at_us,
            active_total: active_by_scope.iter().sum(),
            active_by_scope,
            pending_requests: self.lease_requests.pending().count() as u64,
        }
    }

    pub(crate) fn record_client_origin(&mut self, client: &str, origin: ClientOrigin) {
        if let Some(client) = self.granted.get_mut(client)
            && client.origin != Some(origin)
        {
            client.origin = Some(origin);
            self.connection_audit_generation = self.connection_audit_generation.saturating_add(1);
            if self.connection_audit.len() == MAX_CONNECTION_AUDIT_EVENTS {
                self.connection_audit.pop_front();
                self.connection_audit_evicted = self.connection_audit_evicted.saturating_add(1);
            }
            self.connection_audit.push_back(ConnectionAuditEvent {
                generation: self.connection_audit_generation,
                observed_at: std::time::Instant::now(),
                client_id: client.id.clone(),
                origin,
            });
            self.generation = self.generation.saturating_add(1);
        }
    }

    pub fn connection_audit(&self) -> impl Iterator<Item = &ConnectionAuditEvent> {
        self.connection_audit.iter()
    }

    pub fn connection_audit_evicted(&self) -> u64 {
        self.connection_audit_evicted
    }

    pub fn approve(
        &mut self,
        client_id: &str,
        approval: Approval,
        locally_confirmed_capabilities: Vec<Capability>,
    ) -> Result<Option<IssuedCapability>, ControlError> {
        let pending = self
            .pending
            .get(client_id)
            .cloned()
            .ok_or(ControlError::StaleClient)?;
        if approval == Approval::Deny {
            self.pending.remove(client_id);
            self.generation = self.generation.saturating_add(1);
            return Ok(None);
        }
        if approval == Approval::Remember {
            return Err(ControlError::PersistentApprovalUnavailable);
        }
        let mut capabilities = locally_confirmed_capabilities;
        capabilities.sort();
        capabilities.dedup();
        if capabilities
            .iter()
            .any(|capability| !pending.requested.contains(capability))
        {
            return Err(ControlError::Unauthorized);
        }
        let token = random::<TOKEN_BYTES>()?;
        self.pending.remove(client_id);
        self.generation = self.generation.saturating_add(1);
        let issued = IssuedCapability {
            client_id: pending.id.clone(),
            token: hex(&token),
            capabilities: capabilities.clone(),
        };
        self.granted.insert(
            pending.id.clone(),
            GrantedClient {
                id: pending.id,
                label: pending.label,
                capabilities,
                remembered: false,
                token,
                token_claimable: true,
                origin: None,
            },
        );
        Ok(Some(issued))
    }

    /// Deliver a newly approved credential exactly once to the client that holds the random
    /// pending-client identity returned by the challenge exchange.
    pub fn claim_issued(&mut self, client_id: &str) -> Option<IssuedCapability> {
        let grant = self.granted.get_mut(client_id)?;
        if !grant.token_claimable {
            return None;
        }
        grant.token_claimable = false;
        Some(IssuedCapability {
            client_id: grant.id.clone(),
            token: hex(&grant.token),
            capabilities: grant.capabilities.clone(),
        })
    }

    pub fn authorize(&self, client_id: &str, token_hex: &str, capability: Capability) -> bool {
        if !self.enabled() {
            return false;
        }
        let candidate = decode_hex::<TOKEN_BYTES>(token_hex).unwrap_or([0; TOKEN_BYTES]);
        self.granted.get(client_id).is_some_and(|grant| {
            bool::from(grant.token.ct_eq(&candidate)) && grant.capabilities.contains(&capability)
        })
    }

    pub fn authenticate(&self, client_id: &str, token_hex: &str) -> bool {
        if !self.enabled() {
            return false;
        }
        let candidate = decode_hex::<TOKEN_BYTES>(token_hex).unwrap_or([0; TOKEN_BYTES]);
        self.granted
            .get(client_id)
            .is_some_and(|grant| bool::from(grant.token.ct_eq(&candidate)))
    }

    pub fn revoke(&mut self, client_id: &str) -> bool {
        self.connection_watches
            .retain(|_, entry| entry.client != client_id);
        self.leases.revoke_client(client_id);
        let removed = self.granted.remove(client_id).is_some();
        if removed {
            self.generation = self.generation.saturating_add(1);
        }
        removed
    }

    pub fn lock(&mut self) {
        self.revoke_runtime_authority();
        self.generation = self.generation.saturating_add(1);
    }

    pub fn emergency_stop(&mut self) {
        let acknowledged = self.emergency.latch();
        self.enabled = false;
        self.revoke_runtime_authority();
        self.emergency_acknowledged = Some(acknowledged);
        self.generation = self.generation.saturating_add(1);
    }

    fn revoke_runtime_authority(&mut self) {
        self.connection_watches.clear();
        self.lease_requests.cancel_pending();
        self.leases.clear();
        self.pairing = None;
        self.pending.clear();
        self.granted.clear();
    }
}

fn validate_label(label: &str) -> Result<(), ControlError> {
    (!label.trim().is_empty() && label.len() <= 128)
        .then_some(())
        .ok_or(ControlError::PairingRejected)
}

fn random<const N: usize>() -> Result<[u8; N], ControlError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| ControlError::Randomness)?;
    Ok(bytes)
}

fn digest(bytes: &[u8]) -> [u8; 32] {
    Sha256::digest(bytes).into()
}

fn encode_code(bytes: [u8; 5]) -> String {
    let value = u64::from_be_bytes([0, 0, 0, bytes[0], bytes[1], bytes[2], bytes[3], bytes[4]]);
    let symbols = (0..8)
        .rev()
        .map(|shift| CODE_SYMBOLS[((value >> (shift * 5)) & 31) as usize] as char)
        .collect::<String>();
    format!("{}-{}", &symbols[..4], &symbols[4..])
}

fn hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut result = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        result.push(HEX[(byte >> 4) as usize] as char);
        result.push(HEX[(byte & 15) as usize] as char);
    }
    result
}

fn decode_hex<const N: usize>(value: &str) -> Option<[u8; N]> {
    if value.len() != N * 2 {
        return None;
    }
    let mut output = [0; N];
    for (index, chunk) in value.as_bytes().chunks_exact(2).enumerate() {
        output[index] = (hex_digit(chunk[0])? << 4) | hex_digit(chunk[1])?;
    }
    Some(output)
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

#[cfg(test)]
pub(crate) fn ready_connection(
    plane: &mut ControlPlane,
    client: &IssuedCapability,
    now: std::time::Instant,
) {
    let id = plane
        .reserve_connection_watch(&client.client_id, &client.token, now)
        .unwrap();
    plane
        .activate_connection_watch(&client.client_id, &client.token, id, false, now)
        .unwrap();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn held_input_uses_fresh_requests_without_reopening_cancelled_gestures() {
        let now = std::time::Instant::now();
        let control = std::sync::Arc::new(std::sync::Mutex::new(ControlPlane::default()));
        let (identity, lease) = {
            let mut plane = control.lock().unwrap();
            plane.set_enabled(true);
            let identity = plane.connect_identity("gesture owner").unwrap();
            crate::ready_connection(&mut plane, &identity, std::time::Instant::now());
            let lease = plane
                .leases_mut()
                .approve_local(
                    identity.client_id.clone(),
                    leases::ResourceScope::FullSession,
                    now,
                    None,
                    false,
                    false,
                )
                .unwrap();
            (identity, lease)
        };
        let request = || {
            DesktopPermit::new(
                control.clone(),
                identity.client_id.clone(),
                identity.token.clone(),
                lease,
            )
        };
        let resource = leases::ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: None,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        assert!(
            request()
                .begin_input(&resource, || Err("rejected press".into()))
                .is_err()
        );
        let first = request();
        let mut held = first.begin_input(&resource, || Ok(())).unwrap();
        assert!(
            first
                .begin_input(&resource, || panic!("duplicate gesture owner"))
                .is_err()
        );
        // The old HTTP deadline cannot terminate an otherwise active gesture.
        held.permit.deadline = now;
        assert!(held.check_live().is_ok());
        assert!(
            request()
                .with_input::<()>(&resource, || panic!("gesture stolen"))
                .is_err()
        );
        assert!(request().with_resource(&resource, || Ok(())).is_ok());
        let protected = leases::ResourceEvidence {
            protected: true,
            ..resource
        };
        assert!(
            request()
                .continue_input::<()>(&held, &protected, || panic!(
                    "protected continuation delivered"
                ))
                .is_err()
        );
        assert!(
            request()
                .continue_input(&held, &resource, || Ok(()))
                .is_ok()
        );
        let mut stale = request();
        stale.deadline = now;
        assert!(
            stale
                .continue_input::<()>(&held, &resource, || panic!("expired request delivered"))
                .is_err()
        );
        assert!(held.check_live().is_ok());
        // An error from a delivered continuation ends ownership; it cannot be
        // reacquired using the old gesture object, even under the same lease.
        assert!(
            request()
                .continue_input::<()>(&held, &resource, || Err("platform rejected motion".into()))
                .is_err()
        );
        assert!(held.check_live().is_err());
        assert!(
            request()
                .continue_input::<()>(&held, &resource, || panic!("cancelled gesture restarted"))
                .is_err()
        );
        drop(held);
        let held = request().begin_input(&resource, || Ok(())).unwrap();
        {
            let mut plane = control.lock().unwrap();
            plane.leases_mut().suspend_local(lease).unwrap();
            plane.leases_mut().resume_local(lease, now).unwrap();
        }
        assert!(
            request()
                .continue_input::<()>(&held, &resource, || panic!("resumed cancelled gesture"))
                .is_err()
        );
        let replacement = request().begin_input(&resource, || Ok(())).unwrap();
        // Dropping the cancelled object cannot release a replacement gesture.
        drop(held);
        assert!(replacement.check_live().is_ok());
        drop(replacement);
        assert!(request().with_input(&resource, || Ok(())).is_ok());
    }

    #[test]
    fn owner_boundary_refuses_conflicting_input_but_allows_observation_and_releases_failed_effects()
    {
        let now = std::time::Instant::now();
        let control = std::sync::Arc::new(std::sync::Mutex::new(ControlPlane::default()));
        let (identity, lease) = {
            let mut control = control.lock().unwrap();
            control.set_enabled(true);
            let identity = control.connect_identity("input owner").unwrap();
            crate::ready_connection(&mut control, &identity, now);
            let lease = control
                .leases_mut()
                .approve_local(
                    identity.client_id.clone(),
                    leases::ResourceScope::FullSession,
                    now,
                    Some(now + std::time::Duration::from_secs(1200)),
                    false,
                    false,
                )
                .unwrap();
            (identity, lease)
        };
        let permit = DesktopPermit::new(control.clone(), identity.client_id, identity.token, lease);
        let resource = leases::ResourceEvidence {
            surface: None,
            window: None,
            verified_application: None,
            output: None,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        control
            .lock()
            .unwrap()
            .leases_mut()
            .reserve_input(lease, 0)
            .unwrap();
        assert!(
            permit
                .with_input::<()>(&resource, || panic!("conflicting input executed"))
                .is_err()
        );
        assert!(permit.with_resource(&resource, || Ok(())).is_ok());
        control.lock().unwrap().leases_mut().release_input(lease, 0);
        assert!(
            permit
                .with_input::<()>(&resource, || Err("platform rejected effect".into()))
                .is_err()
        );
        // A different reservation succeeding proves rejection did not retain ownership.
        control
            .lock()
            .unwrap()
            .leases_mut()
            .reserve_input(lease, 0)
            .unwrap();
        control.lock().unwrap().leases_mut().release_input(lease, 0);
        assert!(permit.with_input(&resource, || Ok(())).is_ok());
    }

    #[test]
    fn local_duration_choice_preserves_exact_pending_authority() {
        for duration in [
            Some(Duration::from_secs(1200)),
            Some(Duration::from_secs(7200)),
            Some(Duration::from_secs(420)),
            None,
        ] {
            let now = std::time::Instant::now();
            let mut plane = ControlPlane::default();
            plane.set_enabled(true);
            let client = plane.connect_identity("Local duration fixture").unwrap();
            crate::ready_connection(&mut plane, &client, std::time::Instant::now());
            let request = lease_requests::LeaseRequest {
                renewal: None,
                scope: leases::ResourceScope::FullSession,
                duration: Some(Duration::from_secs(30)),
                allow_resumption: false,
                full_debug: false,
            };
            plane
                .request_lease(&client.client_id, &client.token, request.clone(), now)
                .unwrap();
            for invalid in [Duration::ZERO, Duration::MAX] {
                assert!(
                    plane
                        .approve_lease_with_duration_local(
                            &client.client_id,
                            &request,
                            plane
                                .lease_requests()
                                .pending_generation(&client.client_id)
                                .unwrap_or(0),
                            Some(invalid),
                            now
                        )
                        .is_err()
                );
                assert_eq!(plane.lease_requests().pending().count(), 1);
                assert_eq!(plane.leases().iter().count(), 0);
            }
            let changed = lease_requests::LeaseRequest {
                full_debug: true,
                ..request.clone()
            };
            assert!(
                plane
                    .approve_lease_with_duration_local(
                        &client.client_id,
                        &changed,
                        plane
                            .lease_requests()
                            .pending_generation(&client.client_id)
                            .unwrap_or(0),
                        duration,
                        now
                    )
                    .is_err()
            );
            let id = plane
                .approve_lease_with_duration_local(
                    &client.client_id,
                    &request,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    duration,
                    now,
                )
                .unwrap();
            let lease = plane.leases().iter().find(|lease| lease.id == id).unwrap();
            assert_eq!(lease.expires_at, duration.map(|duration| now + duration));
            assert_eq!(lease.scope, request.scope);
            assert!(!lease.full_debug);
            assert!(!lease.allow_resumption);
            assert_eq!(plane.lease_requests().pending().count(), 0);
            assert!(
                plane
                    .approve_lease_with_duration_local(
                        &client.client_id,
                        &request,
                        plane
                            .lease_requests()
                            .pending_generation(&client.client_id)
                            .unwrap_or(0),
                        duration,
                        now
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn lease_metrics_exclude_paused_disconnected_revoked_and_expired_authority() {
        let now = std::time::Instant::now();
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let client = plane.connect_identity("private metric label").unwrap();
        crate::ready_connection(&mut plane, &client, std::time::Instant::now());
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: Some(Duration::from_secs(1)),
            allow_resumption: true,
            full_debug: false,
        };
        plane
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        assert_eq!(plane.lease_metrics(now, 0).pending_requests, 1);
        let lease = plane
            .approve_lease_local(
                &client.client_id,
                &request,
                plane
                    .lease_requests()
                    .pending_generation(&client.client_id)
                    .unwrap_or(0),
                now,
            )
            .unwrap();
        let active = plane.lease_metrics(now, 42);
        assert_eq!(active.observed_at_us, 42);
        assert_eq!(active.pending_requests, 0);
        assert_eq!(active.active_by_scope, [0, 0, 0, 0, 1]);
        plane.leases.suspend_local(lease).unwrap();
        assert_eq!(plane.lease_metrics(now, 0).active_total, 0);
        plane.leases.resume_local(lease, now).unwrap();
        assert_eq!(plane.lease_metrics(now, 0).active_total, 1);
        plane.disconnect_client(&client.client_id);
        assert_eq!(plane.lease_metrics(now, 0).active_total, 0);
        crate::ready_connection(&mut plane, &client, now);
        plane
            .reconnect_client_authenticated(&client.client_id, &client.token, now)
            .unwrap();
        assert_eq!(plane.lease_metrics(now, 0).active_total, 1);
        assert_eq!(
            plane
                .lease_metrics(now + Duration::from_secs(1), 0)
                .active_total,
            0,
            "expiry is evaluated at observation, before any cleanup tick"
        );
        plane.leases.revoke(lease);
        assert_eq!(plane.lease_metrics(now, 0).active_total, 0);
        let encoded = serde_json::to_string(&active).unwrap();
        assert!(!encoded.contains(&client.client_id) && !encoded.contains("private metric label"));
    }

    #[test]
    fn connection_origin_history_is_bounded_coalesced_and_excludes_unknown_clients() {
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let client = plane.connect_identity("private label canary").unwrap();
        crate::ready_connection(&mut plane, &client, std::time::Instant::now());
        let origin = ClientOrigin {
            address: "127.0.0.1".parse().unwrap(),
            tls: false,
        };
        plane.record_client_origin("unknown client", origin);
        assert_eq!(plane.connection_audit().count(), 0);
        for index in 0..140 {
            let origin = ClientOrigin {
                address: std::net::Ipv4Addr::new(127, 0, 0, index + 1).into(),
                ..origin
            };
            plane.record_client_origin(&client.client_id, origin);
            let generation = plane.generation;
            plane.record_client_origin(&client.client_id, origin);
            assert_eq!(plane.generation, generation, "unchanged peer must coalesce");
        }
        assert_eq!(
            plane.connection_audit().count(),
            MAX_CONNECTION_AUDIT_EVENTS
        );
        assert_eq!(plane.connection_audit_evicted(), 12);
        assert_eq!(plane.connection_audit().next().unwrap().generation, 13);
        assert!(
            plane
                .connection_audit()
                .all(|event| event.client_id == client.client_id)
        );
        let last = plane.client_origin(&client.client_id).unwrap();
        plane.record_client_origin(&client.client_id, ClientOrigin { tls: true, ..last });
        assert_eq!(
            plane.connection_audit_evicted(),
            13,
            "transport changes must be recorded"
        );
        plane.revoke(&client.client_id);
        assert_eq!(
            plane.connection_audit().count(),
            MAX_CONNECTION_AUDIT_EVENTS,
            "revocation must not erase local history"
        );
    }

    #[test]
    fn graceful_connection_permit_checks_delivery_identity_and_lock_at_execution() {
        let now = std::time::Instant::now();
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let client = plane
            .connect_identity("Graceful connection fixture")
            .unwrap();
        crate::ready_connection(&mut plane, &client, std::time::Instant::now());
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: Some(Duration::from_secs(60)),
            allow_resumption: true,
            full_debug: false,
        };
        plane
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        let lease = plane
            .approve_lease_local(
                &client.client_id,
                &request,
                plane
                    .lease_requests()
                    .pending_generation(&client.client_id)
                    .unwrap_or(0),
                now,
            )
            .unwrap();
        let metrics = plane.operation_metrics.clone();
        let control = std::sync::Arc::new(std::sync::Mutex::new(plane));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            for case in [
                "cancelled",
                "expired",
                "wrong_identity",
                "disconnect",
                "locked",
                "reconnect",
            ] {
                metrics
                    .measure(operation_metrics::Method::ClientConnection, async {
                        let cancellation = tokio_util::sync::CancellationToken::new();
                        let mut permit = ClientConnectionPermit::new(
                            control.clone(),
                            client.client_id.clone(),
                            client.token.clone(),
                            &metrics,
                            cancellation.clone(),
                        )
                        .unwrap();
                        match case {
                            "cancelled" => cancellation.cancel(),
                            "expired" => permit.deadline = now,
                            "wrong_identity" => permit.token = "wrong token".into(),
                            _ => (),
                        }
                        if case == "reconnect" {
                            crate::ready_connection(&mut control.lock().unwrap(), &client, now);
                        }
                        let action = if matches!(case, "locked" | "reconnect") {
                            ClientConnectionAction::Reconnect
                        } else {
                            ClientConnectionAction::Disconnect
                        };
                        let result = permit.apply(action, case == "locked");
                        assert_eq!(
                            result.is_ok(),
                            matches!(case, "disconnect" | "reconnect"),
                            "{case}"
                        );
                        let plane = control.lock().unwrap();
                        assert_eq!(
                            plane
                                .leases()
                                .iter()
                                .find(|entry| entry.id == lease)
                                .unwrap()
                                .is_connected(),
                            !matches!(case, "disconnect" | "locked"),
                            "{case}",
                        );
                        Ok::<_, String>(())
                    })
                    .await
                    .unwrap();
            }
        });
        assert!(
            ClientConnectionPermit::new(
                control,
                client.client_id,
                client.token,
                &metrics,
                tokio_util::sync::CancellationToken::new()
            )
            .is_err()
        );
    }

    #[test]
    fn request_lifetime_cancels_abandoned_permits_but_preserves_successful_standing_work() {
        let now = std::time::Instant::now();
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let client = plane.connect_identity("Request lifetime fixture").unwrap();
        crate::ready_connection(&mut plane, &client, std::time::Instant::now());
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: None,
            allow_resumption: false,
            full_debug: false,
        };
        plane
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        let lease = plane
            .approve_lease_local(
                &client.client_id,
                &request,
                plane
                    .lease_requests()
                    .pending_generation(&client.client_id)
                    .unwrap_or(0),
                now,
            )
            .unwrap();
        let metrics = plane.operation_metrics.clone();
        let control = std::sync::Arc::new(std::sync::Mutex::new(plane));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            for outcome in ["success", "error", "abandoned", "transport_cancelled"] {
                let token = tokio_util::sync::CancellationToken::new();
                let request_token = token.clone();
                let request_control = control.clone();
                let request_metrics = metrics.clone();
                let client_id = client.client_id.clone();
                let credential = client.token.clone();
                let (tx, rx) = tokio::sync::oneshot::channel();
                let task = tokio::spawn(async move {
                    request_metrics
                        .measure(operation_metrics::Method::LaunchApplication, async move {
                            let permit =
                                DesktopPermit::new(request_control, client_id, credential, lease);
                            permit.attach_request_cancellation(request_token).unwrap();
                            assert!(tx.send(permit).is_ok());
                            match outcome {
                                "success" => Ok(()),
                                "error" => Err(()),
                                _ => std::future::pending::<Result<(), ()>>().await,
                            }
                        })
                        .await
                });
                let permit = rx.await.unwrap();
                if outcome == "transport_cancelled" {
                    token.cancel();
                    assert!(
                        permit
                            .with_resource::<()>(
                                &leases::ResourceEvidence {
                                    window: None,
                                    surface: None,
                                    output: None,
                                    verified_application: None,
                                    authorized_surface_ancestors: &[],
                                    protected: false,
                                },
                                || panic!("cancelled worker committed an effect")
                            )
                            .is_err()
                    );
                }
                if matches!(outcome, "abandoned" | "transport_cancelled") {
                    task.abort();
                    assert!(task.await.unwrap_err().is_cancelled());
                } else {
                    assert_eq!(task.await.unwrap().is_ok(), outcome == "success");
                }
                token.cancel();
                assert_eq!(permit.check_live().is_ok(), outcome == "success");
                assert_eq!(permit.continued_observation().is_ok(), outcome == "success");
                assert_eq!(permit.check_standing_live().is_ok(), outcome == "success");
            }
        });
        let unscoped = DesktopPermit::new(control, client.client_id, client.token, lease);
        assert!(
            unscoped
                .attach_request_cancellation(tokio_util::sync::CancellationToken::new())
                .is_err()
        );
    }

    #[test]
    fn client_disconnect_cancels_its_requests_and_only_resumes_eligible_leases() {
        let now = std::time::Instant::now();
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let first = plane.connect_identity("First client").unwrap();
        crate::ready_connection(&mut plane, &first, std::time::Instant::now());
        let second = plane.connect_identity("Second client").unwrap();
        crate::ready_connection(&mut plane, &second, std::time::Instant::now());
        let approve = |plane: &mut ControlPlane, client: &IssuedCapability, resumable| {
            let request = lease_requests::LeaseRequest {
                renewal: None,
                scope: leases::ResourceScope::FullSession,
                duration: Some(Duration::from_secs(30)),
                allow_resumption: resumable,
                full_debug: false,
            };
            plane
                .request_lease(&client.client_id, &client.token, request.clone(), now)
                .unwrap();
            (
                plane
                    .approve_lease_local(
                        &client.client_id,
                        &request,
                        plane
                            .lease_requests()
                            .pending_generation(&client.client_id)
                            .unwrap_or(0),
                        now,
                    )
                    .unwrap(),
                request,
            )
        };
        let (temporary, _) = approve(&mut plane, &first, false);
        let (resumable, first_request) = approve(&mut plane, &first, true);
        let (other, second_request) = approve(&mut plane, &second, false);
        for (client, request) in [(&first, &first_request), (&second, &second_request)] {
            plane
                .request_lease(&client.client_id, &client.token, request.clone(), now)
                .unwrap();
        }
        plane.leases.reserve_input(resumable, 42).unwrap();
        assert!(plane.disconnect_client(&first.client_id));
        let generation = plane.generation;
        assert!(!plane.disconnect_client(&first.client_id));
        assert_eq!(plane.generation, generation);
        assert!(!plane.leases.iter().any(|lease| lease.id == temporary));
        let retained = plane
            .leases
            .iter()
            .find(|lease| lease.id == resumable)
            .unwrap();
        assert!(!retained.is_connected());
        assert_eq!(retained.operation_generation, 1);
        assert!(!plane.leases.owns_input(resumable, 42));
        assert!(
            plane
                .leases
                .active_lease(other, &second.client_id, now)
                .is_ok()
        );
        assert_eq!(
            plane
                .lease_requests
                .pending()
                .map(|(client, _)| client)
                .collect::<Vec<_>>(),
            vec![second.client_id.as_str()]
        );
        assert!(
            plane
                .approve_lease_local(
                    &first.client_id,
                    &first_request,
                    plane
                        .lease_requests()
                        .pending_generation(&first.client_id)
                        .unwrap_or(0),
                    now
                )
                .is_err()
        );
        assert!(
            plane
                .reconnect_client_authenticated(&first.client_id, &second.token, now)
                .is_err()
        );
        assert!(
            plane
                .leases
                .active_lease(resumable, &first.client_id, now)
                .is_err()
        );
        crate::ready_connection(&mut plane, &first, now);
        plane
            .reconnect_client_authenticated(&first.client_id, &first.token, now)
            .unwrap();
        assert!(
            plane
                .leases
                .active_lease(resumable, &first.client_id, now)
                .is_ok()
        );
        assert!(!plane.leases.owns_input(resumable, 42));
        assert!(
            plane
                .approve_lease_local(
                    &first.client_id,
                    &first_request,
                    plane
                        .lease_requests()
                        .pending_generation(&first.client_id)
                        .unwrap_or(0),
                    now
                )
                .is_err()
        );
        // Cancellation imposes no denial cooldown and restores no pending card.
        assert!(
            plane
                .request_lease(&first.client_id, &first.token, first_request, now)
                .unwrap()
        );
        plane.leases.suspend_local(resumable).unwrap();
        plane.disconnect_client(&first.client_id);
        crate::ready_connection(&mut plane, &first, now);
        plane
            .reconnect_client_authenticated(&first.client_id, &first.token, now)
            .unwrap();
        let paused = plane
            .leases
            .iter()
            .find(|lease| lease.id == resumable)
            .unwrap();
        assert!(paused.is_connected() && paused.suspended);
        assert!(
            plane
                .leases
                .active_lease(resumable, &first.client_id, now)
                .is_err()
        );
        plane.disconnect_client(&first.client_id);
        crate::ready_connection(&mut plane, &first, now + Duration::from_secs(30));
        plane
            .reconnect_client_authenticated(
                &first.client_id,
                &first.token,
                now + Duration::from_secs(30),
            )
            .unwrap();
        assert!(!plane.leases.iter().any(|lease| lease.id == resumable));
    }

    #[test]
    fn method_history_correlates_authorized_worker_calls_without_recording_rejected_targets() {
        let now = std::time::Instant::now();
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let client = plane
            .connect_identity("private-correlation-client-canary")
            .unwrap();
        crate::ready_connection(&mut plane, &client, std::time::Instant::now());
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: None,
            allow_resumption: false,
            full_debug: false,
        };
        plane
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        let lease = plane
            .approve_lease_local(
                &client.client_id,
                &request,
                plane
                    .lease_requests()
                    .pending_generation(&client.client_id)
                    .unwrap_or(0),
                now,
            )
            .unwrap();
        let audit_client = plane.lease_requests.audit_client_id(&client.client_id);
        let metrics = plane.operation_metrics.clone();
        let control = std::sync::Arc::new(std::sync::Mutex::new(plane));
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime.block_on(async {
            for protected in [false, true] {
                let result = metrics
                    .measure(operation_metrics::Method::WindowAction, async {
                        let permit = DesktopPermit::new(
                            control.clone(),
                            client.client_id.clone(),
                            client.token.clone(),
                            lease,
                        );
                        std::thread::spawn(move || {
                            permit.with_resource(
                                &leases::ResourceEvidence {
                                    window: None,
                                    surface: None,
                                    verified_application: None,
                                    output: None,
                                    authorized_surface_ancestors: &[],
                                    protected,
                                },
                                || Ok(()),
                            )
                        })
                        .join()
                        .unwrap()
                    })
                    .await;
                assert_eq!(result.is_ok(), !protected);
            }
        });
        let snapshot = metrics.snapshot().unwrap();
        assert_eq!(snapshot.recent_completions.len(), 2);
        let authorization = snapshot.recent_completions[0].authorization.unwrap();
        assert_eq!(authorization.lease_id, lease);
        assert_eq!(authorization.client_id, audit_client);
        assert!(authorization.operation_id > 0);
        assert_eq!(authorization.lease_operation_generation, 0);
        assert!(snapshot.recent_completions[1].authorization.is_none());
        let json = serde_json::to_string(&snapshot).unwrap();
        assert!(!json.contains("private-correlation-client-canary"));
        assert!(!json.contains(&client.token));
        let public = metrics.exposition();
        for name in [
            "client_id",
            "lease_id",
            "operation_id",
            "lease_operation_generation",
        ] {
            assert!(!public.contains(name));
        }
    }

    #[test]
    fn local_duration_renewal_preserves_input_owner_and_rejects_stale_cards() {
        let now = std::time::Instant::now();
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let client = plane.connect_identity("Custom renewal fixture").unwrap();
        crate::ready_connection(&mut plane, &client, std::time::Instant::now());
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: Some(Duration::from_secs(30)),
            allow_resumption: false,
            full_debug: false,
        };
        plane
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        let id = plane
            .approve_lease_local(
                &client.client_id,
                &request,
                plane
                    .lease_requests()
                    .pending_generation(&client.client_id)
                    .unwrap_or(0),
                now,
            )
            .unwrap();
        plane.leases.reserve_input(id, 42).unwrap();
        let renewal = lease_requests::LeaseRequest {
            renewal: Some(nickel_session_protocol::RemoteLeaseRenewal {
                lease_id: id,
                generation: 0,
            }),
            duration: Some(Duration::from_secs(1200)),
            ..request
        };
        plane
            .request_lease(&client.client_id, &client.token, renewal.clone(), now)
            .unwrap();
        // A locally selected duration that does not extend the existing lease
        // fails without consuming its card or disturbing ongoing input.
        assert!(
            plane
                .approve_lease_with_duration_local(
                    &client.client_id,
                    &renewal,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    Some(Duration::from_secs(10)),
                    now
                )
                .is_err()
        );
        assert_eq!(plane.lease_requests.pending().count(), 1);
        assert!(plane.leases.owns_input(id, 42));
        assert_eq!(
            plane
                .approve_lease_with_duration_local(
                    &client.client_id,
                    &renewal,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    Some(Duration::from_secs(60)),
                    now
                )
                .unwrap(),
            id
        );
        assert!(plane.leases.owns_input(id, 42));
        let lease = plane
            .leases
            .active_lease(id, &client.client_id, now)
            .unwrap();
        assert_eq!(lease.expires_at, Some(now + Duration::from_secs(60)));
        assert_eq!(lease.renewal_generation, 1);
        assert_eq!(plane.leases.iter().count(), 1);
        assert!(
            plane
                .approve_lease_with_duration_local(
                    &client.client_id,
                    &renewal,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    None,
                    now
                )
                .is_err()
        );
        let current = lease_requests::LeaseRequest {
            renewal: Some(nickel_session_protocol::RemoteLeaseRenewal {
                lease_id: id,
                generation: 1,
            }),
            ..renewal.clone()
        };
        plane
            .request_lease(&client.client_id, &client.token, current.clone(), now)
            .unwrap();
        assert!(
            plane
                .approve_lease_with_duration_local(
                    &client.client_id,
                    &renewal,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    None,
                    now
                )
                .is_err()
        );
        assert_eq!(
            plane
                .approve_lease_with_duration_local(
                    &client.client_id,
                    &current,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    None,
                    now
                )
                .unwrap(),
            id
        );
        assert!(plane.leases.owns_input(id, 42));
        let lease = plane
            .leases
            .active_lease(id, &client.client_id, now)
            .unwrap();
        assert_eq!(lease.expires_at, None);
        assert_eq!(lease.renewal_generation, 2);
        assert_eq!(lease.scope, current.scope);
        assert!(!lease.full_debug);
        assert!(!lease.allow_resumption);
    }

    #[test]
    fn pending_renewal_retires_with_its_lease_without_denial_or_restoring_authority() {
        for state in ["expired", "paused", "revoked", "client_revoked"] {
            let now = std::time::Instant::now();
            let mut plane = ControlPlane::default();
            plane.set_enabled(true);
            let client = plane.connect_identity("Renewal fixture").unwrap();
            crate::ready_connection(&mut plane, &client, std::time::Instant::now());
            let request = lease_requests::LeaseRequest {
                renewal: None,
                scope: leases::ResourceScope::FullSession,
                duration: Some(Duration::from_secs(30)),
                allow_resumption: false,
                full_debug: false,
            };
            plane
                .request_lease(&client.client_id, &client.token, request.clone(), now)
                .unwrap();
            let id = plane
                .approve_lease_local(
                    &client.client_id,
                    &request,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    now,
                )
                .unwrap();
            let renewal = lease_requests::LeaseRequest {
                renewal: Some(nickel_session_protocol::RemoteLeaseRenewal {
                    lease_id: id,
                    generation: 0,
                }),
                duration: Some(Duration::from_secs(1200)),
                ..request.clone()
            };
            plane
                .request_lease(&client.client_id, &client.token, renewal.clone(), now)
                .unwrap();
            plane.reconcile_pending_lease_requests(now);
            assert_eq!(plane.lease_requests.pending().count(), 1);
            let at = match state {
                "expired" => now + Duration::from_secs(30),
                "paused" => {
                    plane.leases.suspend_local(id).unwrap();
                    now
                }
                "revoked" => {
                    plane.leases.revoke(id);
                    now
                }
                "client_revoked" => {
                    plane.revoke(&client.client_id);
                    now
                }
                _ => unreachable!(),
            };
            plane.reconcile_pending_lease_requests(at);
            assert_eq!(plane.lease_requests.pending().count(), 0, "{state}");
            assert!(
                plane
                    .approve_lease_local(
                        &client.client_id,
                        &renewal,
                        plane
                            .lease_requests()
                            .pending_generation(&client.client_id)
                            .unwrap_or(0),
                        at
                    )
                    .is_err()
            );
            let metrics = plane.lease_requests.metrics();
            plane.reconcile_pending_lease_requests(at);
            assert_eq!(
                plane.lease_requests.metrics(),
                metrics,
                "cancellation counted twice"
            );
            if state != "client_revoked" {
                assert!(
                    plane
                        .request_lease(&client.client_id, &client.token, request, at)
                        .unwrap(),
                    "cancellation imposed a denial cooldown"
                );
            }
        }
    }

    #[test]
    fn renewal_request_waits_for_local_approval_and_rejects_replay_and_policy_changes() {
        let now = std::time::Instant::now();
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let client = plane.connect_identity("Renewal fixture").unwrap();
        crate::ready_connection(&mut plane, &client, std::time::Instant::now());
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: Some(Duration::from_secs(1200)),
            allow_resumption: false,
            full_debug: false,
        };
        plane
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        let id = plane
            .approve_lease_local(
                &client.client_id,
                &request,
                plane
                    .lease_requests()
                    .pending_generation(&client.client_id)
                    .unwrap_or(0),
                now,
            )
            .unwrap();
        let original = plane
            .leases
            .active_lease(id, &client.client_id, now)
            .unwrap()
            .expires_at;
        let renewal = lease_requests::LeaseRequest {
            renewal: Some(nickel_session_protocol::RemoteLeaseRenewal {
                lease_id: id,
                generation: 0,
            }),
            ..request
        };
        assert!(matches!(
            plane.request_lease(&client.client_id, &client.token, renewal.clone(), now),
            Err(lease_requests::RequestError::Cooldown { .. })
        ));
        let near = now + Duration::from_secs(1000);
        for step in 1..=34 {
            let at = now + Duration::from_secs(step * 30);
            let old: Vec<_> = plane.connection_watches.keys().copied().collect();
            let id = plane
                .reserve_connection_watch(&client.client_id, &client.token, at)
                .unwrap();
            plane
                .activate_connection_watch(&client.client_id, &client.token, id, false, at)
                .unwrap();
            for previous in old {
                plane.close_connection_watch(previous, at);
            }
        }
        for changed in [
            lease_requests::LeaseRequest {
                full_debug: true,
                ..renewal.clone()
            },
            lease_requests::LeaseRequest {
                allow_resumption: true,
                ..renewal.clone()
            },
        ] {
            assert_eq!(
                plane.request_lease(&client.client_id, &client.token, changed, near),
                Err(lease_requests::RequestError::Invalid)
            );
        }
        assert!(
            plane
                .request_lease(&client.client_id, &client.token, renewal.clone(), near)
                .unwrap()
        );
        assert!(
            !plane
                .request_lease(&client.client_id, &client.token, renewal.clone(), near)
                .unwrap()
        );
        assert_eq!(
            plane
                .leases
                .active_lease(id, &client.client_id, near)
                .unwrap()
                .expires_at,
            original
        );
        assert_eq!(
            plane
                .approve_lease_local(
                    &client.client_id,
                    &renewal,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    near
                )
                .unwrap(),
            id
        );
        assert_eq!(
            plane
                .leases
                .active_lease(id, &client.client_id, near)
                .unwrap()
                .renewal_generation,
            1
        );
        assert!(
            plane
                .approve_lease_local(
                    &client.client_id,
                    &renewal,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    near
                )
                .is_err()
        );
        assert_eq!(
            plane.request_lease(&client.client_id, &client.token, renewal, near),
            Err(lease_requests::RequestError::Invalid)
        );
        assert_eq!(plane.leases.iter().count(), 1);
        use lease_requests::Outcome;
        let events: Vec<_> = plane.lease_requests.audit().collect();
        assert_eq!(
            events.iter().map(|event| event.outcome).collect::<Vec<_>>(),
            vec![
                Outcome::Submitted,
                Outcome::Approved,
                Outcome::Cooldown,
                Outcome::Invalid,
                Outcome::Invalid,
                Outcome::Submitted,
                Outcome::Coalesced,
                Outcome::Approved,
                Outcome::Invalid,
            ]
        );
        assert!(events[0].client_id.is_some());
        assert!(
            events
                .iter()
                .all(|event| event.client_id == events[0].client_id)
        );
        assert!(!format!("{events:?}").contains(&client.client_id));
    }

    #[test]
    fn local_client_block_prevents_first_request_and_unblock_never_restores_authority() {
        let now = std::time::Instant::now();
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let client = plane.connect_identity("fixture").unwrap();
        crate::ready_connection(&mut plane, &client, std::time::Instant::now());
        let other = plane.connect_identity("other fixture").unwrap();
        crate::ready_connection(&mut plane, &other, std::time::Instant::now());
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: Some(std::time::Duration::from_secs(1200)),
            allow_resumption: true,
            full_debug: false,
        };
        plane.block_client_local(&client.client_id, true).unwrap();
        assert_eq!(
            plane.request_lease(&client.client_id, &client.token, request.clone(), now),
            Err(lease_requests::RequestError::Blocked)
        );
        plane.block_client_local(&client.client_id, false).unwrap();
        crate::ready_connection(&mut plane, &client, now);
        for identity in [&client, &other] {
            plane
                .request_lease(&identity.client_id, &identity.token, request.clone(), now)
                .unwrap();
            plane
                .approve_lease_local(
                    &identity.client_id,
                    &request,
                    plane
                        .lease_requests()
                        .pending_generation(&identity.client_id)
                        .unwrap_or(0),
                    now,
                )
                .unwrap();
        }
        plane
            .request_lease(&client.client_id, &client.token, request.clone(), now)
            .unwrap();
        plane.block_client_local(&client.client_id, true).unwrap();
        assert_eq!(plane.lease_requests.pending().count(), 0);
        assert!(
            plane
                .leases
                .iter()
                .all(|lease| lease.client_identity == other.client_id)
        );
        assert_eq!(plane.leases.iter().count(), 1);
        assert!(
            plane
                .approve_lease_local(
                    &client.client_id,
                    &request,
                    plane
                        .lease_requests()
                        .pending_generation(&client.client_id)
                        .unwrap_or(0),
                    now
                )
                .is_err()
        );
        plane.block_client_local(&client.client_id, false).unwrap();
        crate::ready_connection(&mut plane, &client, now);
        assert_eq!(plane.leases.iter().count(), 1);
        assert!(
            plane
                .request_lease(&client.client_id, &client.token, request, now)
                .unwrap()
        );
        assert!(plane.block_client_local("unknown", true).is_err());
    }

    #[test]
    fn diagnostic_authority_requires_full_debug_and_never_crosses_protected_boundary() {
        let now = std::time::Instant::now();
        let control = std::sync::Arc::new(std::sync::Mutex::new(ControlPlane::default()));
        control.lock().unwrap().set_enabled(true);
        let identity = control
            .lock()
            .unwrap()
            .connect_identity("Debug agent")
            .unwrap();
        crate::ready_connection(
            &mut control.lock().unwrap(),
            &identity,
            std::time::Instant::now(),
        );
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: Some(std::time::Duration::from_secs(1800)),
            allow_resumption: false,
            full_debug: false,
        };
        control
            .lock()
            .unwrap()
            .request_lease(&identity.client_id, &identity.token, request.clone(), now)
            .unwrap();
        let ordinary = {
            let mut plane = control.lock().unwrap();
            let generation = plane
                .lease_requests()
                .pending_generation(&identity.client_id)
                .unwrap();
            plane.approve_lease_local(&identity.client_id, &request, generation, now)
        }
        .unwrap();
        let permit = DesktopPermit::new(
            control.clone(),
            identity.client_id.clone(),
            identity.token.clone(),
            ordinary,
        );
        assert!(
            permit
                .with_debug::<()>(false, || panic!("ordinary control read diagnostics"))
                .is_err()
        );
        assert!(
            permit
                .with_debug_input::<()>(false, || panic!("ordinary control changed settings"))
                .is_err()
        );
        let request = lease_requests::LeaseRequest {
            full_debug: true,
            ..request
        };
        control
            .lock()
            .unwrap()
            .request_lease(&identity.client_id, &identity.token, request.clone(), now)
            .unwrap();
        let debug = {
            let mut plane = control.lock().unwrap();
            let generation = plane
                .lease_requests()
                .pending_generation(&identity.client_id)
                .unwrap();
            plane.approve_lease_local(&identity.client_id, &request, generation, now)
        }
        .unwrap();
        let permit = DesktopPermit::new(control.clone(), identity.client_id, identity.token, debug);
        assert_eq!(permit.with_debug(false, || Ok(42)), Ok(42));
        assert!(
            permit
                .with_debug_input::<()>(true, || panic!("protected mutation executed"))
                .is_err()
        );
        control
            .lock()
            .unwrap()
            .leases_mut()
            .reserve_input(debug, 0)
            .unwrap();
        assert!(
            permit
                .with_debug_input::<()>(false, || panic!("held input disrupted"))
                .is_err()
        );
        control.lock().unwrap().leases_mut().release_input(debug, 0);
        assert!(
            permit
                .with_debug_input::<()>(false, || Err("rejected transaction".into()))
                .is_err()
        );
        // Both failure and success must release the temporary reservation.
        control
            .lock()
            .unwrap()
            .leases_mut()
            .reserve_input(debug, 0)
            .unwrap();
        control.lock().unwrap().leases_mut().release_input(debug, 0);
        assert_eq!(permit.with_debug_input(false, || Ok(17)), Ok(17));
        permit
            .with_debug_input_deadline(false, |deadline| {
                assert_eq!(deadline.deadline(), permit.deadline);
                Ok(())
            })
            .unwrap();
        let lifetime = std::sync::Arc::new(operation_metrics::RequestLifetime::default());
        let cancellation = tokio_util::sync::CancellationToken::new();
        lifetime.attach(cancellation.clone());
        let mut cancelled_during_staging = permit.clone();
        cancelled_during_staging.request_lifetime = Some(lifetime);
        assert!(
            cancelled_during_staging
                .with_debug_input_deadline(false, |deadline| {
                    // Simulate transport loss while the authority mutex stays held.
                    cancellation.cancel();
                    assert!(
                        cancelled_during_staging
                            .check_commit_boundary(deadline)
                            .is_err()
                    );
                    // Callers cannot accidentally publish a successful result after
                    // losing request lifetime inside an already accepted effect.
                    Ok(())
                })
                .is_err()
        );
        permit
            .with_debug_input_deadline(false, |deadline| {
                permit.check_commit_boundary(deadline)?;
                assert!(
                    permit
                        .check_commit_boundary(&CommitBoundary {
                            deadline: std::time::Instant::now(),
                            watches: deadline.watches.clone()
                        })
                        .is_err()
                );
                Ok(())
            })
            .unwrap();
        let watch_deadline = control
            .lock()
            .unwrap()
            .ready_connection_deadline(&permit.client, std::time::Instant::now())
            .unwrap();
        let mut long_request = permit.clone();
        long_request.deadline = now + std::time::Duration::from_secs(7200);
        long_request
            .with_debug_input_deadline(false, |deadline| {
                assert_eq!(
                    deadline.deadline(),
                    (now + std::time::Duration::from_secs(1800)).min(watch_deadline)
                );
                Ok(())
            })
            .unwrap();
        let mut expired = permit.clone();
        expired.deadline = std::time::Instant::now();
        assert!(
            expired
                .with_debug_input::<()>(false, || panic!("expired mutation executed"))
                .is_err()
        );
        control
            .lock()
            .unwrap()
            .leases_mut()
            .reserve_input(debug, 0)
            .unwrap();
        control.lock().unwrap().leases_mut().release_input(debug, 0);
        assert!(
            permit
                .with_debug::<()>(true, || panic!("protected diagnostics exposed"))
                .is_err()
        );
        // A standing observation can refresh delivery time, but a pause/resume
        // cycle permanently cancels its original generation even between polls.
        assert_eq!(
            expired
                .continued_observation()
                .unwrap()
                .with_debug(false, || Ok(23)),
            Ok(23)
        );
        assert!(expired.check_standing_live().is_ok());
        assert!(expired.same_lease_as(&permit));
        {
            let _held = control.lock().unwrap();
            let started = std::time::Instant::now();
            assert!(permit.check_standing_live().is_err());
            assert!(started.elapsed() < std::time::Duration::from_millis(50));
        }
        control
            .lock()
            .unwrap()
            .leases_mut()
            .suspend_local(debug)
            .unwrap();
        assert!(permit.continued_observation().is_err());
        assert!(permit.check_standing_live().is_err());
        control
            .lock()
            .unwrap()
            .leases_mut()
            .resume_local(debug, now)
            .unwrap();
        assert!(permit.continued_observation().is_err());
        let fresh = DesktopPermit::new(
            control.clone(),
            permit.client.clone(),
            permit.token.clone(),
            debug,
        );
        assert_eq!(
            fresh
                .continued_observation()
                .unwrap()
                .with_debug(false, || Ok(29)),
            Ok(29)
        );
        // The last admitted ready-watch deadline is part of every staged commit
        // deadline, even while transport cleanup cannot acquire the owner lock.
        let watch_deadline = std::time::Instant::now() + std::time::Duration::from_millis(100);
        for watch in control.lock().unwrap().connection_watches.values_mut() {
            watch.expires_at = watch_deadline;
        }
        let mut executed = false;
        assert!(
            fresh
                .with_debug_input_deadline(false, |deadline| {
                    executed = true;
                    assert_eq!(deadline.deadline(), watch_deadline);
                    std::thread::sleep(
                        deadline
                            .deadline()
                            .saturating_duration_since(std::time::Instant::now())
                            + std::time::Duration::from_millis(1),
                    );
                    Ok(31)
                })
                .is_err()
        );
        assert!(executed, "test must expire during an accepted effect");
        control
            .lock()
            .unwrap()
            .leases_mut()
            .reserve_input(debug, 0)
            .unwrap();
        control.lock().unwrap().leases_mut().release_input(debug, 0);
        control.lock().unwrap().leases_mut().revoke(debug);
        assert!(
            permit
                .with_debug_input::<()>(false, || panic!("revoked mutation executed"))
                .is_err()
        );
        assert!(
            permit
                .with_debug::<()>(false, || panic!("revoked diagnostics executed"))
                .is_err()
        );
    }

    #[test]
    fn lease_request_needs_identity_and_exact_local_approval_then_lock_cancels_it() {
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let display = plane.start_pairing(1).unwrap();
        let pending = plane
            .exchange_short_code(
                &display.ceremony_id,
                &display.short_code,
                "Development agent",
                vec![],
                2,
            )
            .unwrap();
        let now = std::time::Instant::now();
        let request = lease_requests::LeaseRequest {
            renewal: None,
            scope: leases::ResourceScope::FullSession,
            duration: Some(std::time::Duration::from_secs(1200)),
            allow_resumption: true,
            full_debug: false,
        };
        assert!(
            plane
                .request_lease(&pending.id, "invalid", request.clone(), now)
                .is_err()
        );
        let identity = plane
            .approve(&pending.id, Approval::AllowOnce, vec![])
            .unwrap()
            .unwrap();
        crate::ready_connection(&mut plane, &identity, now);
        assert!(
            plane
                .request_lease(&pending.id, &identity.token, request.clone(), now)
                .unwrap()
        );
        assert_eq!(plane.leases().iter().count(), 0);
        let changed = lease_requests::LeaseRequest {
            full_debug: true,
            ..request.clone()
        };
        assert!(
            plane
                .approve_lease_local(
                    &pending.id,
                    &changed,
                    plane
                        .lease_requests()
                        .pending_generation(&pending.id)
                        .unwrap_or(0),
                    now
                )
                .is_err()
        );
        let id = plane
            .approve_lease_local(
                &pending.id,
                &request,
                plane
                    .lease_requests()
                    .pending_generation(&pending.id)
                    .unwrap_or(0),
                now,
            )
            .unwrap();
        assert_eq!(plane.lease_requests().pending().count(), 0);
        assert_eq!(plane.leases().iter().count(), 1);
        let control = std::sync::Arc::new(std::sync::Mutex::new(plane));
        let permit = DesktopPermit::new(
            control.clone(),
            pending.id.clone(),
            identity.token.clone(),
            id,
        );
        let evidence = leases::ResourceEvidence {
            window: None,
            surface: None,
            verified_application: None,
            output: None,
            authorized_surface_ancestors: &[],
            protected: false,
        };
        assert_eq!(permit.with_resource(&evidence, || Ok(42)), Ok(42));
        let capture_delivery_permit = permit.clone();
        control
            .lock()
            .unwrap()
            .leases_mut()
            .suspend_local(id)
            .unwrap();
        control
            .lock()
            .unwrap()
            .leases_mut()
            .resume_local(id, now)
            .unwrap();
        assert!(
            permit
                .with_resource::<()>(&evidence, || panic!("cancelled work executed after resume"))
                .is_err()
        );
        assert!(
            capture_delivery_permit
                .with_resource::<()>(&evidence, || {
                    panic!("capture collected before pause was released after resume")
                })
                .is_err()
        );
        let permit = DesktopPermit::new(control.clone(), pending.id.clone(), identity.token, id);
        assert_eq!(permit.with_resource(&evidence, || Ok(42)), Ok(42));
        control.lock().unwrap().lock();
        let mut executed = false;
        assert!(
            permit
                .with_resource(&evidence, || {
                    executed = true;
                    Ok(())
                })
                .is_err()
        );
        assert!(!executed);
        let mut plane = control.lock().unwrap();
        assert_eq!(plane.leases().iter().count(), 0);
        assert!(plane.leases_mut().take_cancellations().contains(&id));
    }

    #[test]
    fn defaults_disabled_and_pairing_never_grants_before_local_approval() {
        let mut plane = ControlPlane::default();
        assert_eq!(plane.start_pairing(10), Err(ControlError::Disabled));
        plane.set_enabled(true);
        let display = plane.start_pairing(10).unwrap();
        let secret = display.qr_payload.split("secret=").nth(1).unwrap();
        let pending = plane
            .exchange_qr_secret(
                &display.ceremony_id,
                secret,
                "Phone",
                vec![Capability::Observe, Capability::PointerInput],
                11,
            )
            .unwrap();
        assert!(!plane.authorize(&pending.id, secret, Capability::Observe));
        let issued = plane
            .approve(&pending.id, Approval::AllowOnce, vec![Capability::Observe])
            .unwrap()
            .unwrap();
        assert!(plane.authorize(&pending.id, &issued.token, Capability::Observe));
        assert!(!plane.authorize(&pending.id, &issued.token, Capability::PointerInput));
    }

    #[test]
    fn short_code_is_single_use_expiring_and_attempt_bounded() {
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let display = plane.start_pairing(20).unwrap();
        plane
            .exchange_short_code(
                &display.ceremony_id,
                &display.short_code.to_ascii_lowercase(),
                "Phone",
                vec![Capability::Observe],
                21,
            )
            .unwrap();
        assert_eq!(
            plane.exchange_short_code(
                &display.ceremony_id,
                &display.short_code,
                "Replay",
                vec![],
                22
            ),
            Err(ControlError::PairingExpired)
        );

        let expired = plane.start_pairing(100).unwrap();
        assert_eq!(
            plane.exchange_short_code(
                &expired.ceremony_id,
                &expired.short_code,
                "Late",
                vec![],
                100 + PAIRING_LIFETIME_SECS + 1,
            ),
            Err(ControlError::PairingExpired)
        );

        let limited = plane.start_pairing(200).unwrap();
        for _ in 0..MAX_PAIRING_ATTEMPTS - 1 {
            assert_eq!(
                plane.exchange_short_code(&limited.ceremony_id, "NOPE-NOPE", "Bad", vec![], 201),
                Err(ControlError::PairingRejected)
            );
        }
        assert_eq!(
            plane.exchange_short_code(&limited.ceremony_id, "NOPE-NOPE", "Bad", vec![], 201),
            Err(ControlError::AttemptLimit)
        );
    }

    #[test]
    fn pairing_attempt_budget_survives_challenge_rotation() {
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        for index in 0..GLOBAL_PAIRING_ATTEMPTS {
            let display = plane.start_pairing(100 + index as u64).unwrap();
            assert_eq!(
                plane.exchange_short_code(
                    &display.ceremony_id,
                    "NOPE-NOPE",
                    "Guess",
                    vec![],
                    100 + index as u64,
                ),
                Err(ControlError::PairingRejected)
            );
        }
        let blocked = plane.start_pairing(125).unwrap();
        assert_eq!(
            plane.exchange_short_code(
                &blocked.ceremony_id,
                &blocked.short_code,
                "Phone",
                vec![],
                125,
            ),
            Err(ControlError::RateLimited)
        );

        let recovered = plane.start_pairing(200).unwrap();
        assert!(
            plane
                .exchange_short_code(
                    &recovered.ceremony_id,
                    &recovered.short_code,
                    "Phone",
                    vec![],
                    200,
                )
                .is_ok()
        );
    }

    #[test]
    fn lock_disable_and_emergency_stop_revoke_every_transient_authority() {
        for action in 0..3 {
            let mut plane = ControlPlane::default();
            plane.set_enabled(true);
            let display = plane.start_pairing(1).unwrap();
            let pending = plane
                .exchange_short_code(
                    &display.ceremony_id,
                    &display.short_code,
                    "Phone",
                    vec![Capability::KeyboardInput],
                    2,
                )
                .unwrap();
            let issued = plane
                .approve(
                    &pending.id,
                    Approval::AllowOnce,
                    vec![Capability::KeyboardInput],
                )
                .unwrap()
                .unwrap();
            match action {
                0 => plane.lock(),
                1 => plane.set_enabled(false),
                _ => plane.emergency_stop(),
            }
            assert!(!plane.authorize(&pending.id, &issued.token, Capability::KeyboardInput));
            assert_eq!(plane.pending_clients().count(), 0);
        }
    }

    #[test]
    fn remember_is_rejected_until_a_secure_persistent_store_exists() {
        let mut plane = ControlPlane::default();
        plane.set_enabled(true);
        let display = plane.start_pairing(1).unwrap();
        let pending = plane
            .exchange_short_code(
                &display.ceremony_id,
                &display.short_code,
                "Phone",
                vec![Capability::Observe],
                2,
            )
            .unwrap();
        assert!(matches!(
            plane.approve(&pending.id, Approval::Remember, vec![Capability::Observe]),
            Err(ControlError::PersistentApprovalUnavailable)
        ));
        assert_eq!(plane.pending_clients().count(), 1);
        assert_eq!(plane.granted_clients().count(), 0);
    }

    #[test]
    fn emergency_chord_requires_both_physical_sides_and_fires_once_per_hold() {
        use nickel_input::{KeyCode, KeyEdge};
        let mut chord = EmergencyChord::default();
        assert!(!chord.handle_physical(KeyCode::ControlLeft, KeyEdge::Pressed, false));
        assert!(!chord.handle_physical(KeyCode::ControlLeft, KeyEdge::Pressed, true));
        assert!(chord.handle_physical(KeyCode::ControlRight, KeyEdge::Pressed, true));
        assert!(!chord.handle_physical(KeyCode::ControlRight, KeyEdge::Pressed, true));
        assert!(!chord.handle_physical(KeyCode::ControlLeft, KeyEdge::Released, true));
        assert!(chord.handle_physical(KeyCode::ControlLeft, KeyEdge::Pressed, true));
    }
}
