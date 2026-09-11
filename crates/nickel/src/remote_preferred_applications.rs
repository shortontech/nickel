//! Shared bounded preferred-application settings preparation.

use nickel_core::shell_settings::{ShellSettings, settings_path};
use nickel_remote_control::preferred_applications as api;
use nickel_storage::{RegularFileRevision, TransactionLock, regular_file_revision};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io,
    path::PathBuf,
};

const STALE: &str = "preferred applications or installed catalog changed; read current state";
const UNAVAILABLE: &str = "preferred applications unavailable; read current state before retrying";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Catalog {
    pub(crate) generation: u64,
    entries: Vec<(String, String)>,
}

impl Catalog {
    pub(crate) fn new(
        generation: u64,
        entries: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Self, String> {
        if generation == 0 {
            return Err("installed catalog unavailable".into());
        }
        let mut retained = Vec::new();
        for (id, command) in entries {
            if retained.len() == api::MAX_CATALOG_ENTRIES {
                return Err("installed catalog exceeds preferred-application bound".into());
            }
            if !api::valid_id(&id)
                || command.is_empty()
                || command.len() > 4096
                || command.chars().any(char::is_control)
            {
                continue;
            }
            if retained
                .iter()
                .any(|(known, _): &(String, String)| known == &id)
            {
                return Err("installed catalog is ambiguous".into());
            }
            retained.push((id, command));
        }
        Ok(Self {
            generation,
            entries: retained,
        })
    }
    fn command(&self, id: &str) -> Option<&str> {
        self.entries
            .iter()
            .find(|(known, _)| known == id)
            .map(|(_, command)| command.as_str())
    }
    fn id_for_command(&self, command: Option<&str>) -> api::Choice {
        let Some(command) = command else {
            return api::Choice {
                application_id: None,
                unavailable: false,
            };
        };
        let mut matches = self
            .entries
            .iter()
            .filter(|(_, candidate)| candidate == command);
        let first = matches.next();
        if first.is_some() && matches.next().is_none() {
            api::Choice {
                application_id: first.map(|(id, _)| id.clone()),
                unavailable: false,
            }
        } else {
            api::Choice {
                application_id: None,
                unavailable: true,
            }
        }
    }
    fn ids(&self) -> Vec<String> {
        self.entries.iter().map(|(id, _)| id.clone()).collect()
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn production_catalog() -> Result<Catalog, String> {
    let (generation, applications, _) = crate::platform::installed_application_signatures();
    Catalog::new(
        generation,
        applications.iter().filter_map(|application| {
            let command = application.launch_command()?.first()?.clone();
            Some((application.id().to_owned(), command))
        }),
    )
}

pub(crate) struct PreparedRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: ShellSettings,
    catalog: Catalog,
}
impl PreparedRead {
    pub(crate) fn prepare(catalog: Catalog) -> Result<Self, String> {
        Self::at(settings_path().map_err(|_| UNAVAILABLE)?, catalog)
    }
    fn at(path: PathBuf, catalog: Catalog) -> Result<Self, String> {
        let revision = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = match ShellSettings::load_for_update(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                ShellSettings::default()
            }
            Err(_) => return Err(UNAVAILABLE.into()),
        };
        if regular_file_revision(&path).map_err(|_| UNAVAILABLE)? != revision {
            return Err(STALE.into());
        }
        Ok(Self {
            path,
            revision,
            settings,
            catalog,
        })
    }
    pub(crate) fn ensure_current(&self, catalog: &Catalog) -> Result<(), String> {
        if &self.catalog != catalog
            || regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision
        {
            Err(STALE.into())
        } else {
            Ok(())
        }
    }
    fn selection(&self) -> api::Selection {
        api::Selection {
            terminal_application_id: self
                .catalog
                .id_for_command(self.settings.preferred_terminal.as_deref())
                .application_id,
            file_manager_application_id: self
                .catalog
                .id_for_command(self.settings.preferred_file_manager.as_deref())
                .application_id,
        }
    }
    fn token(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.catalog.generation.hash(&mut h);
        // The opaque wire generation binds the observation to the exact native
        // file revision without exposing its timestamps or file identity.
        format!("{:?}", self.revision).hash(&mut h);
        self.settings.preferred_terminal.hash(&mut h);
        self.settings.preferred_file_manager.hash(&mut h);
        h.finish().max(1)
    }
    pub(crate) fn snapshot(&self, observed_at_us: u64) -> api::Snapshot {
        api::Snapshot {
            generation: self.token(),
            catalog_generation: self.catalog.generation,
            observed_at_us,
            terminal: self
                .catalog
                .id_for_command(self.settings.preferred_terminal.as_deref()),
            file_manager: self
                .catalog
                .id_for_command(self.settings.preferred_file_manager.as_deref()),
            available_applications: self.catalog.ids(),
        }
    }
}

pub(crate) struct PreparedChange {
    prior: PreparedRead,
    staged: nickel_storage::StagedWrite,
    _lock: TransactionLock,
}
impl PreparedChange {
    pub(crate) fn prepare(
        catalog: Catalog,
        transaction: &api::Transaction,
    ) -> Result<Self, String> {
        Self::prepare_at(
            settings_path().map_err(|_| UNAVAILABLE)?,
            catalog,
            transaction,
        )
    }

