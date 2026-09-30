//! Presentation-independent configuration service for authorized capability callers.
//! Reads and writes use staged native storage transactions. Callers enforce package
//! grants, validate the observation before commit, recheck authority in the commit
//! callback, and reconcile committed values with the shell even if a reply fails.
use crate::wallpaper_selection::{CandidateRevision, Catalog, ValidatedSelection};
use nickel_core::wallpaper_settings::{
    PreparedWallpaperSettings, WallpaperPosition, WallpaperSettings, settings_path,
};
pub use nickel_remote_control::wallpaper::{Change, Position, Preferences, Snapshot, Transaction};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{io, path::PathBuf};

pub(crate) const STALE: &str = "wallpaper changed; read current wallpaper before retrying";
pub(crate) const UNAVAILABLE: &str = "wallpaper unavailable; read current state before retrying";

fn position(value: WallpaperPosition) -> Position {
    match value {
        WallpaperPosition::Center => Position::Center,
        WallpaperPosition::Tile => Position::Tile,
        WallpaperPosition::Stretch => Position::Stretch,
        WallpaperPosition::Fit => Position::Fit,
        WallpaperPosition::Span => Position::Span,
        WallpaperPosition::Fill => Position::Fill,
    }
}

fn core_position(value: Position) -> WallpaperPosition {
    match value {
        Position::Center => WallpaperPosition::Center,
        Position::Tile => WallpaperPosition::Tile,
        Position::Stretch => WallpaperPosition::Stretch,
        Position::Fit => WallpaperPosition::Fit,
        Position::Span => WallpaperPosition::Span,
        Position::Fill => WallpaperPosition::Fill,
    }
}

fn preferences(settings: &WallpaperSettings) -> Preferences {
    Preferences {
        custom_image_configured: settings.image.is_some(),
        position: position(settings.position),
    }
}

pub struct PreparedRead {
    pub(crate) path: PathBuf,
    revision: Option<RegularFileRevision>,
    pub(crate) settings: WallpaperSettings,
    catalog: Catalog,
}

impl PreparedRead {
    pub fn prepare_with_check(
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        check()?;
        let path = settings_path().map_err(|_| UNAVAILABLE)?;
        let catalog = Catalog::discover(&mut check)?;
        check()?;
        Self::at_with_catalog(path, catalog)
    }

    pub fn at(path: PathBuf) -> Result<Self, String> {
        Self::at_with_catalog(path, Catalog::discover(|| Ok(()))?)
    }

    pub(crate) fn at_with_catalog(path: PathBuf, catalog: Catalog) -> Result<Self, String> {
        let revision = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = match WallpaperSettings::load(&path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                WallpaperSettings::default()
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

    pub fn current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision {
            return Err(STALE.into());
        }
        Ok(())
    }
}

pub struct PreparedChange {
    pub(crate) prior: PreparedRead,
    staged: PreparedWallpaperSettings,
    selected: Option<ValidatedSelection>,
}

impl PreparedChange {
    pub fn prepare_with_check(
        transaction: &Transaction,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        if transaction.generation == 0 {
            return Err(STALE.into());
        }
        let prior = PreparedRead::prepare_with_check(&mut check)?;
        Self::from_read_with_check(prior, transaction, &mut check)
    }

    #[cfg(test)]
    pub fn from_read(prior: PreparedRead, transaction: &Transaction) -> Result<Self, String> {
        Self::from_read_with_check(prior, transaction, &mut || Ok(()))
    }

    pub fn from_read_with_check(
        prior: PreparedRead,
        transaction: &Transaction,
        check: &mut impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        if preferences(&prior.settings) != transaction.prior {
            return Err(STALE.into());
        }
        let mut requested = prior.settings.clone();
        let mut selected = None;
        match &transaction.change {
            Change::SetPosition { position } => requested.position = core_position(*position),
            Change::ResetCustomImage {} => requested.image = None,
            Change::SelectApprovedImage { image_id } => {
                let validated = prior.catalog.validate_selection(image_id, check)?;
                requested.image = Some(validated.path.clone());
                selected = Some(validated);
            }
        }
        let staged =
            PreparedWallpaperSettings::prepare(prior.path.clone(), &prior.settings, requested)
                .map_err(|_| STALE)?;
        Ok(Self {
            prior,
            staged,
            selected,
        })
    }

