//! Bounded typed settings preparation shared by the Windows desktop authority.
//!
//! File contents are read and staged before the winit owner sees a request. The
//! owner performs only a revision check and atomic replacement at a fresh,
//! continuously-authorized commit boundary.

use nickel_core::shell_settings::ShellSettings;
use nickel_core::{
    on_screen_keyboard::KeyboardPreference as CoreKeyboardPreference,
    optional_features::{OptionalFeatureSettings, PreparedKeyboardPreference},
};
use nickel_remote_control::appearance::{
    Animations, Preferences, Snapshot, ThemePreference, Transaction,
};
use nickel_session_protocol::{ShellBehaviorSetting, ShellBehaviorTransaction, ShellBehaviorValue};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{io, path::PathBuf, time::Instant};

const STALE: &str = "appearance changed; read current appearance before retrying";
const UNAVAILABLE: &str = "appearance unavailable; inspect current state before retrying";

const IDLE_STALE: &str = "idle preferences changed; read current state before retrying";
const IDLE_UNAVAILABLE: &str = "idle preferences unavailable; read current state before retrying";
const KEYBOARD_STALE: &str = "keyboard preference changed; read current state before retrying";
const KEYBOARD_UNAVAILABLE: &str =
    "keyboard preference unavailable; read current state before retrying";

fn idle_timeout(value: Option<u32>) -> nickel_remote_control::idle_preferences::Timeout {
    value.map_or(
        nickel_remote_control::idle_preferences::Timeout::Disabled,
        nickel_remote_control::idle_preferences::Timeout::AfterSeconds,
    )
}

fn idle_seconds(value: nickel_remote_control::idle_preferences::Timeout) -> Option<u32> {
    match value {
        nickel_remote_control::idle_preferences::Timeout::Disabled => None,
        nickel_remote_control::idle_preferences::Timeout::AfterSeconds(seconds) => Some(seconds),
    }
}

fn idle_preferences(
    settings: &ShellSettings,
) -> nickel_remote_control::idle_preferences::Preferences {
    nickel_remote_control::idle_preferences::Preferences {
        dim: idle_timeout(settings.idle_dim_seconds),
        suspend: idle_timeout(settings.idle_suspend_seconds),
    }
}

pub(crate) struct PreparedIdleRead {
    path: PathBuf,
    pub(crate) revision: Option<RegularFileRevision>,
    settings: ShellSettings,
}

impl PreparedIdleRead {
    pub(crate) fn prepare() -> Result<Self, String> {
        Self::at(nickel_core::shell_settings::settings_path().map_err(|_| IDLE_UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let revision = regular_file_revision(&path).map_err(|_| IDLE_UNAVAILABLE)?;
        let settings = ShellSettings::load_for_update(&path).map_err(|_| IDLE_UNAVAILABLE)?;
        if regular_file_revision(&path).map_err(|_| IDLE_UNAVAILABLE)? != revision {
            return Err(IDLE_STALE.into());
        }
        Ok(Self {
            path,
            revision,
            settings,
        })
    }

    pub(crate) fn ensure_current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| IDLE_UNAVAILABLE)? != self.revision {
            return Err(IDLE_STALE.into());
        }
        Ok(())
    }

    fn configured(&self) -> nickel_remote_control::idle_preferences::Preferences {
        idle_preferences(&self.settings)
    }
}

pub(crate) struct PreparedIdleChange {
    previous: PreparedIdleRead,
    requested: ShellSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedIdleChange {
    pub(crate) fn prepare(
        transaction: &nickel_remote_control::idle_preferences::Transaction,
    ) -> Result<Self, String> {
        Self::prepare_at(
            nickel_core::shell_settings::settings_path().map_err(|_| IDLE_UNAVAILABLE)?,
            transaction,
        )
    }

    fn prepare_at(
        path: PathBuf,
        transaction: &nickel_remote_control::idle_preferences::Transaction,
    ) -> Result<Self, String> {
        if transaction.generation == 0
            || !transaction.requested.valid_request()
            || transaction.prior == transaction.requested
        {
            return Err(IDLE_STALE.into());
        }
        let previous = PreparedIdleRead::at(path)?;
        let lock = nickel_storage::TransactionLock::try_acquire(&previous.path)
            .map_err(|_| IDLE_UNAVAILABLE)?;
        previous.ensure_current()?;
        if previous.configured() != transaction.prior {
            return Err(IDLE_STALE.into());
        }
        let mut requested = previous.settings.clone();
        requested.idle_dim_seconds = idle_seconds(transaction.requested.dim);
        requested.idle_suspend_seconds = idle_seconds(transaction.requested.suspend);
        let staged = requested
            .stage(&previous.path)
            .map_err(|_| IDLE_UNAVAILABLE)?;
        Ok(Self {
            previous,
            requested,
            staged,
            _lock: lock,
        })
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<(ShellSettings, Option<RegularFileRevision>), String> {
        self.staged
            .commit(|| {
                if regular_file_revision(&self.previous.path)? != self.previous.revision {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, IDLE_STALE));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "idle preference commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)
            })
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    IDLE_STALE
                } else {
                    IDLE_UNAVAILABLE
                }
                .to_owned()
            })?;
        let revision = regular_file_revision(&self.previous.path).map_err(|_| IDLE_UNAVAILABLE)?;
        Ok((self.requested, revision))
    }
}

