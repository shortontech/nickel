//! Bounded application-scale preparation for the Windows desktop authority.
//!
//! Nickel has no native Windows owner for GTK or Qt global scale settings. The
//! production compatibility policy remains useful to Nickel's launch policy,
//! while the two external toolkit capabilities are reported unavailable. Their
//! existing ownership and pending-intent journal fields are preserved verbatim.

use nickel_core::dpi::{ApplicationScalePolicy, ApplicationScaleSettings, Scale120};
use nickel_platform::{ToolkitCapability, ToolkitFamily, ToolkitScaleBackend};
use nickel_remote_control::application_scale as api;
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

const STALE: &str = "application scale changed; read current state before retrying";
const UNAVAILABLE: &str =
    "application scale unavailable or uncertain; read current state before retrying";
const MAX_OBSERVATION_AGE: Duration = Duration::from_secs(1);

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
            ApplicationScalePolicy::Custom(
                Scale120::new(scale_120).ok_or("unsupported application scale")?,
            )
        }
    })
}

fn family(value: ToolkitFamily) -> api::Family {
    match value {
        ToolkitFamily::Gtk => api::Family::Gtk,
        ToolkitFamily::Qt => api::Family::Qt,
    }
}

#[derive(Default)]
struct WindowsToolkitScaleBackend;

impl ToolkitScaleBackend for WindowsToolkitScaleBackend {
    fn capabilities(&self) -> Vec<ToolkitCapability> {
        [ToolkitFamily::Gtk, ToolkitFamily::Qt]
            .into_iter()
            .map(|family| ToolkitCapability {
                family,
                available: false,
                live: false,
                restart_required: true,
            })
            .collect()
    }

    fn read(&self, _family: ToolkitFamily) -> Result<String, String> {
        Err("external toolkit scale unavailable on Windows".into())
    }

    fn write(&self, _family: ToolkitFamily, _value: &str) -> Result<(), String> {
        Err("external toolkit scale unavailable on Windows".into())
    }
}

fn toolkits(settings: &ApplicationScaleSettings) -> Vec<api::Toolkit> {
    WindowsToolkitScaleBackend
        .capabilities()
        .into_iter()
        .map(|capability| {
            let (owned, pending) = match capability.family {
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
                family: family(capability.family),
                available: false,
                observed: None,
                owned,
                pending,
                restart_required: capability.restart_required,
            }
        })
        .collect()
}

pub(crate) struct PreparedRead {
    started_at: Instant,
    completed_at: Instant,
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: ApplicationScaleSettings,
    toolkits: Vec<api::Toolkit>,
}

impl PreparedRead {
    pub(crate) fn prepare() -> Result<Self, String> {
        let path =
            nickel_storage::config_path("application-scale.conf").map_err(|_| UNAVAILABLE)?;
        Self::at(path)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let started_at = Instant::now();
        let revision = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = ApplicationScaleSettings::load(&path).map_err(|_| UNAVAILABLE)?;
        if regular_file_revision(&path).map_err(|_| UNAVAILABLE)? != revision {
            return Err(STALE.into());
        }
        let toolkits = toolkits(&settings);
        Ok(Self {
            started_at,
            completed_at: Instant::now(),
            path,
            revision,
            settings,
            toolkits,
        })
    }

    pub(crate) fn ensure_current(&self, now: Instant) -> Result<(), String> {
        if self.completed_at < self.started_at
            || self.completed_at > now
            || now
                .checked_duration_since(self.started_at)
                .is_none_or(|age| age > MAX_OBSERVATION_AGE)
            || regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision
        {
            return Err(STALE.into());
        }
        Ok(())
    }

    fn interval_us(&self, session_started: Instant, now: Instant) -> Result<(u64, u64), String> {
        self.ensure_current(now)?;
        let elapsed = |at: Instant| {
            at.checked_duration_since(session_started)
                .and_then(|duration| u64::try_from(duration.as_micros()).ok())
                .ok_or_else(|| "invalid application scale observation clock".to_owned())
        };
        Ok((elapsed(self.started_at)?, elapsed(self.completed_at)?))
    }
}

