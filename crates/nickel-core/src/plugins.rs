//! Versioned, platform-neutral declarations for shell plugins.

use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Component, Path},
};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const PLUGIN_API_VERSION: u16 = 1;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_PLUGIN_ENTRY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PLUGIN_DIRECTORIES: usize = 64;
const MAX_ACTIVATION_SETTINGS_BYTES: usize = 16 * 1024;
const MAX_PLUGIN_PREFERENCES_BYTES: usize = 16 * 1024;

fn digest_source(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

/// Installed packages and package errors found immediately below one root.
#[derive(Default)]
pub struct PluginCatalog {
    pub packages: BTreeMap<String, PluginPackageDescriptor>,
    pub failures: Vec<PluginPackageFailure>,
}

/// A package inspected at discovery time without retaining its script.
pub struct PluginPackageDescriptor {
    pub directory: std::path::PathBuf,
    pub manifest: PluginManifest,
    pub source_digest: String,
}

impl PluginPackageDescriptor {
    pub fn load(&self) -> Result<PluginPackage, String> {
        let package = PluginPackage::load(&self.directory)?;
        if package.manifest != self.manifest {
            return Err("plugin manifest changed after discovery".into());
        }
        if digest_source(&package.source) != self.source_digest {
            return Err("plugin script changed after discovery".into());
        }
        Ok(package)
    }
}

pub struct PluginPackageFailure {
    pub directory: String,
    pub reason: String,
}

impl PluginCatalog {
    pub fn discover_default() -> Result<Self, String> {
        let root = nickel_storage::config_path("plugins")
            .map_err(|error| format!("could not locate plugin directory: {error}"))?;
        Self::discover(root)
    }

    pub fn discover(root: impl AsRef<Path>) -> Result<Self, String> {
        let root = root.as_ref();
        let metadata = match std::fs::symlink_metadata(root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(error) => return Err(format!("could not inspect plugin directory: {error}")),
        };
        if !metadata.is_dir() || metadata.file_type().is_symlink() {
            return Err("plugin root must be an ordinary directory".into());
        }
        let mut entries = std::fs::read_dir(root)
            .map_err(|error| format!("could not enumerate plugins: {error}"))?
            .take(MAX_PLUGIN_DIRECTORIES + 1)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| format!("could not enumerate plugins: {error}"))?;
        if entries.len() > MAX_PLUGIN_DIRECTORIES {
            return Err(format!(
                "plugin root exceeds {MAX_PLUGIN_DIRECTORIES} entries"
            ));
        }
        entries.sort_by_key(|entry| entry.file_name());
        let mut catalog = Self::default();
        for entry in entries {
            let directory = entry.file_name().to_string_lossy().into_owned();
            let loaded = (|| {
                if !valid_identifier(&directory) {
                    return Err("plugin directory name is not a valid ID".into());
                }
                let kind = entry
                    .file_type()
                    .map_err(|error| format!("could not inspect package: {error}"))?;
                if !kind.is_dir() || kind.is_symlink() {
                    return Err("plugin package must be an ordinary directory".into());
                }
                let package = PluginPackage::load(entry.path())?;
                if package.manifest.id != directory {
                    return Err("plugin manifest ID does not match its directory".into());
                }
                Ok(PluginPackageDescriptor {
                    directory: entry.path(),
                    manifest: package.manifest,
                    source_digest: digest_source(&package.source),
                })
            })();
            match loaded {
                Ok(package) => {
                    catalog.packages.insert(directory, package);
                }
                Err(reason) => catalog
                    .failures
                    .push(PluginPackageFailure { directory, reason }),
            }
        }
        Ok(catalog)
    }
}

/// A validated plugin directory ready to start in the JavaScript runtime.
#[derive(Clone, Debug)]
pub struct PluginPackage {
    pub manifest: PluginManifest,
    pub source: String,
}

impl PluginPackage {
    pub fn source_digest(&self) -> String {
        digest_source(&self.source)
    }

    pub fn load(directory: impl AsRef<Path>) -> Result<Self, String> {
        let directory = std::fs::canonicalize(directory.as_ref())
            .map_err(|error| format!("could not open plugin directory: {error}"))?;
        if !directory.is_dir() {
            return Err("plugin path is not a directory".into());
        }
        let manifest_bytes =
            nickel_storage::read_regular_file(&directory.join("plugin.json"), MAX_MANIFEST_BYTES)
                .map_err(|error| format!("could not read plugin manifest: {error}"))?
                .ok_or("plugin manifest is missing")?;
        let manifest_source = std::str::from_utf8(&manifest_bytes)
            .map_err(|error| format!("plugin manifest is not UTF-8: {error}"))?;
        let manifest = PluginManifest::from_json(manifest_source)?;
        let entry_path = directory.join(&manifest.entry);
        let resolved_entry = std::fs::canonicalize(&entry_path)
            .map_err(|error| format!("could not open plugin entry: {error}"))?;
        if !resolved_entry.starts_with(&directory) {
            return Err("plugin entry escapes its directory".into());
        }
        let source = nickel_storage::read_regular_file(&entry_path, MAX_PLUGIN_ENTRY_BYTES)
            .map_err(|error| format!("could not read plugin entry: {error}"))?
            .ok_or("plugin entry is missing")?;
        let source = String::from_utf8(source)
            .map_err(|error| format!("plugin entry is not UTF-8: {error}"))?;
        Ok(Self { manifest, source })
    }
}

