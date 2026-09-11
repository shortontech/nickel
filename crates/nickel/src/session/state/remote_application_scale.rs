//! Bounded worker preparation with per-step native owner acceptance. Accepted
//! external setters can finish late; revocation cannot start subsequent steps.
use super::{
    NickelSession, RemoteDesktopBridge, RemoteDesktopRequest,
    remote_settings::{FileRevision, revision},
};
use nickel_core::dpi::{ApplicationScalePolicy, ApplicationScaleSettings, Scale120};
use nickel_platform::{
    LinuxToolkitScaleBackend, PreparedToolkitCommand, RunningToolkitCommand, ToolkitFamily,
    ToolkitScaleBackend,
};
use nickel_platform::{ToolkitRejection, ToolkitWriteError};
use nickel_remote_control::{DesktopPermit, application_scale as api};
use std::{
    cell::RefCell,
    path::PathBuf,
    sync::mpsc::SyncSender,
    time::{Duration, Instant},
};
const MAX_OBSERVATION_AGE: Duration = Duration::from_secs(1);
const STALE: &str = "application scale changed; read current state before retrying";
const UNAVAILABLE: &str =
    "application scale unavailable or uncertain; read current state before retrying";
fn policy(value: ApplicationScalePolicy) -> api::Policy {
    match value {
        ApplicationScalePolicy::FollowNickel => api::Policy::FollowNickel,
        ApplicationScalePolicy::Unchanged => api::Policy::Unchanged,
        ApplicationScalePolicy::Custom(scale) => api::Policy::Custom {
            scale_120: scale.units(),
        },
    }
}
fn requested(value: api::Policy) -> Result<ApplicationScalePolicy, String> {
    Ok(match value {
        api::Policy::FollowNickel => ApplicationScalePolicy::FollowNickel,
        api::Policy::Unchanged => ApplicationScalePolicy::Unchanged,
        api::Policy::Custom { scale_120 } => {
            if !(60..=480).contains(&scale_120) || scale_120 % 30 != 0 {
                return Err("unsupported application scale".into());
            }
            ApplicationScalePolicy::Custom(Scale120::new(scale_120).unwrap())
        }
    })
}
fn family(value: ToolkitFamily) -> api::Family {
    match value {
        ToolkitFamily::Gtk => api::Family::Gtk,
        ToolkitFamily::Qt => api::Family::Qt,
    }
}
pub(super) struct Observation {
    started_at: Instant,
    completed_at: Instant,
    path: PathBuf,
    revision: Option<FileRevision>,
    settings: ApplicationScaleSettings,
    toolkits: Vec<api::Toolkit>,
}
impl Observation {
    fn prepare(path: PathBuf, backend: &dyn ToolkitScaleBackend) -> Result<Self, String> {
        let started_at = Instant::now();
        let before = revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = ApplicationScaleSettings::load(&path).map_err(|_| UNAVAILABLE)?;
        let toolkits = backend
            .capabilities()
            .into_iter()
            .take(2)
            .map(|cap| {
                let (owned, pending) = match cap.family {
                    ToolkitFamily::Gtk => (
                        settings.owned_gtk_applied.is_some(),
                        settings.pending_gtk.is_some(),
                    ),
                    ToolkitFamily::Qt => (
                        settings.owned_qt_applied.is_some(),
                        settings.pending_qt.is_some(),
                    ),
                };
                api::Toolkit {
                    family: family(cap.family),
                    available: cap.available,
                    observed: if cap.available {
                        backend.read(cap.family).ok()
                    } else {
                        None
                    },
                    owned,
                    pending,
                    restart_required: cap.restart_required,
                }
            })
            .collect();
        if revision(&path).map_err(|_| UNAVAILABLE)? != before {
            return Err(STALE.into());
        }
        Ok(Self {
            started_at,
            completed_at: Instant::now(),
            path,
            revision: before,
            settings,
            toolkits,
        })
    }
    fn ensure_fresh(&self, now: Instant) -> Result<(), String> {
        if self.completed_at < self.started_at
            || self.completed_at > now
            || now
                .checked_duration_since(self.started_at)
                .is_none_or(|age| age > MAX_OBSERVATION_AGE)
        {
            return Err(
                "application scale observation is stale; read current state before retrying".into(),
            );
        }
        Ok(())
    }
    fn interval(&self, session_started: Instant, now: Instant) -> Result<(u64, u64), String> {
        self.ensure_fresh(now)?;
        let micros = |at: Instant| {
            at.checked_duration_since(session_started)
                .and_then(|elapsed| u64::try_from(elapsed.as_micros()).ok())
                .ok_or_else(|| "invalid application scale observation clock".to_owned())
        };
        Ok((micros(self.started_at)?, micros(self.completed_at)?))
    }
}
#[derive(Default)]
pub(super) struct ScaleState {
    generation: u64,
    observed: Option<(
        Option<FileRevision>,
        ApplicationScaleSettings,
        Vec<api::Toolkit>,
    )>,
}
impl ScaleState {
    fn observe(
        &mut self,
        value: Observation,
        session_started: Instant,
        delivered_at: Instant,
    ) -> Result<api::Snapshot, String> {
        let (observation_started_at_us, observed_at_us) =
            value.interval(session_started, delivered_at)?;
        if self
            .observed
            .as_ref()
            .is_none_or(|(rev, settings, toolkits)| {
                *rev != value.revision || *settings != value.settings || *toolkits != value.toolkits
            })
        {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("scale generation exhausted")?;
        }
        let snapshot = api::Snapshot {
            generation: self.generation,
            observation_started_at_us,
            observed_at_us,
            atomic: false,
            policy: policy(value.settings.policy),
            toolkits: value.toolkits.clone(),
        };
        self.observed = Some((value.revision, value.settings, value.toolkits));
        Ok(snapshot)
    }
    fn validate(
        &self,
        value: &Observation,
        transaction: &api::Transaction,
        now: Instant,
    ) -> Result<(), String> {
        value.ensure_fresh(now)?;
        if transaction.generation == 0
            || self.generation == u64::MAX
            || transaction.generation != self.generation
            || transaction.prior != policy(value.settings.policy)
            || self
                .observed
                .as_ref()
                .is_none_or(|(rev, settings, toolkits)| {
                    *rev != value.revision
                        || *settings != value.settings
                        || *toolkits != value.toolkits
                })
        {
            return Err(STALE.into());
        }
        Ok(())
    }
}
pub(super) enum Request {
    Observe {
        observation: Observation,
        reply: SyncSender<Result<api::Snapshot, String>>,
    },
    Begin {
        observation: Observation,
        transaction: api::Transaction,
        reply: SyncSender<Result<(), String>>,
    },
    Persist {
        path: PathBuf,
        revision: Option<FileRevision>,
        staged: nickel_storage::StagedDurableWrite,
        reply: SyncSender<
            Result<(Option<FileRevision>, nickel_storage::DirectorySyncReceipt), String>,
        >,
    },
    Write {
        path: PathBuf,
        revision: Option<FileRevision>,
        prepared: nickel_platform::PreparedToolkitWrite,
        reply: SyncSender<Result<nickel_platform::ToolkitWriteCompletion, ToolkitWriteError>>,
    },
    Start {
        path: PathBuf,
        revision: Option<FileRevision>,
        command: PreparedToolkitCommand,
        mutating: bool,
        reply: SyncSender<Result<RunningToolkitCommand, String>>,
    },
}
struct Backend<'a> {
    bridge: &'a RemoteDesktopBridge,
    permit: DesktopPermit,
    backend: LinuxToolkitScaleBackend,
    path: PathBuf,
    _transaction_lock: nickel_storage::TransactionLock,
    revision: RefCell<Option<FileRevision>>,
}
impl Backend<'_> {
    fn send(&self, request: Request) -> Result<(), String> {
        self.permit.with_debug(false, || Ok(()))?;
        self.bridge
            .sender
            .try_send(RemoteDesktopRequest::ApplicationScale {
                permit: self.permit.clone(),
                request,
            })
            .map_err(|_| "application scale owner queue is busy or stopped".into())
    }
    fn persist(&self, settings: &ApplicationScaleSettings) -> Result<(), String> {
        self.permit.with_debug(false, || Ok(()))?;
        let staged = settings.stage(&self.path).map_err(|_| UNAVAILABLE)?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.send(Request::Persist {
            path: self.path.clone(),
            revision: self.revision.borrow().clone(),
            staged,
            reply,
        })?;
        let (revision, receipt) = response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| UNAVAILABLE)??;
        receipt.sync().map_err(|_| UNAVAILABLE)?;
        self.permit.check_live()?;
        if ApplicationScaleSettings::load(&self.path).map_err(|_| UNAVAILABLE)? != *settings {
            return Err(STALE.into());
        }
        *self.revision.borrow_mut() = revision;
        Ok(())
    }
    fn observation(&self) -> Result<Observation, String> {
        self.permit.with_debug(false, || Ok(()))?;
        let value = Observation::prepare(self.path.clone(), self)?;
        self.permit.with_debug(false, || Ok(()))?;
        Ok(value)
    }
    fn observe(&self) -> Result<api::Snapshot, String> {
        let observation = self.observation()?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.send(Request::Observe { observation, reply })?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| UNAVAILABLE)?
    }
}
impl ToolkitScaleBackend for Backend<'_> {
    fn capabilities(&self) -> Vec<nickel_platform::ToolkitCapability> {
        self.backend.capabilities()
    }
    fn read(&self, family: ToolkitFamily) -> Result<String, String> {
        if family == ToolkitFamily::Qt {
            self.permit.check_live()?;
            let result = nickel_platform::read_qt_scale();
            self.permit.check_live()?;
            return result;
        }
        let result = self.run(self.backend.prepare_read(family)?, false)?;
        nickel_platform::canonical_toolkit_value(family, &result)
    }
    fn writable(&self, family: ToolkitFamily) -> Result<bool, String> {
        Ok(family != ToolkitFamily::Gtk
            || self.run(self.backend.prepare_writable()?, false)? == "true")
    }
    fn write(&self, family: ToolkitFamily, value: &str) -> Result<(), String> {
        let expected = self.read(family)?;
        self.write_checked(family, value, &expected)
            .map_err(|error| format!("{error:?}"))
    }
    fn write_checked(
        &self,
        family: ToolkitFamily,
        value: &str,
        expected: &str,
    ) -> Result<(), ToolkitWriteError> {
        let prepared = nickel_platform::PreparedToolkitWrite::prepare(family, value, expected)?;
        prepared.check_observed(&self.read(family).map_err(ToolkitWriteError::not_accepted)?)?;
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.send(Request::Write {
            path: self.path.clone(),
            revision: self.revision.borrow().clone(),
            prepared,
            reply,
        })
        .map_err(ToolkitWriteError::not_accepted)?;
        receive_write_receipt(&response, Duration::from_secs(2))?.wait(|| self.permit.check_live())
    }
}
impl Backend<'_> {
    fn run(&self, command: PreparedToolkitCommand, mutating: bool) -> Result<String, String> {
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        self.send(Request::Start {
            path: self.path.clone(),
            revision: self.revision.borrow().clone(),
            command,
            mutating,
            reply,
        })?;
        response
            .recv_timeout(Duration::from_secs(2))
            .map_err(|_| UNAVAILABLE)??
            .wait(|| self.permit.check_live())
    }
}