#[derive(Default)]
pub(crate) struct IdleState {
    generation: u64,
    observed: Option<(
        Option<RegularFileRevision>,
        nickel_remote_control::idle_preferences::Preferences,
    )>,
    applied_generation: u64,
}

impl IdleState {
    pub(crate) fn observe(
        &mut self,
        read: &PreparedIdleRead,
        applied: nickel_remote_control::idle_preferences::Preferences,
        observed_at_us: u64,
    ) -> Result<nickel_remote_control::idle_preferences::Snapshot, String> {
        let configured = read.configured();
        let observed = (read.revision.clone(), configured);
        if self.observed.as_ref() != Some(&observed) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("idle preference generation exhausted")?;
            self.observed = Some(observed);
        }
        if configured == applied {
            self.applied_generation = self.generation;
        }
        Ok(nickel_remote_control::idle_preferences::Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
            applied,
            applied_generation: self.applied_generation,
            pending: configured != applied,
        })
    }

    pub(crate) fn validate(
        &self,
        prepared: &PreparedIdleChange,
        transaction: &nickel_remote_control::idle_preferences::Transaction,
    ) -> Result<(), String> {
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref().is_none_or(|(revision, prior)| {
                revision != &prepared.previous.revision || *prior != transaction.prior
            })
        {
            return Err(IDLE_STALE.into());
        }
        Ok(())
    }

    pub(crate) fn invalidate(&mut self) {
        self.observed = None;
    }
}

fn keyboard_preference(
    value: CoreKeyboardPreference,
) -> nickel_remote_control::keyboard_preference::Preference {
    use nickel_remote_control::keyboard_preference::Preference;
    match value {
        CoreKeyboardPreference::Automatic => Preference::Automatic,
        CoreKeyboardPreference::Enabled => Preference::Enabled,
        CoreKeyboardPreference::Disabled => Preference::Disabled,
    }
}

fn core_keyboard_preference(
    value: nickel_remote_control::keyboard_preference::Preference,
) -> CoreKeyboardPreference {
    use nickel_remote_control::keyboard_preference::Preference;
    match value {
        Preference::Automatic => CoreKeyboardPreference::Automatic,
        Preference::Enabled => CoreKeyboardPreference::Enabled,
        Preference::Disabled => CoreKeyboardPreference::Disabled,
    }
}

pub(crate) struct PreparedKeyboardRead {
    path: PathBuf,
    pub(crate) revision: Option<RegularFileRevision>,
    settings: OptionalFeatureSettings,
}

impl PreparedKeyboardRead {
    pub(crate) fn prepare() -> Result<Self, String> {
        Self::at(nickel_core::optional_features::settings_path().map_err(|_| KEYBOARD_UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let revision = regular_file_revision(&path).map_err(|_| KEYBOARD_UNAVAILABLE)?;
        let settings = match OptionalFeatureSettings::load(&path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                OptionalFeatureSettings::default()
            }
            Err(_) => return Err(KEYBOARD_UNAVAILABLE.into()),
        };
        if regular_file_revision(&path).map_err(|_| KEYBOARD_UNAVAILABLE)? != revision {
            return Err(KEYBOARD_STALE.into());
        }
        Ok(Self {
            path,
            revision,
            settings,
        })
    }

    pub(crate) fn ensure_current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| KEYBOARD_UNAVAILABLE)? != self.revision {
            return Err(KEYBOARD_STALE.into());
        }
        Ok(())
    }

    pub(crate) fn configured(&self) -> nickel_remote_control::keyboard_preference::Preference {
        keyboard_preference(self.settings.on_screen_keyboard)
    }

    pub(crate) fn generation(&self) -> u64 {
        self.settings.on_screen_keyboard_generation
    }
}

pub(crate) struct PreparedKeyboardChange {
    previous: PreparedKeyboardRead,
    staged: PreparedKeyboardPreference,
}

impl PreparedKeyboardChange {
    pub(crate) fn prepare(
        transaction: &nickel_remote_control::keyboard_preference::Transaction,
    ) -> Result<Self, String> {
        Self::prepare_at(
            nickel_core::optional_features::settings_path().map_err(|_| KEYBOARD_UNAVAILABLE)?,
            transaction,
        )
    }

    fn prepare_at(
        path: PathBuf,
        transaction: &nickel_remote_control::keyboard_preference::Transaction,
    ) -> Result<Self, String> {
        if transaction.prior == transaction.requested {
            return Err(KEYBOARD_STALE.into());
        }
        let previous = PreparedKeyboardRead::at(path)?;
        if transaction.generation != previous.generation()
            || transaction.prior != previous.configured()
            || previous.generation() == u64::MAX
        {
            return Err(KEYBOARD_STALE.into());
        }
        let staged = PreparedKeyboardPreference::prepare(
            previous.path.clone(),
            &previous.settings,
            core_keyboard_preference(transaction.requested),
        )
        .map_err(|_| KEYBOARD_STALE)?;
        Ok(Self { previous, staged })
    }