/// Explicit per-profile choices. An absent ID retains the bundled default.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginActivationSettings {
    version: u8,
    enabled: BTreeMap<String, bool>,
    #[serde(default)]
    approved: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct PluginApproval {
    author: Option<String>,
    version: Option<String>,
    entry: String,
    source_digest: String,
    capabilities: Vec<PluginCapability>,
    surfaces: Vec<PluginSurface>,
    provides_slots: Vec<PluginProvidedSlot>,
    contributes: Vec<PluginContribution>,
}

impl PluginApproval {
    fn from_manifest(manifest: &PluginManifest, source_digest: &str) -> Self {
        Self {
            author: manifest.author.clone(),
            version: manifest.version.clone(),
            entry: manifest.entry.clone(),
            source_digest: source_digest.to_owned(),
            capabilities: manifest.capabilities.clone(),
            surfaces: manifest.surfaces.clone(),
            provides_slots: manifest.provides_slots.clone(),
            contributes: manifest.contributes.clone(),
        }
    }

    fn fingerprint(manifest: &PluginManifest, source_digest: &str) -> String {
        let approval = Self::from_manifest(manifest, source_digest);
        let bytes =
            serde_json::to_vec(&approval).expect("validated plugin approval is serializable");
        format!("{:x}", Sha256::digest(bytes))
    }
}

impl Default for PluginActivationSettings {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: BTreeMap::new(),
            approved: BTreeMap::new(),
        }
    }
}

impl PluginActivationSettings {
    pub fn load_default() -> io::Result<Self> {
        Self::load(nickel_storage::config_path("plugin-activation.json")?)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let bytes =
            nickel_storage::read_regular_file(path.as_ref(), MAX_ACTIVATION_SETTINGS_BYTES)?
                .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;
        let settings: Self = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if settings.version != 1
            || settings.enabled.len() > 64
            || settings.enabled.keys().any(|id| !valid_identifier(id))
            || settings.approved.len() > 64
            || settings.approved.keys().any(|id| !valid_identifier(id))
            || settings.approved.values().any(|fingerprint| {
                fingerprint.len() != 64 || !fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit())
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid plugin activation settings",
            ));
        }
        Ok(settings)
    }

    pub fn desired_enabled(&self, id: &str, bundled_default: bool) -> bool {
        self.enabled.get(id).copied().unwrap_or(bundled_default)
    }

    pub fn approval_current(&self, manifest: &PluginManifest, source_digest: &str) -> bool {
        self.approved.get(&manifest.id)
            == Some(&PluginApproval::fingerprint(manifest, source_digest))
    }

    pub fn update_default(id: &str, enabled: bool) -> io::Result<()> {
        Self::update(
            nickel_storage::config_path("plugin-activation.json")?,
            id,
            enabled,
        )
    }

    pub fn update(path: impl AsRef<Path>, id: &str, enabled: bool) -> io::Result<()> {
        Self::update_inner(path, id, enabled, None)
    }

    pub fn update_manifest_default(
        manifest: &PluginManifest,
        source_digest: &str,
        enabled: bool,
    ) -> io::Result<()> {
        Self::update_inner(
            nickel_storage::config_path("plugin-activation.json")?,
            &manifest.id,
            enabled,
            Some((manifest, source_digest)),
        )
    }

    pub fn update_manifest(
        path: impl AsRef<Path>,
        manifest: &PluginManifest,
        source_digest: &str,
        enabled: bool,
    ) -> io::Result<()> {
        Self::update_inner(path, &manifest.id, enabled, Some((manifest, source_digest)))
    }

    fn update_inner(
        path: impl AsRef<Path>,
        id: &str,
        enabled: bool,
        manifest: Option<(&PluginManifest, &str)>,
    ) -> io::Result<()> {
        if !valid_identifier(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid plugin ID",
            ));
        }
        let path = path.as_ref();
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        let mut settings = match Self::load(path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        settings.enabled.insert(id.to_owned(), enabled);
        if enabled && let Some((manifest, source_digest)) = manifest {
            if source_digest.len() != 64
                || !source_digest.bytes().all(|byte| byte.is_ascii_hexdigit())
            {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "invalid plugin digest",
                ));
            }
            settings.approved.insert(
                id.to_owned(),
                PluginApproval::fingerprint(manifest, source_digest),
            );
        }
        if settings.enabled.len() > 64 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "too many plugins",
            ));
        }
        let bytes = serde_json::to_vec(&settings).map_err(io::Error::other)?;
        if bytes.len() > MAX_ACTIVATION_SETTINGS_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "plugin activation settings exceed 16 KiB",
            ));
        }
        nickel_storage::stage_write(path, bytes)?.commit(|| Ok(()))
    }
}

/// Per-plugin values. The manifest remains the authority for keys, types, and
/// defaults, so a plugin never gains access to another plugin's preferences.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginPreferences {
    version: u8,
    values: BTreeMap<String, serde_json::Value>,
}