pub(crate) struct PreparedChange {
    prior: PreparedRead,
    requested: ApplicationScaleSettings,
    outcomes: Vec<api::Outcome>,
    staged: nickel_storage::StagedDurableWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedChange {
    pub(crate) fn prepare(transaction: &api::Transaction) -> Result<Self, String> {
        let path =
            nickel_storage::config_path("application-scale.conf").map_err(|_| UNAVAILABLE)?;
        Self::at(path, transaction)
    }

    fn at(path: PathBuf, transaction: &api::Transaction) -> Result<Self, String> {
        let lock = nickel_storage::TransactionLock::try_acquire(&path)
            .map_err(|_| "application scale transaction busy")?;
        let prior = PreparedRead::at(path)?;
        if transaction.generation == 0 || transaction.prior != policy(prior.settings.policy) {
            return Err(STALE.into());
        }
        let requested_policy = requested(transaction.requested)?;
        let mut settings = prior.settings.clone();
        let report = nickel_platform::transact_application_scale(
            &WindowsToolkitScaleBackend,
            &mut settings,
            requested_policy,
            |_| Ok(()),
            || Ok(()),
        )?;
        let staged = settings.stage(&prior.path).map_err(|_| UNAVAILABLE)?;
        let outcomes = report
            .outcomes
            .into_iter()
            .map(|outcome| {
                use nickel_platform::ToolkitOutcomeKind as Kind;
                api::Outcome {
                    family: family(outcome.family),
                    kind: match outcome.kind {
                        Kind::Unchanged => api::OutcomeKind::Unchanged,
                        Kind::Confirmed => api::OutcomeKind::Confirmed,
                        Kind::ExternalConflict => api::OutcomeKind::ExternalConflict,
                        Kind::Unavailable => api::OutcomeKind::Unavailable,
                        Kind::Failed => api::OutcomeKind::Failed,
                        Kind::Uncertain => api::OutcomeKind::Uncertain,
                    },
                    restart_required: outcome.restart_required,
                }
            })
            .collect();
        Ok(Self {
            prior,
            requested: settings,
            outcomes,
            staged,
            _lock: lock,
        })
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<CommittedChange, String> {
        let path = self.prior.path.clone();
        let previous_revision = self.prior.revision.clone();
        let requested = self.requested;
        let outcomes = self.outcomes;
        let receipt = self
            .staged
            .commit(|| {
                if regular_file_revision(&path)? != previous_revision {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, STALE));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "application scale commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)
            })
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    STALE
                } else {
                    UNAVAILABLE
                }
                .to_owned()
            })?;
        // Windows has no deferred directory synchronization: replacement uses
        // MOVEFILE_WRITE_THROUGH. This remains meaningful in portable tests.
        receipt.sync().map_err(|_| UNAVAILABLE)?;
        let revision = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        Ok(CommittedChange {
            revision,
            settings: requested,
            outcomes,
        })
    }
}

pub(crate) struct CommittedChange {
    revision: Option<RegularFileRevision>,
    settings: ApplicationScaleSettings,
    pub(crate) outcomes: Vec<api::Outcome>,
}

#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    observed: Option<(
        Option<RegularFileRevision>,
        ApplicationScaleSettings,
        Vec<api::Toolkit>,
    )>,
}

