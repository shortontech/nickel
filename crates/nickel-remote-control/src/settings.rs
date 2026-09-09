use std::{fs, io::ErrorKind, path::Path};

use serde::{Deserialize, Serialize};

const SETTINGS_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteAiControlSettings {
    version: u16,
    pub requested_enabled: bool,
    pub generation: u64,
}

impl Default for RemoteAiControlSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            requested_enabled: false,
            generation: 0,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("remote-control settings are unavailable: {0}")]
    Unavailable(#[from] std::io::Error),
    #[error("remote-control settings are invalid: {0}")]
    Invalid(String),
    #[error("remote-control settings cannot be decoded: {0}")]
    Decode(#[from] toml::de::Error),
    #[error("remote-control settings cannot be encoded: {0}")]
    Encode(#[from] toml::ser::Error),
}

impl RemoteAiControlSettings {
    pub fn default_path() -> Result<std::path::PathBuf, SettingsError> {
        Ok(nickel_storage::config_path("remote-ai-control.toml")?)
    }

    pub fn load_default() -> Result<Self, SettingsError> {
        Self::load(Self::default_path()?)
    }

    /// Missing and malformed state fail closed. The error remains available to Settings while the
    /// caller can safely use `Default` as its effective preference.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SettingsError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(SettingsError::Unavailable(error)),
        };
        let settings: Self = toml::from_str(&text)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn set_requested(&mut self, enabled: bool) -> bool {
        if self.requested_enabled == enabled {
            return false;
        }
        self.requested_enabled = enabled;
        self.generation = self.generation.saturating_add(1);
        true
    }

    pub fn save(&self, path: impl AsRef<Path>) -> Result<(), SettingsError> {
        self.validate()?;
        let encoded = toml::to_string_pretty(self)?;
        nickel_storage::atomic_write(path.as_ref(), encoded)?;
        Ok(())
    }

    fn validate(&self) -> Result<(), SettingsError> {
        if self.version != SETTINGS_VERSION {
            return Err(SettingsError::Invalid(format!(
                "unsupported version {}",
                self.version
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_defaults_disabled_and_corruption_never_enables() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote.toml");
        assert_eq!(
            RemoteAiControlSettings::load(&path).unwrap(),
            Default::default()
        );
        fs::write(&path, "requested_enabled = true\nnot valid").unwrap();
        assert!(RemoteAiControlSettings::load(&path).is_err());
        assert!(!RemoteAiControlSettings::default().requested_enabled);
    }

    #[test]
    fn persisted_preference_is_versioned_atomic_and_generation_bearing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote.toml");
        let mut settings = RemoteAiControlSettings::default();
        assert!(settings.set_requested(true));
        assert!(!settings.set_requested(true));
        settings.save(&path).unwrap();
        assert_eq!(RemoteAiControlSettings::load(&path).unwrap(), settings);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