    pub fn commit(self, check: impl FnOnce() -> io::Result<()>) -> io::Result<WallpaperSettings> {
        let selected = self.selected;
        self.staged.commit(|| {
            if let Some(selected) = &selected {
                selected.ensure_current().map_err(io::Error::other)?;
            }
            check()
        })
    }
}

#[derive(Default)]
pub struct WallpaperState {
    generation: u64,
    pub(crate) observed: Option<(
        Option<RegularFileRevision>,
        Preferences,
        Vec<CandidateRevision>,
    )>,
}

impl WallpaperState {
    /// Validate the owner-issued observation and retire it after durable commit.
    pub fn commit(
        &mut self,
        prepared: PreparedChange,
        transaction: &Transaction,
        check_boundary: impl FnOnce() -> io::Result<()>,
    ) -> Result<WallpaperSettings, String> {
        self.validate(&prepared, transaction)?;
        let committed = prepared.commit(check_boundary).map_err(|_| UNAVAILABLE)?;
        self.invalidate();
        Ok(committed)
    }

    /// Retire the accepted observation after a committed write or uncertain result.
    pub fn invalidate(&mut self) {
        self.observed = None;
    }

    pub fn observe(
        &mut self,
        read: &PreparedRead,
        observed_at_us: u64,
    ) -> Result<Snapshot, String> {
        read.current()?;
        let configured = preferences(&read.settings);
        let catalog_revisions = read.catalog.revisions();
        if self.observed.as_ref().is_none_or(|value| {
            value
                != &(
                    read.revision.clone(),
                    configured.clone(),
                    catalog_revisions.clone(),
                )
        }) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("wallpaper generation exhausted")?;
            self.observed = Some((read.revision.clone(), configured.clone(), catalog_revisions));
        }
        Ok(Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
            images: read.catalog.choices(read.settings.image.as_deref()),
            selected_image_decoded: false,
            runtime_reload_requested: false,
        })
    }

    pub fn validate(
        &self,
        prepared: &PreparedChange,
        transaction: &Transaction,
    ) -> Result<(), String> {
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self
                .observed
                .as_ref()
                .is_none_or(|(revision, configured, catalog_revisions)| {
                    revision != &prepared.prior.revision
                        || configured != &transaction.prior
                        || catalog_revisions != &prepared.prior.catalog.revisions()
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

    #[test]
    fn public_service_rejects_stale_commit_and_resets_image_transactionally() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wallpaper-settings");
        let settings = WallpaperSettings {
            image: Some("/private/wallpaper.png".into()),
            position: WallpaperPosition::Fit,
        };
        settings.save(&path).unwrap();
        let mut state = WallpaperState::default();
        let snapshot = state
            .observe(&PreparedRead::at(path.clone()).unwrap(), 1)
            .unwrap();
        let mut transaction = Transaction {
            generation: snapshot.generation,
            prior: snapshot.configured,
            change: Change::ResetCustomImage {},
        };
        let prepared =
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &transaction)
                .unwrap();
        transaction.generation += 1;
        assert!(
            state
                .commit(prepared, &transaction, || panic!(
                    "stale authority reached commit"
                ))
                .is_err()
        );
        assert_eq!(WallpaperSettings::load(&path).unwrap(), settings);
        transaction.generation -= 1;
        let prepared =
            PreparedChange::from_read(PreparedRead::at(path.clone()).unwrap(), &transaction)
                .unwrap();
        let committed = state.commit(prepared, &transaction, || Ok(())).unwrap();
        assert_eq!(committed.image, None);
        assert_eq!(committed.position, WallpaperPosition::Fit);
        let observed = state.observe(&PreparedRead::at(path).unwrap(), 2).unwrap();
        assert_eq!(observed.generation, transaction.generation + 1);
    }

    #[test]
    fn transaction_preserves_hidden_path_and_rejects_file_aba() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("wallpaper-settings");
        let settings = WallpaperSettings {
            image: Some("/private/wallpaper.png".into()),
            position: WallpaperPosition::Fill,
        };
        settings.save(&path).unwrap();
        let read = PreparedRead::at(path.clone()).unwrap();
        let prior = preferences(&settings);
        let transaction = Transaction {
            generation: 1,
            prior: prior.clone(),
            change: Change::SetPosition {
                position: Position::Fit,
            },
        };
        let mut state = WallpaperState::default();
        assert_eq!(state.observe(&read, 1).unwrap().generation, 1);
        let prepared = PreparedChange::from_read(read, &transaction).unwrap();
        state.validate(&prepared, &transaction).unwrap();
        std::fs::write(&path, "image=/private/wallpaper.png\nposition=tile\n").unwrap();
        assert!(prepared.commit(|| Ok(())).is_err());
        assert_eq!(
            WallpaperSettings::load(&path).unwrap().position,
            WallpaperPosition::Tile
        );

        let reset = Transaction {
            generation: 1,
            prior,
            change: Change::ResetCustomImage {},
        };
        assert!(PreparedChange::from_read(PreparedRead::at(path).unwrap(), &reset).is_err());
    }
}
