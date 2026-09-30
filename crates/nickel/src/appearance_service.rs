//! Presentation-independent configuration service for authorized capability callers.
//! Reads and writes use staged native storage transactions. Callers enforce package
//! grants, validate the observation before commit, recheck authority in the commit
//! callback, and reconcile committed values with the shell even if a reply fails.
use nickel_core::shell_settings::ShellSettings;
pub use nickel_remote_control::appearance::{
    Animations, Preferences, Snapshot, ThemePreference, Transaction,
};
use nickel_storage::{RegularFileRevision as FileRevision, regular_file_revision as revision};
use std::{io, path::PathBuf, time::Instant};

pub(crate) const STALE: &str = "appearance changed; read current appearance before retrying";
pub(crate) const UNAVAILABLE: &str =
    "appearance unavailable; inspect current state before retrying";
pub(crate) fn preferences(settings: &ShellSettings) -> Preferences {
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

pub struct PreparedRead {
    path: PathBuf,
    pub(crate) revision: Option<FileRevision>,
    pub(crate) settings: ShellSettings,
}
impl PreparedRead {
    pub fn prepare() -> Result<Self, String> {
        Self::at(nickel_core::shell_settings::settings_path().map_err(|_| UNAVAILABLE)?)
    }
    pub fn at(path: PathBuf) -> Result<Self, String> {
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
    pub fn current(&self) -> Result<(), String> {
        if revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision {
            return Err(STALE.into());
        }
        Ok(())
    }
}
pub struct PreparedChange {
    previous: PreparedRead,
    requested: ShellSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}
impl PreparedChange {
    pub fn prepare(transaction: &Transaction) -> Result<Self, String> {
        Self::from_read(PreparedRead::prepare()?, transaction)
    }
    pub fn from_read(previous: PreparedRead, transaction: &Transaction) -> Result<Self, String> {
        let lock = nickel_storage::TransactionLock::try_acquire(&previous.path)
            .map_err(|_| UNAVAILABLE)?;
        previous.current()?;
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
            _lock: lock,
        })
    }
    pub fn commit(
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
pub struct AppearanceState {
    generation: u64,
    pub(crate) observed: Option<(Option<FileRevision>, Preferences)>,
}
impl AppearanceState {
    /// Validate the owner-issued observation and retire it after durable commit.
    pub fn commit(
        &mut self,
        prepared: PreparedChange,
        transaction: &Transaction,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<(ShellSettings, Result<Option<FileRevision>, String>), String> {
        self.validate(&prepared, transaction)?;
        let committed = prepared.commit(deadline, check_boundary)?;
        self.invalidate();
        Ok(committed)
    }

    /// Retire the accepted observation after a committed write or uncertain result.
    pub fn invalidate(&mut self) {
        self.observed = None;
    }

    /// Observe a stable configuration read. The caller supplies the observation clock.
    pub fn snapshot(
        &mut self,
        read: &PreparedRead,
        observed_at_us: u64,
    ) -> Result<Snapshot, String> {
        read.current()?;
        self.observe(
            read.revision.clone(),
            preferences(&read.settings),
            observed_at_us,
        )
    }

    pub(crate) fn observe(
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
    pub fn validate(
        &self,
        prepared: &PreparedChange,
        transaction: &Transaction,
    ) -> Result<(), String> {
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
    fn public_service_preserves_arbitrary_custom_hue_and_intensity() {
        let (_root, path, request) = fixture();
        let mut state = AppearanceState::default();
        let snapshot = state
            .snapshot(&PreparedRead::at(path.clone()).unwrap(), 42)
            .unwrap();
        assert_eq!(snapshot.observed_at_us, 42);
        let mut transaction = request;
        transaction.generation = snapshot.generation;
        transaction.prior = snapshot.configured;
        transaction.requested.accent_hue = Some(359);
        transaction.requested.accent_intensity = Some(0);
        let prepared =
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &transaction)
                .unwrap();
        state
            .commit(
                prepared,
                &transaction,
                Instant::now() + Duration::from_secs(1),
                || Ok(()),
            )
            .unwrap()
            .1
            .unwrap();
        let observed = state
            .snapshot(&PreparedRead::at(path).unwrap(), 43)
            .unwrap();
        assert_eq!(observed.generation, transaction.generation + 1);
        assert_eq!(observed.configured, transaction.requested);
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
            assert_eq!(fs::read_dir(root.path()).unwrap().count(), 2);
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
        external.stage(&path).unwrap().commit(|| Ok(())).unwrap();
        previous.stage(&path).unwrap().commit(|| Ok(())).unwrap();
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

    #[test]
    fn prepared_appearance_excludes_cooperative_local_writers() {
        let (_root, path, request) = fixture();
        let previous = ShellSettings::load(&path).unwrap();
        let prepared =
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &request).unwrap();
        let replacement = ShellSettings {
            idle_lock_seconds: Some(321),
            ..previous.clone()
        };

        assert_eq!(
            replacement.save(&path).unwrap_err().kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), previous);

        drop(prepared);
        replacement.save(&path).unwrap();
        assert_eq!(ShellSettings::load(&path).unwrap(), replacement);
    }
}
