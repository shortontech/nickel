//! Non-security idle policy staged off-thread and applied by the session owner.
use super::{NickelSession, ShellSettings};
use crate::session::state::remote_settings::{FileRevision, revision};
use nickel_core::idle::{IdleEffect, IdlePolicy};
use nickel_remote_control::idle_preferences::{Preferences, Snapshot, Timeout, Transaction};
use std::{io, path::PathBuf, time::Instant};

const STALE: &str = "idle preferences changed; read current state before retrying";
const UNAVAILABLE: &str = "idle preferences unavailable; read current state before retrying";

fn timeout(value: Option<u32>) -> Timeout {
    value.map_or(Timeout::Disabled, Timeout::AfterSeconds)
}

fn seconds(value: Timeout) -> Option<u32> {
    match value {
        Timeout::Disabled => None,
        Timeout::AfterSeconds(seconds) => Some(seconds),
    }
}

fn configured(settings: &ShellSettings) -> Preferences {
    Preferences {
        dim: timeout(settings.idle_dim_seconds),
        suspend: timeout(settings.idle_suspend_seconds),
    }
}

fn applied(policy: IdlePolicy) -> Preferences {
    let as_seconds = |value: Option<std::time::Duration>| {
        value.map_or(Timeout::Disabled, |duration| {
            Timeout::AfterSeconds(u32::try_from(duration.as_secs()).unwrap_or(u32::MAX))
        })
    };
    Preferences {
        dim: as_seconds(policy.dim_after),
        suspend: as_seconds(policy.suspend_after),
    }
}

pub(super) struct PreparedRead {
    path: PathBuf,
    revision: Option<FileRevision>,
    settings: ShellSettings,
}

impl PreparedRead {
    pub(super) fn prepare() -> Result<Self, String> {
        Self::at(nickel_core::shell_settings::settings_path().map_err(|_| UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let revision = revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = ShellSettings::load_for_update(&path).map_err(|_| UNAVAILABLE)?;
        if super::remote_settings::revision(&path).map_err(|_| UNAVAILABLE)? != revision {
            return Err(STALE.into());
        }
        Ok(Self {
            path,
            revision,
            settings,
        })
    }

    fn current(&self) -> Result<(), String> {
        if revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision {
            return Err(STALE.into());
        }
        Ok(())
    }
}

pub(super) struct PreparedChange {
    previous: PreparedRead,
    requested: ShellSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedChange {
    pub(super) fn prepare(transaction: &Transaction) -> Result<Self, String> {
        Self::prepare_at(
            nickel_core::shell_settings::settings_path().map_err(|_| UNAVAILABLE)?,
            transaction,
        )
    }

    fn prepare_at(path: PathBuf, transaction: &Transaction) -> Result<Self, String> {
        if !transaction.requested.valid_request() || transaction.prior == transaction.requested {
            return Err(STALE.into());
        }
        let previous = PreparedRead::at(path)?;
        let lock = nickel_storage::TransactionLock::try_acquire(&previous.path)
            .map_err(|_| UNAVAILABLE)?;
        previous.current()?;
        if configured(&previous.settings) != transaction.prior {
            return Err(STALE.into());
        }
        let mut requested = previous.settings.clone();
        requested.idle_dim_seconds = seconds(transaction.requested.dim);
        requested.idle_suspend_seconds = seconds(transaction.requested.suspend);
        let staged = requested.stage(&previous.path).map_err(|_| UNAVAILABLE)?;
        Ok(Self {
            previous,
            requested,
            staged,
            _lock: lock,
        })
    }

    fn commit(
        self,
        deadline: Instant,
        check: impl FnOnce() -> Result<(), String>,
    ) -> Result<(ShellSettings, Option<FileRevision>), String> {
        self.staged
            .commit(|| {
                if revision(&self.previous.path)? != self.previous.revision {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, STALE));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "idle preference commit expired",
                    ));
                }
                check().map_err(io::Error::other)
            })
            .map_err(|_| UNAVAILABLE)?;
        let revision = revision(&self.previous.path).map_err(|_| UNAVAILABLE)?;
        Ok((self.requested, revision))
    }
}

#[derive(Default)]
pub(super) struct IdlePreferenceState {
    generation: u64,
    observed: Option<(Option<FileRevision>, Preferences)>,
    applied_generation: u64,
}

impl IdlePreferenceState {
    fn observe(
        &mut self,
        revision: Option<FileRevision>,
        configured: Preferences,
        applied: Preferences,
        observed_at_us: u64,
    ) -> Result<Snapshot, String> {
        if self
            .observed
            .as_ref()
            .is_none_or(|value| value != &(revision.clone(), configured))
        {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("idle preference generation exhausted")?;
            self.observed = Some((revision, configured));
        }
        if configured == applied {
            self.applied_generation = self.generation;
        }
        Ok(Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
            applied,
            applied_generation: self.applied_generation,
            pending: configured != applied,
        })
    }

    fn validate(&self, prepared: &PreparedChange, transaction: &Transaction) -> Result<(), String> {
        if transaction.generation == 0
            || transaction.generation != self.generation
            || self.observed.as_ref().is_none_or(|(revision, prior)| {
                *revision != prepared.previous.revision || *prior != transaction.prior
            })
        {
            return Err(STALE.into());
        }
        Ok(())
    }
}

