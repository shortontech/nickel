//! Bounded terminal-presentation settings for the Windows desktop authority.
//!
//! Preparation reads and stages the production `TerminalSettings` file away
//! from the winit owner. The owner performs the final revision, deadline and
//! authorization checks immediately before replacement. Executable and working
//! directory values are preserved in the file but never enter the remote API.

use nickel_core::terminal_settings::{
    PreparedTerminalSettings, TerminalCursorStyle, TerminalSettings, settings_path,
};
use nickel_remote_control::terminal_presentation as api;
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{
    io,
    path::PathBuf,
    time::{Duration, Instant},
};

const STALE: &str = "terminal presentation changed; read current state before retrying";
const UNAVAILABLE: &str =
    "terminal presentation unavailable or uncertain; read current state before retrying";
const MAX_OBSERVATION_AGE: Duration = Duration::from_secs(1);

fn cursor(value: TerminalCursorStyle) -> api::CursorStyle {
    match value {
        TerminalCursorStyle::Block => api::CursorStyle::Block,
        TerminalCursorStyle::Beam => api::CursorStyle::Beam,
        TerminalCursorStyle::Underline => api::CursorStyle::Underline,
    }
}

fn core_cursor(value: api::CursorStyle) -> TerminalCursorStyle {
    match value {
        api::CursorStyle::Block => TerminalCursorStyle::Block,
        api::CursorStyle::Beam => TerminalCursorStyle::Beam,
        api::CursorStyle::Underline => TerminalCursorStyle::Underline,
    }
}

fn preferences(settings: &TerminalSettings) -> api::Preferences {
    api::Preferences {
        font_family: settings.font_family.clone(),
        font_size_tenths: settings.font_size_tenths,
        scrollback_lines: settings.scrollback_lines,
        cursor_style: cursor(settings.cursor_style),
        foreground: settings.foreground,
        background: settings.background,
        close_on_successful_exit: settings.close_on_successful_exit,
    }
}

fn apply(settings: &mut TerminalSettings, requested: &api::Preferences) -> Result<(), String> {
    if !requested.valid() {
        return Err("terminal presentation value is outside its supported range".into());
    }
    settings.font_family.clone_from(&requested.font_family);
    settings.font_size_tenths = requested.font_size_tenths;
    settings.scrollback_lines = requested.scrollback_lines;
    settings.cursor_style = core_cursor(requested.cursor_style);
    settings.foreground = requested.foreground;
    settings.background = requested.background;
    settings.close_on_successful_exit = requested.close_on_successful_exit;
    Ok(())
}

pub(crate) struct PreparedRead {
    started_at: Instant,
    completed_at: Instant,
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: TerminalSettings,
}