    pub(crate) fn validate(
        &self,
        transaction: &nickel_remote_control::keyboard_preference::Transaction,
    ) -> Result<(), String> {
        if transaction.generation != self.previous.generation()
            || transaction.prior != self.previous.configured()
        {
            return Err(KEYBOARD_STALE.into());
        }
        Ok(())
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<(OptionalFeatureSettings, Option<RegularFileRevision>), String> {
        let path = self.previous.path.clone();
        let settings = self
            .staged
            .commit(|| {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "keyboard preference commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)
            })
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    KEYBOARD_STALE
                } else {
                    KEYBOARD_UNAVAILABLE
                }
                .to_owned()
            })?;
        let revision = regular_file_revision(&path).map_err(|_| KEYBOARD_UNAVAILABLE)?;
        Ok((settings, revision))
    }
}

pub(crate) fn keyboard_snapshot(
    read: &PreparedKeyboardRead,
    runtime: nickel_session_protocol::OnScreenKeyboardSnapshot,
    observed_at_us: u64,
) -> nickel_remote_control::keyboard_preference::Snapshot {
    nickel_remote_control::keyboard_preference::Snapshot {
        generation: read.generation(),
        observed_at_us,
        configured: read.configured(),
        runtime_generation: runtime.generation,
        runtime_enabled: runtime.enabled,
        touchscreen_present: runtime.touchscreen_present,
        environment_override: runtime.environment_override,
        pending: runtime.generation != read.generation(),
    }
}

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

const FILE_ICONS_STALE: &str = "file icon settings changed; read current state before retrying";
const FILE_ICONS_UNAVAILABLE: &str =
    "file icon settings unavailable; read current state before retrying";
const MAX_THEME_ID_BYTES: usize = 128;
const MAX_THEMES: usize = 256;

const SHELL_BEHAVIOR_STALE: &str =
    "shell behavior changed; read current diagnostics before retrying";
const SHELL_BEHAVIOR_UNAVAILABLE: &str =
    "shell behavior unavailable; read current diagnostics before retrying";

fn shell_behavior_value(
    settings: &ShellSettings,
    setting: ShellBehaviorSetting,
) -> ShellBehaviorValue {
    match setting {
        ShellBehaviorSetting::BarDisplayScope => {
            ShellBehaviorValue::Toggle(settings.bar_on_all_displays)
        }
        ShellBehaviorSetting::BarWindowScope => {
            ShellBehaviorValue::Toggle(settings.all_windows_on_every_bar)
        }
        ShellBehaviorSetting::DesktopCount => ShellBehaviorValue::Count(settings.desktop_count),
    }
}

fn apply_shell_behavior_value(
    settings: &mut ShellSettings,
    setting: ShellBehaviorSetting,
    value: ShellBehaviorValue,
) -> Result<(), String> {
    match (setting, value) {
        (ShellBehaviorSetting::BarDisplayScope, ShellBehaviorValue::Toggle(value)) => {
            settings.bar_on_all_displays = value;
        }
        (ShellBehaviorSetting::BarWindowScope, ShellBehaviorValue::Toggle(value)) => {
            settings.all_windows_on_every_bar = value;
        }
        (ShellBehaviorSetting::DesktopCount, ShellBehaviorValue::Count(value))
            if (1..=nickel_core::shell_settings::MAX_CONFIGURED_WORKSPACES).contains(&value) =>
        {
            settings.desktop_count = value;
        }
        (ShellBehaviorSetting::DesktopCount, ShellBehaviorValue::Count(_)) => {
            return Err("desktop count is outside the supported range".into());
        }
        _ => return Err("shell behavior setting and value types do not match".into()),
    }
    Ok(())
}

pub(crate) struct PreparedShellBehaviorChange {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    previous: ShellSettings,
    requested: ShellSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedShellBehaviorChange {
    pub(crate) fn prepare(transaction: &ShellBehaviorTransaction) -> Result<Self, String> {
        Self::prepare_at(
            nickel_core::shell_settings::settings_path().map_err(|_| SHELL_BEHAVIOR_UNAVAILABLE)?,
            transaction,
        )
    }

    fn prepare_at(path: PathBuf, transaction: &ShellBehaviorTransaction) -> Result<Self, String> {
        let lock = nickel_storage::TransactionLock::try_acquire(&path)
            .map_err(|_| SHELL_BEHAVIOR_UNAVAILABLE)?;
        let revision = regular_file_revision(&path).map_err(|_| SHELL_BEHAVIOR_UNAVAILABLE)?;
        let previous =
            ShellSettings::load_for_update(&path).map_err(|_| SHELL_BEHAVIOR_UNAVAILABLE)?;
        if regular_file_revision(&path).map_err(|_| SHELL_BEHAVIOR_UNAVAILABLE)? != revision
            || shell_behavior_value(&previous, transaction.setting) != transaction.prior
        {
            return Err(SHELL_BEHAVIOR_STALE.into());
        }
        let mut requested = previous.clone();
        apply_shell_behavior_value(&mut requested, transaction.setting, transaction.requested)?;
        let staged = requested
            .stage(&path)
            .map_err(|_| SHELL_BEHAVIOR_UNAVAILABLE)?;
        Ok(Self {
            path,
            revision,
            previous,
            requested,
            staged,
            _lock: lock,
        })
    }

    pub(crate) fn ensure_current(
        &self,
        transaction: &ShellBehaviorTransaction,
    ) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| SHELL_BEHAVIOR_UNAVAILABLE)?
            != self.revision
            || shell_behavior_value(&self.previous, transaction.setting) != transaction.prior
            || shell_behavior_value(&self.requested, transaction.setting) != transaction.requested
        {
            return Err(SHELL_BEHAVIOR_STALE.into());
        }
        Ok(())
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<ShellSettings, String> {
        self.staged
            .commit(|| {
                if regular_file_revision(&self.path)? != self.revision {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        SHELL_BEHAVIOR_STALE,
                    ));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "shell behavior commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)
            })
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    SHELL_BEHAVIOR_STALE
                } else {
                    SHELL_BEHAVIOR_UNAVAILABLE
                }
                .to_owned()
            })?;
        Ok(self.requested)
    }
}