fn receive_write_receipt<T>(
    response: &std::sync::mpsc::Receiver<Result<T, ToolkitWriteError>>,
    timeout: Duration,
) -> Result<T, ToolkitWriteError> {
    response
        .recv_timeout(timeout)
        .map_err(|_| ToolkitWriteError::Uncertain)?
}

fn backend(bridge: &RemoteDesktopBridge, permit: DesktopPermit) -> Result<Backend<'_>, String> {
    let path = nickel_storage::config_path("application-scale.conf").map_err(|_| UNAVAILABLE)?;
    Ok(Backend {
        bridge,
        permit,
        backend: LinuxToolkitScaleBackend::detect(),
        _transaction_lock: nickel_storage::TransactionLock::try_acquire(&path)
            .map_err(|_| "application scale transaction busy")?,
        revision: RefCell::new(revision(&path).map_err(|_| UNAVAILABLE)?),
        path,
    })
}
pub(super) fn read(
    bridge: &RemoteDesktopBridge,
    permit: DesktopPermit,
) -> Result<api::Snapshot, String> {
    let _admission = bridge.settings_staging.acquire()?;
    backend(bridge, permit)?.observe()
}
pub(super) fn transact(
    bridge: &RemoteDesktopBridge,
    permit: DesktopPermit,
    transaction: api::Transaction,
) -> Result<api::TransactionOutcome, String> {
    let _admission = bridge.settings_staging.acquire()?;
    let requested = requested(transaction.requested)?;
    let backend = backend(bridge, permit)?;
    let observation = backend.observation()?;
    let mut settings = observation.settings.clone();
    *backend.revision.borrow_mut() = observation.revision.clone();
    let (reply, response) = std::sync::mpsc::sync_channel(1);
    backend.send(Request::Begin {
        observation,
        transaction,
        reply,
    })?;
    response
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| UNAVAILABLE)??;
    let report = nickel_platform::transact_application_scale(
        &backend,
        &mut settings,
        requested,
        |settings| backend.persist(settings),
        || backend.permit.check_live(),
    )?;
    let snapshot = backend.observe()?;
    let outcomes = report
        .outcomes
        .into_iter()
        .map(|outcome| {
            use nickel_platform::ToolkitOutcomeKind as K;
            api::Outcome {
                family: family(outcome.family),
                kind: match outcome.kind {
                    K::Unchanged => api::OutcomeKind::Unchanged,
                    K::Confirmed => api::OutcomeKind::Confirmed,
                    K::ExternalConflict => api::OutcomeKind::ExternalConflict,
                    K::Unavailable => api::OutcomeKind::Unavailable,
                    K::Failed => api::OutcomeKind::Failed,
                    K::Uncertain => api::OutcomeKind::Uncertain,
                },
                restart_required: outcome.restart_required,
            }
        })
        .collect();
    Ok(api::TransactionOutcome { snapshot, outcomes })
}
impl NickelSession {
    fn scale_input_busy(&mut self) -> bool {
        self.poll_remote_controller_ownership()
            || self.remote_held_keyboard.is_some()
            || self.remote_held_pointer.is_some()
            || !self.active_touch_slots.is_empty()
            || self.internal_ui.pointer_interaction_active()
            || self.internal_ui.desktop_keyboard_interaction_active()
            || self.seat.get_keyboard().is_some_and(|keyboard| {
                !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
            })
            || self
                .seat
                .get_pointer()
                .is_some_and(|pointer| pointer.is_grabbed())
            || self
                .internal_shell
                .as_ref()
                .is_some_and(|shell| shell.pointer_interaction_active())
    }
    pub(super) fn remote_application_scale(&mut self, permit: DesktopPermit, request: Request) {
        let protected = self.locked
            || self.shell_recovery_visible()
            || self
                .internal_ui
                .focused()
                .is_some_and(|surface| self.internal_ui.remote_access_protected(surface));
        match request {
            Request::Observe { observation, reply } => {
                let result = permit.with_debug(protected, || {
                    if revision(&observation.path).map_err(|_| UNAVAILABLE)? != observation.revision
                    {
                        return Err(STALE.into());
                    }
                    self.remote_application_scale.observe(
                        observation,
                        self.start_time,
                        Instant::now(),
                    )
                });
                let _ = reply.send(result);
            }
            Request::Begin {
                observation,
                transaction,
                reply,
            } => {
                let busy = self.scale_input_busy();
                let result = permit.with_debug_input_deadline(protected, |deadline| {
                    if busy {
                        return Err("shared input is busy".into());
                    }
                    if revision(&observation.path).map_err(|_| UNAVAILABLE)? != observation.revision
                    {
                        return Err(STALE.into());
                    }
                    permit.check_commit_boundary(deadline)?;
                    self.remote_application_scale.validate(
                        &observation,
                        &transaction,
                        Instant::now(),
                    )
                });
                let _ = reply.send(result);
            }
            Request::Persist {
                path,
                revision: previous,
                staged,
                reply,
            } => {
                let busy = self.scale_input_busy();
                let result = permit.with_debug_input_deadline(protected, |deadline| {
                    if busy {
                        return Err("shared input is busy".into());
                    }
                    let receipt = staged
                        .commit(|| {
                            if revision(&path)? != previous {
                                return Err(std::io::Error::other(STALE));
                            }
                            permit
                                .check_commit_boundary(deadline)
                                .map_err(std::io::Error::other)
                        })
                        .map_err(|_| UNAVAILABLE)?;
                    self.remote_application_scale.observed = None;
                    Ok((revision(&path).map_err(|_| UNAVAILABLE)?, receipt))
                });
                self.record_remote_settings_transaction(&permit, &result);
                let _ = reply.send(result);
            }
            Request::Write {
                path,
                revision: previous,
                prepared,
                reply,
            } => {
                let busy = self.scale_input_busy();
                let mut attempted = None;
                let authorized = permit.with_debug_input_deadline(protected, |boundary| {
                    if busy {
                        return Err("shared input is busy".into());
                    }
                    if revision(&path).map_err(|_| UNAVAILABLE)? != previous {
                        return Err(STALE.into());
                    }
                    attempted = Some(prepared.accept(|| permit.check_commit_boundary(boundary)));
                    Ok(())
                });
                let result = match (authorized, attempted) {
                    (Ok(()), Some(result)) => result,
                    (Err(_), Some(Ok(_))) => Err(ToolkitWriteError::Uncertain),
                    (_, Some(Err(error))) => Err(error),
                    _ => Err(ToolkitWriteError::NotAccepted(ToolkitRejection::Failed)),
                };
                let _ = reply.send(result);
            }
            Request::Start {
                path,
                revision: previous,
                command,
                mutating,
                reply,
            } => {
                let busy = self.scale_input_busy();
                let child_capacity = self.remote_launched_children.len() < 32;
                let mut started = None;
                let authorized = permit.with_debug_input_deadline(protected, |deadline| {
                    if mutating && busy {
                        return Err("shared input is busy".into());
                    }
                    if revision(&path).map_err(|_| UNAVAILABLE)? != previous {
                        return Err(STALE.into());
                    }
                    permit.check_commit_boundary(deadline)?;
                    if !child_capacity {
                        return Err("remote child reaper is full".into());
                    }
                    started = Some(command.spawn()?);
                    Ok(())
                });
                if authorized.is_err()
                    && let Some(command) = started.take()
                {
                    self.remote_launched_children
                        .push(command.abort_into_child());
                }
                let result = authorized.and_then(|()| started.ok_or_else(|| UNAVAILABLE.into()));
                if let Err(error) = reply.send(result)
                    && let Ok(command) = error.0
                {
                    self.remote_launched_children
                        .push(command.abort_into_child());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn observation(started_at: Instant, completed_at: Instant) -> Observation {
        Observation {
            started_at,
            completed_at,
            path: PathBuf::new(),
            revision: None,
            settings: ApplicationScaleSettings::default(),
            toolkits: Vec::new(),
        }
    }
    #[test]
    fn owner_delivery_preserves_worker_interval_and_non_atomic_semantics() {
        let session = Instant::now();
        let mut state = ScaleState::default();
        let snapshot = state
            .observe(
                observation(
                    session + Duration::from_millis(100),
                    session + Duration::from_millis(200),
                ),
                session,
                session + Duration::from_millis(800),
            )
            .unwrap();
        assert_eq!(snapshot.observation_started_at_us, 100_000);
        assert_eq!(snapshot.observed_at_us, 200_000);
        assert!(!snapshot.atomic);
    }
    #[test]
    fn stale_worker_reads_cannot_advance_generation_or_enter_transaction() {
        let session = Instant::now();
        let mut state = ScaleState::default();
        let stale = || observation(session, session + Duration::from_millis(100));
        assert!(
            state
                .observe(stale(), session, session + Duration::from_millis(1001))
                .is_err()
        );
        assert_eq!(state.generation, 0);
        assert!(state.observed.is_none());
        let snapshot = state
            .observe(stale(), session, session + Duration::from_millis(200))
            .unwrap();
        let transaction = api::Transaction {
            generation: snapshot.generation,
            prior: snapshot.policy,
            requested: api::Policy::Unchanged,
        };
        assert!(
            state
                .validate(
                    &stale(),
                    &transaction,
                    session + Duration::from_millis(1001)
                )
                .is_err()
        );
        assert!(
            observation(session + Duration::from_millis(1), session)
                .ensure_fresh(session)
                .is_err()
        );
        assert!(
            observation(session, session + Duration::from_millis(1))
                .ensure_fresh(session)
                .is_err()
        );
    }
    #[test]
    fn worker_interval_brackets_actual_production_backend_read() {
        struct Backend(std::cell::Cell<Option<Instant>>);
        impl ToolkitScaleBackend for Backend {
            fn capabilities(&self) -> Vec<nickel_platform::ToolkitCapability> {
                vec![nickel_platform::ToolkitCapability {
                    family: ToolkitFamily::Qt,
                    available: true,
                    live: false,
                    restart_required: true,
                }]
            }
            fn read(&self, _: ToolkitFamily) -> Result<String, String> {
                self.0.set(Some(Instant::now()));
                Ok("follow-nickel".into())
            }
            fn write(&self, _: ToolkitFamily, _: &str) -> Result<(), String> {
                unreachable!()
            }
        }
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend(std::cell::Cell::new(None));
        let value = Observation::prepare(directory.path().join("missing"), &backend).unwrap();
        let read = backend.0.get().unwrap();
        assert!(value.started_at <= read && read <= value.completed_at);
    }
    #[test]
    fn lost_or_late_owner_receipt_is_uncertain_even_when_setter_completed() {
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Result<(), ToolkitWriteError>>(1);
        assert_eq!(
            receive_write_receipt(&receiver, Duration::ZERO),
            Err(ToolkitWriteError::Uncertain)
        );
        sender.send(Ok(())).unwrap();
        drop(sender);
        assert_eq!(receive_write_receipt(&receiver, Duration::ZERO), Ok(()));
        assert_eq!(
            receive_write_receipt(&receiver, Duration::ZERO),
            Err(ToolkitWriteError::Uncertain)
        );
        let (sender, receiver) = std::sync::mpsc::sync_channel::<Result<(), ToolkitWriteError>>(1);
        sender
            .send(Err(ToolkitWriteError::NotAccepted(
                ToolkitRejection::ExternalConflict,
            )))
            .unwrap();
        assert_eq!(
            receive_write_receipt(&receiver, Duration::ZERO),
            Err(ToolkitWriteError::NotAccepted(
                ToolkitRejection::ExternalConflict
            ))
        );
    }
}
