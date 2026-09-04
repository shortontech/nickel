//! Portable policy and persistence for optional Nickel features.

use std::{
    fs, io,
    path::{Path, PathBuf},
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ApplyRequirement {
    Live,
    ShellRestart,
    ApplicationRestart,
    Login,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeatureCapability {
    pub support: FeatureSupport,
    pub installation: FeatureInstallation,
    pub health: FeatureHealth,
    pub policy: FeaturePolicy,
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
        } else if generation != acknowledged_generation {
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
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionalFeatureSettings {
    pub version: u16,
    pub codex_enabled: bool,
    pub codex_source: CodexSource,
}

impl Default for OptionalFeatureSettings {
    fn default() -> Self {
        Self {
            version: 1,
            codex_enabled: true,
            codex_source: CodexSource::default(),
        }
    }
}

impl OptionalFeatureSettings {
    pub fn load_default() -> Self {
        settings_path().and_then(Self::load).unwrap_or_default()
    }
    pub fn save_default(&self) -> io::Result<()> {
        self.save(settings_path()?)
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
                "version=1\ncodex.enabled={}\ncodex.source={source}\n",
                self.codex_enabled
            ),
        )
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
}
