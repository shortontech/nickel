//! Portable policy and persistence for optional Nickel features.

use nickel_storage::{atomic_write, config_path, read_regular_file, stage_write};
use std::{
    io,
    path::{Path, PathBuf},
};

const MAX_SETTINGS_BYTES: usize = 64 * 1024;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OptionalFeatureId {
    Codex,
    OnScreenKeyboard,
}

impl OptionalFeatureId {
    pub const fn stable_id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::OnScreenKeyboard => "on_screen_keyboard",
        }
    }
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub enum CodexSource {
    #[default]
    CompatibleInstalled,
    Bundled,
    ApprovedRemote,
    Executable(PathBuf),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureSupport {
    Supported,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureInstallation {
    Installed,
    Missing,
    Incompatible,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureHealth {
    Unknown,
    Loading,
    SignedOut,
    Ready,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeaturePolicy {
    Editable,
    ForceEnabled,
    ForceDisabled,
}

pub fn codex_policy() -> (FeaturePolicy, Option<String>) {
    let Some(value) = std::env::var_os("NICKEL_POLICY_CODEX") else {
        return (FeaturePolicy::Editable, None);
    };
    let value = value.to_string_lossy();
    let policy = policy_from_value(&value);
    let source = (policy != FeaturePolicy::Editable)
        .then(|| "System policy (NICKEL_POLICY_CODEX)".to_owned());
    (policy, source)
}

pub fn policy_from_value(value: &str) -> FeaturePolicy {
    match value.trim().to_ascii_lowercase().as_str() {
        "enabled" | "force-enabled" | "on" => FeaturePolicy::ForceEnabled,
        "disabled" | "force-disabled" | "off" => FeaturePolicy::ForceDisabled,
        _ => FeaturePolicy::Editable,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyRequirement {
    Live,
    ShellRestart,
    ApplicationRestart,
    Login,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReleaseChannel {
    Stable,
    Preview,
    Development,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureCapability {
    pub support: FeatureSupport,
    pub installation: FeatureInstallation,
    pub health: FeatureHealth,
    pub policy: FeaturePolicy,
    /// Human-readable authority for a forced policy. Never populated for an
    /// ordinary user preference.
    pub policy_source: Option<String>,
    pub required_permissions: Vec<String>,
    pub configuration_destination: Option<String>,
    pub apply_requirement: ApplyRequirement,
    pub source_label: String,
    pub diagnostic: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FeatureEffectiveState {
    Disabled,
    Enabling,
    Enabled,
    Unavailable,
    Rejected,
    Stale,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureState {
    pub requested_enabled: bool,
    pub effective: FeatureEffectiveState,
    pub generation: u64,
    pub acknowledged_generation: u64,
    pub capability: FeatureCapability,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexPresentation {
    Hidden,
    Recoverable,
    Projects,
}

/// Generation-bearing shell projection which keeps support, installation,
/// preference, and runtime health as independent facts.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexAvailabilityProjection {
    pub support: FeatureSupport,
    pub installation: FeatureInstallation,
    pub enabled: bool,
    pub health: FeatureHealth,
    pub generation: u64,
    pub reason: Option<String>,
}

impl CodexAvailabilityProjection {
    pub fn new(
        support: FeatureSupport,
        installation: FeatureInstallation,
        enabled: bool,
        health: FeatureHealth,
        generation: u64,
        reason: Option<String>,
    ) -> Self {
        Self {
            support,
            installation,
            enabled,
            health,
            generation,
            reason: reason.map(|reason| reason.chars().take(256).collect()),
        }
    }

    pub fn presentation(&self) -> CodexPresentation {
        if !self.enabled
            || self.support == FeatureSupport::Unsupported
            || self.installation == FeatureInstallation::Missing
        {
            CodexPresentation::Hidden
        } else if self.installation == FeatureInstallation::Installed
            && self.health == FeatureHealth::Ready
        {
            CodexPresentation::Projects
        } else {
            CodexPresentation::Recoverable
        }
    }

    pub fn accepts_after(&self, current_generation: u64) -> bool {
        self.generation >= current_generation
    }
}

impl FeatureState {
    pub fn resolve(
        requested_enabled: bool,
        generation: u64,
        acknowledged_generation: u64,
        capability: FeatureCapability,
    ) -> Self {
        let requested_enabled = match capability.policy {
            FeaturePolicy::ForceEnabled => true,
            FeaturePolicy::ForceDisabled => false,
            FeaturePolicy::Editable => requested_enabled,
        };
        let effective = if !requested_enabled {
            FeatureEffectiveState::Disabled
        } else if capability.support == FeatureSupport::Unsupported
            || capability.installation != FeatureInstallation::Installed
        {
            FeatureEffectiveState::Unavailable
        } else if acknowledged_generation > generation {
            FeatureEffectiveState::Stale
        } else if generation > acknowledged_generation
            || matches!(
                capability.health,
                FeatureHealth::Unknown | FeatureHealth::Loading
            )
        {
            FeatureEffectiveState::Enabling
        } else if matches!(capability.health, FeatureHealth::Failed) {
            FeatureEffectiveState::Rejected
        } else {
            FeatureEffectiveState::Enabled
        };
        Self {
            requested_enabled,
            effective,
            generation,
            acknowledged_generation,
            capability,
        }
    }

    pub fn editable(&self) -> bool {
        self.capability.policy == FeaturePolicy::Editable
    }

    pub fn apply_label(&self) -> &'static str {
        match self.capability.apply_requirement {
            ApplyRequirement::Live => "Applies live",
            ApplyRequirement::ShellRestart => "Applies after shell restart",
            ApplyRequirement::ApplicationRestart => "Applies after application restart",
            ApplyRequirement::Login => "Applies after login",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionalFeatureSettings {
    pub version: u16,
    pub codex_enabled: bool,
    /// Monotonic request identity used to reject stale runtime acknowledgements.
    pub codex_generation: u64,
    pub codex_source: CodexSource,
    pub on_screen_keyboard: crate::on_screen_keyboard::KeyboardPreference,
    pub on_screen_keyboard_generation: u64,
}

impl Default for OptionalFeatureSettings {
    fn default() -> Self {
        Self {
            version: 1,
            codex_enabled: true,
            codex_generation: 0,
            codex_source: CodexSource::default(),
            on_screen_keyboard: Default::default(),
            on_screen_keyboard_generation: 0,
        }
    }
}

impl OptionalFeatureSettings {
    pub fn default_for_release_channel(_channel: ReleaseChannel) -> Self {
        // Codex is shipped as an enabled-by-default integration in every current
        // channel. Keeping this decision explicit prevents install/update code
        // from confusing a channel default with a persisted preference.
        Self::default()
    }
    pub fn effective_codex_enabled(&self) -> bool {
        match codex_policy().0 {
            FeaturePolicy::ForceEnabled => true,
            FeaturePolicy::ForceDisabled => false,
            FeaturePolicy::Editable => self.codex_enabled,
        }
    }
    pub fn load_default() -> Self {
        settings_path().and_then(Self::load).unwrap_or_default()
    }
    pub fn save_default(&self) -> io::Result<()> {
        self.save(settings_path()?)
    }

    pub fn update_default(update: impl FnOnce(&mut Self)) -> io::Result<Self> {
        Self::update(settings_path()?, update)
    }

    pub fn update(path: impl AsRef<Path>, update: impl FnOnce(&mut Self)) -> io::Result<Self> {
        let path = path.as_ref();
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        let mut settings = match Self::load(path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        update(&mut settings);
        settings.write_unlocked(path)?;
        Ok(settings)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = read_regular_file(path.as_ref(), MAX_SETTINGS_BYTES)?
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let contents = std::str::from_utf8(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut settings = Self::default();
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            match key.trim() {
                "version" => settings.version = value.trim().parse().unwrap_or(1),
                "codex.enabled" => {
                    if let Some(enabled) = parse_bool(value) {
                        settings.codex_enabled = enabled;
                    }
                }
                "codex.generation" => {
                    settings.codex_generation = value.trim().parse().unwrap_or_default();
                }
                "codex.source" => settings.codex_source = parse_source(value.trim()),
                "on_screen_keyboard.preference" => {
                    settings.on_screen_keyboard =
                        crate::on_screen_keyboard::KeyboardPreference::parse(value)
                            .unwrap_or_default();
                }
                "on_screen_keyboard.generation" => {
                    settings.on_screen_keyboard_generation =
                        value.trim().parse().unwrap_or_default();
                }
                _ => {}
            }
        }
        settings.version = 1;
        Ok(settings)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        self.write_unlocked(path)
    }

    fn write_unlocked(&self, path: &Path) -> io::Result<()> {
        atomic_write(path, self.encode())
    }

    fn encode(&self) -> String {
        let source = match &self.codex_source {
            CodexSource::CompatibleInstalled => "installed".to_owned(),
            CodexSource::Bundled => "bundled".to_owned(),
            CodexSource::ApprovedRemote => "remote".to_owned(),
            CodexSource::Executable(path) => format!("executable:{}", path.to_string_lossy()),
        };
        format!(
            "version=1\ncodex.enabled={}\ncodex.generation={}\ncodex.source={source}\non_screen_keyboard.preference={}\non_screen_keyboard.generation={}\n",
            self.codex_enabled,
            self.codex_generation,
            self.on_screen_keyboard.as_str(),
            self.on_screen_keyboard_generation
        )
    }
}

/// A keyboard-preference-only replacement which retains every Codex field and
/// owns the cross-process lock through its checked atomic rename.
pub struct PreparedKeyboardPreference {
    path: PathBuf,
    revision: Option<nickel_storage::RegularFileRevision>,
    requested: OptionalFeatureSettings,
    staged: nickel_storage::StagedWrite,
    _lock: nickel_storage::TransactionLock,
}

impl PreparedKeyboardPreference {
    pub fn prepare(
        path: PathBuf,
        prior: &OptionalFeatureSettings,
        preference: crate::on_screen_keyboard::KeyboardPreference,
    ) -> io::Result<Self> {
        let lock = nickel_storage::TransactionLock::try_acquire(&path)?;
        let revision = nickel_storage::regular_file_revision(&path)?;
        let current = match OptionalFeatureSettings::load(&path) {
            Ok(value) => value,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                OptionalFeatureSettings::default()
            }
            Err(error) => return Err(error),
        };
        if nickel_storage::regular_file_revision(&path)? != revision || &current != prior {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "optional feature settings changed",
            ));
        }
        let mut requested = current;
        requested.on_screen_keyboard = preference;
        requested.on_screen_keyboard_generation = requested
            .on_screen_keyboard_generation
            .checked_add(1)
            .ok_or_else(|| io::Error::other("keyboard preference generation exhausted"))?;
        let staged = stage_write(&path, requested.encode())?;
        Ok(Self {
            path,
            revision,
            requested,
            staged,
            _lock: lock,
        })
    }

    pub fn commit(
        self,
        check: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<OptionalFeatureSettings> {
        self.staged.commit(|| {
            if nickel_storage::regular_file_revision(&self.path)? != self.revision {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "optional feature settings changed",
                ));
            }
            check()
        })?;
        Ok(self.requested)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionalFeatureRuntime {
    pub version: u16,
    pub codex_generation: u64,
    pub codex_effective: FeatureEffectiveState,
    pub codex_health: FeatureHealth,
    pub codex_support: FeatureSupport,
    pub codex_installation: FeatureInstallation,
    pub active_windows: u32,
    pub background_workers: u32,
    pub subscriptions: u32,
    pub warm_surfaces: u32,
    pub cache_entries: u32,
    pub source_label: String,
    pub diagnostic: Option<String>,
}

impl Default for OptionalFeatureRuntime {
    fn default() -> Self {
        Self {
            version: 1,
            codex_generation: 0,
            codex_effective: FeatureEffectiveState::Disabled,
            codex_health: FeatureHealth::Unknown,
            codex_support: FeatureSupport::Supported,
            codex_installation: FeatureInstallation::Missing,
            active_windows: 0,
            background_workers: 0,
            subscriptions: 0,
            warm_surfaces: 0,
            cache_entries: 0,
            source_label: String::new(),
            diagnostic: None,
        }
    }
}

impl OptionalFeatureRuntime {
    pub fn load_default() -> Self {
        runtime_path().and_then(Self::load).unwrap_or_default()
    }

    pub fn save_default(&self) -> io::Result<()> {
        self.save(runtime_path()?)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes = read_regular_file(path.as_ref(), MAX_SETTINGS_BYTES)?
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let contents = std::str::from_utf8(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        let mut runtime = Self::default();
        for line in contents.lines() {
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match key.trim() {
                "version" => runtime.version = value.parse().unwrap_or(1),
                "codex.generation" => runtime.codex_generation = value.parse().unwrap_or(0),
                "codex.effective" => runtime.codex_effective = parse_effective(value),
                "codex.health" => runtime.codex_health = parse_health(value),
                "codex.support" => runtime.codex_support = parse_support(value),
                "codex.installation" => runtime.codex_installation = parse_installation(value),
                "codex.active_windows" => runtime.active_windows = value.parse().unwrap_or(0),
                "codex.background_workers" => {
                    runtime.background_workers = value.parse().unwrap_or(0)
                }
                "codex.subscriptions" => runtime.subscriptions = value.parse().unwrap_or(0),
                "codex.warm_surfaces" => runtime.warm_surfaces = value.parse().unwrap_or(0),
                "codex.cache_entries" => runtime.cache_entries = value.parse().unwrap_or(0),
                "codex.source" => runtime.source_label = value.to_owned(),
                "codex.diagnostic" => {
                    runtime.diagnostic = (!value.is_empty()).then(|| value.to_owned())
                }
                _ => {}
            }
        }
        runtime.version = 1;
        Ok(runtime)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        atomic_write(
            path,
            format!(
                "version=1\ncodex.generation={}\ncodex.effective={}\ncodex.health={}\ncodex.support={}\ncodex.installation={}\ncodex.active_windows={}\ncodex.background_workers={}\ncodex.subscriptions={}\ncodex.warm_surfaces={}\ncodex.cache_entries={}\ncodex.source={}\ncodex.diagnostic={}\n",
                self.codex_generation,
                format_effective(self.codex_effective),
                format_health(self.codex_health),
                format_support(self.codex_support),
                format_installation(self.codex_installation),
                self.active_windows,
                self.background_workers,
                self.subscriptions,
                self.warm_surfaces,
                self.cache_entries,
                sanitize(&self.source_label),
                sanitize(self.diagnostic.as_deref().unwrap_or(""))
            ),
        )
    }

    pub fn disabled_is_quiescent(&self) -> bool {
        self.codex_effective == FeatureEffectiveState::Disabled
            && self.active_windows == 0
            && self.background_workers == 0
            && self.subscriptions == 0
            && self.warm_surfaces == 0
            && self.cache_entries == 0
    }
}

fn sanitize(value: &str) -> String {
    value.replace(['\n', '\r'], " ")
}
fn format_effective(value: FeatureEffectiveState) -> &'static str {
    match value {
        FeatureEffectiveState::Disabled => "disabled",
        FeatureEffectiveState::Enabling => "enabling",
        FeatureEffectiveState::Enabled => "enabled",
        FeatureEffectiveState::Unavailable => "unavailable",
        FeatureEffectiveState::Rejected => "rejected",
        FeatureEffectiveState::Stale => "stale",
    }
}
fn parse_effective(value: &str) -> FeatureEffectiveState {
    match value {
        "enabling" => FeatureEffectiveState::Enabling,
        "enabled" => FeatureEffectiveState::Enabled,
        "unavailable" => FeatureEffectiveState::Unavailable,
        "rejected" => FeatureEffectiveState::Rejected,
        "stale" => FeatureEffectiveState::Stale,
        _ => FeatureEffectiveState::Disabled,
    }
}
fn format_health(value: FeatureHealth) -> &'static str {
    match value {
        FeatureHealth::Unknown => "unknown",
        FeatureHealth::Loading => "loading",
        FeatureHealth::SignedOut => "signed-out",
        FeatureHealth::Ready => "ready",
        FeatureHealth::Failed => "failed",
    }
}
fn parse_health(value: &str) -> FeatureHealth {
    match value {
        "loading" => FeatureHealth::Loading,
        "signed-out" => FeatureHealth::SignedOut,
        "ready" => FeatureHealth::Ready,
        "failed" => FeatureHealth::Failed,
        _ => FeatureHealth::Unknown,
    }
}
fn format_support(value: FeatureSupport) -> &'static str {
    match value {
        FeatureSupport::Supported => "supported",
        FeatureSupport::Unsupported => "unsupported",
    }
}
fn parse_support(value: &str) -> FeatureSupport {
    match value {
        "unsupported" => FeatureSupport::Unsupported,
        _ => FeatureSupport::Supported,
    }
}
fn format_installation(value: FeatureInstallation) -> &'static str {
    match value {
        FeatureInstallation::Installed => "installed",
        FeatureInstallation::Missing => "missing",
        FeatureInstallation::Incompatible => "incompatible",
    }
}
fn parse_installation(value: &str) -> FeatureInstallation {
    match value {
        "installed" => FeatureInstallation::Installed,
        "incompatible" => FeatureInstallation::Incompatible,
        _ => FeatureInstallation::Missing,
    }
}

fn parse_bool(value: &str) -> Option<bool> {
    match value.trim() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}
fn parse_source(value: &str) -> CodexSource {
    match value {
        "bundled" => CodexSource::Bundled,
        "remote" => CodexSource::ApprovedRemote,
        value if value.starts_with("executable:") => {
            CodexSource::Executable(PathBuf::from(&value[11..]))
        }
        _ => CodexSource::CompatibleInstalled,
    }
}

pub fn settings_path() -> io::Result<PathBuf> {
    config_path("optional-features")
}

fn runtime_path() -> io::Result<PathBuf> {
    settings_path().map(|path| path.with_file_name("optional-features-runtime"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    fn capability(
        support: FeatureSupport,
        installation: FeatureInstallation,
        health: FeatureHealth,
        policy: FeaturePolicy,
    ) -> FeatureCapability {
        FeatureCapability {
            support,
            installation,
            health,
            policy,
            policy_source: None,
            required_permissions: Vec::new(),
            configuration_destination: Some("optional-features/codex".into()),
            apply_requirement: ApplyRequirement::Live,
            source_label: "fixture".into(),
            diagnostic: None,
        }
    }
    #[test]
    fn state_matrix_is_truthful() {
        assert_eq!(
            FeatureState::resolve(
                false,
                1,
                1,
                capability(
                    FeatureSupport::Supported,
                    FeatureInstallation::Installed,
                    FeatureHealth::Ready,
                    FeaturePolicy::Editable
                )
            )
            .effective,
            FeatureEffectiveState::Disabled
        );
        assert_eq!(
            FeatureState::resolve(
                true,
                1,
                1,
                capability(
                    FeatureSupport::Unsupported,
                    FeatureInstallation::Installed,
                    FeatureHealth::Ready,
                    FeaturePolicy::Editable
                )
            )
            .effective,
            FeatureEffectiveState::Unavailable
        );
        assert_eq!(
            FeatureState::resolve(
                true,
                2,
                1,
                capability(
                    FeatureSupport::Supported,
                    FeatureInstallation::Installed,
                    FeatureHealth::Ready,
                    FeaturePolicy::Editable
                )
            )
            .effective,
            FeatureEffectiveState::Enabling
        );
        assert_eq!(
            FeatureState::resolve(
                true,
                1,
                1,
                capability(
                    FeatureSupport::Supported,
                    FeatureInstallation::Installed,
                    FeatureHealth::Failed,
                    FeaturePolicy::Editable
                )
            )
            .effective,
            FeatureEffectiveState::Rejected
        );
        assert_eq!(
            FeatureState::resolve(
                true,
                1,
                1,
                capability(
                    FeatureSupport::Supported,
                    FeatureInstallation::Installed,
                    FeatureHealth::Ready,
                    FeaturePolicy::Editable
                )
            )
            .effective,
            FeatureEffectiveState::Enabled
        );
    }

    #[test]
    fn codex_presentation_matrix_keeps_only_recoverable_installations_visible() {
        for support in [FeatureSupport::Supported, FeatureSupport::Unsupported] {
            for installation in [
                FeatureInstallation::Installed,
                FeatureInstallation::Missing,
                FeatureInstallation::Incompatible,
            ] {
                for enabled in [false, true] {
                    for health in [
                        FeatureHealth::Unknown,
                        FeatureHealth::Loading,
                        FeatureHealth::SignedOut,
                        FeatureHealth::Ready,
                        FeatureHealth::Failed,
                    ] {
                        let projection = CodexAvailabilityProjection::new(
                            support,
                            installation,
                            enabled,
                            health,
                            7,
                            None,
                        );
                        let expected = if !enabled
                            || support == FeatureSupport::Unsupported
                            || installation == FeatureInstallation::Missing
                        {
                            CodexPresentation::Hidden
                        } else if installation == FeatureInstallation::Installed
                            && health == FeatureHealth::Ready
                        {
                            CodexPresentation::Projects
                        } else {
                            CodexPresentation::Recoverable
                        };
                        assert_eq!(
                            projection.presentation(),
                            expected,
                            "{support:?} {installation:?} {enabled} {health:?}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn codex_projection_bounds_reasons_and_rejects_older_generations() {
        let projection = CodexAvailabilityProjection::new(
            FeatureSupport::Supported,
            FeatureInstallation::Installed,
            true,
            FeatureHealth::Failed,
            12,
            Some("x".repeat(400)),
        );
        assert_eq!(projection.reason.as_ref().unwrap().chars().count(), 256);
        assert!(!projection.accepts_after(13));
        assert!(projection.accepts_after(12));
    }
    #[test]
    fn every_application_requirement_is_truthful() {
        let mut state = FeatureState::resolve(
            false,
            0,
            0,
            capability(
                FeatureSupport::Supported,
                FeatureInstallation::Installed,
                FeatureHealth::Ready,
                FeaturePolicy::Editable,
            ),
        );
        for (requirement, label) in [
            (ApplyRequirement::Live, "Applies live"),
            (
                ApplyRequirement::ShellRestart,
                "Applies after shell restart",
            ),
            (
                ApplyRequirement::ApplicationRestart,
                "Applies after application restart",
            ),
            (ApplyRequirement::Login, "Applies after login"),
        ] {
            state.capability.apply_requirement = requirement;
            assert_eq!(state.apply_label(), label);
        }
    }
    #[test]
    fn policy_is_distinct_from_preference() {
        let disabled = FeatureState::resolve(
            true,
            1,
            1,
            capability(
                FeatureSupport::Supported,
                FeatureInstallation::Installed,
                FeatureHealth::Ready,
                FeaturePolicy::ForceDisabled,
            ),
        );
        assert!(!disabled.requested_enabled);
        assert!(!disabled.editable());
        let enabled = FeatureState::resolve(
            false,
            1,
            1,
            capability(
                FeatureSupport::Supported,
                FeatureInstallation::Installed,
                FeatureHealth::Ready,
                FeaturePolicy::ForceEnabled,
            ),
        );
        assert!(enabled.requested_enabled);
        assert!(!enabled.editable());
    }
    #[test]
    fn explicit_policy_source_values_are_dynamic_and_truthful() {
        assert_eq!(
            policy_from_value("force-enabled"),
            FeaturePolicy::ForceEnabled
        );
        assert_eq!(policy_from_value("OFF"), FeaturePolicy::ForceDisabled);
        assert_eq!(policy_from_value("invalid"), FeaturePolicy::Editable);
    }
    #[test]
    fn persistence_round_trip_and_corruption_fallback() {
        let path =
            std::env::temp_dir().join(format!("nickel-optional-features-{}", std::process::id()));
        let expected = OptionalFeatureSettings {
            codex_enabled: false,
            codex_source: CodexSource::Executable(PathBuf::from("/opt/codex")),
            ..Default::default()
        };
        expected.save(&path).unwrap();
        assert_eq!(OptionalFeatureSettings::load(&path).unwrap(), expected);
        fs::write(
            &path,
            "version=nope\ncodex.enabled=nonsense\ncodex.source=unknown\n",
        )
        .unwrap();
        assert_eq!(
            OptionalFeatureSettings::load(&path).unwrap(),
            OptionalFeatureSettings::default()
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn keyboard_preference_updates_preserve_codex_and_migrate_missing_values() {
        use crate::on_screen_keyboard::KeyboardPreference;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("features.conf");
        fs::write(
            &path,
            "version=1\ncodex.enabled=false\ncodex.generation=42\n",
        )
        .unwrap();
        let loaded = OptionalFeatureSettings::load(&path).unwrap();
        assert_eq!(loaded.on_screen_keyboard, KeyboardPreference::Automatic);
        for preference in [
            KeyboardPreference::Enabled,
            KeyboardPreference::Disabled,
            KeyboardPreference::Automatic,
        ] {
            let updated = OptionalFeatureSettings::update(&path, |settings| {
                settings.on_screen_keyboard = preference;
                settings.on_screen_keyboard_generation += 1;
            })
            .unwrap();
            assert!(!updated.codex_enabled);
            assert_eq!(updated.codex_generation, 42);
            assert_eq!(OptionalFeatureSettings::load(&path).unwrap(), updated);
        }
        let stored = OptionalFeatureSettings::load(&path).unwrap();
        assert_eq!(stored.on_screen_keyboard_generation, 3);
        // Environment resolution cannot mutate the stored preference.
        assert!(
            crate::on_screen_keyboard::resolve_enablement(
                stored.on_screen_keyboard,
                crate::on_screen_keyboard::KeyboardOverride::Enabled,
                crate::on_screen_keyboard::TouchscreenPresence::Absent,
            )
            .enabled
        );
        assert_eq!(OptionalFeatureSettings::load(&path).unwrap(), stored);
    }

    #[test]
    fn every_source_and_generation_round_trip() {
        let sources = [
            CodexSource::CompatibleInstalled,
            CodexSource::Bundled,
            CodexSource::ApprovedRemote,
            CodexSource::Executable(PathBuf::from("/custom/codex")),
        ];
        for (index, source) in sources.into_iter().enumerate() {
            let path = std::env::temp_dir().join(format!(
                "nickel-optional-source-{}-{index}",
                std::process::id()
            ));
            let expected = OptionalFeatureSettings {
                codex_enabled: index % 2 == 0,
                codex_generation: index as u64 + 9,
                codex_source: source,
                ..Default::default()
            };
            expected.save(&path).unwrap();
            assert_eq!(OptionalFeatureSettings::load(&path).unwrap(), expected);
            fs::remove_file(path).unwrap();
        }
    }

    #[test]
    fn stale_acknowledgements_remain_pending() {
        let state = FeatureState::resolve(
            true,
            8,
            7,
            capability(
                FeatureSupport::Supported,
                FeatureInstallation::Installed,
                FeatureHealth::Ready,
                FeaturePolicy::Editable,
            ),
        );
        assert_eq!(state.effective, FeatureEffectiveState::Enabling);
        assert_eq!(state.acknowledged_generation, 7);
        let future = FeatureState::resolve(
            true,
            8,
            9,
            capability(
                FeatureSupport::Supported,
                FeatureInstallation::Installed,
                FeatureHealth::Ready,
                FeaturePolicy::Editable,
            ),
        );
        assert_eq!(future.effective, FeatureEffectiveState::Stale);
    }

    #[test]
    fn runtime_diagnostics_round_trip_and_prove_quiescence() {
        let path =
            std::env::temp_dir().join(format!("nickel-optional-runtime-{}", std::process::id()));
        let disabled = OptionalFeatureRuntime {
            codex_generation: 17,
            codex_installation: FeatureInstallation::Installed,
            ..Default::default()
        };
        disabled.save(&path).unwrap();
        let loaded = OptionalFeatureRuntime::load(&path).unwrap();
        assert_eq!(loaded, disabled);
        assert!(loaded.disabled_is_quiescent());
        let active = OptionalFeatureRuntime {
            codex_effective: FeatureEffectiveState::Disabled,
            background_workers: 1,
            ..Default::default()
        };
        assert!(!active.disabled_is_quiescent());
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn upgrade_keeps_an_explicitly_disabled_preference() {
        let path =
            std::env::temp_dir().join(format!("nickel-optional-upgrade-{}", std::process::id()));
        fs::write(
            &path,
            "version=0\ncodex.enabled=false\nunknown.future=value\n",
        )
        .unwrap();
        let loaded = OptionalFeatureSettings::load(&path).unwrap();
        assert!(!loaded.codex_enabled);
        assert_eq!(loaded.version, 1);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn settings_windows_rebase_on_the_latest_persisted_generation() {
        let path =
            std::env::temp_dir().join(format!("nickel-optional-concurrent-{}", std::process::id()));
        OptionalFeatureSettings::default().save(&path).unwrap();
        let _stale_first = OptionalFeatureSettings::load(&path).unwrap();
        let _stale_second = OptionalFeatureSettings::load(&path).unwrap();
        OptionalFeatureSettings::update(&path, |settings| {
            settings.codex_enabled = false;
            settings.codex_generation += 1;
        })
        .unwrap();
        OptionalFeatureSettings::update(&path, |settings| {
            settings.codex_source = CodexSource::Bundled;
            settings.codex_generation += 1;
        })
        .unwrap();
        let merged = OptionalFeatureSettings::load(&path).unwrap();
        assert!(!merged.codex_enabled);
        assert_eq!(merged.codex_source, CodexSource::Bundled);
        assert_eq!(merged.codex_generation, 2);
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn settings_and_runtime_reject_oversized_and_non_utf8_transport() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("optional-features");
        fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES + 1]).unwrap();
        assert_eq!(
            OptionalFeatureSettings::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(&path, b"version=1\ninvalid=\xff\n").unwrap();
        assert_eq!(
            OptionalFeatureSettings::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        // Keep the two persisted schemas under the same transport bound.
        fs::write(&path, vec![b' '; MAX_SETTINGS_BYTES + 1]).unwrap();
        assert_eq!(
            OptionalFeatureRuntime::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
        fs::write(&path, b"version=1\ninvalid=\xff\n").unwrap();
        assert_eq!(
            OptionalFeatureRuntime::load(&path).unwrap_err().kind(),
            io::ErrorKind::InvalidData
        );
    }

    #[test]
    fn staged_keyboard_change_preserves_codex_and_rejects_cancel_or_replacement() {
        use crate::on_screen_keyboard::KeyboardPreference;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("optional-features");
        let prior = OptionalFeatureSettings {
            codex_enabled: false,
            codex_generation: 42,
            codex_source: CodexSource::Executable("/private/codex".into()),
            on_screen_keyboard_generation: 7,
            ..Default::default()
        };
        prior.save(&path).unwrap();
        let cancelled =
            PreparedKeyboardPreference::prepare(path.clone(), &prior, KeyboardPreference::Enabled)
                .unwrap();
        assert!(
            cancelled
                .commit(|| Err(io::Error::other("cancelled")))
                .is_err()
        );
        assert_eq!(OptionalFeatureSettings::load(&path).unwrap(), prior);

        let stale =
            PreparedKeyboardPreference::prepare(path.clone(), &prior, KeyboardPreference::Disabled)
                .unwrap();
        fs::write(
            &path,
            prior
                .encode()
                .replace("codex.generation=42", "codex.generation=43"),
        )
        .unwrap();
        assert!(stale.commit(|| Ok(())).is_err());

        let current = OptionalFeatureSettings::load(&path).unwrap();
        let accepted = PreparedKeyboardPreference::prepare(
            path.clone(),
            &current,
            KeyboardPreference::Enabled,
        )
        .unwrap()
        .commit(|| Ok(()))
        .unwrap();
        assert_eq!(accepted.codex_generation, 43);
        assert_eq!(accepted.codex_source, prior.codex_source);
        assert_eq!(accepted.on_screen_keyboard, KeyboardPreference::Enabled);
        assert_eq!(accepted.on_screen_keyboard_generation, 8);
    }
}
