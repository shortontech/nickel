//! Bounded launcher-favorites preparation shared by the Windows desktop owner.
//!
//! Storage reads and staging happen before the winit owner receives the final
//! request. The owner supplies and later revalidates the production launcher's
//! exact application-ID catalog before committing the staged replacement.

use nickel_core::launcher_preferences::{
    LauncherPreferences, PreparedLauncherPreferences, preferences_path,
};
use nickel_remote_control::launcher_favorites::{self as api, Change, Snapshot, Transaction};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{collections::HashSet, io, path::PathBuf, time::Instant};

const MAX_CATALOG_APPLICATIONS: usize = 4_096;
const STALE: &str = "launcher favorites or application catalog changed; read current favorites";
const UNAVAILABLE: &str = "launcher favorites unavailable; read current state before retrying";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct Catalog {
    generation: u64,
    application_ids: Vec<String>,
}

impl Catalog {
    pub(crate) fn current(
        generation: u64,
        application_ids: impl IntoIterator<Item = String>,
    ) -> Result<Self, String> {
        if generation == 0 {
            return Err("Windows launcher catalog generation is unavailable".into());
        }
        let mut retained = Vec::new();
        for id in application_ids {
            if retained.len() == MAX_CATALOG_APPLICATIONS {
                return Err("Windows launcher application catalog exceeds its bound".into());
            }
            // IDs outside the remote schema remain valid local launcher entries,
            // but cannot be disclosed or targeted through this API.
            if id.is_empty()
                || id.len() > api::MAX_APPLICATION_ID_BYTES
                || id.chars().any(char::is_control)
            {
                continue;
            }
            if retained
                .iter()
                .any(|known: &String| known.eq_ignore_ascii_case(&id))
            {
                return Err("Windows launcher application catalog is ambiguous".into());
            }
            retained.push(id);
        }
        Ok(Self {
            generation,
            application_ids: retained,
        })
    }

    fn exact(&self, id: &str) -> Option<&str> {
        self.application_ids
            .iter()
            .find(|candidate| candidate.as_str() == id)
            .map(String::as_str)
    }

    fn canonical(&self, stored: &str) -> Option<&str> {
        self.application_ids
            .iter()
            .find(|candidate| candidate.eq_ignore_ascii_case(stored))
            .map(String::as_str)
    }
}

fn projection(preferences: &LauncherPreferences, catalog: &Catalog) -> (Vec<String>, usize) {
    let mut favorites = Vec::new();
    let mut unavailable = 0;
    for stored in preferences.favorites() {
        match catalog.canonical(stored) {
            Some(id) if !favorites.iter().any(|entry| entry == id) => favorites.push(id.to_owned()),
            _ => unavailable += 1,
        }
    }
    (favorites, unavailable)
}

pub(crate) struct PreparedRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    preferences: LauncherPreferences,
    catalog: Catalog,
    favorites: Vec<String>,
    unavailable: usize,
}

impl PreparedRead {
    pub(crate) fn prepare(catalog: Catalog) -> Result<Self, String> {
        Self::at(preferences_path().map_err(|_| UNAVAILABLE)?, catalog)
    }

    fn at(path: PathBuf, catalog: Catalog) -> Result<Self, String> {
        let before = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let preferences = match LauncherPreferences::load(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound && before.is_none() => {
                LauncherPreferences::default()
            }
            Err(_) => return Err(UNAVAILABLE.into()),
        };
        let (favorites, unavailable) = projection(&preferences, &catalog);
        if regular_file_revision(&path).map_err(|_| UNAVAILABLE)? != before {
            return Err(STALE.into());
        }
        Ok(Self {
            path,
            revision: before,
            preferences,
            catalog,
            favorites,
            unavailable,
        })
    }

    pub(crate) fn ensure_current(&self, catalog: &Catalog) -> Result<(), String> {
        if catalog != &self.catalog
            || regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision
        {
            return Err(STALE.into());
        }
        Ok(())
    }

    pub(crate) fn preferences(&self) -> &LauncherPreferences {
        &self.preferences
    }
}

pub(crate) struct PreparedChange {
    prior: PreparedRead,
    staged: PreparedLauncherPreferences,
}

impl PreparedChange {
    pub(crate) fn prepare(catalog: Catalog, transaction: &Transaction) -> Result<Self, String> {
        if !transaction.valid() {
            return Err("invalid favorites transaction".into());
        }
        let prior = PreparedRead::prepare(catalog)?;
        if prior.catalog.generation != transaction.catalog_generation
            || prior.favorites != transaction.prior
        {
            return Err(STALE.into());
        }
        let requested =
            changed_preferences(&prior.preferences, &prior.catalog, &transaction.change)?;
        let staged =
            PreparedLauncherPreferences::prepare(prior.path.clone(), &prior.preferences, requested)
                .map_err(|_| STALE)?;
        prior.ensure_current(&prior.catalog)?;
        Ok(Self { prior, staged })
    }

