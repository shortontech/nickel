//! Wallpaper settings are prepared off-thread and committed by the desktop owner.
use super::NickelSession;
use crate::wallpaper_selection::{CandidateRevision, Catalog, ValidatedSelection};
use nickel_core::wallpaper_settings::{
    PreparedWallpaperSettings, WallpaperPosition, WallpaperSettings, settings_path,
};
use nickel_remote_control::{
    DesktopPermit,
    wallpaper::{Change, Position, Preferences, Snapshot, Transaction},
};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{io, path::PathBuf};

const STALE: &str = "wallpaper changed; read current wallpaper before retrying";
const UNAVAILABLE: &str = "wallpaper unavailable; read current state before retrying";

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

pub(super) struct PreparedRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: WallpaperSettings,
    catalog: Catalog,
}

impl PreparedRead {
    pub(super) fn prepare_with_check(
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        check()?;
        let path = settings_path().map_err(|_| UNAVAILABLE)?;
        let catalog = Catalog::discover(&mut check)?;
        check()?;
        Self::at_with_catalog(path, catalog)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        Self::at_with_catalog(path, Catalog::discover(|| Ok(()))?)
    }

    fn at_with_catalog(path: PathBuf, catalog: Catalog) -> Result<Self, String> {
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

    fn current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision {
            return Err(STALE.into());
        }
        Ok(())
    }
}

pub(super) struct PreparedChange {
    prior: PreparedRead,
    staged: PreparedWallpaperSettings,
    selected: Option<ValidatedSelection>,
}

impl PreparedChange {
    pub(super) fn prepare_with_check(
        transaction: &Transaction,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<Self, String> {
        if transaction.generation == 0 {
            return Err(STALE.into());
        }
        let prior = PreparedRead::prepare_with_check(&mut check)?;
        Self::from_read_with_check(prior, transaction, &mut check)
    }

    fn from_read_with_check(
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

    fn commit(self, check: impl FnOnce() -> io::Result<()>) -> io::Result<WallpaperSettings> {
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
pub(super) struct WallpaperState {
    generation: u64,
    observed: Option<(
        Option<RegularFileRevision>,
        Preferences,
        Vec<CandidateRevision>,
    )>,
}

impl WallpaperState {
    fn observe(&mut self, read: &PreparedRead, observed_at_us: u64) -> Result<Snapshot, String> {
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

    fn validate(&self, prepared: &PreparedChange, transaction: &Transaction) -> Result<(), String> {
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

impl NickelSession {
    pub(super) fn remote_read_wallpaper(
        &mut self,
        permit: &DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.current()?;
        permit.with_debug(self.locked || self.shell_recovery_visible(), || {
            let observed_at_us = self
                .start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64;
            self.remote_wallpaper.observe(&prepared, observed_at_us)
        })
    }

    pub(super) fn remote_change_wallpaper(
        &mut self,
        permit: &DesktopPermit,
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
        let path = prepared.prior.path.clone();
        let mut committed = None;
        let authorized = permit.with_debug_input_deadline(protected, |boundary| {
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
            self.remote_wallpaper.validate(&prepared, &transaction)?;
            committed = Some(
                prepared
                    .commit(|| {
                        permit
                            .check_commit_boundary(boundary)
                            .map_err(io::Error::other)
                    })
                    .map_err(|_| UNAVAILABLE)?,
            );
            self.remote_wallpaper.observed = None;
            Ok(())
        });
        let requested = match committed {
            Some(requested) => requested,
            None => {
                authorized?;
                return Err(UNAVAILABLE.into());
            }
        };
        // Reconcile an accepted rename even if response authorization raced it.
        self.notify_shell_settings_changed();
        authorized?;
        let read = PreparedRead::at(path)?;
        if read.settings != requested {
            return Err(STALE.into());
        }
        let observed_at_us = self
            .start_time
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX)) as u64;
        let mut snapshot = self.remote_wallpaper.observe(&read, observed_at_us)?;
        snapshot.selected_image_decoded =
            matches!(transaction.change, Change::SelectApprovedImage { .. });
        snapshot.runtime_reload_requested = true;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
