use std::{
    fs,
    io::{ErrorKind, Read},
    path::Path,
};

use serde::{Deserialize, Serialize};

const SETTINGS_VERSION: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteAiControlSettings {
    version: u16,
    pub requested_enabled: bool,
    #[serde(default = "audible_default")]
    pub audible_indications: bool,
    pub generation: u64,
}

fn audible_default() -> bool {
    true
}

impl Default for RemoteAiControlSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            requested_enabled: true,
            audible_indications: true,
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

    /// A missing preference starts the capability-free listener. Malformed state is reported;
    /// no saved client authority is loaded by either this preference or its default.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, SettingsError> {
        let mut options = fs::OpenOptions::new();
        options.read(true);
        // Do not wait for a FIFO writer or follow a replaced preference symlink.
        // Inspect the opened handle, closing the metadata/open race for devices.
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.custom_flags(libc::O_NONBLOCK | libc::O_NOFOLLOW);
        }
        let file = match options.open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(SettingsError::Unavailable(error)),
        };
        let metadata = file.metadata()?;
        if !metadata.is_file() {
            return Err(SettingsError::Invalid(
                "settings must be a regular file".into(),
            ));
        }
        if metadata.len() > 64 * 1024 {
            return Err(SettingsError::Invalid("settings exceed size limit".into()));
        }
        let mut text = String::new();
        file.take(64 * 1024 + 1).read_to_string(&mut text)?;
        if text.len() > 64 * 1024 {
            return Err(SettingsError::Invalid("settings exceed size limit".into()));
        }
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

    #[cfg(unix)]
    #[test]
    fn preference_special_files_are_rejected_without_waiting_for_a_writer() {
        use std::{
            ffi::CString,
            os::unix::{ffi::OsStrExt, fs::symlink},
            time::{Duration, Instant},
        };
        let directory = tempfile::tempdir().unwrap();
        let fifo = directory.path().join("fifo");
        let name = CString::new(fifo.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a valid NUL-terminated path in this owned temp directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        let link = directory.path().join("link");
        symlink(&fifo, &link).unwrap();
        let before = Instant::now();
        for path in [
            fifo.as_path(),
            link.as_path(),
            Path::new("/dev/zero"),
            directory.path(),
        ] {
            assert!(RemoteAiControlSettings::load(path).is_err());
        }
        assert!(before.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn audible_preference_is_backward_compatible_and_round_trips() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote.toml");
        fs::write(
            &path,
            "version = 1\nrequested_enabled = true\ngeneration = 9\n",
        )
        .unwrap();
        let mut settings = RemoteAiControlSettings::load(&path).unwrap();
        assert!(settings.audible_indications);
        settings.audible_indications = false;
        settings.save(&path).unwrap();
        assert_eq!(RemoteAiControlSettings::load(&path).unwrap(), settings);
        assert_eq!(settings.generation, 9);
    }
    #[test]
    fn missing_defaults_to_capability_free_listener_and_corruption_is_reported() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote.toml");
        assert_eq!(
            RemoteAiControlSettings::load(&path).unwrap(),
            Default::default()
        );
        fs::write(&path, "requested_enabled = true\nnot valid").unwrap();
        assert!(RemoteAiControlSettings::load(&path).is_err());
        assert!(RemoteAiControlSettings::default().requested_enabled);
    }

    #[test]
    fn persisted_preference_is_versioned_atomic_and_generation_bearing() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("remote.toml");
        let mut settings = RemoteAiControlSettings::default();
        assert!(settings.set_requested(false));
        assert!(!settings.set_requested(false));
        settings.save(&path).unwrap();
        assert_eq!(RemoteAiControlSettings::load(&path).unwrap(), settings);
        assert_eq!(fs::read_dir(directory.path()).unwrap().count(), 1);
    }
}