fn valid_theme_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_THEME_ID_BYTES
        && !value.contains(['/', '\\', '\0'])
        && value != "."
        && value != ".."
}

fn file_icon_preferences(
    settings: &ShellSettings,
) -> nickel_remote_control::file_icons::Preferences {
    use nickel_core::shell_settings::FileIconPreference;
    use nickel_remote_control::file_icons::{Preferences, Provider};
    Preferences {
        provider: match settings.file_icon_provider {
            FileIconPreference::Nickel => Provider::Nickel,
            FileIconPreference::System => Provider::System,
        },
        theme: settings
            .file_icon_theme
            .as_deref()
            .filter(|theme| valid_theme_id(theme))
            .map(str::to_owned),
    }
}

fn file_icon_themes(configured: Option<&str>) -> Vec<nickel_remote_control::file_icons::Theme> {
    use nickel_remote_control::file_icons::Theme;
    let mut installed = nickel_platform::installed_icon_themes();
    installed.retain(|theme| valid_theme_id(theme));
    installed.sort_by_key(|theme| theme.to_ascii_lowercase());
    installed.dedup();
    let configured_available =
        configured.is_some_and(|selected| installed.iter().any(|theme| theme == selected));
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

pub(crate) struct PreparedFileIconsRead {
    path: PathBuf,
    pub(crate) revision: Option<RegularFileRevision>,
    settings: ShellSettings,
    themes: Vec<nickel_remote_control::file_icons::Theme>,
    provider_revision: u64,
}

impl PreparedFileIconsRead {
    pub(crate) fn prepare() -> Result<Self, String> {
        Self::at(nickel_core::shell_settings::settings_path().map_err(|_| FILE_ICONS_UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let revision = regular_file_revision(&path).map_err(|_| FILE_ICONS_UNAVAILABLE)?;
        let settings = ShellSettings::load_for_update(&path).map_err(|_| FILE_ICONS_UNAVAILABLE)?;
        let themes = file_icon_themes(settings.file_icon_theme.as_deref());
        let provider_revision =
            nickel_platform::path_icon_theme_revision(settings.file_icon_theme.as_deref());
        if regular_file_revision(&path).map_err(|_| FILE_ICONS_UNAVAILABLE)? != revision {
            return Err(FILE_ICONS_STALE.into());
        }
        Ok(Self {
            path,
            revision,
            settings,
            themes,
            provider_revision,
        })
    }

    pub(crate) fn ensure_current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| FILE_ICONS_UNAVAILABLE)? != self.revision
            || nickel_platform::path_icon_theme_revision(self.settings.file_icon_theme.as_deref())
                != self.provider_revision
        {
            return Err(FILE_ICONS_STALE.into());
        }
        Ok(())
    }
}

pub(crate) struct PreparedFileIconsChange {
    prior: PreparedFileIconsRead,
    requested: ShellSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedFileIconsChange {
    pub(crate) fn prepare(
        transaction: &nickel_remote_control::file_icons::Transaction,
    ) -> Result<Self, String> {
        Self::prepare_at(
            nickel_core::shell_settings::settings_path().map_err(|_| FILE_ICONS_UNAVAILABLE)?,
            transaction,
        )
    }

    fn prepare_at(
        path: PathBuf,
        transaction: &nickel_remote_control::file_icons::Transaction,
    ) -> Result<Self, String> {
        use nickel_core::shell_settings::FileIconPreference;
        use nickel_remote_control::file_icons::Change;
        let prior = PreparedFileIconsRead::at(path)?;
        let lock = nickel_storage::TransactionLock::try_acquire(&prior.path)
            .map_err(|_| FILE_ICONS_UNAVAILABLE)?;
        prior.ensure_current()?;
        if transaction.generation == 0
            || transaction.prior != file_icon_preferences(&prior.settings)
        {
            return Err(FILE_ICONS_STALE.into());
        }
        let mut requested = prior.settings.clone();
        match &transaction.change {
            Change::SetNickelProvider {} => {
                requested.file_icon_provider = FileIconPreference::Nickel
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
        let staged = requested
            .stage(&prior.path)
            .map_err(|_| FILE_ICONS_UNAVAILABLE)?;
        Ok(Self {
            prior,
            requested,
            staged,
            _lock: lock,
        })
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<(ShellSettings, Option<RegularFileRevision>), String> {
        self.staged
            .commit(|| {
                if regular_file_revision(&self.prior.path)? != self.prior.revision
                    || nickel_platform::path_icon_theme_revision(
                        self.prior.settings.file_icon_theme.as_deref(),
                    ) != self.prior.provider_revision
                {
                    return Err(io::Error::new(io::ErrorKind::InvalidData, FILE_ICONS_STALE));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "file icon commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)
            })
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    FILE_ICONS_STALE
                } else {
                    FILE_ICONS_UNAVAILABLE
                }
                .to_owned()
            })?;
        let revision =
            regular_file_revision(&self.prior.path).map_err(|_| FILE_ICONS_UNAVAILABLE)?;
        Ok((self.requested, revision))
    }
}

#[derive(Default)]
pub(crate) struct FileIconState {
    generation: u64,
    observed: Option<(
        Option<RegularFileRevision>,
        nickel_remote_control::file_icons::Preferences,
        Vec<nickel_remote_control::file_icons::Theme>,
        u64,
    )>,
}

impl FileIconState {
    pub(crate) fn observe(
        &mut self,
        read: &PreparedFileIconsRead,
        observed_at_us: u64,
        cache_refresh_requested: bool,
    ) -> Result<nickel_remote_control::file_icons::Snapshot, String> {
        let configured = file_icon_preferences(&read.settings);
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
        Ok(nickel_remote_control::file_icons::Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
            themes: read.themes.clone(),
            cache_refresh_requested,
        })
    }

    pub(crate) fn validate(
        &self,
        prepared: &PreparedFileIconsChange,
        transaction: &nickel_remote_control::file_icons::Transaction,
    ) -> Result<(), String> {
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
            return Err(FILE_ICONS_STALE.into());
        }
        Ok(())
    }

    pub(crate) fn invalidate(&mut self) {
        self.observed = None;
    }
}

const WALLPAPER_STALE: &str = "wallpaper changed; read current wallpaper before retrying";
const WALLPAPER_UNAVAILABLE: &str = "wallpaper unavailable; read current state before retrying";

fn wallpaper_position(
    value: nickel_core::wallpaper_settings::WallpaperPosition,
) -> nickel_remote_control::wallpaper::Position {
    use nickel_core::wallpaper_settings::WallpaperPosition as Source;
    use nickel_remote_control::wallpaper::Position as Target;
    match value {
        Source::Center => Target::Center,
        Source::Tile => Target::Tile,
        Source::Stretch => Target::Stretch,
        Source::Fit => Target::Fit,
        Source::Span => Target::Span,
        Source::Fill => Target::Fill,
    }
}

fn core_wallpaper_position(
    value: nickel_remote_control::wallpaper::Position,
) -> nickel_core::wallpaper_settings::WallpaperPosition {
    use nickel_core::wallpaper_settings::WallpaperPosition as Target;
    use nickel_remote_control::wallpaper::Position as Source;
    match value {
        Source::Center => Target::Center,
        Source::Tile => Target::Tile,
        Source::Stretch => Target::Stretch,
        Source::Fit => Target::Fit,
        Source::Span => Target::Span,
        Source::Fill => Target::Fill,
    }
}

fn wallpaper_preferences(
    settings: &nickel_core::wallpaper_settings::WallpaperSettings,
) -> nickel_remote_control::wallpaper::Preferences {
    nickel_remote_control::wallpaper::Preferences {
        custom_image_configured: settings.image.is_some(),
        position: wallpaper_position(settings.position),
    }
}

pub(crate) struct PreparedWallpaperRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: nickel_core::wallpaper_settings::WallpaperSettings,
}