impl PreparedRead {
    pub(crate) fn prepare() -> Result<Self, String> {
        Self::at(settings_path().map_err(|_| UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let started_at = Instant::now();
        let revision = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = match TerminalSettings::load(&path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                TerminalSettings::default()
            }
            Err(_) => return Err(UNAVAILABLE.into()),
        };
        if regular_file_revision(&path).map_err(|_| UNAVAILABLE)? != revision {
            return Err(STALE.into());
        }
        Ok(Self {
            started_at,
            completed_at: Instant::now(),
            path,
            revision,
            settings,
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
}

pub(crate) struct PreparedChange {
    prior: PreparedRead,
    requested: TerminalSettings,
    staged: PreparedTerminalSettings,
}

impl PreparedChange {
    pub(crate) fn prepare(transaction: &api::Transaction) -> Result<Self, String> {
        Self::from_read(PreparedRead::prepare()?, transaction)
    }

    fn from_read(prior: PreparedRead, transaction: &api::Transaction) -> Result<Self, String> {
        if transaction.generation == 0
            || !transaction.prior.valid()
            || preferences(&prior.settings) != transaction.prior
        {
            return Err(STALE.into());
        }
        let mut requested = prior.settings.clone();
        apply(&mut requested, &transaction.requested)?;
        let staged = PreparedTerminalSettings::prepare(
            prior.path.clone(),
            &prior.settings,
            requested.clone(),
        )
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData {
                STALE
            } else {
                UNAVAILABLE
            }
            .to_owned()
        })?;
        Ok(Self {
            prior,
            requested,
            staged,
        })
    }

    pub(crate) fn ensure_current(&self, now: Instant) -> Result<(), String> {
        self.prior.ensure_current(now)
    }

    fn configured(&self) -> api::Preferences {
        preferences(&self.requested)
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<CommittedChange, String> {
        let (settings, revision) = self
            .staged
            .commit_with_revision(|| {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "terminal presentation commit expired",
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
        Ok(CommittedChange { revision, settings })
    }
}

pub(crate) struct CommittedChange {
    revision: Option<RegularFileRevision>,
    settings: TerminalSettings,
}

#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    observed: Option<(Option<RegularFileRevision>, api::Preferences)>,
}

impl State {
    pub(crate) fn observe(
        &mut self,
        prepared: &PreparedRead,
        observed_at_us: u64,
    ) -> Result<api::Snapshot, String> {
        let configured = preferences(&prepared.settings);
        let value = (prepared.revision.clone(), configured.clone());
        if self.observed.as_ref() != Some(&value) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("terminal presentation generation exhausted")?;
            self.observed = Some(value);
        }
        Ok(snapshot(
            self.generation,
            observed_at_us,
            configured,
            &prepared.settings,
        ))
    }

    pub(crate) fn validate(
        &self,
        prepared: &PreparedChange,
        transaction: &api::Transaction,
        now: Instant,
    ) -> Result<(), String> {
        prepared.ensure_current(now)?;
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || prepared.configured() != transaction.requested
            || self.observed.as_ref().is_none_or(|(revision, configured)| {
                revision != &prepared.prior.revision || configured != &transaction.prior
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
        let configured = preferences(&committed.settings);
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("terminal presentation generation exhausted")?;
        self.observed = Some((committed.revision.clone(), configured.clone()));
        Ok(snapshot(
            self.generation,
            observed_at_us,
            configured,
            &committed.settings,
        ))
    }
}

fn snapshot(
    generation: u64,
    observed_at_us: u64,
    configured: api::Preferences,
    settings: &TerminalSettings,
) -> api::Snapshot {
    api::Snapshot {
        generation,
        observed_at_us,
        configured,
        custom_shell_configured: settings.default_shell.is_some(),
        initial_directory_configured: settings.initial_working_directory.is_some(),
        // nickel-terminal loads this file while constructing each process and
        // retains the resulting settings for that process and its later tabs.
        applies_to_new_terminals: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (tempfile::TempDir, PathBuf, api::Transaction) {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("terminal-settings");
        let settings = TerminalSettings {
            default_shell: Some("private-shell".into()),
            initial_working_directory: Some(PathBuf::from("private-directory")),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let prior = preferences(&settings);
        let mut requested = prior.clone();
        requested.font_family = "Iosevka".into();
        requested.font_size_tenths = 175;
        requested.scrollback_lines = 25_000;
        requested.cursor_style = api::CursorStyle::Beam;
        requested.foreground = 0xffee_ddcc;
        requested.background = 0xff11_2233;
        requested.close_on_successful_exit = true;
        (
            root,
            path,
            api::Transaction {
                generation: 1,
                prior,
                requested,
            },
        )
    }

    #[test]
    fn windows_terminal_commit_preserves_launch_policy_and_checks_boundary() {
        let (_root, path, transaction) = fixture();
        let read = PreparedRead::at(path.clone()).unwrap();
        let mut state = State::default();
        let observed = state.observe(&read, 10).unwrap();
        assert_eq!(observed.generation, transaction.generation);
        assert!(observed.custom_shell_configured);
        assert!(observed.initial_directory_configured);
        assert!(observed.applies_to_new_terminals);

        let denied = PreparedChange::from_read(read, &transaction).unwrap();
        state
            .validate(&denied, &transaction, Instant::now())
            .unwrap();
        assert!(
            denied
                .commit(Instant::now() + Duration::from_secs(1), || Err(
                    "revoked".into()
                ))
                .is_err()
        );
        assert_eq!(
            preferences(&TerminalSettings::load(&path).unwrap()),
            transaction.prior
        );

        let expired =
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &transaction)
                .unwrap();
        assert!(expired.commit(Instant::now(), || Ok(())).is_err());
        assert_eq!(
            preferences(&TerminalSettings::load(&path).unwrap()),
            transaction.prior
        );

        let accepted =
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &transaction)
                .unwrap()
                .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
                .unwrap();
        let snapshot = state.observe_committed(&accepted, 20).unwrap();
        assert_eq!(snapshot.generation, 2);
        assert_eq!(snapshot.configured, transaction.requested);
        assert!(snapshot.custom_shell_configured);
        assert!(snapshot.initial_directory_configured);
        let actual = TerminalSettings::load(path).unwrap();
        assert_eq!(actual.default_shell.as_deref(), Some("private-shell"));
        assert_eq!(
            actual.initial_working_directory.as_deref(),
            Some(std::path::Path::new("private-directory"))
        );
    }

    #[test]
    fn windows_terminal_state_rejects_generation_file_aba_and_invalid_values() {
        let (_root, path, mut transaction) = fixture();
        let read = PreparedRead::at(path.clone()).unwrap();
        let mut state = State::default();
        assert_eq!(state.observe(&read, 1).unwrap().generation, 1);
        let prepared = PreparedChange::from_read(read, &transaction).unwrap();
        state
            .validate(&prepared, &transaction, Instant::now())
            .unwrap();

        // An uncooperative writer does not honor Nickel's transaction lock.
        // Replacing and restoring identical bytes must still invalidate the
        // opaque file identity retained by the staged transaction.
        let original = std::fs::read(&path).unwrap();
        let external = String::from_utf8(original.clone())
            .unwrap()
            .replace("font_size_tenths=140", "font_size_tenths=180");
        nickel_storage::stage_write(&path, external)
            .unwrap()
            .commit(|| Ok(()))
            .unwrap();
        nickel_storage::stage_write(&path, original)
            .unwrap()
            .commit(|| Ok(()))
            .unwrap();
        assert!(
            prepared
                .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
                .is_err()
        );

        let fresh = PreparedRead::at(path.clone()).unwrap();
        assert_eq!(state.observe(&fresh, 2).unwrap().generation, 2);
        assert!(
            PreparedChange::from_read(fresh, &transaction)
                .and_then(|change| state.validate(&change, &transaction, Instant::now()))
                .is_err()
        );

        transaction.generation = 2;
        transaction.requested.font_family = "bad\nfamily".into();
        assert!(PreparedChange::from_read(PreparedRead::at(path).unwrap(), &transaction).is_err());
    }
}
