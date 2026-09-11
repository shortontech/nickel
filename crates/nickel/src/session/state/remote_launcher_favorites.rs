//! Installed favorites preparation is bounded/off-owner; rename retains original authority.
use super::NickelSession;
use nickel_core::launcher_preferences::{
    LauncherPreferences, PreparedLauncherPreferences, preferences_path,
};
use nickel_remote_control::{
    DesktopPermit,
    launcher_favorites::{self as api, Change, Snapshot, Transaction},
};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{path::PathBuf, sync::Arc};
const STALE: &str = "launcher favorites or application catalog changed; read current favorites";
const UNAVAILABLE: &str = "launcher favorites unavailable; read current state before retrying";

pub(super) struct PreparedRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    preferences: LauncherPreferences,
    catalog_generation: u64,
    catalog: Arc<[crate::model::Application]>,
    favorites: Vec<String>,
    unavailable: usize,
}
fn projection(
    preferences: &LauncherPreferences,
    catalog: &[crate::model::Application],
) -> (Vec<String>, usize) {
    let mut favorites = Vec::new();
    let mut unavailable = 0;
    for stored in preferences.favorites() {
        let found = catalog
            .iter()
            .find(|app| app.matches_native_id(stored))
            .map(crate::model::Application::id)
            .filter(|id| {
                !id.is_empty()
                    && id.len() <= api::MAX_APPLICATION_ID_BYTES
                    && !id.chars().any(char::is_control)
            });
        match found {
            Some(id) if !favorites.iter().any(|entry| entry == id) => favorites.push(id.to_owned()),
            _ => unavailable += 1,
        }
    }
    (favorites, unavailable)
}
impl PreparedRead {
    pub fn prepare() -> Result<Self, String> {
        let path = preferences_path().map_err(|_| UNAVAILABLE)?;
        let before = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let preferences = match LauncherPreferences::load(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound && before.is_none() => {
                LauncherPreferences::default()
            }
            Err(_) => return Err(UNAVAILABLE.into()),
        };
        let (catalog_generation, catalog, _) = crate::platform::installed_application_signatures();
        if catalog_generation == 0 {
            return Err("installed catalog unavailable".into());
        }
        let (favorites, unavailable) = projection(&preferences, &catalog);
        if regular_file_revision(&path).map_err(|_| UNAVAILABLE)? != before {
            return Err(STALE.into());
        }
        Ok(Self {
            path,
            revision: before,
            preferences,
            catalog_generation,
            catalog,
            favorites,
            unavailable,
        })
    }
    fn current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision
            || crate::platform::installed_application_signatures().0 != self.catalog_generation
        {
            return Err(STALE.into());
        }
        Ok(())
    }
}
pub(super) struct PreparedChange {
    prior: PreparedRead,
    staged: PreparedLauncherPreferences,
}

pub(super) enum SemanticFavoriteAction {
    Toggle(String),
    MoveLeft(String),
    MoveRight(String),
}

pub(super) struct PreparedSemanticFavorite {
    prior: PreparedRead,
    staged: PreparedLauncherPreferences,
}

impl PreparedSemanticFavorite {
    pub(super) fn prepare(action: SemanticFavoriteAction) -> Result<Self, String> {
        let prior = PreparedRead::prepare()?;
        let (visible, _) = projection(&prior.preferences, &prior.catalog);
        let change = match action {
            SemanticFavoriteAction::Toggle(application_id) => {
                let app = prior
                    .catalog
                    .iter()
                    .find(|app| app.id() == application_id)
                    .ok_or("installed application is unavailable")?;
                if visible.iter().any(|id| id == app.id()) {
                    Change::Remove { application_id }
                } else {
                    Change::Add { application_id }
                }
            }
            SemanticFavoriteAction::MoveLeft(application_id) => Change::Reorder {
                application_ids: moved_favorite(visible, &application_id, -1)?,
            },
            SemanticFavoriteAction::MoveRight(application_id) => Change::Reorder {
                application_ids: moved_favorite(visible, &application_id, 1)?,
            },
        };
        let requested = changed_preferences(&prior.preferences, &prior.catalog, &change)?;
        let staged =
            PreparedLauncherPreferences::prepare(prior.path.clone(), &prior.preferences, requested)
                .map_err(|_| STALE)?;
        prior.current()?;
        Ok(Self { prior, staged })
    }
}