impl PreparedWallpaperRead {
    pub(crate) fn prepare() -> Result<Self, String> {
        Self::at(
            nickel_core::wallpaper_settings::settings_path().map_err(|_| WALLPAPER_UNAVAILABLE)?,
        )
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let revision = regular_file_revision(&path).map_err(|_| WALLPAPER_UNAVAILABLE)?;
        let settings = match nickel_core::wallpaper_settings::WallpaperSettings::load(&path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                nickel_core::wallpaper_settings::WallpaperSettings::default()
            }
            Err(_) => return Err(WALLPAPER_UNAVAILABLE.into()),
        };
        if regular_file_revision(&path).map_err(|_| WALLPAPER_UNAVAILABLE)? != revision {
            return Err(WALLPAPER_STALE.into());
        }
        Ok(Self {
            path,
            revision,
            settings,
        })
    }

    pub(crate) fn ensure_current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| WALLPAPER_UNAVAILABLE)? != self.revision {
            return Err(WALLPAPER_STALE.into());
        }
        Ok(())
    }

    fn configured(&self) -> nickel_remote_control::wallpaper::Preferences {
        wallpaper_preferences(&self.settings)
    }

    pub(crate) fn settings(&self) -> &nickel_core::wallpaper_settings::WallpaperSettings {
        &self.settings
    }
}

pub(crate) struct PreparedWallpaperChange {
    prior: PreparedWallpaperRead,
    requested: nickel_core::wallpaper_settings::WallpaperSettings,
    staged: nickel_core::wallpaper_settings::PreparedWallpaperSettings,
}

impl PreparedWallpaperChange {
    pub(crate) fn prepare(
        transaction: &nickel_remote_control::wallpaper::Transaction,
    ) -> Result<Self, String> {
        Self::from_read(PreparedWallpaperRead::prepare()?, transaction)
    }

