//! Bounded typed settings preparation shared by the Windows desktop authority.
//!
//! File contents are read and staged before the winit owner sees a request. The
//! owner performs only a revision check and atomic replacement at a fresh,
//! continuously-authorized commit boundary.

use nickel_core::shell_settings::ShellSettings;
use nickel_remote_control::appearance::{
    Animations, Preferences, Snapshot, ThemePreference, Transaction,
};
use nickel_storage::{RegularFileRevision, regular_file_revision};
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

pub(crate) struct PreparedAppearanceRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: ShellSettings,
}

impl PreparedAppearanceRead {
    pub(crate) fn prepare() -> Result<Self, String> {
        Self::at(nickel_core::shell_settings::settings_path().map_err(|_| UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let before = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = ShellSettings::load_for_update(&path).map_err(|_| UNAVAILABLE)?;
        if regular_file_revision(&path).map_err(|_| UNAVAILABLE)? != before {
            return Err(STALE.into());
        }
        Ok(Self {
            path,
            revision: before,
            settings,
        })
    }

    pub(crate) fn ensure_current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision {
            return Err(STALE.into());
        }
        Ok(())
    }

    fn configured(&self) -> Preferences {
        preferences(&self.settings)
    }
}

pub(crate) struct PreparedAppearanceChange {
    previous: PreparedAppearanceRead,
    requested: ShellSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedAppearanceChange {
    pub(crate) fn prepare(transaction: &Transaction) -> Result<Self, String> {
        Self::from_read(PreparedAppearanceRead::prepare()?, transaction)
    }

    fn from_read(
        previous: PreparedAppearanceRead,
        transaction: &Transaction,
    ) -> Result<Self, String> {
        let lock = nickel_storage::TransactionLock::try_acquire(&previous.path)
            .map_err(|_| UNAVAILABLE)?;
        previous.ensure_current()?;
        if transaction.generation == 0
            || !transaction.prior.valid()
            || previous.configured() != transaction.prior
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
            _lock: lock,
        })
    }

    fn configured(&self) -> Preferences {
        preferences(&self.requested)
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<(ShellSettings, Result<Option<RegularFileRevision>, String>), String> {
        self.staged
            .commit(|| {
                if regular_file_revision(&self.previous.path)? != self.previous.revision {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, STALE));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "appearance commit expired",
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
        let revision = regular_file_revision(&self.previous.path).map_err(|_| UNAVAILABLE.into());
        Ok((self.requested, revision))
    }
}

#[derive(Default)]
pub(crate) struct AppearanceState {
    generation: u64,
    observed: Option<(Option<RegularFileRevision>, Preferences)>,
}

impl AppearanceState {
    pub(crate) fn observe(
        &mut self,
        prepared: &PreparedAppearanceRead,
        observed_at_us: u64,
    ) -> Result<Snapshot, String> {
        let configured = prepared.configured();
        let value = (prepared.revision.clone(), configured.clone());
        if self.observed.as_ref() != Some(&value) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("appearance generation exhausted")?;
        }
        self.observed = Some(value);
        Ok(Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
        })
    }

    pub(crate) fn validate(
        &self,
        prepared: &PreparedAppearanceChange,
        transaction: &Transaction,
    ) -> Result<(), String> {
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref().is_none_or(|(revision, configured)| {
                *revision != prepared.previous.revision || *configured != transaction.prior
            })
            || prepared.configured() != transaction.requested
        {
            return Err(STALE.into());
        }
        Ok(())
    }

    pub(crate) fn observe_committed(
        &mut self,
        revision: Option<RegularFileRevision>,
        settings: &ShellSettings,
        observed_at_us: u64,
    ) -> Result<Snapshot, String> {
        let configured = preferences(settings);
        let value = (revision, configured.clone());
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("appearance generation exhausted")?;
        self.observed = Some(value);
        Ok(Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
        })
    }

    pub(crate) fn invalidate(&mut self) {
        self.observed = None;
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
            preferred_terminal: Some("protected-terminal".into()),
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
    fn appearance_commit_preserves_protected_fields_and_checks_boundary() {
        let (_root, path, request) = fixture();
        let staged = PreparedAppearanceChange::from_read(
            PreparedAppearanceRead::at(path.clone()).unwrap(),
            &request,
        )
        .unwrap();
        assert!(
            staged
                .commit(Instant::now() + Duration::from_secs(1), || Err(
                    "revoked".into()
                ))
                .is_err()
        );
        assert_eq!(
            preferences(&ShellSettings::load(&path).unwrap()),
            request.prior
        );

        let expired = PreparedAppearanceChange::from_read(
            PreparedAppearanceRead::at(path.clone()).unwrap(),
            &request,
        )
        .unwrap();
        assert!(expired.commit(Instant::now(), || Ok(())).is_err());
        assert_eq!(
            preferences(&ShellSettings::load(&path).unwrap()),
            request.prior
        );

        let staged = PreparedAppearanceChange::from_read(
            PreparedAppearanceRead::at(path.clone()).unwrap(),
            &request,
        )
        .unwrap();
        let (committed, revision) = staged
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        assert!(revision.unwrap().is_some());
        assert_eq!(preferences(&committed), request.requested);
        assert_eq!(committed.idle_lock_seconds, Some(123));
        assert_eq!(
            committed.preferred_terminal.as_deref(),
            Some("protected-terminal")
        );
    }

    #[test]
    fn appearance_state_rejects_generation_and_file_aba() {
        let (_root, path, request) = fixture();
        let read = PreparedAppearanceRead::at(path.clone()).unwrap();
        let mut state = AppearanceState::default();
        assert_eq!(state.observe(&read, 1).unwrap().generation, 1);
        assert_eq!(state.observe(&read, 2).unwrap().generation, 1);
        let prepared = PreparedAppearanceChange::from_read(read, &request).unwrap();
        state.validate(&prepared, &request).unwrap();

        let original = ShellSettings::load(&path).unwrap();
        let mut external = original.clone();
        external.accent_hue = Some(3);
        external.stage(&path).unwrap().commit(|| Ok(())).unwrap();
        original.stage(&path).unwrap().commit(|| Ok(())).unwrap();
        assert!(
            prepared
                .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
                .is_err()
        );

        let fresh = PreparedAppearanceRead::at(path.clone()).unwrap();
        assert_eq!(state.observe(&fresh, 3).unwrap().generation, 2);
        let stale = PreparedAppearanceChange::from_read(fresh, &request).unwrap();
        assert!(state.validate(&stale, &request).is_err());
        assert_eq!(ShellSettings::load(&path).unwrap(), original);
        drop(stale);
        assert_eq!(fs::read_dir(path.parent().unwrap()).unwrap().count(), 2);
    }

    #[test]
    fn appearance_preparation_rejects_out_of_range_values() {
        let (_root, path, mut request) = fixture();
        request.requested.accent_hue = Some(360);
        assert!(
            PreparedAppearanceChange::from_read(
                PreparedAppearanceRead::at(path.clone()).unwrap(),
                &request,
            )
            .is_err()
        );
    }
}