fn moved_favorite(
    mut favorites: Vec<String>,
    application_id: &str,
    direction: isize,
) -> Result<Vec<String>, String> {
    let index = favorites
        .iter()
        .position(|id| id == application_id)
        .ok_or("favorite is unavailable")?;
    let destination = index
        .saturating_add_signed(direction)
        .min(favorites.len() - 1);
    if destination != index {
        favorites.swap(index, destination);
    }
    Ok(favorites)
}
impl PreparedChange {
    pub fn prepare(transaction: &Transaction) -> Result<Self, String> {
        if !transaction.valid() {
            return Err("invalid favorites transaction".into());
        }
        let prior = PreparedRead::prepare()?;
        if prior.catalog_generation != transaction.catalog_generation
            || prior.favorites != transaction.prior
        {
            return Err(STALE.into());
        }
        let requested =
            changed_preferences(&prior.preferences, &prior.catalog, &transaction.change)?;
        let staged =
            PreparedLauncherPreferences::prepare(prior.path.clone(), &prior.preferences, requested)
                .map_err(|_| STALE)?;
        prior.current()?;
        Ok(Self { prior, staged })
    }
}
fn changed_preferences(
    prior: &LauncherPreferences,
    catalog: &[crate::model::Application],
    change: &Change,
) -> Result<LauncherPreferences, String> {
    let exact = |id: &str| {
        catalog
            .iter()
            .find(|app| app.id() == id)
            .ok_or_else(|| "installed application is unavailable".to_owned())
    };
    let mut favorites = prior.favorites().to_vec();
    match change {
        Change::Add { application_id } => {
            let app = exact(application_id)?;
            if !favorites.iter().any(|stored| app.matches_native_id(stored)) {
                if favorites.len() == api::MAX_FAVORITES {
                    return Err("favorite limit reached".into());
                }
                favorites.push(application_id.clone());
            }
        }
        Change::Remove { application_id } => {
            let app = exact(application_id)?;
            favorites.retain(|stored| !app.matches_native_id(stored));
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
            let mut emitted = std::collections::HashSet::new();
            for stored in &mut favorites {
                if let Some(app) = catalog.iter().find(|app| app.matches_native_id(stored))
                    && visible.iter().any(|id| id == app.id())
                    && emitted.insert(app.id())
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
pub(super) struct FavoritesState {
    generation: u64,
    observed: Option<(Option<RegularFileRevision>, u64, Vec<String>, usize)>,
}
impl FavoritesState {
    fn observe(
        &mut self,
        prepared: &PreparedRead,
        time: u64,
        runtime_applied: bool,
    ) -> Result<Snapshot, String> {
        let value = (
            prepared.revision.clone(),
            prepared.catalog_generation,
            prepared.favorites.clone(),
            prepared.unavailable,
        );
        if self.observed.as_ref() != Some(&value) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("favorites generation exhausted")?;
        }
        self.observed = Some(value);
        Ok(Snapshot {
            generation: self.generation,
            catalog_generation: prepared.catalog_generation,
            observed_at_us: time,
            favorites: prepared.favorites.clone(),
            unavailable_favorites: prepared.unavailable,
            runtime_applied,
        })
    }
    fn validate(&self, prepared: &PreparedRead, transaction: &Transaction) -> Result<(), String> {
        let expected = (
            prepared.revision.clone(),
            prepared.catalog_generation,
            prepared.favorites.clone(),
            prepared.unavailable,
        );
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref() != Some(&expected)
        {
            return Err(STALE.into());
        }
        Ok(())
    }
}
impl NickelSession {
    pub(super) fn remote_commit_semantic_favorite(
        &mut self,
        permit: &DesktopPermit,
        origin: &nickel_remote_control::leases::ResourceId,
        output: &nickel_remote_control::leases::ResourceId,
        prepared: PreparedSemanticFavorite,
    ) -> Result<(), String> {
        let controller_busy = self.poll_remote_controller_ownership();
        let ancestors = self.remote_surface_ancestors(origin);
        let evidence = nickel_remote_control::leases::ResourceEvidence {
            window: None,
            surface: Some(origin),
            output: Some(output),
            verified_application: None,
            authorized_surface_ancestors: &ancestors,
            protected: false,
        };
        let prior = prepared.prior;
        let mut committed = None;
        let authorized = permit.with_input_boundary(&evidence, |boundary| {
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
                || self.internal_shell.as_ref().is_some_and(|shell| {
                    shell.pointer_interaction_active() || shell.launcher_preferences_busy()
                })
            {
                return Err("shared input or local preferences are busy".into());
            }
            if self.internal_shell.is_none() {
                return Err(UNAVAILABLE.into());
            }
            prior.current()?;
            committed = Some(
                prepared
                    .staged
                    .commit(|| {
                        permit
                            .check_commit_boundary(boundary)
                            .map_err(std::io::Error::other)
                    })
                    .map_err(|_| UNAVAILABLE)?,
            );
            self.remote_launcher_favorites.observed = None;
            Ok(())
        });
        let Some(preferences) = committed else {
            authorized?;
            return Err(UNAVAILABLE.into());
        };
        let changed = self
            .internal_shell
            .as_mut()
            .ok_or(UNAVAILABLE)?
            .apply_committed_launcher_preferences(preferences)?;
        self.sync_internal_shell_changes(Some(&changed));
        self.schedule_internal_shell_deadline();
        authorized
    }

    pub(super) fn remote_read_launcher_favorites(
        &mut self,
        permit: &DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(self.locked || self.shell_recovery_visible(), || {
            prepared.current()?;
            let shell = self.internal_shell.as_ref().ok_or(UNAVAILABLE)?;
            if shell.launcher_preferences_busy() {
                return Err("local preference action is pending".into());
            }
            let runtime_applied = shell.launcher_favorites_match(&prepared.preferences);
            self.remote_launcher_favorites.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
                runtime_applied,
            )
        })
    }
    pub(super) fn remote_change_launcher_favorites(
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
        let mut committed = None;
        let prior = prepared.prior;
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
                || self.internal_shell.as_ref().is_some_and(|shell| {
                    shell.pointer_interaction_active() || shell.launcher_preferences_busy()
                })
            {
                return Err("shared input or local preferences are busy".into());
            }
            if self.internal_shell.is_none() {
                return Err(UNAVAILABLE.into());
            }
            prior.current()?;
            self.remote_launcher_favorites
                .validate(&prior, &transaction)?;
            committed = Some(
                prepared
                    .staged
                    .commit(|| {
                        permit
                            .check_commit_boundary(boundary)
                            .map_err(std::io::Error::other)
                    })
                    .map_err(|_| UNAVAILABLE)?,
            );
            self.remote_launcher_favorites.observed = None;
            Ok(())
        });
        let Some(preferences) = committed else {
            authorized?;
            return Err(UNAVAILABLE.into());
        };
        let changed = self
            .internal_shell
            .as_mut()
            .ok_or(UNAVAILABLE)?
            .apply_committed_launcher_preferences(preferences.clone())?;
        self.sync_internal_shell_changes(Some(&changed));
        self.schedule_internal_shell_deadline();
        authorized?;
        let revision = regular_file_revision(&prior.path).map_err(|_| UNAVAILABLE)?;
        let (favorites, unavailable) = projection(&preferences, &prior.catalog);
        let observation = PreparedRead {
            preferences,
            revision,
            favorites,
            unavailable,
            ..prior
        };
        self.remote_launcher_favorites.observe(
            &observation,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
            true,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn catalog() -> Arc<[crate::model::Application]> {
        ["one.desktop", "two.desktop"]
            .into_iter()
            .map(|id| crate::model::Application::new(id.into(), id.into(), None, None, None))
            .collect::<Vec<_>>()
            .into()
    }
    #[test]
    fn installed_favorite_changes_preserve_hidden_records_and_reject_other_ids() {
        let catalog = catalog();
        let mut preferences = LauncherPreferences::default();
        preferences.replace_favorites(["place:/private/unavailable".into(), "one.desktop".into()]);
        preferences.record_launch("private-history-canary");
        let added = changed_preferences(
            &preferences,
            &catalog,
            &Change::Add {
                application_id: "two.desktop".into(),
            },
        )
        .unwrap();
        let reordered = changed_preferences(
            &added,
            &catalog,
            &Change::Reorder {
                application_ids: vec!["two.desktop".into(), "one.desktop".into()],
            },
        )
        .unwrap();
        assert_eq!(
            reordered.favorites(),
            ["place:/private/unavailable", "two.desktop", "one.desktop"]
        );
        assert_eq!(reordered.recents(), ["private-history-canary"]);
        let removed = changed_preferences(
            &reordered,
            &catalog,
            &Change::Remove {
                application_id: "two.desktop".into(),
            },
        )
        .unwrap();
        assert_eq!(removed, preferences);
        assert!(
            changed_preferences(
                &preferences,
                &catalog,
                &Change::Add {
                    application_id: "/tmp/arbitrary.desktop".into()
                }
            )
            .is_err()
        );
        assert!(
            changed_preferences(
                &preferences,
                &catalog,
                &Change::Reorder {
                    application_ids: vec!["two.desktop".into()]
                }
            )
            .is_err()
        );
        assert_eq!(
            projection(&preferences, &catalog),
            (vec!["one.desktop".into()], 1)
        );
    }

    #[test]
    fn semantic_pin_moves_are_bounded_and_keep_exact_membership() {
        let favorites = vec!["one.desktop".into(), "two.desktop".into()];
        assert_eq!(
            moved_favorite(favorites.clone(), "two.desktop", -1).unwrap(),
            ["two.desktop", "one.desktop"]
        );
        assert_eq!(
            moved_favorite(favorites.clone(), "one.desktop", -1).unwrap(),
            favorites
        );
        assert!(moved_favorite(vec!["one.desktop".into()], "missing.desktop", 1).is_err());
    }
    #[test]
    fn favorites_generation_rejects_same_content_replacement_and_projection_hides_history() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("preferences");
        let mut preferences = LauncherPreferences::default();
        preferences.replace_favorites(["place:/private/unavailable".into(), "one.desktop".into()]);
        preferences.record_launch("private-history-canary");
        preferences.save(&path).unwrap();
        let catalog = catalog();
        let (favorites, unavailable) = projection(&preferences, &catalog);
        let mut prepared = PreparedRead {
            revision: regular_file_revision(&path).unwrap(),
            path: path.clone(),
            preferences: preferences.clone(),
            catalog_generation: 1,
            catalog,
            favorites,
            unavailable,
        };
        let mut state = FavoritesState::default();
        let first = state.observe(&prepared, 1, true).unwrap();
        let serialized = serde_json::to_string(&first).unwrap();
        assert!(!serialized.contains("private"));
        assert!(!serialized.contains("place:"));
        let transaction = Transaction {
            generation: first.generation,
            catalog_generation: 1,
            prior: first.favorites,
            change: Change::Add {
                application_id: "two.desktop".into(),
            },
        };
        state.validate(&prepared, &transaction).unwrap();
        preferences.save(&path).unwrap();
        prepared.revision = regular_file_revision(&path).unwrap();
        let second = state.observe(&prepared, 2, true).unwrap();
        assert!(second.generation > transaction.generation);
        assert!(state.validate(&prepared, &transaction).is_err());
    }
}