    fn from_read(
        prior: PreparedWallpaperRead,
        transaction: &nickel_remote_control::wallpaper::Transaction,
    ) -> Result<Self, String> {
        use nickel_remote_control::wallpaper::Change;
        if transaction.generation == 0 || prior.configured() != transaction.prior {
            return Err(WALLPAPER_STALE.into());
        }
        let mut requested = prior.settings.clone();
        match transaction.change {
            Change::SetPosition { position } => {
                requested.position = core_wallpaper_position(position)
            }
            Change::ResetCustomImage {} => requested.image = None,
        }
        let staged = nickel_core::wallpaper_settings::PreparedWallpaperSettings::prepare(
            prior.path.clone(),
            &prior.settings,
            requested.clone(),
        )
        .map_err(|error| {
            if error.kind() == io::ErrorKind::InvalidData {
                WALLPAPER_STALE
            } else {
                WALLPAPER_UNAVAILABLE
            }
            .to_owned()
        })?;
        Ok(Self {
            prior,
            requested,
            staged,
        })
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<nickel_core::wallpaper_settings::WallpaperSettings, String> {
        self.staged
            .commit(|| {
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "wallpaper commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)
            })
            .map_err(|error| {
                if error.kind() == io::ErrorKind::InvalidData {
                    WALLPAPER_STALE
                } else {
                    WALLPAPER_UNAVAILABLE
                }
                .to_owned()
            })?;
        Ok(self.requested)
    }
}

#[derive(Default)]
pub(crate) struct WallpaperState {
    generation: u64,
    observed: Option<(
        Option<RegularFileRevision>,
        nickel_remote_control::wallpaper::Preferences,
    )>,
}

impl WallpaperState {
    pub(crate) fn observe(
        &mut self,
        prepared: &PreparedWallpaperRead,
        observed_at_us: u64,
        runtime_reload_requested: bool,
    ) -> Result<nickel_remote_control::wallpaper::Snapshot, String> {
        let configured = prepared.configured();
        let value = (prepared.revision.clone(), configured.clone());
        if self.observed.as_ref() != Some(&value) {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("wallpaper generation exhausted")?;
            self.observed = Some(value);
        }
        Ok(nickel_remote_control::wallpaper::Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
            runtime_reload_requested,
        })
    }

    pub(crate) fn validate(
        &self,
        prepared: &PreparedWallpaperChange,
        transaction: &nickel_remote_control::wallpaper::Transaction,
    ) -> Result<(), String> {
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref().is_none_or(|(revision, configured)| {
                revision != &prepared.prior.revision || configured != &transaction.prior
            })
        {
            return Err(WALLPAPER_STALE.into());
        }
        Ok(())
    }

    pub(crate) fn invalidate(&mut self) {
        self.observed = None;
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

    #[test]
    fn shell_behavior_commit_preserves_security_and_unrelated_settings() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell.conf");
        let settings = ShellSettings {
            desktop_count: 4,
            idle_lock_seconds: Some(731),
            preferred_terminal: Some("private-terminal".into()),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let transaction = ShellBehaviorTransaction {
            setting: ShellBehaviorSetting::DesktopCount,
            prior: ShellBehaviorValue::Count(4),
            requested: ShellBehaviorValue::Count(6),
            topology_generation: 12,
        };
        let prepared = PreparedShellBehaviorChange::prepare_at(path.clone(), &transaction).unwrap();
        let accepted = prepared
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        assert_eq!(accepted.desktop_count, 6);
        assert_eq!(accepted.idle_lock_seconds, Some(731));
        assert_eq!(
            accepted.preferred_terminal.as_deref(),
            Some("private-terminal")
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), accepted);
    }

    #[test]
    fn shell_behavior_commit_rejects_wrong_types_aba_expiry_and_cancellation() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell.conf");
        let settings = ShellSettings {
            desktop_count: 4,
            idle_lock_seconds: Some(731),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let original = std::fs::read(&path).unwrap();
        let wrong_type = ShellBehaviorTransaction {
            setting: ShellBehaviorSetting::DesktopCount,
            prior: ShellBehaviorValue::Count(4),
            requested: ShellBehaviorValue::Toggle(true),
            topology_generation: 12,
        };
        assert!(PreparedShellBehaviorChange::prepare_at(path.clone(), &wrong_type).is_err());

        let transaction = ShellBehaviorTransaction {
            setting: ShellBehaviorSetting::DesktopCount,
            prior: ShellBehaviorValue::Count(4),
            requested: ShellBehaviorValue::Count(6),
            topology_generation: 12,
        };
        let prepared = PreparedShellBehaviorChange::prepare_at(path.clone(), &transaction).unwrap();
        std::fs::remove_file(&path).unwrap();
        std::fs::write(&path, original).unwrap();
        assert!(
            prepared
                .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
                .is_err()
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), settings);

        let expired = PreparedShellBehaviorChange::prepare_at(path.clone(), &transaction).unwrap();
        assert!(expired.commit(Instant::now(), || Ok(())).is_err());
        assert_eq!(ShellSettings::load(&path).unwrap(), settings);

