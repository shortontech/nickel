//! Portable policy and persistence for optional Nickel features.

use std::{
    fs, io,
    path::{Path, PathBuf},
    thread,
    time::{Duration, SystemTime},
};

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum OptionalFeatureId {
    Codex,
}

impl OptionalFeatureId {
    pub const fn stable_id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
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
            || capability.health == FeatureHealth::Loading
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
}

impl Default for OptionalFeatureSettings {
    fn default() -> Self {
        Self {
            version: 1,
            codex_enabled: true,
            codex_generation: 0,
            codex_source: CodexSource::default(),
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
        let lock = path.with_extension("lock");
        if let Some(parent) = lock.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut acquired = None;
        for _ in 0..200 {
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&lock)
            {
                Ok(file) => {
                    acquired = Some(file);
                    break;
                }
                Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                    let stale = fs::metadata(&lock)
                        .and_then(|metadata| metadata.modified())
                        .ok()
                        .and_then(|modified| SystemTime::now().duration_since(modified).ok())
                        .is_some_and(|age| age >= Duration::from_secs(10));
                    if stale {
                        let _ = fs::remove_file(&lock);
                    }
                    thread::sleep(Duration::from_millis(5));
                }
                Err(error) => return Err(error),
            }
        }
        let guard = acquired.ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::WouldBlock,
                "optional feature settings are busy",
            )
        })?;
        let mut settings = match Self::load(path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                drop(guard);
                let _ = fs::remove_file(&lock);
                return Err(error);
            }
        };
        update(&mut settings);
        let result = settings.save(path);
        drop(guard);
        let _ = fs::remove_file(lock);
        result.map(|()| settings)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
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
                _ => {}
            }
        }
        settings.version = 1;
        Ok(settings)
    }

    pub fn save(&self, path: impl AsRef<Path>) -> io::Result<()> {
        let path = path.as_ref();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let source = match &self.codex_source {
            CodexSource::CompatibleInstalled => "installed".to_owned(),
            CodexSource::Bundled => "bundled".to_owned(),
            CodexSource::ApprovedRemote => "remote".to_owned(),
            CodexSource::Executable(path) => format!("executable:{}", path.to_string_lossy()),
        };
        fs::write(
            path,
            format!(
                "version=1\ncodex.enabled={}\ncodex.generation={}\ncodex.source={source}\n",
                self.codex_enabled, self.codex_generation
            ),
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionalFeatureRuntime {
    pub version: u16,
    pub codex_generation: u64,
    pub codex_effective: FeatureEffectiveState,
    pub codex_health: FeatureHealth,
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
        let contents = fs::read_to_string(path)?;
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
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(
            path,
            format!(
                "version=1\ncodex.generation={}\ncodex.effective={}\ncodex.health={}\ncodex.active_windows={}\ncodex.background_workers={}\ncodex.subscriptions={}\ncodex.warm_surfaces={}\ncodex.cache_entries={}\ncodex.source={}\ncodex.diagnostic={}\n",
                self.codex_generation,
                format_effective(self.codex_effective),
                format_health(self.codex_health),
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

fn settings_path() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    if let Some(local) = std::env::var_os("LOCALAPPDATA") {
        return Ok(PathBuf::from(local)
            .join("Nickel")
            .join("optional-features"));
    }
    #[cfg(target_os = "windows")]
    return Err(io::Error::new(
        io::ErrorKind::NotFound,
        "LOCALAPPDATA is not set",
    ));
    #[cfg(target_os = "macos")]
    if let Some(home) = std::env::var_os("HOME") {
        return Ok(PathBuf::from(home).join("Library/Application Support/Nickel/optional-features"));
    }
    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_CONFIG_HOME and HOME are not set",
                )
            })?;
        Ok(config.join("nickel").join("optional-features"))
    }
    #[cfg(target_os = "macos")]
    Err(io::Error::new(io::ErrorKind::NotFound, "HOME is not set"))
}

fn runtime_path() -> io::Result<PathBuf> {
    settings_path().map(|path| path.with_file_name("optional-features-runtime"))
}

#[cfg(test)]
mod tests {
    use super::*;
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
}
