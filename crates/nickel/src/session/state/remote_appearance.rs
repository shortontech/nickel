//! Appearance configuration is staged off-thread and committed by the desktop owner.
use super::{
    NickelSession, ShellSettings,
    remote_settings::{FileRevision, revision},
};
use nickel_remote_control::appearance::{
    Animations, Preferences, Snapshot, ThemePreference, Transaction,
};
use std::{io, path::PathBuf, time::Instant};

const STALE: &str = "appearance changed; read current appearance before retrying";
const UNAVAILABLE: &str = "appearance unavailable; inspect current state before retrying";
fn preferences(settings: &ShellSettings) -> Preferences {
    use nickel_core::shell_settings::{AnimationLevel as A, ThemePreference as T};
    Preferences {
        theme: match settings.theme {
            T::System => ThemePreference::System,
            T::Light => ThemePreference::Light,
            T::Dark => ThemePreference::Dark,
        },
        accent_hue: settings.accent_hue,
        accent_intensity: settings.accent_intensity,
        reduce_transparency: settings.reduce_transparency,
        animations: match settings.animations {
            A::Off => Animations::Off,
            A::Reduced => Animations::Reduced,
            A::Normal => Animations::Normal,
        },
    }
}
fn apply(settings: &mut ShellSettings, requested: &Preferences) -> Result<(), String> {
    use nickel_core::shell_settings::{AnimationLevel as A, ThemePreference as T};
    if !requested.valid() {
        return Err("appearance value is outside its supported range".into());
    }
    settings.theme = match requested.theme {
        ThemePreference::System => T::System,
        ThemePreference::Light => T::Light,
        ThemePreference::Dark => T::Dark,
    };
    settings.accent_hue = requested.accent_hue;
    settings.accent_intensity = requested.accent_intensity;
    settings.reduce_transparency = requested.reduce_transparency;
    settings.animations = match requested.animations {
        Animations::Off => A::Off,
        Animations::Reduced => A::Reduced,
        Animations::Normal => A::Normal,
    };
    Ok(())
}

