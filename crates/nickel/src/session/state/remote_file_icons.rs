//! File-icon settings are prepared off-thread and committed by the desktop owner.
use super::{
    NickelSession,
    remote_settings::{FileRevision, revision},
};
use nickel_core::shell_settings::{FileIconPreference, ShellSettings};
use nickel_remote_control::{
    DesktopPermit,
    file_icons::{Change, Preferences, Provider, Snapshot, Theme, Transaction},
};
use std::{io, path::PathBuf};

const MAX_THEME_ID_BYTES: usize = 128;
const MAX_THEMES: usize = 256;
const STALE: &str = "file icon settings changed; read current state before retrying";
const UNAVAILABLE: &str = "file icon settings unavailable; read current state before retrying";

fn provider(value: FileIconPreference) -> Provider {
    match value {
        FileIconPreference::Nickel => Provider::Nickel,
        FileIconPreference::System => Provider::System,
    }
}

fn preferences(settings: &ShellSettings) -> Preferences {
    Preferences {
        provider: provider(settings.file_icon_provider),
        theme: settings
            .file_icon_theme
            .as_deref()
            .filter(|theme| valid_theme_id(theme))
            .map(str::to_owned),
    }
}

fn valid_theme_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_THEME_ID_BYTES
        && !value.contains(['/', '\\', '\0'])
        && value != "."
        && value != ".."
}

fn theme_catalog(configured: Option<&str>) -> Vec<Theme> {
    let mut installed = nickel_platform::installed_icon_themes();
    installed.retain(|theme| valid_theme_id(theme));
    installed.sort_by_key(|theme| theme.to_ascii_lowercase());
    installed.dedup();
    let configured_available =
        configured.is_some_and(|selected| installed.iter().any(|t| t == selected));
    let reserve_missing =
        usize::from(configured.is_some_and(valid_theme_id) && !configured_available);
    installed.truncate(MAX_THEMES - reserve_missing);
    let mut themes = installed
        .into_iter()
        .map(|id| Theme {
            configured: configured == Some(id.as_str()),
            id,
            available: true,
        })
        .collect::<Vec<_>>();
    if let Some(id) = configured.filter(|id| valid_theme_id(id) && !configured_available) {
        themes.push(Theme {
            id: id.to_owned(),
            configured: true,
            available: false,
        });
    }
    themes
}

pub(super) struct PreparedRead {
    path: PathBuf,
    revision: Option<FileRevision>,
    settings: ShellSettings,
    themes: Vec<Theme>,
    provider_revision: u64,
}

