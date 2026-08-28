use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use url::Url;

const SETTINGS_VERSION: u32 = 1;
const MAX_HOSTS: usize = 64;
const MAX_ID: usize = 64;
const MAX_NAME: usize = 128;
const MAX_ENDPOINT: usize = 2048;
const MAX_CWD: usize = 4096;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteHost {
    pub id: String,
    pub name: String,
    pub endpoint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_env: Option<String>,
    pub default_cwd: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CodexSettings {
    version: u32,
    pub selected: String,
    #[serde(default)]
    pub hosts: Vec<RemoteHost>,
}

impl Default for CodexSettings {
    fn default() -> Self {
        Self {
            version: SETTINGS_VERSION,
            selected: "local".into(),
            hosts: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum SettingsError {
    #[error("Codex host settings are unavailable: {0}")]
    Unavailable(String),
    #[error("invalid Codex host settings: {0}")]
    Invalid(String),
    #[error("cannot read Codex host settings: {0}")]
    Read(#[source] std::io::Error),
    #[error("cannot decode Codex host settings: {0}")]
    Decode(#[source] toml::de::Error),
    #[error("cannot encode Codex host settings: {0}")]
    Encode(#[source] toml::ser::Error),
    #[error("cannot save Codex host settings: {0}")]
    Save(#[source] std::io::Error),
}

impl CodexSettings {
    pub fn default_path() -> Result<PathBuf, SettingsError> {
        dirs::config_dir()
            .map(|directory| directory.join("nickel").join("codex-hosts.toml"))
            .ok_or_else(|| {
                SettingsError::Unavailable("platform config directory is missing".into())
            })
    }

    pub fn load_default() -> Result<Self, SettingsError> {
        Self::load(&Self::default_path()?)
    }

    pub fn load(path: &Path) -> Result<Self, SettingsError> {
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(SettingsError::Read(error)),
        };
        let settings: Self = toml::from_str(&text).map_err(SettingsError::Decode)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save_default(&self) -> Result<(), SettingsError> {
        self.save(&Self::default_path()?)
    }

    pub fn save(&self, path: &Path) -> Result<(), SettingsError> {
        self.validate()?;
        let encoded = toml::to_string_pretty(self).map_err(SettingsError::Encode)?;
        let parent = path.parent().ok_or_else(|| {
            SettingsError::Invalid("settings path has no parent directory".into())
        })?;
        create_private_directory(parent).map_err(SettingsError::Save)?;
        let result = (|| {
            let mut file = tempfile::Builder::new()
                .prefix(".codex-hosts-")
                .suffix(".tmp")
                .tempfile_in(parent)?;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                file.as_file()
                    .set_permissions(fs::Permissions::from_mode(0o600))?;
            }
            file.write_all(encoded.as_bytes())?;
            file.as_file().sync_all()?;
            file.persist(path).map_err(|error| error.error)?;
            sync_directory(parent)?;
            Ok::<_, std::io::Error>(())
        })();
        result.map_err(SettingsError::Save)
    }

    pub fn validate(&self) -> Result<(), SettingsError> {
        if self.version != SETTINGS_VERSION {
            return Err(SettingsError::Invalid(format!(
                "unsupported settings version {}",
                self.version
            )));
        }
        if self.hosts.len() > MAX_HOSTS {
            return Err(SettingsError::Invalid(format!(
                "at most {MAX_HOSTS} remote hosts are supported"
            )));
        }
        let mut ids = std::collections::HashSet::new();
        for host in &self.hosts {
            host.validate()?;
            if !ids.insert(&host.id) {
                return Err(SettingsError::Invalid(format!(
                    "duplicate remote host id {}",
                    host.id
                )));
            }
        }
        if self.selected != "local" && !ids.contains(&self.selected) {
            return Err(SettingsError::Invalid(format!(
                "selected remote host {} does not exist",
                self.selected
            )));
        }
        Ok(())
    }

    pub fn selected_host(&self) -> Option<&RemoteHost> {
        (self.selected != "local")
            .then(|| self.hosts.iter().find(|host| host.id == self.selected))
            .flatten()
    }

    pub fn remove_host(&mut self, id: &str) -> bool {
        let original = self.hosts.len();
        self.hosts.retain(|host| host.id != id);
        let removed = self.hosts.len() != original;
        if removed && self.selected == id {
            self.selected = "local".into();
        }
        removed
    }
}

impl RemoteHost {
    pub fn validate(&self) -> Result<(), SettingsError> {
        validate_host_identifier(&self.id)?;
        if self.id == "local" {
            return Err(SettingsError::Invalid(
                "remote host id local is reserved".into(),
            ));
        }
        validate_text("remote host name", &self.name, MAX_NAME)?;
        validate_text("remote endpoint", &self.endpoint, MAX_ENDPOINT)?;
        let endpoint = Url::parse(&self.endpoint)
            .map_err(|error| SettingsError::Invalid(format!("invalid remote endpoint: {error}")))?;
        if !matches!(endpoint.scheme(), "ws" | "wss") {
            return Err(SettingsError::Invalid(
                "remote endpoint must use ws or wss".into(),
            ));
        }
        if endpoint.host_str().is_none() {
            return Err(SettingsError::Invalid(
                "remote endpoint must include a host".into(),
            ));
        }
        if !endpoint.username().is_empty() || endpoint.password().is_some() {
            return Err(SettingsError::Invalid(
                "remote endpoint must not contain credentials".into(),
            ));
        }
        if endpoint.fragment().is_some() {
            return Err(SettingsError::Invalid(
                "remote endpoint must not contain a fragment".into(),
            ));
        }
        if let Some(token_env) = &self.token_env {
            validate_identifier("token environment variable", token_env, MAX_NAME)?;
        }
        validate_text("remote working directory", &self.default_cwd, MAX_CWD)?;
        if !remote_path_is_absolute(&self.default_cwd) {
            return Err(SettingsError::Invalid(
                "remote working directory must be absolute".into(),
            ));
        }
        Ok(())
    }
}

fn validate_host_identifier(value: &str) -> Result<(), SettingsError> {
    validate_text("remote host id", value, MAX_ID)?;
    if !value.bytes().enumerate().all(|(index, byte)| {
        byte == b'-'
            || byte == b'_'
            || byte.is_ascii_alphanumeric() && (index > 0 || !byte.is_ascii_digit())
    }) {
        return Err(SettingsError::Invalid(
            "remote host id must contain only ASCII letters, digits, hyphens, and underscores and cannot start with a digit"
                .into(),
        ));
    }
    Ok(())
}

fn validate_identifier(label: &str, value: &str, maximum: usize) -> Result<(), SettingsError> {
    validate_text(label, value, maximum)?;
    if !value.bytes().enumerate().all(|(index, byte)| {
        byte == b'_' || byte.is_ascii_alphanumeric() && (index > 0 || !byte.is_ascii_digit())
    }) {
        return Err(SettingsError::Invalid(format!(
            "{label} must contain only ASCII letters, digits, and underscores and cannot start with a digit"
        )));
    }
    Ok(())
}

fn validate_text(label: &str, value: &str, maximum: usize) -> Result<(), SettingsError> {
    if value.trim().is_empty() {
        return Err(SettingsError::Invalid(format!("{label} cannot be blank")));
    }
    if value.len() > maximum {
        return Err(SettingsError::Invalid(format!(
            "{label} exceeds {maximum} bytes"
        )));
    }
    if value.chars().any(char::is_control) {
        return Err(SettingsError::Invalid(format!(
            "{label} must not contain control characters"
        )));
    }
    Ok(())
}

fn remote_path_is_absolute(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with("\\\\")
        || path
            .as_bytes()
            .get(1..3)
            .is_some_and(|suffix| suffix[0] == b':' && matches!(suffix[1], b'/' | b'\\'))
}

fn create_private_directory(path: &Path) -> std::io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn sync_directory(path: &Path) -> std::io::Result<()> {
    #[cfg(unix)]
    {
        fs::File::open(path)?.sync_all()
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    fn host() -> RemoteHost {
        RemoteHost {
            id: "workstation".into(),
            name: "Workstation".into(),
            endpoint: "wss://codex.example.test/app-server".into(),
            token_env: Some("NICKEL_CODEX_TOKEN".into()),
            default_cwd: "/projects/nickel".into(),
        }
    }

    #[test]
    fn missing_settings_are_local_without_creating_a_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing.toml");
        assert_eq!(
            CodexSettings::load(&path).unwrap(),
            CodexSettings::default()
        );
        assert!(!path.exists());
    }

    #[test]
    fn settings_round_trip_preserves_hosts_in_a_private_file() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nickel").join("hosts.toml");
        let settings = CodexSettings {
            version: SETTINGS_VERSION,
            selected: "workstation".into(),
            hosts: vec![host()],
        };
        settings.save(&path).unwrap();
        assert_eq!(CodexSettings::load(&path).unwrap(), settings);
        let stored = fs::read_to_string(&path).unwrap();
        assert!(stored.contains("NICKEL_CODEX_TOKEN"));
        assert!(!stored.contains("actual-secret-value"));
        #[cfg(unix)]
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }

    #[test]
    fn invalid_settings_remain_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("hosts.toml");
        fs::write(&path, "version = 99\nselected = \"local\"\n").unwrap();
        assert!(CodexSettings::load(&path).is_err());
        assert_eq!(
            fs::read_to_string(path).unwrap(),
            "version = 99\nselected = \"local\"\n"
        );
    }

    #[test]
    fn validation_rejects_credentials_duplicates_and_relative_paths() {
        let mut credential = host();
        credential.endpoint = "wss://user:secret@example.test".into();
        assert_eq!(
            credential.validate().unwrap_err().to_string(),
            "invalid Codex host settings: remote endpoint must not contain credentials"
        );

        let mut relative = host();
        relative.default_cwd = "projects/nickel".into();
        assert_eq!(
            relative.validate().unwrap_err().to_string(),
            "invalid Codex host settings: remote working directory must be absolute"
        );

        let duplicate = CodexSettings {
            version: SETTINGS_VERSION,
            selected: "local".into(),
            hosts: vec![host(), host()],
        };
        assert_eq!(
            duplicate.validate().unwrap_err().to_string(),
            "invalid Codex host settings: duplicate remote host id workstation"
        );
    }

    #[test]
    fn removing_the_selected_host_falls_back_to_local() {
        let mut settings = CodexSettings {
            version: SETTINGS_VERSION,
            selected: "workstation".into(),
            hosts: vec![host()],
        };
        assert!(settings.remove_host("workstation"));
        assert_eq!(settings.selected, "local");
        assert!(settings.hosts.is_empty());
    }
}