    fn prepare_at(
        path: PathBuf,
        catalog: Catalog,
        transaction: &api::Transaction,
    ) -> Result<Self, String> {
        if !transaction.valid() {
            return Err("invalid preferred-applications transaction".into());
        }
        let lock = TransactionLock::try_acquire(&path).map_err(|_| UNAVAILABLE)?;
        let prior = PreparedRead::at(path.clone(), catalog)?;
        if prior.token() != transaction.generation
            || prior.catalog.generation != transaction.catalog_generation
            || prior.selection() != transaction.prior
        {
            return Err(STALE.into());
        }
        let mut requested = prior.settings.clone();
        requested.preferred_terminal = resolve(
            &prior.catalog,
            transaction.requested.terminal_application_id.as_deref(),
        )?;
        requested.preferred_file_manager = resolve(
            &prior.catalog,
            transaction.requested.file_manager_application_id.as_deref(),
        )?;
        let staged = requested.stage(&path).map_err(|_| UNAVAILABLE)?;
        Ok(Self {
            prior,
            staged,
            _lock: lock,
        })
    }
    pub(crate) fn ensure_current(&self, catalog: &Catalog) -> Result<(), String> {
        self.prior.ensure_current(catalog)
    }
    pub(crate) fn commit(
        self,
        deadline: std::time::Instant,
        check: impl FnOnce() -> Result<(), String>,
    ) -> Result<PreparedRead, String> {
        let path = self.prior.path.clone();
        let catalog = self.prior.catalog.clone();
        let prior_revision = self.prior.revision.clone();
        self.staged
            .commit(|| {
                if std::time::Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "preferred application commit expired",
                    ));
                }
                if regular_file_revision(&path)? != prior_revision {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, STALE));
                }
                check().map_err(io::Error::other)
            })
            .map_err(|_| UNAVAILABLE)?;
        PreparedRead::at(path, catalog)
    }
}
fn resolve(catalog: &Catalog, id: Option<&str>) -> Result<Option<String>, String> {
    id.map(|id| {
        catalog
            .command(id)
            .map(str::to_owned)
            .ok_or_else(|| "installed application is unavailable".to_owned())
    })
    .transpose()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn projection_never_exposes_commands_and_system_default_round_trips() {
        let catalog = Catalog::new(
            4,
            [
                ("term".into(), "/private/terminal".into()),
                ("files".into(), "/private/files".into()),
            ],
        )
        .unwrap();
        let settings = ShellSettings {
            preferred_terminal: Some("/private/terminal".into()),
            ..ShellSettings::default()
        };
        let read = PreparedRead {
            path: "missing".into(),
            revision: None,
            settings,
            catalog,
        };
        let json = serde_json::to_string(&read.snapshot(9)).unwrap();
        assert!(json.contains("term"));
        assert!(!json.contains("/private"));
    }

    #[test]
    fn transaction_preserves_unrelated_and_protected_settings() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("shell-settings");
        let original = ShellSettings {
            idle_lock_seconds: Some(73),
            accent_hue: Some(217),
            preferred_terminal: Some("/private/old".into()),
            ..ShellSettings::default()
        };
        original.save(&path).unwrap();
        let catalog = Catalog::new(
            4,
            [
                ("terminal".into(), "/private/new-terminal".into()),
                ("files".into(), "/private/files".into()),
            ],
        )
        .unwrap();
        let read = PreparedRead::at(path.clone(), catalog.clone()).unwrap();
        let transaction = api::Transaction {
            generation: read.token(),
            catalog_generation: 4,
            prior: read.selection(),
            requested: api::Selection {
                terminal_application_id: Some("terminal".into()),
                file_manager_application_id: None,
            },
        };
        let committed = PreparedChange::prepare_at(path.clone(), catalog, &transaction)
            .unwrap()
            .commit(
                std::time::Instant::now() + std::time::Duration::from_secs(1),
                || Ok(()),
            )
            .unwrap();
        let actual = ShellSettings::load(&path).unwrap();
        assert_eq!(
            actual.preferred_terminal.as_deref(),
            Some("/private/new-terminal")
        );
        assert_eq!(actual.preferred_file_manager, None);
        assert_eq!(actual.idle_lock_seconds, Some(73));
        assert_eq!(actual.accent_hue, Some(217));
        let json = serde_json::to_string(&committed.snapshot(11)).unwrap();
        assert!(!json.contains("/private"));
    }
}
