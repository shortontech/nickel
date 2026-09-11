//! Terminal launch-policy adapter shared by the Linux and Windows owners.
//!
//! Catalog discovery and settings staging happen away from the presentation
//! owner. The prepared value retains private canonical paths; the public MCP
//! value contains fixed identities only.
use nickel_core::terminal_settings::{
    MAX_TERMINAL_SETTING_TEXT, PreparedTerminalSettings, TerminalSettings, settings_path,
};
use nickel_remote_control::terminal_launch_policy as api;
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const STALE: &str = "terminal launch policy changed; read current state before retrying";
const UNAVAILABLE: &str =
    "terminal launch policy unavailable or uncertain; read current state before retrying";
const MAX_OBSERVATION_AGE: Duration = Duration::from_secs(1);

#[derive(Clone, Debug, Eq, PartialEq)]
struct ShellEntry {
    identity: api::ShellIdentity,
    executable: PathBuf,
    revision: RegularFileRevision,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct DirectoryEntry {
    identity: api::DirectoryIdentity,
    path: PathBuf,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Catalog {
    shells: Vec<ShellEntry>,
    directories: Vec<DirectoryEntry>,
}

impl Catalog {
    fn discover() -> Result<Self, String> {
        Ok(Self {
            shells: shell_candidates()
                .into_iter()
                .filter_map(|(identity, candidates)| {
                    candidates
                        .iter()
                        .find_map(|candidate| installed_executable(candidate))
                        .filter(|(executable, _)| terminal_path_text(executable).is_some())
                        .map(|(executable, revision)| ShellEntry {
                            identity,
                            executable,
                            revision,
                        })
                })
                .collect(),
            directories: directory_candidates()
                .into_iter()
                .filter_map(|(identity, path)| {
                    owned_canonical_directory(&path)
                        .filter(|path| terminal_path_text(path).is_some())
                        .map(|path| DirectoryEntry { identity, path })
                })
                .collect(),
        })
    }

    fn shell(&self, identity: api::ShellIdentity) -> Option<&ShellEntry> {
        self.shells.iter().find(|entry| entry.identity == identity)
    }

    fn directory(&self, identity: api::DirectoryIdentity) -> Option<&DirectoryEntry> {
        self.directories
            .iter()
            .find(|entry| entry.identity == identity)
    }

    fn public_shells(&self) -> Vec<api::ShellIdentity> {
        self.shells.iter().map(|entry| entry.identity).collect()
    }

    fn public_directories(&self) -> Vec<api::DirectoryIdentity> {
        self.directories
            .iter()
            .map(|entry| entry.identity)
            .collect()
    }
}

#[cfg(not(target_os = "windows"))]
fn shell_candidates() -> Vec<(api::ShellIdentity, Vec<PathBuf>)> {
    use api::ShellIdentity as S;
    vec![
        (S::BourneShell, vec!["/bin/sh".into(), "/usr/bin/sh".into()]),
        (S::Bash, vec!["/bin/bash".into(), "/usr/bin/bash".into()]),
        (S::Dash, vec!["/bin/dash".into(), "/usr/bin/dash".into()]),
        (S::Zsh, vec!["/bin/zsh".into(), "/usr/bin/zsh".into()]),
        (S::Fish, vec!["/bin/fish".into(), "/usr/bin/fish".into()]),
    ]
}

#[cfg(target_os = "windows")]
fn shell_candidates() -> Vec<(api::ShellIdentity, Vec<PathBuf>)> {
    use api::ShellIdentity as S;
    let windows = std::env::var_os("SystemRoot")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let program_files = std::env::var_os("ProgramFiles")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"));
    vec![
        (S::CommandPrompt, vec![windows.join(r"System32\cmd.exe")]),
        (
            S::WindowsPowerShell,
            vec![windows.join(r"System32\WindowsPowerShell\v1.0\powershell.exe")],
        ),
        (
            S::PowerShell,
            vec![program_files.join(r"PowerShell\7\pwsh.exe")],
        ),
    ]
}

fn directory_candidates() -> Vec<(api::DirectoryIdentity, PathBuf)> {
    use api::DirectoryIdentity as D;
    [
        (D::Home, dirs::home_dir()),
        (D::Desktop, dirs::desktop_dir()),
        (D::Documents, dirs::document_dir()),
        (D::Downloads, dirs::download_dir()),
    ]
    .into_iter()
    .filter_map(|(identity, path)| path.map(|path| (identity, path)))
    .collect()
}

fn owned_canonical_directory(path: &Path) -> Option<PathBuf> {
    let home = dirs::home_dir().and_then(|home| canonical_directory(&home))?;
    let path = canonical_directory(path)?;
    if !path.starts_with(&home) {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if fs::metadata(&path).ok()?.uid() != fs::metadata(&home).ok()?.uid() {
            return None;
        }
    }
    Some(path)
}

fn installed_executable(path: &Path) -> Option<(PathBuf, RegularFileRevision)> {
    let path = fs::canonicalize(path).ok()?;
    let metadata = fs::metadata(&path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o111 == 0 {
            return None;
        }
    }
    regular_file_revision(&path)
        .ok()
        .flatten()
        .map(|revision| (path, revision))
}

fn canonical_directory(path: &Path) -> Option<PathBuf> {
    let path = fs::canonicalize(path).ok()?;
    fs::metadata(&path).ok()?.is_dir().then_some(path)
}

fn observed_policy(settings: &TerminalSettings, catalog: &Catalog) -> api::ObservedPolicy {
    let shell = settings.default_shell.as_deref().and_then(|configured| {
        fs::canonicalize(configured).ok().and_then(|configured| {
            catalog
                .shells
                .iter()
                .find(|entry| entry.executable == configured)
                .map(|entry| entry.identity)
        })
    });
    let directory = settings
        .initial_working_directory
        .as_deref()
        .and_then(canonical_directory)
        .and_then(|configured| {
            catalog
                .directories
                .iter()
                .find(|entry| entry.path == configured)
                .map(|entry| entry.identity)
        });
    api::ObservedPolicy {
        configured: api::Policy {
            default_shell: shell,
            initial_directory: directory,
        },
        shell_unavailable: settings.default_shell.is_some() && shell.is_none(),
        directory_unavailable: settings.initial_working_directory.is_some() && directory.is_none(),
    }
}

fn requested_settings(
    mut settings: TerminalSettings,
    requested: &api::Policy,
    catalog: &Catalog,
) -> Result<TerminalSettings, String> {
    settings.default_shell = requested
        .default_shell
        .map(|identity| {
            catalog
                .shell(identity)
                .and_then(|entry| terminal_path_text(&entry.executable))
                .ok_or("requested shell is not in the current installed catalog")
        })
        .transpose()?;
    settings.initial_working_directory = requested
        .initial_directory
        .map(|identity| {
            catalog
                .directory(identity)
                .filter(|entry| terminal_path_text(&entry.path).is_some())
                .map(|entry| entry.path.clone())
                .ok_or("requested directory is not in the current owner catalog")
        })
        .transpose()?;
    Ok(settings)
}

fn terminal_path_text(path: &Path) -> Option<String> {
    let value = path.to_str()?;
    (!value.is_empty()
        && value.len() <= MAX_TERMINAL_SETTING_TEXT
        && !value.chars().any(char::is_control))
    .then(|| value.to_owned())
}

pub(crate) struct PreparedRead {
    started_at: Instant,
    completed_at: Instant,
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: TerminalSettings,
    catalog: Catalog,
    policy: api::ObservedPolicy,
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
        let catalog = Catalog::discover()?;
        let policy = observed_policy(&settings, &catalog);
        if regular_file_revision(&path).map_err(|_| UNAVAILABLE)? != revision {
            return Err(STALE.into());
        }
        Ok(Self {
            started_at,
            completed_at: Instant::now(),
            path,
            revision,
            settings,
            catalog,
            policy,
        })
    }

    pub(crate) fn ensure_current(&self, now: Instant) -> Result<(), String> {
        if self.completed_at < self.started_at
            || self.completed_at > now
            || now
                .checked_duration_since(self.started_at)
                .is_none_or(|age| age > MAX_OBSERVATION_AGE)
            || regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision
            || Catalog::discover()? != self.catalog
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
        if !transaction.valid() || prior.policy != transaction.prior {
            return Err(STALE.into());
        }
        let requested = requested_settings(
            prior.settings.clone(),
            &transaction.requested,
            &prior.catalog,
        )?;
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

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<CommittedChange, String> {
        self.commit_with_catalog_check(deadline, Catalog::discover, check_boundary)
    }

    fn commit_with_catalog_check(
        self,
        deadline: Instant,
        current_catalog: impl FnOnce() -> Result<Catalog, String>,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<CommittedChange, String> {
        let expected_catalog = self.prior.catalog.clone();
        let (settings, revision) = self
            .staged
            .commit_with_revision(|| {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "terminal launch-policy commit expired",
                    ));
                }
                if current_catalog().map_err(io::Error::other)? != expected_catalog {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "terminal launch-policy catalog changed",
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
        if settings != self.requested {
            return Err(UNAVAILABLE.into());
        }
        Ok(CommittedChange {
            revision,
            settings,
            catalog: expected_catalog,
        })
    }
}

pub(crate) struct CommittedChange {
    revision: Option<RegularFileRevision>,
    settings: TerminalSettings,
    catalog: Catalog,
}

#[derive(Default)]
pub(crate) struct State {
    generation: u64,
    observed: Option<Observation>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Observation {
    revision: Option<RegularFileRevision>,
    catalog: Catalog,
    policy: api::ObservedPolicy,
}

impl State {
    pub(crate) fn observe(
        &mut self,
        prepared: &PreparedRead,
        observed_at_us: u64,
    ) -> Result<api::Snapshot, String> {
        let observation = Observation {
            revision: prepared.revision.clone(),
            catalog: prepared.catalog.clone(),
            policy: prepared.policy.clone(),
        };
        if self.observed.as_ref() != Some(&observation) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("terminal launch-policy generation exhausted")?;
            self.observed = Some(observation);
        }
        Ok(snapshot(
            self.generation,
            observed_at_us,
            &prepared.catalog,
            prepared.policy.clone(),
        ))
    }

    pub(crate) fn validate(
        &self,
        prepared: &PreparedChange,
        transaction: &api::Transaction,
        now: Instant,
    ) -> Result<(), String> {
        prepared.ensure_current(now)?;
        let expected = Observation {
            revision: prepared.prior.revision.clone(),
            catalog: prepared.prior.catalog.clone(),
            policy: prepared.prior.policy.clone(),
        };
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref() != Some(&expected)
            || transaction.prior != prepared.prior.policy
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
        let policy = observed_policy(&committed.settings, &committed.catalog);
        self.generation = self
            .generation
            .checked_add(1)
            .ok_or("terminal launch-policy generation exhausted")?;
        self.observed = Some(Observation {
            revision: committed.revision.clone(),
            catalog: committed.catalog.clone(),
            policy: policy.clone(),
        });
        Ok(snapshot(
            self.generation,
            observed_at_us,
            &committed.catalog,
            policy,
        ))
    }
}

fn snapshot(
    generation: u64,
    observed_at_us: u64,
    catalog: &Catalog,
    policy: api::ObservedPolicy,
) -> api::Snapshot {
    api::Snapshot {
        generation,
        observed_at_us,
        available_shells: catalog.public_shells(),
        available_directories: catalog.public_directories(),
        policy,
        applies_to_new_terminals: true,
    }
}

pub(crate) fn outcome(snapshot: api::Snapshot) -> api::TransactionOutcome {
    api::TransactionOutcome {
        applies_to_new_terminals: snapshot.applies_to_new_terminals,
        existing_terminals_changed: false,
        snapshot,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture_catalog(root: &Path) -> Catalog {
        let shell = root.join("shell");
        std::fs::write(&shell, "fixture shell").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&shell, std::fs::Permissions::from_mode(0o700)).unwrap();
        }
        let directory = root.join("home");
        std::fs::create_dir(&directory).unwrap();
        Catalog {
            shells: vec![ShellEntry {
                identity: api::ShellIdentity::Bash,
                executable: fs::canonicalize(&shell).unwrap(),
                revision: regular_file_revision(&shell).unwrap().unwrap(),
            }],
            directories: vec![DirectoryEntry {
                identity: api::DirectoryIdentity::Home,
                path: fs::canonicalize(&directory).unwrap(),
            }],
        }
    }

    fn read_at(path: PathBuf, catalog: Catalog) -> PreparedRead {
        let settings = TerminalSettings::load(&path).unwrap();
        PreparedRead {
            started_at: Instant::now(),
            completed_at: Instant::now(),
            revision: regular_file_revision(&path).unwrap(),
            policy: observed_policy(&settings, &catalog),
            path,
            settings,
            catalog,
        }
    }

    #[test]
    fn typed_change_resolves_private_catalog_values_and_preserves_presentation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("terminal-settings");
        let settings = TerminalSettings {
            font_family: "Iosevka".into(),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let catalog = fixture_catalog(root.path());
        let read = read_at(path.clone(), catalog.clone());
        let mut state = State::default();
        let snapshot = state.observe(&read, 1).unwrap();
        let transaction = api::Transaction {
            generation: snapshot.generation,
            prior: snapshot.policy,
            requested: api::Policy {
                default_shell: Some(api::ShellIdentity::Bash),
                initial_directory: Some(api::DirectoryIdentity::Home),
            },
        };
        let denied = PreparedChange::from_read(read, &transaction).unwrap();
        assert!(
            denied
                .commit_with_catalog_check(
                    Instant::now() + Duration::from_secs(1),
                    || Ok(catalog.clone()),
                    || Err("revoked".into()),
                )
                .is_err()
        );
        assert_eq!(TerminalSettings::load(&path).unwrap(), settings);

        let prepared =
            PreparedChange::from_read(read_at(path.clone(), catalog.clone()), &transaction)
                .unwrap();
        let committed = prepared
            .commit_with_catalog_check(
                Instant::now() + Duration::from_secs(1),
                || Ok(catalog.clone()),
                || Ok(()),
            )
            .unwrap();
        let result = outcome(state.observe_committed(&committed, 2).unwrap());
        assert!(result.applies_to_new_terminals);
        assert!(!result.existing_terminals_changed);
        let actual = TerminalSettings::load(path).unwrap();
        assert_eq!(actual.font_family, "Iosevka");
        assert_eq!(
            fs::canonicalize(actual.default_shell.unwrap()).unwrap(),
            committed.catalog.shells[0].executable
        );
        assert_eq!(
            fs::canonicalize(actual.initial_working_directory.unwrap()).unwrap(),
            committed.catalog.directories[0].path
        );
    }

    #[test]
    fn unknown_catalog_identity_and_private_values_fail_closed() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("terminal-settings");
        TerminalSettings {
            default_shell: Some("/private/not-catalogued".into()),
            initial_working_directory: Some("/private/not-catalogued".into()),
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        let read = read_at(path, fixture_catalog(root.path()));
        assert!(read.policy.shell_unavailable);
        assert!(read.policy.directory_unavailable);
        let transaction = api::Transaction {
            generation: 1,
            prior: read.policy.clone(),
            requested: api::Policy {
                default_shell: Some(api::ShellIdentity::Fish),
                initial_directory: None,
            },
        };
        assert!(PreparedChange::from_read(read, &transaction).is_err());
    }
}