impl PluginPreferences {
    pub fn load_default(manifest: &PluginManifest) -> io::Result<Self> {
        Self::load(Self::default_path(manifest)?)
    }

    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let Some(bytes) =
            nickel_storage::read_regular_file(path.as_ref(), MAX_PLUGIN_PREFERENCES_BYTES)?
        else {
            return Ok(Self::default());
        };
        let settings: Self = serde_json::from_slice(&bytes)
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if settings.version != 1
            || settings.values.len() > 32
            || settings.values.iter().any(|(key, value)| {
                !valid_identifier(key)
                    || !matches!(
                        value,
                        serde_json::Value::Bool(_)
                            | serde_json::Value::Number(_)
                            | serde_json::Value::String(_)
                    )
            })
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid plugin preferences",
            ));
        }
        Ok(settings)
    }

    pub fn effective(&self, manifest: &PluginManifest) -> BTreeMap<String, serde_json::Value> {
        manifest
            .settings
            .iter()
            .map(|setting| {
                let value = self
                    .values
                    .get(&setting.id)
                    .filter(|value| setting.kind.accepts(value))
                    .cloned()
                    .unwrap_or_else(|| setting.kind.default_value());
                (setting.id.clone(), value)
            })
            .collect()
    }

    pub fn update_default(
        manifest: &PluginManifest,
        key: &str,
        value: serde_json::Value,
    ) -> io::Result<()> {
        Self::update(Self::default_path(manifest)?, manifest, key, value)
    }

    pub fn update(
        path: impl AsRef<Path>,
        manifest: &PluginManifest,
        key: &str,
        value: serde_json::Value,
    ) -> io::Result<()> {
        let setting = manifest
            .settings
            .iter()
            .find(|setting| setting.id == key)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "unknown plugin setting"))?;
        if !setting.kind.accepts(&value) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "plugin setting value is outside its declared type or bounds",
            ));
        }
        let path = path.as_ref();
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        let current = Self::load(path)?;
        let mut values = current.effective(manifest);
        values.insert(key.to_owned(), value);
        let bytes = serde_json::to_vec(&Self { version: 1, values }).map_err(io::Error::other)?;
        if bytes.len() > MAX_PLUGIN_PREFERENCES_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "plugin preferences exceed 16 KiB",
            ));
        }
        nickel_storage::stage_write(path, bytes)?.commit(|| Ok(()))
    }

    fn default_path(manifest: &PluginManifest) -> io::Result<std::path::PathBuf> {
        nickel_storage::config_path(&format!("plugin-settings/{}.json", manifest.id))
    }
}