impl NickelSession {
    fn idle_observed_at_us(&self) -> u64 {
        self.start_time
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX)) as u64
    }

    pub(super) fn remote_read_idle_preferences(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.current()?;
        permit.with_debug(self.locked || self.shell_recovery_visible(), || Ok(()))?;
        let runtime = applied(self.idle_controller.policy());
        let observed_at_us = self.idle_observed_at_us();
        self.remote_idle_preferences.observe(
            prepared.revision,
            configured(&prepared.settings),
            runtime,
            observed_at_us,
        )
    }

    pub(super) fn remote_change_idle_preferences(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        transaction: Transaction,
        prepared: PreparedChange,
    ) -> Result<Snapshot, String> {
        let controller_busy = self.poll_remote_controller_ownership();
        let protected = self.locked
            || self.shell_recovery_visible()
            || self
                .internal_ui
                .focused()
                .is_some_and(|surface| self.internal_ui.remote_access_protected(surface));
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
            if controller_busy
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
            {
                return Err("shared input is busy".into());
            }
            self.remote_idle_preferences
                .validate(&prepared, &transaction)?;
            committed = Some(prepared.commit(boundary.deadline(), || {
                permit.check_commit_boundary(boundary)
            })?);
            self.remote_idle_preferences.observed = None;
            Ok(())
        });
        let (settings, revision) = match committed {
            Some(value) => value,
            None => {
                authorization?;
                return Err(UNAVAILABLE.into());
            }
        };
        let policy = IdlePolicy::from_seconds(
            settings.idle_dim_seconds,
            settings.idle_lock_seconds,
            settings.idle_suspend_seconds,
        );
        if self
            .idle_controller
            .replace_policy(policy, self.start_time.elapsed())
            == Some(IdleEffect::Undim)
        {
            self.dimmed = false;
            self.request_output_redraw();
        }
        if let Some(shell) = self.internal_shell.as_mut() {
            let changed = shell.apply_prepared_shell_settings(settings.clone());
            if !changed.is_empty() {
                self.sync_internal_shell_changes(Some(&changed));
            }
        }
        self.notify_shell_settings_changed();
        self.schedule_internal_shell_deadline();
        authorization?;
        let configured = configured(&settings);
        let runtime = applied(self.idle_controller.policy());
        let observed_at_us = self.idle_observed_at_us();
        self.remote_idle_preferences
            .observe(revision, configured, runtime, observed_at_us)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transaction() -> Transaction {
        Transaction {
            generation: 1,
            prior: Preferences {
                dim: Timeout::AfterSeconds(300),
                suspend: Timeout::Disabled,
            },
            requested: Preferences {
                dim: Timeout::AfterSeconds(600),
                suspend: Timeout::AfterSeconds(3600),
            },
        }
    }

    #[test]
    fn transaction_preserves_lock_and_unrelated_settings_and_rejects_replacement() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell.conf");
        let prior = ShellSettings {
            idle_lock_seconds: Some(17),
            preferred_terminal: Some("owned-terminal".into()),
            ..Default::default()
        };
        prior.save(&path).unwrap();
        let prepared = PreparedChange::prepare_at(path.clone(), &transaction()).unwrap();
        assert_eq!(
            prior.save(&path).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let mut external = prior.clone();
        external.idle_lock_seconds = Some(19);
        external.stage(&path).unwrap().commit(|| Ok(())).unwrap();
        assert!(
            prepared
                .commit(
                    Instant::now() + std::time::Duration::from_secs(1),
                    || Ok(()),
                )
                .is_err()
        );
        prior.save(&path).unwrap();

        let prepared = PreparedChange::prepare_at(path.clone(), &transaction()).unwrap();
        let (accepted, _) = prepared
            .commit(
                Instant::now() + std::time::Duration::from_secs(1),
                || Ok(()),
            )
            .unwrap();
        assert_eq!(accepted.idle_dim_seconds, Some(600));
        assert_eq!(accepted.idle_suspend_seconds, Some(3600));
        assert_eq!(accepted.idle_lock_seconds, Some(17));
        assert_eq!(
            accepted.preferred_terminal.as_deref(),
            Some("owned-terminal")
        );
    }

    #[test]
    fn observation_distinguishes_external_pending_state_from_runtime_acknowledgement() {
        let mut state = IdlePreferenceState::default();
        let configured = Preferences {
            dim: Timeout::AfterSeconds(300),
            suspend: Timeout::Disabled,
        };
        let stale_runtime = Preferences {
            dim: Timeout::AfterSeconds(600),
            suspend: Timeout::Disabled,
        };
        let pending = state.observe(None, configured, stale_runtime, 7).unwrap();
        assert!(pending.pending);
        assert_eq!(pending.applied_generation, 0);
        let acknowledged = state.observe(None, configured, configured, 8).unwrap();
        assert!(!acknowledged.pending);
        assert_eq!(acknowledged.applied_generation, acknowledged.generation);
        assert_eq!(acknowledged.observed_at_us, 8);
    }
}