pub(super) struct PreparedRead {
    path: PathBuf,
    revision: Option<FileRevision>,
    settings: ShellSettings,
}
impl PreparedRead {
    pub fn prepare() -> Result<Self, String> {
        Self::at(nickel_core::shell_settings::settings_path().map_err(|_| UNAVAILABLE)?)
    }
    fn at(path: PathBuf) -> Result<Self, String> {
        let before = revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = ShellSettings::load_for_update(&path).map_err(|_| UNAVAILABLE)?;
        if revision(&path).map_err(|_| UNAVAILABLE)? != before {
            return Err(STALE.into());
        }
        Ok(Self {
            path,
            revision: before,
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
}
impl PreparedChange {
    pub fn prepare(transaction: &Transaction) -> Result<Self, String> {
        Self::from_read(PreparedRead::prepare()?, transaction)
    }
    fn from_read(previous: PreparedRead, transaction: &Transaction) -> Result<Self, String> {
        if transaction.generation == 0
            || !transaction.prior.valid()
            || preferences(&previous.settings) != transaction.prior
        {
            return Err(STALE.into());
        }
        let mut requested = previous.settings.clone();
        apply(&mut requested, &transaction.requested)?;
        let staged = requested.stage(&previous.path).map_err(|_| UNAVAILABLE)?;
        Ok(Self {
            previous,
            requested,
            staged,
        })
    }
    fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<(ShellSettings, Result<Option<FileRevision>, String>), String> {
        self.staged
            .commit(|| {
                if revision(&self.previous.path)? != self.previous.revision {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, STALE));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "appearance commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)?;
                Ok(())
            })
            .map_err(|_| UNAVAILABLE)?;
        // A metadata failure after rename is uncertain, but must not suppress
        // reconciliation of the settings that were already committed.
        let current = revision(&self.previous.path).map_err(|_| UNAVAILABLE.to_owned());
        Ok((self.requested, current))
    }
}
#[derive(Default)]
pub(super) struct AppearanceState {
    generation: u64,
    observed: Option<(Option<FileRevision>, Preferences)>,
}
impl AppearanceState {
    fn observe(
        &mut self,
        revision: Option<FileRevision>,
        configured: Preferences,
        observed_at_us: u64,
    ) -> Result<Snapshot, String> {
        if self
            .observed
            .as_ref()
            .is_none_or(|(previous, values)| *previous != revision || *values != configured)
        {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("appearance generation exhausted")?;
            self.observed = Some((revision, configured.clone()));
        }
        Ok(Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
        })
    }
    fn validate(&self, prepared: &PreparedChange, transaction: &Transaction) -> Result<(), String> {
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref().is_none_or(|(revision, values)| {
                *revision != prepared.previous.revision || *values != transaction.prior
            })
        {
            return Err(STALE.into());
        }
        Ok(())
    }
}
impl NickelSession {
    pub(super) fn remote_read_appearance(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.current()?;
        permit.with_debug(self.locked || self.shell_recovery_visible(), || {
            self.remote_appearance.observe(
                prepared.revision,
                preferences(&prepared.settings),
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            )
        })
    }
    pub(super) fn remote_change_appearance(
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
        let authorization = permit.with_debug_input_deadline(protected, |deadline| {
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
            self.remote_appearance.validate(&prepared, &transaction)?;
            committed = Some(prepared.commit(deadline.deadline(), || {
                permit.check_commit_boundary(deadline)
            })?);
            self.remote_appearance.observed = None;
            Ok(())
        });
        let (requested, revision) = match committed {
            Some(committed) => committed,
            None => {
                authorization?;
                return Err(UNAVAILABLE.into());
            }
        };
        // Even if emergency stop races an already accepted rename, reconcile
        // that committed configuration before returning the cancelled outcome.
        let configured = preferences(&requested);
        let mut changed = self
            .internal_shell
            .as_mut()
            .map(|shell| shell.apply_prepared_shell_settings(requested))
            .unwrap_or_default();
        let theme = self
            .internal_shell
            .as_ref()
            .map(crate::internal_shell::InternalShellCoordinator::semantic_theme);
        if let (Some(theme), Some(mut codex)) = (theme, self.internal_codex.take()) {
            changed.extend(codex.set_theme(&mut self.internal_ui, theme));
            self.internal_codex = Some(codex);
        }
        changed.sort_unstable();
        changed.dedup();
        if !changed.is_empty() {
            self.sync_internal_shell_changes(Some(&changed));
        }
        self.notify_shell_settings_changed();
        self.schedule_internal_shell_deadline();
        authorization?;
        self.remote_appearance.observe(
            revision?,
            configured,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, time::Duration};
    fn fixture() -> (tempfile::TempDir, PathBuf, Transaction) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell.conf");
        let settings = ShellSettings {
            idle_lock_seconds: Some(123),
            preferred_terminal: Some("owned-terminal".into()),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let prior = preferences(&settings);
        let mut requested = prior.clone();
        requested.accent_hue = Some(271);
        requested.accent_intensity = Some(63);
        requested.theme = ThemePreference::Light;
        (
            root,
            path,
            Transaction {
                generation: 1,
                prior,
                requested,
            },
        )
    }
    #[test]
    fn staged_appearance_preserves_other_settings_and_requires_live_commit_boundary() {
        let (root, path, request) = fixture();
        for cancelled in [false, true] {
            let staged =
                PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &request)
                    .unwrap();
            let deadline = if cancelled {
                Instant::now() + Duration::from_secs(1)
            } else {
                Instant::now()
            };
            assert!(staged.commit(deadline, || Err("cancelled".into())).is_err());
            assert_eq!(
                preferences(&ShellSettings::load(&path).unwrap()),
                request.prior
            );
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
        }
        let staged =
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &request).unwrap();
        let (committed, revision) = staged
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        assert!(revision.unwrap().is_some());
        assert_eq!(preferences(&committed), request.requested);
        let actual = ShellSettings::load(&path).unwrap();
        assert_eq!(actual, committed);
        assert_eq!(actual.idle_lock_seconds, Some(123));
        assert_eq!(actual.preferred_terminal.as_deref(), Some("owned-terminal"));
    }
    #[test]
    fn appearance_observation_rejects_stale_generation_and_file_aba() {
        let (_root, path, request) = fixture();
        let read = PreparedRead::at(path.clone()).unwrap();
        let mut state = AppearanceState::default();
        assert_eq!(
            state
                .observe(read.revision.clone(), request.prior.clone(), 1)
                .unwrap()
                .generation,
            1
        );
        assert_eq!(
            state
                .observe(read.revision.clone(), request.prior.clone(), 2)
                .unwrap()
                .generation,
            1
        );
        let prepared = PreparedChange::from_read(read, &request).unwrap();
        state.validate(&prepared, &request).unwrap();
        let previous = ShellSettings::load(&path).unwrap();
        let mut external = previous.clone();
        external.accent_hue = Some(3);
        external.save(&path).unwrap();
        previous.save(&path).unwrap();
        assert!(
            prepared
                .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
                .is_err()
        );
        let fresh = PreparedRead::at(path.clone()).unwrap();
        assert_eq!(
            state
                .observe(fresh.revision.clone(), request.prior.clone(), 3)
                .unwrap()
                .generation,
            2
        );
        let prepared = PreparedChange::from_read(fresh, &request).unwrap();
        assert!(state.validate(&prepared, &request).is_err());
        assert_eq!(ShellSettings::load(&path).unwrap(), previous);
    }
    #[test]
    fn appearance_rejects_unsupported_values_protected_fields_and_unbounded_files() {
        let (_root, path, mut request) = fixture();
        request.requested.accent_hue = Some(360);
        assert!(
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &request).is_err()
        );
        request.requested.accent_hue = None;
        request.requested.accent_intensity = Some(101);
        assert!(
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &request).is_err()
        );
        let mut value = serde_json::to_value(&request.prior).unwrap();
        value["remote_control_enabled"] = serde_json::json!(true);
        assert!(serde_json::from_value::<Preferences>(value).is_err());
        fs::write(&path, vec![b'x'; 65537]).unwrap();
        assert!(PreparedRead::at(path).is_err());
    }
}