impl PreparedRead {
    pub(super) fn prepare() -> Result<Self, String> {
        Self::at(nickel_core::shell_settings::settings_path().map_err(|_| UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let before = revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = ShellSettings::load_for_update(&path).map_err(|_| UNAVAILABLE)?;
        let themes = theme_catalog(settings.file_icon_theme.as_deref());
        let provider_revision =
            nickel_platform::path_icon_theme_revision(settings.file_icon_theme.as_deref());
        if revision(&path).map_err(|_| UNAVAILABLE)? != before {
            return Err(STALE.into());
        }
        Ok(Self {
            path,
            revision: before,
            settings,
            themes,
            provider_revision,
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
    prior: PreparedRead,
    requested: ShellSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedChange {
    pub(super) fn prepare(transaction: &Transaction) -> Result<Self, String> {
        Self::prepare_at(
            nickel_core::shell_settings::settings_path().map_err(|_| UNAVAILABLE)?,
            transaction,
        )
    }

    fn prepare_at(path: PathBuf, transaction: &Transaction) -> Result<Self, String> {
        let prior = PreparedRead::at(path)?;
        let lock =
            nickel_storage::TransactionLock::try_acquire(&prior.path).map_err(|_| UNAVAILABLE)?;
        prior.current()?;
        if transaction.generation == 0 || transaction.prior != preferences(&prior.settings) {
            return Err(STALE.into());
        }
        let mut requested = prior.settings.clone();
        match &transaction.change {
            Change::SetNickelProvider {} => {
                requested.file_icon_provider = FileIconPreference::Nickel;
            }
            Change::UseSystemDefault {} => {
                requested.file_icon_provider = FileIconPreference::System;
                requested.file_icon_theme = None;
            }
            Change::SetInstalledSystemTheme { theme_id } => {
                if !valid_theme_id(theme_id)
                    || !prior
                        .themes
                        .iter()
                        .any(|theme| theme.available && theme.id == *theme_id)
                {
                    return Err("file icon theme is not in the bounded installed catalog".into());
                }
                requested.file_icon_provider = FileIconPreference::System;
                requested.file_icon_theme = Some(theme_id.clone());
            }
        }
        let staged = requested.stage(&prior.path).map_err(|_| UNAVAILABLE)?;
        Ok(Self {
            prior,
            requested,
            staged,
            _lock: lock,
        })
    }
}

#[derive(Default)]
pub(super) struct FileIconState {
    generation: u64,
    observed: Option<(Option<FileRevision>, Preferences, Vec<Theme>, u64)>,
}

impl FileIconState {
    fn observe(&mut self, read: &PreparedRead, observed_at_us: u64) -> Result<Snapshot, String> {
        let configured = preferences(&read.settings);
        let observed = (
            read.revision.clone(),
            configured.clone(),
            read.themes.clone(),
            read.provider_revision,
        );
        if self.observed.as_ref() != Some(&observed) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("file icon generation exhausted")?;
            self.observed = Some(observed);
        }
        Ok(Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
            themes: read.themes.clone(),
            cache_refresh_requested: false,
        })
    }

    fn validate(&self, prepared: &PreparedChange, transaction: &Transaction) -> Result<(), String> {
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref().is_none_or(
                |(revision, configured, themes, provider_revision)| {
                    revision != &prepared.prior.revision
                        || configured != &transaction.prior
                        || themes != &prepared.prior.themes
                        || *provider_revision != prepared.prior.provider_revision
                },
            )
        {
            return Err(STALE.into());
        }
        Ok(())
    }
}

impl NickelSession {
    pub(super) fn remote_read_file_icons(
        &mut self,
        permit: &DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.current()?;
        permit.with_debug(self.locked || self.shell_recovery_visible(), || {
            self.remote_file_icons.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            )
        })
    }

    pub(super) fn remote_change_file_icons(
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
        let requested = prepared.requested.clone();
        let mut committed = false;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
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
            self.remote_file_icons.validate(&prepared, &transaction)?;
            prepared
                .staged
                .commit(|| {
                    if revision(&path)? != prepared.prior.revision {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, STALE));
                    }
                    permit
                        .check_commit_boundary(boundary)
                        .map_err(io::Error::other)
                })
                .map_err(|_| UNAVAILABLE)?;
            committed = true;
            self.remote_file_icons.observed = None;
            Ok(())
        });
        if !committed {
            authorization?;
            return Err(UNAVAILABLE.into());
        }
        self.notify_shell_settings_changed();
        authorization?;
        let read = PreparedRead::at(path)?;
        if read.settings != requested {
            return Err(STALE.into());
        }
        let mut snapshot = self.remote_file_icons.observe(
            &read,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        )?;
        snapshot.cache_refresh_requested = true;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_preserves_an_unavailable_configured_theme_and_is_bounded() {
        let themes = theme_catalog(Some("missing/nickel-theme"));
        assert!(themes.len() <= MAX_THEMES);
        assert!(
            themes
                .iter()
                .all(|theme| theme.id != "missing/nickel-theme")
        );
        let themes = theme_catalog(Some("missing-nickel-theme"));
        assert!(themes.iter().any(|theme| {
            theme.id == "missing-nickel-theme" && theme.configured && !theme.available
        }));
    }

    #[test]
    fn transaction_preserves_unrelated_settings_and_rejects_paths_and_aba() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell.conf");
        let settings = ShellSettings {
            idle_lock_seconds: Some(123),
            file_icon_provider: FileIconPreference::Nickel,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let read = PreparedRead::at(path.clone()).unwrap();
        let transaction = Transaction {
            generation: 1,
            prior: preferences(&settings),
            change: Change::UseSystemDefault {},
        };
        let mut state = FileIconState::default();
        assert_eq!(state.observe(&read, 1).unwrap().generation, 1);
        let prepared = PreparedChange::prepare_at(path.clone(), &transaction).unwrap();
        state.validate(&prepared, &transaction).unwrap();
        assert_eq!(
            settings.save(&path).unwrap_err().kind(),
            io::ErrorKind::WouldBlock
        );
        let mut external = settings.clone();
        external.idle_lock_seconds = Some(321);
        external.stage(&path).unwrap().commit(|| Ok(())).unwrap();
        assert!(
            prepared
                .staged
                .commit(|| {
                    if revision(&path)? != prepared.prior.revision {
                        return Err(io::Error::new(io::ErrorKind::InvalidData, STALE));
                    }
                    Ok(())
                })
                .is_err()
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), external);

        let invalid = Transaction {
            generation: 1,
            prior: preferences(&external),
            change: Change::SetInstalledSystemTheme {
                theme_id: "../private/icons".into(),
            },
        };
        assert!(PreparedChange::prepare_at(path, &invalid).is_err());
    }

    #[test]
    fn nickel_provider_commit_preserves_unrelated_and_unavailable_theme_intent() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell.conf");
        let settings = ShellSettings {
            file_icon_provider: FileIconPreference::Nickel,
            file_icon_theme: Some("temporarily-missing-theme".into()),
            preferred_terminal: Some("owned-terminal".into()),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let transaction = Transaction {
            generation: 1,
            prior: preferences(&settings),
            change: Change::SetNickelProvider {},
        };
        let prepared = PreparedChange::prepare_at(path.clone(), &transaction).unwrap();
        prepared.staged.commit(|| Ok(())).unwrap();
        let actual = ShellSettings::load(&path).unwrap();
        assert_eq!(actual.file_icon_provider, FileIconPreference::Nickel);
        assert_eq!(
            actual.file_icon_theme.as_deref(),
            Some("temporarily-missing-theme")
        );
        assert_eq!(actual.preferred_terminal.as_deref(), Some("owned-terminal"));
    }
}