        let cancelled =
            PreparedShellBehaviorChange::prepare_at(path.clone(), &transaction).unwrap();
        assert!(
            cancelled
                .commit(Instant::now() + Duration::from_secs(1), || {
                    Err("revoked".into())
                })
                .is_err()
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), settings);
    }

    #[test]
    fn idle_commit_preserves_lock_and_unrelated_settings_and_checks_boundary() {
        use nickel_remote_control::idle_preferences::{
            Preferences as IdlePreferences, Timeout, Transaction as IdleTransaction,
        };

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell.conf");
        let settings = ShellSettings {
            idle_dim_seconds: Some(300),
            idle_lock_seconds: Some(731),
            idle_suspend_seconds: None,
            preferred_terminal: Some("private-terminal".into()),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let read = PreparedIdleRead::at(path.clone()).unwrap();
        let mut state = IdleState::default();
        let snapshot = state.observe(&read, read.configured(), 10).unwrap();
        let transaction = IdleTransaction {
            generation: snapshot.generation,
            prior: snapshot.configured,
            requested: IdlePreferences {
                dim: Timeout::AfterSeconds(600),
                suspend: Timeout::AfterSeconds(3600),
            },
        };

        let denied = PreparedIdleChange::prepare_at(path.clone(), &transaction).unwrap();
        assert!(
            denied
                .commit(Instant::now() + Duration::from_secs(1), || Err(
                    "revoked".into()
                ))
                .is_err()
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), settings);

        let staged = PreparedIdleChange::prepare_at(path.clone(), &transaction).unwrap();
        state.validate(&staged, &transaction).unwrap();
        let (committed, _) = staged
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        assert_eq!(committed.idle_dim_seconds, Some(600));
        assert_eq!(committed.idle_suspend_seconds, Some(3600));
        assert_eq!(committed.idle_lock_seconds, Some(731));
        assert_eq!(
            committed.preferred_terminal.as_deref(),
            Some("private-terminal")
        );
    }

    #[test]
    fn keyboard_commit_preserves_codex_fields_and_reports_runtime_acknowledgement() {
        use nickel_core::optional_features::CodexSource;
        use nickel_remote_control::keyboard_preference::{
            Preference, Transaction as KeyboardTransaction,
        };

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("optional-features.conf");
        let settings = OptionalFeatureSettings {
            codex_enabled: false,
            codex_generation: 41,
            codex_source: CodexSource::Executable("private-codex-path".into()),
            on_screen_keyboard: CoreKeyboardPreference::Automatic,
            on_screen_keyboard_generation: 7,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let transaction = KeyboardTransaction {
            generation: 7,
            prior: Preference::Automatic,
            requested: Preference::Enabled,
        };

        let expired = PreparedKeyboardChange::prepare_at(path.clone(), &transaction).unwrap();
        assert!(expired.commit(Instant::now(), || Ok(())).is_err());
        assert_eq!(OptionalFeatureSettings::load(&path).unwrap(), settings);

        let staged = PreparedKeyboardChange::prepare_at(path.clone(), &transaction).unwrap();
        staged.validate(&transaction).unwrap();
        let (committed, _) = staged
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        assert_eq!(
            committed.on_screen_keyboard,
            CoreKeyboardPreference::Enabled
        );
        assert_eq!(committed.on_screen_keyboard_generation, 8);
        assert!(!committed.codex_enabled);
        assert_eq!(committed.codex_generation, 41);
        assert_eq!(
            committed.codex_source,
            CodexSource::Executable("private-codex-path".into())
        );

        let read = PreparedKeyboardRead::at(path).unwrap();
        let pending = keyboard_snapshot(
            &read,
            nickel_session_protocol::OnScreenKeyboardSnapshot {
                generation: 7,
                enabled: false,
                touchscreen_present: true,
                environment_override: false,
                ..Default::default()
            },
            19,
        );
        assert!(pending.pending);
        assert_eq!(pending.runtime_generation, 7);
        let acknowledged = keyboard_snapshot(
            &read,
            nickel_session_protocol::OnScreenKeyboardSnapshot {
                generation: 8,
                enabled: true,
                touchscreen_present: true,
                environment_override: false,
                ..Default::default()
            },
            20,
        );
        assert!(!acknowledged.pending);
        assert!(acknowledged.runtime_enabled);
        assert_eq!(acknowledged.observed_at_us, 20);
    }

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
    fn file_icon_commit_preserves_unrelated_settings_and_checks_boundary() {
        use nickel_core::shell_settings::FileIconPreference;
        use nickel_remote_control::file_icons::{Change, Transaction as FileIconTransaction};

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("shell.conf");
        let settings = ShellSettings {
            file_icon_provider: FileIconPreference::System,
            file_icon_theme: Some("temporarily-missing-theme".into()),
            idle_lock_seconds: Some(731),
            preferred_terminal: Some("private-terminal".into()),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let read = PreparedFileIconsRead::at(path.clone()).unwrap();
        let mut state = FileIconState::default();
        let snapshot = state.observe(&read, 10, false).unwrap();
        let transaction = FileIconTransaction {
            generation: snapshot.generation,
            prior: snapshot.configured,
            change: Change::UseSystemDefault {},
        };

        let denied = PreparedFileIconsChange::prepare_at(path.clone(), &transaction).unwrap();
        assert!(
            denied
                .commit(Instant::now() + Duration::from_secs(1), || Err(
                    "revoked".into()
                ))
                .is_err()
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), settings);

        let staged = PreparedFileIconsChange::prepare_at(path.clone(), &transaction).unwrap();
        staged
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        let actual = ShellSettings::load(&path).unwrap();
        assert_eq!(actual.file_icon_provider, FileIconPreference::System);
        assert_eq!(actual.file_icon_theme, None);
        assert_eq!(actual.idle_lock_seconds, Some(731));
        assert_eq!(
            actual.preferred_terminal.as_deref(),
            Some("private-terminal")
        );
    }

    #[test]
    fn wallpaper_commit_preserves_hidden_image_and_checks_boundary() {
        use nickel_core::wallpaper_settings::{WallpaperPosition, WallpaperSettings};
        use nickel_remote_control::wallpaper::{Change, Position, Transaction};

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wallpaper-settings");
        let original = WallpaperSettings {
            image: Some("C:\\Users\\owner\\private-wallpaper.png".into()),
            position: WallpaperPosition::Fill,
        };
        original.save(&path).unwrap();
        let mut state = WallpaperState::default();
        let read = PreparedWallpaperRead::at(path.clone()).unwrap();
        let snapshot = state.observe(&read, 10, false).unwrap();
        assert_eq!(snapshot.generation, 1);
        assert!(snapshot.configured.custom_image_configured);
        assert!(!snapshot.runtime_reload_requested);
        let transaction = Transaction {
            generation: snapshot.generation,
            prior: snapshot.configured,
            change: Change::SetPosition {
                position: Position::Fit,
            },
        };

        let denied = PreparedWallpaperChange::from_read(read, &transaction).unwrap();
        state.validate(&denied, &transaction).unwrap();
        assert!(
            denied
                .commit(Instant::now() + Duration::from_secs(1), || Err(
                    "revoked".into()
                ))
                .is_err()
        );
        assert_eq!(WallpaperSettings::load(&path).unwrap(), original);

        let staged = PreparedWallpaperChange::from_read(
            PreparedWallpaperRead::at(path.clone()).unwrap(),
            &transaction,
        )
        .unwrap();
        state.validate(&staged, &transaction).unwrap();
        let requested = staged
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        assert_eq!(requested.image, original.image);
        assert_eq!(requested.position, WallpaperPosition::Fit);
        let read = PreparedWallpaperRead::at(path.clone()).unwrap();
        let committed = state.observe(&read, 20, true).unwrap();
        assert_eq!(committed.generation, 2);
        assert!(committed.runtime_reload_requested);
        assert_eq!(WallpaperSettings::load(&path).unwrap(), requested);

        let reset_transaction = Transaction {
            generation: committed.generation,
            prior: committed.configured,
            change: Change::ResetCustomImage {},
        };
        let reset = PreparedWallpaperChange::from_read(read, &reset_transaction).unwrap();
        state.validate(&reset, &reset_transaction).unwrap();
        let reset = reset
            .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
            .unwrap();
        assert_eq!(reset.image, None);
        assert_eq!(reset.position, WallpaperPosition::Fit);
    }

    #[test]
    fn wallpaper_transaction_rejects_deadline_file_aba_and_stale_generation() {
        use nickel_core::wallpaper_settings::{WallpaperPosition, WallpaperSettings};
        use nickel_remote_control::wallpaper::{Change, Position, Transaction};

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("wallpaper-settings");
        let original = WallpaperSettings {
            image: Some("C:\\Users\\owner\\private-wallpaper.png".into()),
            position: WallpaperPosition::Fill,
        };
        original.save(&path).unwrap();
        let mut state = WallpaperState::default();
        let read = PreparedWallpaperRead::at(path.clone()).unwrap();
        let snapshot = state.observe(&read, 1, false).unwrap();
        let transaction = Transaction {
            generation: snapshot.generation,
            prior: snapshot.configured,
            change: Change::SetPosition {
                position: Position::Center,
            },
        };

        let expired = PreparedWallpaperChange::from_read(read, &transaction).unwrap();
        assert!(expired.commit(Instant::now(), || Ok(())).is_err());
        assert_eq!(WallpaperSettings::load(&path).unwrap(), original);

        let replaced = PreparedWallpaperChange::from_read(
            PreparedWallpaperRead::at(path.clone()).unwrap(),
            &transaction,
        )
        .unwrap();
        std::fs::write(
            &path,
            "image=C:\\Users\\owner\\private-wallpaper.png\nposition=tile\n",
        )
        .unwrap();
        assert!(
            replaced
                .commit(Instant::now() + Duration::from_secs(1), || Ok(()))
                .is_err()
        );
        assert_eq!(
            WallpaperSettings::load(&path).unwrap().position,
            WallpaperPosition::Tile
        );

        let changed = PreparedWallpaperRead::at(path).unwrap();
        let changed_snapshot = state.observe(&changed, 2, false).unwrap();
        assert_eq!(changed_snapshot.generation, 2);
        let stale_transaction = Transaction {
            generation: 1,
            prior: changed_snapshot.configured,
            change: Change::ResetCustomImage {},
        };
        let stale = PreparedWallpaperChange::from_read(changed, &stale_transaction).unwrap();
        assert!(state.validate(&stale, &stale_transaction).is_err());
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