impl Default for PluginPreferences {
    fn default() -> Self {
        Self {
            version: 1,
            values: BTreeMap::new(),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub api_version: u16,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub entry: String,
    #[serde(default)]
    pub surfaces: Vec<PluginSurface>,
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,
    #[serde(default)]
    pub provides_slots: Vec<PluginProvidedSlot>,
    #[serde(default)]
    pub contributes: Vec<PluginContribution>,
    #[serde(default)]
    pub settings: Vec<PluginSetting>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
pub struct PluginSetting {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub description: String,
    #[serde(flatten)]
    pub kind: PluginSettingKind,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PluginSettingKind {
    Boolean {
        default: bool,
    },
    Integer {
        default: i64,
        min: i64,
        max: i64,
    },
    Text {
        default: String,
        max_length: u16,
    },
    Choice {
        default: String,
        options: Vec<String>,
    },
}

impl PluginSettingKind {
    pub fn default_value(&self) -> serde_json::Value {
        match self {
            Self::Boolean { default } => serde_json::Value::Bool(*default),
            Self::Integer { default, .. } => serde_json::Value::from(*default),
            Self::Text { default, .. } | Self::Choice { default, .. } => {
                serde_json::Value::String(default.clone())
            }
        }
    }

    pub fn accepts(&self, value: &serde_json::Value) -> bool {
        match self {
            Self::Boolean { .. } => value.is_boolean(),
            Self::Integer { min, max, .. } => value
                .as_i64()
                .is_some_and(|number| (*min..=*max).contains(&number)),
            Self::Text { max_length, .. } => value
                .as_str()
                .is_some_and(|text| text.len() <= usize::from(*max_length)),
            Self::Choice { options, .. } => value
                .as_str()
                .is_some_and(|selection| options.iter().any(|option| option == selection)),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginProvidedSlot {
    pub id: String,
    pub contract: PluginSlotContract,
    #[serde(default)]
    pub replaceable: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginContribution {
    pub target_plugin: String,
    pub target_slot: String,
    pub contract: PluginSlotContract,
    pub mode: PluginContributionMode,
    #[serde(default)]
    pub priority: i16,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSlotContract {
    Badge,
    Widget,
    Action,
    Section,
}

impl PluginSlotContract {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Badge => "badge",
            Self::Widget => "widget",
            Self::Action => "action",
            Self::Section => "section",
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginContributionMode {
    Add,
    Replace,
}

impl PluginContributionMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Add => "add",
            Self::Replace => "replace",
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginSurface {
    pub id: String,
    pub kind: PluginSurfaceKind,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub bottom_offset: u32,
    #[serde(default)]
    pub output: PluginOutputScope,
}

/// Stable manifest identity for a plugin-owned surface. The host pairs this
/// with an output instance when it creates a native presentation slot.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct PluginSurfaceKey {
    pub plugin_id: String,
    pub surface_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSurfaceKind {
    Panel,
    Dock,
    Desktop,
    Window,
    Dialog,
    Overlay,
}

impl PluginSurfaceKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Panel => "panel",
            Self::Dock => "dock",
            Self::Desktop => "desktop",
            Self::Window => "window",
            Self::Dialog => "dialog",
            Self::Overlay => "overlay",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginOutputScope {
    #[default]
    Primary,
    All,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, Hash, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginCapability {
    LauncherShow,
    ControlCenterShow,
    OnScreenKeyboardShow,
    ApplicationsRead,
    ApplicationsLaunch,
    ApplicationsPin,
    WindowsRead,
    WindowsFocus,
    WindowsContext,
    DesktopRead,
    DesktopArrange,
    DesktopFilesOpen,
    DesktopFilesManage,
    TrayRead,
    TrayActivate,
    TrayContext,
    AudioRead,
    AudioControl,
    NetworkRead,
    NetworkControl,
    BluetoothRead,
    BluetoothControl,
    DesktopControl,
    DisplayControl,
    SessionControl,
    WorkspacesRead,
    WorkspacesSwitch,
    NotificationsRead,
    NotificationsAct,
    SettingsRead,
    SettingsWrite,
    SettingsShow,
    ProjectsRead,
    ProjectsOpen,
    ProjectsMenuShow,
    SessionLogoutRequest,
    RunCommand,
}

impl PluginCapability {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::LauncherShow => "launcher-show",
            Self::ControlCenterShow => "control-center-show",
            Self::OnScreenKeyboardShow => "on-screen-keyboard-show",
            Self::ApplicationsRead => "applications-read",
            Self::ApplicationsLaunch => "applications-launch",
            Self::ApplicationsPin => "applications-pin",
            Self::WindowsRead => "windows-read",
            Self::WindowsFocus => "windows-focus",
            Self::WindowsContext => "windows-context",
            Self::DesktopRead => "desktop-read",
            Self::DesktopArrange => "desktop-arrange",
            Self::DesktopFilesOpen => "desktop-files-open",
            Self::DesktopFilesManage => "desktop-files-manage",
            Self::TrayRead => "tray-read",
            Self::TrayActivate => "tray-activate",
            Self::TrayContext => "tray-context",
            Self::AudioRead => "audio-read",
            Self::AudioControl => "audio-control",
            Self::NetworkRead => "network-read",
            Self::NetworkControl => "network-control",
            Self::BluetoothRead => "bluetooth-read",
            Self::BluetoothControl => "bluetooth-control",
            Self::DesktopControl => "desktop-control",
            Self::DisplayControl => "display-control",
            Self::SessionControl => "session-control",
            Self::WorkspacesRead => "workspaces-read",
            Self::WorkspacesSwitch => "workspaces-switch",
            Self::NotificationsRead => "notifications-read",
            Self::NotificationsAct => "notifications-act",
            Self::SettingsRead => "settings-read",
            Self::SettingsWrite => "settings-write",
            Self::SettingsShow => "settings-show",
            Self::ProjectsRead => "projects-read",
            Self::ProjectsOpen => "projects-open",
            Self::ProjectsMenuShow => "projects-menu-show",
            Self::SessionLogoutRequest => "session-logout-request",
            Self::RunCommand => "run-command",
        }
    }
}

impl PluginManifest {
    pub fn from_json(source: &str) -> Result<Self, String> {
        if source.len() > MAX_MANIFEST_BYTES {
            return Err("plugin manifest exceeds 64 KiB".into());
        }
        let manifest: Self = serde_json::from_str(source)
            .map_err(|error| format!("invalid plugin manifest: {error}"))?;
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.api_version != PLUGIN_API_VERSION {
            return Err(format!(
                "unsupported plugin API version {}",
                self.api_version
            ));
        }
        if !valid_identifier(&self.id) {
            return Err(
                "plugin ID must contain lowercase letters, digits, dots, or hyphens".into(),
            );
        }
        if self.name.trim().is_empty() || self.name.len() > 120 {
            return Err("plugin name must contain 1 to 120 characters".into());
        }
        if self.author.as_ref().is_some_and(|author| {
            author.trim().is_empty() || author.len() > 120 || author.chars().any(char::is_control)
        }) {
            return Err("plugin author must contain 1 to 120 printable characters".into());
        }
        if self.version.as_ref().is_some_and(|version| {
            version.is_empty()
                || version.len() > 64
                || !version
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-' | b'+'))
        }) {
            return Err("plugin version must contain 1 to 64 version characters".into());
        }
        if !safe_relative_path(&self.entry) || !self.entry.ends_with(".js") {
            return Err(
                "plugin entry must be a relative .js path inside the plugin directory".into(),
            );
        }
        if self.surfaces.len() > 16 {
            return Err("plugin declares more than 16 surfaces".into());
        }
        let mut ids = HashSet::new();
        for surface in &self.surfaces {
            if !valid_identifier(&surface.id) || !ids.insert(&surface.id) {
                return Err(format!("invalid or duplicate surface ID {:?}", surface.id));
            }
            if !(1..=8192).contains(&surface.width) || !(1..=8192).contains(&surface.height) {
                return Err(format!("surface {:?} has invalid dimensions", surface.id));
            }
            if surface.bottom_offset > 8192 {
                return Err(format!(
                    "surface {:?} has an invalid bottom offset",
                    surface.id
                ));
            }
        }
        let mut capabilities = HashSet::new();
        for capability in &self.capabilities {
            if !capabilities.insert(capability) {
                return Err(format!("duplicate capability {capability:?}"));
            }
        }
        if self.provides_slots.len() > 32 || self.contributes.len() > 32 {
            return Err("plugin declares too many composition slots".into());
        }
        let mut slot_ids = HashSet::new();
        for slot in &self.provides_slots {
            if !valid_identifier(&slot.id) || !slot_ids.insert(&slot.id) {
                return Err(format!("invalid or duplicate provided slot {:?}", slot.id));
            }
        }
        let mut contribution_targets = HashSet::new();
        for contribution in &self.contributes {
            if !valid_identifier(&contribution.target_plugin)
                || !valid_identifier(&contribution.target_slot)
                || contribution.target_plugin == self.id
            {
                return Err("contribution needs another valid plugin and slot".into());
            }
            if !contribution_targets
                .insert((&contribution.target_plugin, &contribution.target_slot))
            {
                return Err("duplicate contribution target".into());
            }
        }
        if self.settings.len() > 32 {
            return Err("plugin declares too many settings".into());
        }
        let mut setting_ids = HashSet::new();
        for setting in &self.settings {
            if !valid_identifier(&setting.id) || !setting_ids.insert(&setting.id) {
                return Err(format!("invalid or duplicate setting ID {:?}", setting.id));
            }
            if setting.label.trim().is_empty()
                || setting.label.len() > 80
                || setting.description.len() > 256
            {
                return Err(format!(
                    "setting {:?} has an invalid label or description",
                    setting.id
                ));
            }
            let valid = match &setting.kind {
                PluginSettingKind::Boolean { .. } => true,
                PluginSettingKind::Integer { default, min, max } => {
                    *min >= -1_000_000_000
                        && *max <= 1_000_000_000
                        && min <= default
                        && default <= max
                }
                PluginSettingKind::Text {
                    default,
                    max_length,
                } => (1..=1024).contains(max_length) && default.len() <= usize::from(*max_length),
                PluginSettingKind::Choice { default, options } => {
                    (1..=32).contains(&options.len())
                        && options
                            .iter()
                            .all(|option| !option.is_empty() && option.len() <= 120)
                        && options.iter().collect::<HashSet<_>>().len() == options.len()
                        && options.contains(default)
                }
            };
            if !valid {
                return Err(format!(
                    "setting {:?} has an invalid default or bounds",
                    setting.id
                ));
            }
        }
        Ok(())
    }
}

fn valid_identifier(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte))
        && !value.starts_with('.')
        && !value.starts_with('-')
        && !value.ends_with('.')
        && !value.ends_with('-')
        && !value.contains("..")
}

fn safe_relative_path(value: &str) -> bool {
    !value.is_empty()
        && !value.contains(':')
        && std::path::Path::new(value)
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && !value.contains('\\')
}

/// Memory measured by the host for one active plugin. `None` means the
/// measurement is unavailable, not zero bytes.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct PluginMemory {
    pub js_heap_bytes: Option<u64>,
    pub native_ui_bytes: Option<u64>,
    pub texture_bytes: Option<u64>,
    pub timers: u32,
    pub subscriptions: u32,
}

impl PluginMemory {
    pub fn tracked_bytes(&self) -> Option<u64> {
        let measured = [self.js_heap_bytes, self.native_ui_bytes, self.texture_bytes];
        measured.iter().any(Option::is_some).then(|| {
            measured
                .into_iter()
                .flatten()
                .fold(0_u64, u64::saturating_add)
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginHealth {
    Disabled,
    Starting,
    Running,
    Failed(String),
}

#[derive(Clone, Debug)]
pub struct RegisteredPlugin {
    pub manifest: PluginManifest,
    pub desired_enabled: bool,
    pub health: PluginHealth,
    pub memory: PluginMemory,
    pub tracked_peak_bytes: Option<u64>,
}

#[derive(Default)]
pub struct PluginRegistry {
    entries: BTreeMap<String, RegisteredPlugin>,
}

impl PluginRegistry {
    pub fn register(&mut self, manifest: PluginManifest) -> Result<(), String> {
        manifest.validate()?;
        if self.entries.contains_key(&manifest.id) {
            return Err(format!("plugin {:?} is already registered", manifest.id));
        }
        self.entries.insert(
            manifest.id.clone(),
            RegisteredPlugin {
                manifest,
                desired_enabled: false,
                health: PluginHealth::Disabled,
                memory: PluginMemory::default(),
                tracked_peak_bytes: None,
            },
        );
        Ok(())
    }

    pub fn get(&self, id: &str) -> Option<&RegisteredPlugin> {
        self.entries.get(id)
    }

    pub fn entries(&self) -> impl Iterator<Item = &RegisteredPlugin> {
        self.entries.values()
    }

    /// Records desired activation. The runtime reports `Running` only after
    /// it has created the plugin host successfully.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<bool, String> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| format!("unknown plugin {id:?}"))?;
        if entry.desired_enabled == enabled {
            return Ok(false);
        }
        entry.desired_enabled = enabled;
        entry.health = if enabled {
            PluginHealth::Starting
        } else {
            PluginHealth::Disabled
        };
        entry.memory = PluginMemory::default();
        entry.tracked_peak_bytes = None;
        Ok(true)
    }

    pub fn mark_running(&mut self, id: &str) -> Result<(), String> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| format!("unknown plugin {id:?}"))?;
        if !entry.desired_enabled {
            return Err(format!("plugin {id:?} is disabled"));
        }
        entry.health = PluginHealth::Running;
        Ok(())
    }

    pub fn mark_failed(&mut self, id: &str, error: String) -> Result<(), String> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| format!("unknown plugin {id:?}"))?;
        entry.health = PluginHealth::Failed(error);
        entry.memory = PluginMemory::default();
        entry.tracked_peak_bytes = None;
        Ok(())
    }

    pub fn record_memory(&mut self, id: &str, memory: PluginMemory) -> Result<(), String> {
        let entry = self
            .entries
            .get_mut(id)
            .ok_or_else(|| format!("unknown plugin {id:?}"))?;
        if entry.health != PluginHealth::Running {
            return Err(format!("plugin {id:?} is not running"));
        }
        if let Some(current) = memory.tracked_bytes() {
            entry.tracked_peak_bytes = Some(entry.tracked_peak_bytes.unwrap_or(0).max(current));
        }
        entry.memory = memory;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovers_valid_packages_and_reports_invalid_siblings() {
        let root = tempfile::tempdir().unwrap();
        let valid = root.path().join("org.nickel.hello-panel");
        std::fs::create_dir(&valid).unwrap();
        std::fs::write(valid.join("plugin.json"), VALID).unwrap();
        std::fs::write(valid.join("main.js"), "function App() {}").unwrap();
        let invalid = root.path().join("org.example.mismatch");
        std::fs::create_dir(&invalid).unwrap();
        std::fs::write(invalid.join("plugin.json"), VALID).unwrap();
        std::fs::write(invalid.join("main.js"), "function App() {}").unwrap();
        let catalog = PluginCatalog::discover(root.path()).unwrap();
        assert_eq!(catalog.packages.len(), 1);
        assert!(catalog.packages.contains_key("org.nickel.hello-panel"));
        assert!(
            catalog.packages["org.nickel.hello-panel"]
                .load()
                .unwrap()
                .source
                .contains("function App")
        );
        assert_eq!(catalog.failures.len(), 1);
        assert_eq!(catalog.failures[0].directory, "org.example.mismatch");
        assert!(catalog.failures[0].reason.contains("does not match"));

        std::fs::write(
            valid.join("plugin.json"),
            VALID.replace("Hello Panel", "Changed Panel"),
        )
        .unwrap();
        assert!(
            catalog.packages["org.nickel.hello-panel"]
                .load()
                .unwrap_err()
                .contains("changed after discovery")
        );
        std::fs::write(valid.join("plugin.json"), VALID).unwrap();
        std::fs::write(valid.join("main.js"), "function App() { return 1; }").unwrap();
        assert!(
            catalog.packages["org.nickel.hello-panel"]
                .load()
                .unwrap_err()
                .contains("script changed after discovery")
        );
    }

    #[cfg(unix)]
    #[test]
    fn discovery_rejects_linked_package_directories() {
        use std::os::unix::fs::symlink;

        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        symlink(outside.path(), root.path().join("org.example.linked")).unwrap();
        let catalog = PluginCatalog::discover(root.path()).unwrap();
        assert!(catalog.packages.is_empty());
        assert_eq!(catalog.failures.len(), 1);
        assert!(catalog.failures[0].reason.contains("ordinary directory"));
    }

    #[test]
    fn discovery_bounds_the_number_of_directory_entries() {
        let root = tempfile::tempdir().unwrap();
        for index in 0..=MAX_PLUGIN_DIRECTORIES {
            std::fs::create_dir(root.path().join(format!("org.example.plugin-{index}"))).unwrap();
        }
        assert!(
            PluginCatalog::discover(root.path())
                .err()
                .is_some_and(|error| error.contains("exceeds 64 entries"))
        );
    }

    #[test]
    fn loads_a_bounded_plugin_package_from_disk() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("plugin.json"), VALID).unwrap();
        std::fs::write(
            directory.path().join("main.js"),
            "function App() { return null; }",
        )
        .unwrap();
        let package = PluginPackage::load(directory.path()).unwrap();
        assert_eq!(package.manifest.id, "org.nickel.hello-panel");
        assert!(package.source.contains("function App"));

        std::fs::write(
            directory.path().join("main.js"),
            vec![b'x'; MAX_PLUGIN_ENTRY_BYTES + 1],
        )
        .unwrap();
        assert!(PluginPackage::load(directory.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_an_entry_link_outside_the_plugin_directory() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        std::fs::write(directory.path().join("plugin.json"), VALID).unwrap();
        std::fs::write(outside.path().join("main.js"), "function App() {}").unwrap();
        symlink(
            outside.path().join("main.js"),
            directory.path().join("main.js"),
        )
        .unwrap();
        assert!(PluginPackage::load(directory.path()).is_err());
    }

    #[test]
    fn activation_choice_survives_restart_and_rejects_corrupt_storage() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plugin-activation.json");
        let id = "org.nickel.hello-panel";
        PluginActivationSettings::update(&path, id, true).unwrap();
        let loaded = PluginActivationSettings::load(&path).unwrap();
        assert!(loaded.desired_enabled(id, false));
        assert!(!loaded.desired_enabled("org.nickel.launcher", false));
        PluginActivationSettings::update(&path, id, false).unwrap();
        assert!(
            !PluginActivationSettings::load(&path)
                .unwrap()
                .desired_enabled(id, true)
        );

        std::fs::write(
            &path,
            format!(r#"{{"version":1,"enabled":{{"{id}":true}}}}"#),
        )
        .unwrap();
        let legacy = PluginActivationSettings::load(&path).unwrap();
        assert!(legacy.desired_enabled(id, false));
        assert!(!legacy.approval_current(
            &PluginManifest::from_json(VALID).unwrap(),
            &digest_source("old")
        ));

        std::fs::write(&path, b"{broken").unwrap();
        assert!(PluginActivationSettings::update(&path, id, true).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
    }

    #[test]
    fn installed_plugin_approval_requires_review_after_grants_or_version_change() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plugin-activation.json");
        let mut manifest = PluginManifest::from_json(VALID).unwrap();
        let id = manifest.id.clone();
        let source_digest = digest_source("function App() {}");
        PluginActivationSettings::update(&path, &id, true).unwrap();
        let settings = PluginActivationSettings::load(&path).unwrap();
        assert!(settings.desired_enabled(&id, false));
        assert!(!settings.approval_current(&manifest, &source_digest));

        PluginActivationSettings::update_manifest(&path, &manifest, &source_digest, true).unwrap();
        let settings = PluginActivationSettings::load(&path).unwrap();
        assert!(settings.approval_current(&manifest, &source_digest));
        assert!(
            !settings.approval_current(&manifest, &digest_source("function App() { return 1; }"))
        );
        manifest.version = Some("2.0.0".into());
        assert!(!settings.approval_current(&manifest, &source_digest));
        manifest.version = None;
        manifest.capabilities.push(PluginCapability::DesktopRead);
        assert!(!settings.approval_current(&manifest, &source_digest));
        manifest.capabilities.clear();
        manifest.contributes.push(PluginContribution {
            target_plugin: "org.nickel.taskbar".into(),
            target_slot: "task-badge".into(),
            contract: PluginSlotContract::Badge,
            mode: PluginContributionMode::Add,
            priority: 0,
        });
        assert!(!settings.approval_current(&manifest, &source_digest));
    }

    const VALID: &str = r#"{
        "api_version": 1,
        "id": "org.nickel.hello-panel",
        "name": "Hello Panel",
        "entry": "main.js",
        "surfaces": [{"id":"main","kind":"panel","width":360,"height":64,"bottom_offset":24,"output":"all"}],
        "capabilities": []
    }"#;

    #[test]
    fn accepts_a_portable_panel_manifest() {
        let manifest = PluginManifest::from_json(VALID).unwrap();
        assert_eq!(manifest.surfaces[0].bottom_offset, 24);
        assert_eq!(manifest.surfaces[0].output, PluginOutputScope::All);
        assert!(manifest.author.is_none());
        assert!(manifest.version.is_none());
        let identified = PluginManifest::from_json(&VALID.replace(
            "\"name\": \"Hello Panel\",",
            "\"name\": \"Hello Panel\", \"author\": \"Example Org\", \"version\": \"1.2.3-beta\",",
        ))
        .unwrap();
        assert_eq!(identified.author.as_deref(), Some("Example Org"));
        assert_eq!(identified.version.as_deref(), Some("1.2.3-beta"));
        assert!(
            PluginManifest::from_json(&VALID.replace(
                "\"name\": \"Hello Panel\",",
                "\"name\": \"Hello Panel\", \"version\": \"1.0/forged\",",
            ))
            .is_err()
        );
    }

    #[test]
    fn rejects_unsafe_entry_and_duplicate_surface_ids() {
        let unsafe_entry = VALID.replace("main.js", "../main.js");
        assert!(PluginManifest::from_json(&unsafe_entry).is_err());
        let duplicate = VALID.replace(
            "\"output\":\"all\"}",
            "\"output\":\"all\"},{\"id\":\"main\",\"kind\":\"dock\",\"width\":100,\"height\":50}",
        );
        assert!(PluginManifest::from_json(&duplicate).is_err());
    }

    #[test]
    fn bounds_surfaces_in_one_package() {
        let mut manifest = PluginManifest::from_json(VALID).unwrap();
        let template = manifest.surfaces[0].clone();
        manifest.surfaces = (0..17)
            .map(|index| PluginSurface {
                id: format!("surface-{index}"),
                ..template.clone()
            })
            .collect();
        assert!(manifest.validate().unwrap_err().contains("16 surfaces"));
    }

    #[test]
    fn rejects_unknown_versions_and_grants() {
        assert!(
            PluginManifest::from_json(&VALID.replace("\"api_version\": 1", "\"api_version\": 2"))
                .is_err()
        );
        assert!(
            PluginManifest::from_json(&VALID.replace(
                "\"capabilities\": []",
                "\"capabilities\": [\"unrestricted-native\"]"
            ))
            .is_err()
        );
    }

    #[test]
    fn validates_typed_composition_declarations() {
        let source = VALID.replace(
            "\"capabilities\": []",
            "\"capabilities\": [], \"provides_slots\": [{\"id\":\"metrics\",\"contract\":\"widget\",\"replaceable\":true}], \"contributes\": [{\"target_plugin\":\"org.nickel.taskbar\",\"target_slot\":\"task-badge\",\"contract\":\"badge\",\"mode\":\"add\"}]",
        );
        let manifest = PluginManifest::from_json(&source).unwrap();
        assert!(manifest.provides_slots[0].replaceable);
        assert_eq!(manifest.contributes[0].mode, PluginContributionMode::Add);
        assert!(
            PluginManifest::from_json(&source.replace("\"mode\":\"add\"", "\"mode\":\"mutate\""))
                .is_err()
        );
        assert!(
            PluginManifest::from_json(
                &source.replace("org.nickel.taskbar", "org.nickel.hello-panel")
            )
            .is_err()
        );
        assert!(
            PluginManifest::from_json(&source.replace(
                "\"target_slot\":\"task-badge\"",
                "\"target_slot\":\"../task-badge\""
            ))
            .is_err()
        );
    }

    #[test]
    fn validates_bounded_plugin_setting_declarations() {
        let source = VALID.replace(
            "\"capabilities\": []",
            r#""capabilities": [], "settings": [
                {"id":"show-count","label":"Show count","kind":"boolean","default":true},
                {"id":"refresh-minutes","label":"Refresh interval","kind":"integer","default":5,"min":1,"max":60},
                {"id":"account","label":"Account","kind":"text","default":"","max_length":120},
                {"id":"style","label":"Style","kind":"choice","default":"compact","options":["compact","wide"]}
            ]"#,
        );
        let manifest = PluginManifest::from_json(&source).unwrap();
        assert_eq!(manifest.settings.len(), 4);
        assert_eq!(
            manifest.settings[0].kind.default_value(),
            serde_json::json!(true)
        );
        assert!(manifest.settings[1].kind.accepts(&serde_json::json!(30)));
        assert!(!manifest.settings[1].kind.accepts(&serde_json::json!(61)));
        assert!(!manifest.settings[2].kind.accepts(&serde_json::json!(42)));
        assert!(
            !manifest.settings[3]
                .kind
                .accepts(&serde_json::json!("unknown"))
        );
        assert!(PluginManifest::from_json(&source.replace("\"max\":60", "\"max\":2")).is_err());
        assert!(
            PluginManifest::from_json(
                &source.replace("\"compact\",\"wide\"", "\"compact\",\"compact\"")
            )
            .is_err()
        );
        assert!(
            PluginManifest::from_json(&source.replace("\"id\":\"account\"", "\"id\":\"style\""))
                .is_err()
        );
    }

    #[test]
    fn plugin_preferences_persist_valid_values_and_preserve_corrupt_files() {
        let manifest = PluginManifest::from_json(&VALID.replace(
            "\"capabilities\": []",
            r#""capabilities": [], "settings": [
                {"id":"show-count","label":"Show count","kind":"boolean","default":true},
                {"id":"refresh-minutes","label":"Refresh interval","kind":"integer","default":5,"min":1,"max":60}
            ]"#,
        ))
        .unwrap();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("plugin-settings.json");
        let defaults = PluginPreferences::load(&path).unwrap().effective(&manifest);
        assert_eq!(defaults["show-count"], serde_json::json!(true));
        assert_eq!(defaults["refresh-minutes"], serde_json::json!(5));
        assert!(
            PluginPreferences::update(&path, &manifest, "refresh-minutes", serde_json::json!(61))
                .is_err()
        );
        assert!(!path.exists());
        PluginPreferences::update(&path, &manifest, "show-count", serde_json::json!(false))
            .unwrap();
        let loaded = PluginPreferences::load(&path).unwrap().effective(&manifest);
        assert_eq!(loaded["show-count"], serde_json::json!(false));
        assert_eq!(loaded["refresh-minutes"], serde_json::json!(5));
        assert!(
            PluginPreferences::update(&path, &manifest, "other", serde_json::json!(true)).is_err()
        );
        std::fs::write(&path, b"{broken").unwrap();
        assert!(
            PluginPreferences::update(&path, &manifest, "show-count", serde_json::json!(true))
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
    }

    #[test]
    fn registry_tracks_activation_and_clears_stale_memory_on_disable() {
        let manifest = PluginManifest::from_json(VALID).unwrap();
        let id = manifest.id.clone();
        let mut registry = PluginRegistry::default();
        registry.register(manifest.clone()).unwrap();
        assert!(registry.register(manifest).is_err());
        assert!(registry.set_enabled(&id, true).unwrap());
        assert_eq!(registry.get(&id).unwrap().health, PluginHealth::Starting);
        registry.mark_running(&id).unwrap();
        registry
            .record_memory(
                &id,
                PluginMemory {
                    js_heap_bytes: None,
                    native_ui_bytes: Some(4096),
                    ..PluginMemory::default()
                },
            )
            .unwrap();
        assert_eq!(
            registry.get(&id).unwrap().memory.native_ui_bytes,
            Some(4096)
        );
        assert_eq!(registry.get(&id).unwrap().tracked_peak_bytes, Some(4096));
        registry
            .record_memory(
                &id,
                PluginMemory {
                    native_ui_bytes: Some(1024),
                    ..PluginMemory::default()
                },
            )
            .unwrap();
        assert_eq!(registry.get(&id).unwrap().tracked_peak_bytes, Some(4096));
        registry.set_enabled(&id, false).unwrap();
        assert_eq!(registry.get(&id).unwrap().memory, PluginMemory::default());
        assert_eq!(registry.get(&id).unwrap().tracked_peak_bytes, None);
        assert!(registry.mark_running(&id).is_err());
    }
}
