//! Versioned, platform-neutral declarations for shell plugins.

use std::{
    collections::{BTreeMap, HashSet},
    io,
    path::{Component, Path},
};

use serde::{Deserialize, Serialize};

pub const PLUGIN_API_VERSION: u16 = 1;
pub const MAX_MANIFEST_BYTES: usize = 64 * 1024;
pub const MAX_PLUGIN_ENTRY_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PLUGIN_DIRECTORIES: usize = 64;
const MAX_ACTIVATION_SETTINGS_BYTES: usize = 16 * 1024;

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
}

impl PluginPackageDescriptor {
    pub fn load(&self) -> Result<PluginPackage, String> {
        let package = PluginPackage::load(&self.directory)?;
        if package.manifest != self.manifest {
            return Err("plugin manifest changed after discovery".into());
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
}

impl Default for PluginActivationSettings {
    fn default() -> Self {
        Self {
            version: 1,
            enabled: BTreeMap::new(),
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

    pub fn update_default(id: &str, enabled: bool) -> io::Result<()> {
        Self::update(
            nickel_storage::config_path("plugin-activation.json")?,
            id,
            enabled,
        )
    }

    pub fn update(path: impl AsRef<Path>, id: &str, enabled: bool) -> io::Result<()> {
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

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    pub api_version: u16,
    pub id: String,
    pub name: String,
    pub entry: String,
    #[serde(default)]
    pub surfaces: Vec<PluginSurface>,
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginOutputScope {
    #[default]
    Primary,
    All,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq)]
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
    TrayRead,
    TrayActivate,
    TrayContext,
    AudioRead,
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
            Self::TrayRead => "tray-read",
            Self::TrayActivate => "tray-activate",
            Self::TrayContext => "tray-context",
            Self::AudioRead => "audio-read",
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
        if !safe_relative_path(&self.entry) || !self.entry.ends_with(".js") {
            return Err(
                "plugin entry must be a relative .js path inside the plugin directory".into(),
            );
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

        std::fs::write(&path, b"{broken").unwrap();
        assert!(PluginActivationSettings::update(&path, id, true).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"{broken");
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