impl State {
    pub(crate) fn observe(
        &mut self,
        prepared: &PreparedRead,
        session_started: Instant,
        now: Instant,
    ) -> Result<api::Snapshot, String> {
        let (observation_started_at_us, observed_at_us) =
            prepared.interval_us(session_started, now)?;
        let value = (
            prepared.revision.clone(),
            prepared.settings.clone(),
            prepared.toolkits.clone(),
        );
        if self.observed.as_ref() != Some(&value) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("scale generation exhausted")?;
        }
        self.observed = Some(value);
        Ok(api::Snapshot {
            generation: self.generation,
            observation_started_at_us,
            observed_at_us,
            atomic: false,
            policy: policy(prepared.settings.policy),
            toolkits: prepared.toolkits.clone(),
        })
    }

    pub(crate) fn validate(
        &self,
        prepared: &PreparedChange,
        transaction: &api::Transaction,
        now: Instant,
    ) -> Result<(), String> {
        prepared.prior.ensure_current(now)?;
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || transaction.prior != policy(prepared.prior.settings.policy)
            || self.observed.as_ref().is_none_or(|observed| {
                observed
                    != &(
                        prepared.prior.revision.clone(),
                        prepared.prior.settings.clone(),
                        prepared.prior.toolkits.clone(),
                    )
            })
        {
            return Err(STALE.into());
        }
        Ok(())
    }

    pub(crate) fn observe_committed(
        &mut self,
        committed: &CommittedChange,
        observed_at_us: u64,
    ) -> Result<api::Snapshot, String> {
        let toolkits = toolkits(&committed.settings);
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("scale generation exhausted")?;
        self.observed = Some((
            committed.revision.clone(),
            committed.settings.clone(),
            toolkits.clone(),
        ));
        Ok(api::Snapshot {
            generation: self.generation,
            observation_started_at_us: observed_at_us,
            observed_at_us,
            atomic: false,
            policy: policy(committed.settings.policy),
            toolkits,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::dpi::ToolkitScaleIntent;

    fn transaction(
        generation: u64,
        prior: api::Policy,
        requested: api::Policy,
    ) -> api::Transaction {
        api::Transaction {
            generation,
            prior,
            requested,
        }
    }

    #[test]
    fn windows_policy_commit_preserves_toolkit_ownership_and_checks_boundary() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application-scale.conf");
        let existing = ApplicationScaleSettings {
            policy: ApplicationScalePolicy::FollowNickel,
            owned_gtk_previous: Some("1".into()),
            owned_gtk_applied: Some("2".into()),
            owned_qt_previous: Some("follow-nickel".into()),
            owned_qt_applied: Some("1.250000".into()),
            pending_gtk: Some(ToolkitScaleIntent {
                previous: "1".into(),
                requested: "2".into(),
                restoring: false,
                terminal: false,
            }),
            pending_qt: None,
        };
        existing.save(&path).unwrap();
        let read = PreparedRead::at(path.clone()).unwrap();
        let mut state = State::default();
        let snapshot = state
            .observe(&read, read.started_at, Instant::now())
            .unwrap();
        assert_eq!(snapshot.toolkits.len(), 2);
        assert!(snapshot.toolkits.iter().all(|toolkit| !toolkit.available));
        assert!(snapshot.toolkits[0].owned);
        assert!(snapshot.toolkits[0].pending);
        let request = transaction(
            snapshot.generation,
            snapshot.policy,
            api::Policy::Custom { scale_120: 150 },
        );

        let denied = PreparedChange::at(path.clone(), &request).unwrap();
        state.validate(&denied, &request, Instant::now()).unwrap();
        assert!(
            denied
                .commit(Instant::now() + Duration::from_secs(1), || Err(
                    "revoked".into()
                ))
                .is_err()
        );
        assert_eq!(ApplicationScaleSettings::load(&path).unwrap(), existing);

        let accepted = PreparedChange::at(path.clone(), &request).unwrap();
        state.validate(&accepted, &request, Instant::now()).unwrap();
        let committed = accepted
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        assert_eq!(committed.outcomes.len(), 2);
        assert!(
            committed
                .outcomes
                .iter()
                .all(|outcome| matches!(outcome.kind, api::OutcomeKind::Unavailable))
        );
        let actual = ApplicationScaleSettings::load(&path).unwrap();
        assert_eq!(
            actual.policy,
            ApplicationScalePolicy::Custom(Scale120::new(150).unwrap())
        );
        assert_eq!(actual.owned_gtk_previous, existing.owned_gtk_previous);
        assert_eq!(actual.owned_gtk_applied, existing.owned_gtk_applied);
        assert_eq!(actual.owned_qt_previous, existing.owned_qt_previous);
        assert_eq!(actual.owned_qt_applied, existing.owned_qt_applied);
        assert_eq!(actual.pending_gtk, existing.pending_gtk);
    }

    #[test]
    fn windows_policy_rejects_stale_generation_file_aba_and_invalid_scale() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("application-scale.conf");
        let initial = ApplicationScaleSettings::default();
        initial.save(&path).unwrap();
        let read = PreparedRead::at(path.clone()).unwrap();
        let mut state = State::default();
        let snapshot = state
            .observe(&read, read.started_at, Instant::now())
            .unwrap();
        let request = transaction(snapshot.generation, snapshot.policy, api::Policy::Unchanged);
        let prepared = PreparedChange::at(path.clone(), &request).unwrap();
        // Simulate an uncooperative writer that ignores Nickel's sibling lock,
        // including an ABA back to the same serialized value.
        std::fs::write(&path, "version=1\npolicy=custom:120\n").unwrap();
        std::fs::write(&path, "version=1\npolicy=follow\n").unwrap();
        assert!(
            prepared
                .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
                .is_err()
        );
        let stale = PreparedChange::at(
            path.clone(),
            &transaction(
                snapshot.generation + 1,
                snapshot.policy,
                api::Policy::Unchanged,
            ),
        )
        .unwrap();
        assert!(
            state
                .validate(
                    &stale,
                    &transaction(
                        snapshot.generation + 1,
                        snapshot.policy,
                        api::Policy::Unchanged,
                    ),
                    Instant::now(),
                )
                .is_err()
        );
        drop(stale);
        assert!(
            PreparedChange::at(
                path,
                &transaction(
                    snapshot.generation,
                    snapshot.policy,
                    api::Policy::Custom { scale_120: 121 }
                )
            )
            .is_err()
        );
    }
}