    pub(crate) fn ensure_current(&self, catalog: &Catalog) -> Result<(), String> {
        self.prior.ensure_current(catalog)
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<Committed, String> {
        let path = self.prior.path.clone();
        let catalog = self.prior.catalog.clone();
        let preferences = self
            .staged
            .commit(|| {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "launcher favorites commit expired",
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
        let revision = regular_file_revision(&path).map_err(|_| UNAVAILABLE.into());
        Ok(Committed {
            path,
            revision,
            preferences,
            catalog,
        })
    }
}

pub(crate) struct Committed {
    path: PathBuf,
    revision: Result<Option<RegularFileRevision>, String>,
    preferences: LauncherPreferences,
    catalog: Catalog,
}

impl Committed {
    pub(crate) fn preferences(&self) -> &LauncherPreferences {
        &self.preferences
    }

    pub(crate) fn into_observation(self) -> Result<PreparedRead, String> {
        let revision = self.revision?;
        let (favorites, unavailable) = projection(&self.preferences, &self.catalog);
        Ok(PreparedRead {
            path: self.path,
            revision,
            preferences: self.preferences,
            catalog: self.catalog,
            favorites,
            unavailable,
        })
    }
}

fn changed_preferences(
    prior: &LauncherPreferences,
    catalog: &Catalog,
    change: &Change,
) -> Result<LauncherPreferences, String> {
    let mut favorites = prior.favorites().to_vec();
    match change {
        Change::Add { application_id } => {
            let id = catalog
                .exact(application_id)
                .ok_or("installed application is unavailable")?;
            if !favorites
                .iter()
                .any(|stored| stored.eq_ignore_ascii_case(id))
            {
                if favorites.len() == api::MAX_FAVORITES {
                    return Err("favorite limit reached".into());
                }
                favorites.push(id.to_owned());
            }
        }
        Change::Remove { application_id } => {
            let id = catalog
                .exact(application_id)
                .ok_or("installed application is unavailable")?;
            favorites.retain(|stored| !stored.eq_ignore_ascii_case(id));
        }
        Change::Reorder { application_ids } => {
            if !api::valid_ids(application_ids) {
                return Err("invalid favorites reorder".into());
            }
            let (visible, _) = projection(prior, catalog);
            if application_ids.len() != visible.len()
                || !application_ids.iter().all(|id| visible.contains(id))
            {
                return Err(
                    "reorder must contain each current installed favorite exactly once".into(),
                );
            }
            let mut ordered = application_ids.iter();
            let mut emitted = HashSet::new();
            for stored in &mut favorites {
                if let Some(id) = catalog.canonical(stored)
                    && visible.iter().any(|visible| visible == id)
                    && emitted.insert(id.to_owned())
                {
                    *stored = ordered.next().ok_or(STALE)?.clone();
                }
            }
        }
    }
    let mut requested = prior.clone();
    requested.replace_favorites(favorites);
    Ok(requested)
}

#[derive(Default)]
pub(crate) struct FavoritesState {
    generation: u64,
    observed: Option<(Option<RegularFileRevision>, Catalog, Vec<String>, usize)>,
}

impl FavoritesState {
    pub(crate) fn observe(
        &mut self,
        prepared: &PreparedRead,
        observed_at_us: u64,
        runtime_applied: bool,
    ) -> Result<Snapshot, String> {
        let observed = (
            prepared.revision.clone(),
            prepared.catalog.clone(),
            prepared.favorites.clone(),
            prepared.unavailable,
        );
        if self.observed.as_ref() != Some(&observed) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("favorites generation exhausted")?;
            self.observed = Some(observed);
        }
        Ok(Snapshot {
            generation: self.generation,
            catalog_generation: prepared.catalog.generation,
            observed_at_us,
            favorites: prepared.favorites.clone(),
            unavailable_favorites: prepared.unavailable,
            runtime_applied,
        })
    }

    pub(crate) fn validate(
        &self,
        prepared: &PreparedChange,
        transaction: &Transaction,
    ) -> Result<(), String> {
        let prior = &prepared.prior;
        let expected = (
            prior.revision.clone(),
            prior.catalog.clone(),
            prior.favorites.clone(),
            prior.unavailable,
        );
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref() != Some(&expected)
        {
            return Err(STALE.into());
        }
        Ok(())
    }

    pub(crate) fn invalidate(&mut self) {
        self.observed = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn catalog(generation: u64) -> Catalog {
        Catalog::current(generation, ["One", "Two"].map(str::to_owned)).unwrap()
    }

    #[test]
    fn transaction_preserves_hidden_favorites_and_recent_history() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("launcher.conf");
        let mut preferences = LauncherPreferences::default();
        preferences.replace_favorites(["private-unavailable".into(), "one".into()]);
        preferences.record_launch("private-history-canary");
        preferences.save(&path).unwrap();

        let read = PreparedRead::at(path.clone(), catalog(7)).unwrap();
        let mut state = FavoritesState::default();
        let snapshot = state.observe(&read, 10, true).unwrap();
        assert_eq!(snapshot.favorites, ["One"]);
        assert_eq!(snapshot.unavailable_favorites, 1);
        let transaction = Transaction {
            generation: snapshot.generation,
            catalog_generation: snapshot.catalog_generation,
            prior: snapshot.favorites,
            change: Change::Add {
                application_id: "Two".into(),
            },
        };
        let prepared = PreparedChange::prepare_at(path.clone(), catalog(7), &transaction).unwrap();
        state.validate(&prepared, &transaction).unwrap();
        let committed = prepared
            .commit(
                Instant::now() + std::time::Duration::from_secs(1),
                || Ok(()),
            )
            .unwrap();
        let observation = committed.into_observation().unwrap();
        let result = state.observe(&observation, 20, true).unwrap();
        assert_eq!(result.favorites, ["One", "Two"]);
        let persisted = LauncherPreferences::load(path).unwrap();
        assert_eq!(persisted.favorites(), ["private-unavailable", "one", "Two"]);
        assert_eq!(persisted.recents(), ["private-history-canary"]);
    }

    #[test]
    fn preparation_holds_cooperative_lock_and_commit_checks_boundary() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("launcher.conf");
        LauncherPreferences::default().save(&path).unwrap();
        let read = PreparedRead::at(path.clone(), catalog(2)).unwrap();
        let mut state = FavoritesState::default();
        let snapshot = state.observe(&read, 1, true).unwrap();
        let transaction = Transaction {
            generation: snapshot.generation,
            catalog_generation: snapshot.catalog_generation,
            prior: Vec::new(),
            change: Change::Add {
                application_id: "One".into(),
            },
        };
        let prepared = PreparedChange::prepare_at(path.clone(), catalog(2), &transaction).unwrap();
        assert!(PreparedChange::prepare_at(path.clone(), catalog(2), &transaction).is_err());
        assert!(
            prepared
                .commit(Instant::now() + std::time::Duration::from_secs(1), || Err(
                    "revoked".into()
                ))
                .is_err()
        );
        assert!(
            LauncherPreferences::load(path)
                .unwrap()
                .favorites()
                .is_empty()
        );
    }

    #[test]
    fn state_rejects_file_and_catalog_aba() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("launcher.conf");
        LauncherPreferences::default().save(&path).unwrap();
        let read = PreparedRead::at(path.clone(), catalog(3)).unwrap();
        let mut state = FavoritesState::default();
        state.observe(&read, 1, true).unwrap();
        let original = LauncherPreferences::load(&path).unwrap();
        let mut changed = original.clone();
        changed.record_launch("changed-and-restored");
        changed.save(&path).unwrap();
        original.save(&path).unwrap();
        assert!(read.ensure_current(&catalog(3)).is_err());

        let fresh = PreparedRead::at(path.clone(), catalog(3)).unwrap();
        let snapshot = state.observe(&fresh, 2, true).unwrap();
        let transaction = Transaction {
            generation: snapshot.generation,
            catalog_generation: snapshot.catalog_generation,
            prior: Vec::new(),
            change: Change::Add {
                application_id: "One".into(),
            },
        };
        let prepared = PreparedChange::prepare_at(path.clone(), catalog(3), &transaction).unwrap();
        assert!(prepared.ensure_current(&catalog(4)).is_err());
        let changed_catalog = PreparedRead::at(path, catalog(4)).unwrap();
        state.observe(&changed_catalog, 3, true).unwrap();
        assert!(state.validate(&prepared, &transaction).is_err());
    }

    impl PreparedChange {
        fn prepare_at(
            path: PathBuf,
            catalog: Catalog,
            transaction: &Transaction,
        ) -> Result<Self, String> {
            if !transaction.valid() {
                return Err("invalid favorites transaction".into());
            }
            let prior = PreparedRead::at(path, catalog)?;
            if prior.catalog.generation != transaction.catalog_generation
                || prior.favorites != transaction.prior
            {
                return Err(STALE.into());
            }
            let requested =
                changed_preferences(&prior.preferences, &prior.catalog, &transaction.change)?;
            let staged = PreparedLauncherPreferences::prepare(
                prior.path.clone(),
                &prior.preferences,
                requested,
            )
            .map_err(|_| STALE)?;
            prior.ensure_current(&prior.catalog)?;
            Ok(Self { prior, staged })
        }
    }
}
