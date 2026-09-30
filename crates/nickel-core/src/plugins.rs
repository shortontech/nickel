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
pub const MAX_PLUGIN_CSS_BYTES: usize = 256 * 1024;
pub const MAX_PLUGIN_MODULE_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PLUGIN_MODULE_TOTAL_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PLUGIN_MODULES: usize = 128;
pub const MAX_PLUGIN_IMAGE_BYTES: usize = 4 * 1024 * 1024;
pub const MAX_PLUGIN_IMAGE_TOTAL_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_PLUGIN_DIRECTORIES: usize = 64;
const MAX_ACTIVATION_SETTINGS_BYTES: usize = 16 * 1024;
const MAX_PLUGIN_PREFERENCES_BYTES: usize = 16 * 1024;

fn digest_source(source: &str) -> String {
    format!("{:x}", Sha256::digest(source.as_bytes()))
}

fn digest_package(
    source: &str,
    stylesheet: &str,
    modules: &[PluginSourceFile],
    images: &BTreeMap<String, Vec<u8>>,
) -> String {
    if images.is_empty() && stylesheet.is_empty() && modules.is_empty() {
        return digest_source(source);
    }
    let mut digest = Sha256::new();
    digest.update(source.as_bytes());
    digest.update((stylesheet.len() as u64).to_le_bytes());
    digest.update(stylesheet.as_bytes());
    for module in modules {
        digest.update((module.path.len() as u64).to_le_bytes());
        digest.update(module.path.as_bytes());
        digest.update((module.source.len() as u64).to_le_bytes());
        digest.update(module.source.as_bytes());
    }
    for (id, bytes) in images {
        digest.update((id.len() as u64).to_le_bytes());
        digest.update(id.as_bytes());
        digest.update((bytes.len() as u64).to_le_bytes());
        digest.update(bytes);
    }
    format!("{:x}", digest.finalize())
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
        if package.source_digest() != self.source_digest {
            return Err("plugin content changed after discovery".into());
        }
        Ok(package)
    }
}

/// A discovered package backed by disk or immutable bundled source.
/// Both sources enter the same runtime, grants, and lifecycle paths.
pub struct PluginPackageSource {
    pub manifest: PluginManifest,
    pub source_digest: String,
    source: PackageSource,
}

enum PackageSource {
    Directory(PluginPackageDescriptor),
    Embedded(PluginPackage),
}

impl From<PluginPackageDescriptor> for PluginPackageSource {
    fn from(descriptor: PluginPackageDescriptor) -> Self {
        Self {
            manifest: descriptor.manifest.clone(),
            source_digest: descriptor.source_digest.clone(),
            source: PackageSource::Directory(descriptor),
        }
    }
}

impl PluginPackageSource {
    pub fn embedded(package: PluginPackage) -> Self {
        Self {
            manifest: package.manifest.clone(),
            source_digest: package.source_digest(),
            source: PackageSource::Embedded(package),
        }
    }

    pub fn load(&self) -> Result<PluginPackage, String> {
        let package = match &self.source {
            PackageSource::Directory(descriptor) => descriptor.load()?,
            PackageSource::Embedded(package) => package.clone(),
        };
        if package.manifest != self.manifest || package.source_digest() != self.source_digest {
            return Err("plugin source changed after registration".into());
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
                if package.manifest.claims_native_shell_surface() {
                    return Err("desktop and screenshot presentation are native Rust UI".into());
                }
                let source_digest = package.source_digest();
                Ok(PluginPackageDescriptor {
                    directory: entry.path(),
                    manifest: package.manifest,
                    source_digest,
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
    pub stylesheet: String,
    /// All ordinary JavaScript, JSX source, and CSS modules in the package.
    /// The host selects only the entry's reachable graph for evaluation.
    pub modules: Vec<PluginSourceFile>,
    pub images: BTreeMap<String, Vec<u8>>,
}

/// One package-relative JavaScript, JSX, or CSS source file. Hosts pass these
/// to `nickel-plugin-runtime` as a single module graph.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PluginSourceFile {
    pub path: String,
    pub source: String,
}

impl PluginPackage {
    pub fn source_digest(&self) -> String {
        digest_package(&self.source, &self.stylesheet, &self.modules, &self.images)
    }

    /// Loads a package from an immutable asset catalog, without filesystem access.
    /// Uses the same manifest, source, stylesheet, and image bounds as disk loading.
    pub fn from_embedded(files: &[(&str, &[u8])]) -> Result<Self, String> {
        let mut catalog = BTreeMap::new();
        for &(path, bytes) in files {
            if path.is_empty()
                || path.contains(['\\', ':'])
                || path
                    .split('/')
                    .any(|part| part.is_empty() || part == "." || part == "..")
            {
                return Err("embedded asset has an invalid package-relative path".into());
            }
            if catalog.insert(path, bytes).is_some() {
                return Err("embedded asset path is duplicated".into());
            }
        }
        let read = |path: &str, limit: usize| -> Result<&[u8], String> {
            let bytes = catalog
                .get(path)
                .copied()
                .ok_or_else(|| format!("embedded asset {path:?} is missing"))?;
            if bytes.len() > limit {
                return Err(format!("embedded asset {path:?} exceeds size limit"));
            }
            Ok(bytes)
        };
        let text = |path: &str, limit| -> Result<String, String> {
            std::str::from_utf8(read(path, limit)?)
                .map(str::to_owned)
                .map_err(|_| format!("embedded asset {path:?} is not UTF-8"))
        };
        let manifest = PluginManifest::from_json(&text("plugin.json", MAX_MANIFEST_BYTES)?)?;
        let source = text(&manifest.entry, MAX_PLUGIN_ENTRY_BYTES)?;
        let stylesheet = manifest
            .stylesheet
            .as_deref()
            .map(|path| text(path, MAX_PLUGIN_CSS_BYTES))
            .transpose()?
            .unwrap_or_default();
        let mut modules = Vec::new();
        let mut total = 0usize;
        for (&path, bytes) in &catalog {
            if !matches!(path.rsplit('.').next(), Some("js" | "jsx" | "css")) {
                continue;
            }
            total = total.saturating_add(bytes.len());
            if modules.len() >= MAX_PLUGIN_MODULES || total > MAX_PLUGIN_MODULE_TOTAL_BYTES {
                return Err("embedded package exceeds source module limits".into());
            }
            modules.push(PluginSourceFile {
                path: path.into(),
                source: text(path, MAX_PLUGIN_MODULE_BYTES)?,
            });
        }
        let mut images = BTreeMap::new();
        let mut total = 0usize;
        for image in &manifest.images {
            let bytes = read(&image.path, MAX_PLUGIN_IMAGE_BYTES)?;
            total = total.saturating_add(bytes.len());
            if total > MAX_PLUGIN_IMAGE_TOTAL_BYTES {
                return Err("embedded package exceeds image limits".into());
            }
            images.insert(image.id.clone(), bytes.to_vec());
        }
        Ok(Self {
            manifest,
            source,
            stylesheet,
            modules,
            images,
        })
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
        let stylesheet = Self::load_stylesheet(&directory, &manifest)?;
        let images = Self::load_images(&directory, &manifest)?;
        let modules = Self::load_module_sources(&directory)?;
        Ok(Self {
            manifest,
            source,
            stylesheet,
            modules,
            images,
        })
    }

    /// Loads the ordinary source module files in an installed package.
    /// Asset files and package metadata are intentionally excluded.
    pub fn load_module_sources(
        directory: impl AsRef<Path>,
    ) -> Result<Vec<PluginSourceFile>, String> {
        let directory = std::fs::canonicalize(directory.as_ref())
            .map_err(|error| format!("could not open plugin directory: {error}"))?;
        if !directory.is_dir() {
            return Err("plugin path is not a directory".into());
        }
        let mut files = Vec::new();
        let mut pending = vec![directory.clone()];
        let mut total = 0_usize;
        while let Some(current) = pending.pop() {
            let mut entries = std::fs::read_dir(&current)
                .map_err(|error| format!("could not enumerate plugin modules: {error}"))?
                .collect::<Result<Vec<_>, _>>()
                .map_err(|error| format!("could not enumerate plugin modules: {error}"))?;
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries.into_iter().rev() {
                let kind = entry
                    .file_type()
                    .map_err(|error| format!("could not inspect plugin module: {error}"))?;
                if kind.is_symlink() {
                    return Err("plugin modules cannot contain symbolic links".into());
                }
                if kind.is_dir() {
                    pending.push(entry.path());
                    continue;
                }
                if !kind.is_file() {
                    continue;
                }
                let path = entry.path();
                let extension = path.extension().and_then(|value| value.to_str());
                if !matches!(extension, Some("js" | "jsx" | "css")) {
                    continue;
                }
                if files.len() >= MAX_PLUGIN_MODULES {
                    return Err(format!(
                        "plugin exceeds {MAX_PLUGIN_MODULES} source modules"
                    ));
                }
                let bytes = nickel_storage::read_regular_file(&path, MAX_PLUGIN_MODULE_BYTES)
                    .map_err(|error| format!("could not read plugin module: {error}"))?
                    .ok_or("plugin module is missing")?;
                total = total.saturating_add(bytes.len());
                if total > MAX_PLUGIN_MODULE_TOTAL_BYTES {
                    return Err("plugin source modules exceed 4 MiB in total".into());
                }
                let source = String::from_utf8(bytes)
                    .map_err(|error| format!("plugin module is not UTF-8: {error}"))?;
                let relative = path
                    .strip_prefix(&directory)
                    .expect("enumerated path remains in package");
                let path = relative
                    .components()
                    .map(|part| part.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/");
                files.push(PluginSourceFile { path, source });
            }
        }
        files.sort_by(|left, right| left.path.cmp(&right.path));
        Ok(files)
    }

    pub fn load_stylesheet(
        directory: impl AsRef<Path>,
        manifest: &PluginManifest,
    ) -> Result<String, String> {
        let Some(relative_path) = &manifest.stylesheet else {
            return Ok(String::new());
        };
        let directory = std::fs::canonicalize(directory.as_ref())
            .map_err(|error| format!("could not open plugin directory: {error}"))?;
        let path = directory.join(relative_path);
        let resolved = std::fs::canonicalize(&path)
            .map_err(|error| format!("could not open plugin stylesheet: {error}"))?;
        if !resolved.starts_with(&directory) {
            return Err("plugin stylesheet escapes its directory".into());
        }
        let bytes = nickel_storage::read_regular_file(&path, MAX_PLUGIN_CSS_BYTES)
            .map_err(|error| format!("could not read plugin stylesheet: {error}"))?
            .ok_or("plugin stylesheet is missing")?;
        String::from_utf8(bytes).map_err(|error| format!("plugin stylesheet is not UTF-8: {error}"))
    }

    pub fn load_images(
        directory: impl AsRef<Path>,
        manifest: &PluginManifest,
    ) -> Result<BTreeMap<String, Vec<u8>>, String> {
        manifest.validate()?;
        let directory = std::fs::canonicalize(directory.as_ref())
            .map_err(|error| format!("could not open plugin directory: {error}"))?;
        let mut images = BTreeMap::new();
        let mut total_bytes = 0_usize;
        for asset in &manifest.images {
            let path = directory.join(&asset.path);
            let resolved = std::fs::canonicalize(&path)
                .map_err(|error| format!("could not open plugin image {:?}: {error}", asset.id))?;
            if !resolved.starts_with(&directory) {
                return Err(format!("plugin image {:?} escapes its directory", asset.id));
            }
            let bytes = nickel_storage::read_regular_file(&path, MAX_PLUGIN_IMAGE_BYTES)
                .map_err(|error| format!("could not read plugin image {:?}: {error}", asset.id))?
                .ok_or_else(|| format!("plugin image {:?} is missing", asset.id))?;
            total_bytes = total_bytes.saturating_add(bytes.len());
            if total_bytes > MAX_PLUGIN_IMAGE_TOTAL_BYTES {
                return Err("plugin images exceed 16 MiB in total".into());
            }
            images.insert(asset.id.clone(), bytes);
        }
        Ok(images)
    }
}

/// Explicit per-profile choices. An absent ID retains the bundled default.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct PluginActivationSettings {
    version: u8,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    selected_shell: Option<String>,
    enabled: BTreeMap<String, bool>,
    #[serde(default)]
    approved: BTreeMap<String, String>,
}

#[derive(Serialize)]
struct PluginApproval {
    #[serde(skip_serializing_if = "Option::is_none")]
    composition: Option<crate::package_composition::ShellPackageComposition>,
    author: Option<String>,
    version: Option<String>,
    entry: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    stylesheet: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    images: Vec<PluginImageAsset>,
    source_digest: String,
    capabilities: Vec<PluginCapability>,
    surfaces: Vec<PluginSurface>,
}

impl PluginApproval {
    fn from_manifest(manifest: &PluginManifest, source_digest: &str) -> Self {
        Self {
            composition: manifest.composition.clone(),
            author: manifest.author.clone(),
            version: manifest.version.clone(),
            entry: manifest.entry.clone(),
            stylesheet: manifest.stylesheet.clone(),
            images: manifest.images.clone(),
            source_digest: source_digest.to_owned(),
            capabilities: manifest.capabilities.clone(),
            surfaces: manifest.surfaces.clone(),
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
            selected_shell: None,
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
        if settings
            .selected_shell
            .as_deref()
            .is_some_and(|id| !valid_identifier(id))
            || settings.version != 1
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

    pub fn selected_shell(&self) -> Option<&str> {
        self.selected_shell.as_deref()
    }

    pub fn select_shell_default(id: &str) -> io::Result<()> {
        Self::select_shell(nickel_storage::config_path("plugin-activation.json")?, id)
    }

    pub fn select_shell(path: impl AsRef<Path>, id: &str) -> io::Result<()> {
        if !valid_identifier(id) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid shell identity",
            ));
        }
        let path = path.as_ref();
        let _lock = nickel_storage::TransactionLock::try_acquire(path)?;
        let mut settings = match Self::load(&path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound => Self::default(),
            Err(error) => return Err(error),
        };
        settings.selected_shell = Some(id.into());
        let bytes = serde_json::to_vec(&settings).map_err(io::Error::other)?;
        if bytes.len() > MAX_ACTIVATION_SETTINGS_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "plugin activation settings exceed limit",
            ));
        }
        nickel_storage::stage_write(&path, bytes)?.commit(|| Ok(()))
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
    #[serde(default)]
    pub composition: Option<crate::package_composition::ShellPackageComposition>,
    pub api_version: u16,
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub author: Option<String>,
    #[serde(default)]
    pub version: Option<String>,
    pub entry: String,
    #[serde(default)]
    pub stylesheet: Option<String>,
    #[serde(default)]
    pub images: Vec<PluginImageAsset>,
    #[serde(default)]
    pub surfaces: Vec<PluginSurface>,
    #[serde(default)]
    pub validation_data: BTreeMap<String, serde_json::Value>,
    #[serde(default)]
    pub capabilities: Vec<PluginCapability>,
    #[serde(default)]
    pub settings: Vec<PluginSetting>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PluginImageAsset {
    pub id: String,
    pub path: String,
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
pub struct PluginSurface {
    /// Ordinary surfaces open at activation by default. Transients stay explicit.
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub initially_open: bool,
    pub id: String,
    pub kind: PluginSurfaceKind,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub bottom_offset: u32,
    #[serde(default)]
    pub reserve_work_area: bool,
    #[serde(default)]
    pub output: PluginOutputScope,
    #[serde(default, skip_serializing_if = "PluginSurfaceAnchor::is_center")]
    pub anchor: PluginSurfaceAnchor,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub offset_x: i32,
    #[serde(default, skip_serializing_if = "is_zero_i32")]
    pub offset_y: i32,
    #[serde(default, skip_serializing_if = "is_false")]
    pub passive: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
}

fn default_true() -> bool {
    true
}
fn is_true(value: &bool) -> bool {
    *value
}

fn is_zero_i32(value: &i32) -> bool {
    *value == 0
}

fn is_false(value: &bool) -> bool {
    !*value
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum PluginSurfaceAnchor {
    #[default]
    Center,
    TopLeft,
    TopCenter,
    TopRight,
    BottomLeft,
    BottomCenter,
    BottomRight,
}

impl PluginSurfaceAnchor {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::TopLeft => "top-left",
            Self::TopCenter => "top-center",
            Self::TopRight => "top-right",
            Self::BottomLeft => "bottom-left",
            Self::BottomCenter => "bottom-center",
            Self::BottomRight => "bottom-right",
        }
    }

    pub fn is_center(&self) -> bool {
        *self == Self::Center
    }

    pub fn position(
        self,
        output: (i32, i32, u32, u32),
        size: (u32, u32),
        offset: (i32, i32),
    ) -> (i32, i32) {
        let (x, y, output_width, output_height) = output;
        let (width, height) = size;
        let remaining_x = output_width.saturating_sub(width).min(i32::MAX as u32) as i32;
        let remaining_y = output_height.saturating_sub(height).min(i32::MAX as u32) as i32;
        let anchor_x = match self {
            Self::TopLeft | Self::BottomLeft => 0,
            Self::TopRight | Self::BottomRight => remaining_x,
            Self::Center | Self::TopCenter | Self::BottomCenter => remaining_x / 2,
        };
        let anchor_y = match self {
            Self::TopLeft | Self::TopCenter | Self::TopRight => 0,
            Self::BottomLeft | Self::BottomCenter | Self::BottomRight => remaining_y,
            Self::Center => remaining_y / 2,
        };
        (
            x.saturating_add(anchor_x.saturating_add(offset.0))
                .clamp(x, x.saturating_add(remaining_x)),
            y.saturating_add(anchor_y.saturating_add(offset.1))
                .clamp(y, y.saturating_add(remaining_y)),
        )
    }
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
    OnScreenKeyboardInput,
    ApplicationsRead,
    ApplicationsLaunch,
    ApplicationsPin,
    AssociationsRead,
    AssociationsControl,
    PluginsRead,
    PluginsControl,
    FeaturesRead,
    FeaturesControl,
    ShortcutsRead,
    PreferencesRead,
    PreferencesControl,
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
    AppearanceRead,
    AppearanceControl,
    WallpaperRead,
    WallpaperControl,
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
            Self::OnScreenKeyboardInput => "on-screen-keyboard-input",
            Self::ApplicationsRead => "applications-read",
            Self::ApplicationsLaunch => "applications-launch",
            Self::ApplicationsPin => "applications-pin",
            Self::AssociationsRead => "associations-read",
            Self::AssociationsControl => "associations-control",
            Self::PluginsRead => "plugins-read",
            Self::PluginsControl => "plugins-control",
            Self::FeaturesRead => "features-read",
            Self::FeaturesControl => "features-control",
            Self::ShortcutsRead => "shortcuts-read",
            Self::PreferencesRead => "preferences-read",
            Self::PreferencesControl => "preferences-control",
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
            Self::AppearanceRead => "appearance-read",
            Self::AppearanceControl => "appearance-control",
            Self::WallpaperRead => "wallpaper-read",
            Self::WallpaperControl => "wallpaper-control",
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
    pub fn claims_native_shell_surface(&self) -> bool {
        matches!(
            self.id.as_str(),
            "org.nickel.desktop" | "org.nickel.screenshot"
        ) || self
            .surfaces
            .iter()
            .any(|surface| surface.kind == PluginSurfaceKind::Desktop)
    }

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
        if let Some(composition) = &self.composition {
            composition
                .validate()
                .map_err(|error| format!("invalid composition: {error:?}"))?;
            if composition.id != self.id || self.version.as_deref() != Some(&composition.version) {
                return Err("composition identity must match its package manifest".into());
            }
        }
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
        if self.stylesheet.as_ref().is_some_and(|path| {
            !safe_relative_path(path) || !path.ends_with(".css") || *path == self.entry
        }) {
            return Err(
                "plugin stylesheet must be a relative .css path inside the plugin directory".into(),
            );
        }
        if self.images.len() > 16 {
            return Err("plugin declares more than 16 images".into());
        }
        let mut image_ids = HashSet::new();
        let mut image_paths = HashSet::new();
        for image in &self.images {
            if !valid_identifier(&image.id) || !image_ids.insert(&image.id) {
                return Err(format!("invalid or duplicate image ID {:?}", image.id));
            }
            if !safe_relative_path(&image.path)
                || ![".png", ".jpg", ".jpeg", ".webp"]
                    .iter()
                    .any(|extension| image.path.ends_with(extension))
                || image.path == self.entry
                || !image_paths.insert(&image.path)
            {
                return Err(format!("invalid or duplicate image path {:?}", image.path));
            }
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
            if !(-8192..=8192).contains(&surface.offset_x)
                || !(-8192..=8192).contains(&surface.offset_y)
            {
                return Err(format!(
                    "surface {:?} has an invalid anchor offset",
                    surface.id
                ));
            }
            if !matches!(
                surface.kind,
                PluginSurfaceKind::Window | PluginSurfaceKind::Dialog | PluginSurfaceKind::Overlay
            ) && (surface.anchor != PluginSurfaceAnchor::Center
                || surface.offset_x != 0
                || surface.offset_y != 0)
            {
                return Err(format!(
                    "surface {:?} cannot use window anchoring",
                    surface.id
                ));
            }
            if surface.passive && surface.kind != PluginSurfaceKind::Overlay {
                return Err(format!(
                    "surface {:?} can be passive only as an overlay",
                    surface.id
                ));
            }
            if !matches!(
                surface.kind,
                PluginSurfaceKind::Panel | PluginSurfaceKind::Dock
            ) && surface.bottom_offset != 0
            {
                return Err(format!(
                    "surface {:?} cannot use a bottom offset",
                    surface.id
                ));
            }
            if surface.reserve_work_area
                && (surface.kind != PluginSurfaceKind::Panel || surface.bottom_offset != 0)
            {
                return Err(format!(
                    "surface {:?} can reserve work area only as an edge panel",
                    surface.id
                ));
            }
        }
        for (surface_id, data) in &self.validation_data {
            if !ids.contains(surface_id) || !data.is_object() {
                return Err(format!(
                    "validation data for {surface_id:?} must name a declared surface and contain an object"
                ));
            }
            if ["settings", "slots", "surface"]
                .iter()
                .any(|reserved| data.get(reserved).is_some())
            {
                return Err(format!(
                    "validation data for {surface_id:?} cannot replace host-owned fields"
                ));
            }
        }
        for surface in &self.surfaces {
            if let Some(owner) = &surface.owner {
                if surface.kind != PluginSurfaceKind::Dialog {
                    return Err(format!(
                        "only a dialog can declare an owner: {:?}",
                        surface.id
                    ));
                }
                if !self.surfaces.iter().any(|candidate| {
                    candidate.id == *owner
                        && candidate.kind == PluginSurfaceKind::Window
                        && candidate.output == surface.output
                }) {
                    return Err(format!(
                        "dialog {:?} must name a window on the same output as its owner",
                        surface.id
                    ));
                }
            }
        }
        let mut capabilities = HashSet::new();
        for capability in &self.capabilities {
            if !capabilities.insert(capability) {
                return Err(format!("duplicate capability {capability:?}"));
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
    #[test]
    fn embedded_package_enforces_paths_required_assets_and_source_bounds() {
        let manifest = br#"{"api_version":1,"id":"org.example.embedded","name":"Embedded","entry":"src/main.js","stylesheet":"src/theme.css"}"#;
        let files: &[(&str, &[u8])] = &[
            ("plugin.json", manifest),
            ("src/main.js", b"export default function App() {}"),
            ("src/theme.css", b"text { color: #ffffff; }"),
        ];
        let package = super::PluginPackage::from_embedded(files).unwrap();
        assert_eq!(package.modules.len(), 2);
        assert_eq!(package.manifest.id, "org.example.embedded");
        assert!(super::PluginPackage::from_embedded(&files[..2]).is_err());
        let mut duplicate = files.to_vec();
        duplicate.push(files[1]);
        assert!(super::PluginPackage::from_embedded(&duplicate).is_err());
        for path in [
            "../outside.js",
            "/outside.js",
            "C:/outside.js",
            "src//main.js",
        ] {
            let mut invalid = files.to_vec();
            invalid.push((path, b""));
            assert!(super::PluginPackage::from_embedded(&invalid).is_err());
        }
        let oversized = vec![b' '; super::MAX_PLUGIN_MODULE_BYTES + 1];
        let mut invalid = files.to_vec();
        invalid.push(("src/large.js", &oversized));
        assert!(super::PluginPackage::from_embedded(&invalid).is_err());
    }

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
                .contains("content changed after discovery")
        );
    }

    #[test]
    fn installed_packages_cannot_claim_native_shell_surfaces() {
        let root = tempfile::tempdir().unwrap();
        for (id, desktop_kind) in [
            ("org.nickel.desktop", false),
            ("org.nickel.screenshot", false),
            ("org.example.desktop-claim", true),
        ] {
            let directory = root.path().join(id);
            std::fs::create_dir(&directory).unwrap();
            let mut manifest = VALID.replace("org.nickel.hello-panel", id);
            if desktop_kind {
                manifest = manifest
                    .replace("\"kind\":\"panel\"", "\"kind\":\"desktop\"")
                    .replace("\"bottom_offset\":24,", "");
            }
            std::fs::write(directory.join("plugin.json"), manifest).unwrap();
            std::fs::write(directory.join("main.js"), "function App() {}").unwrap();
        }
        let catalog = PluginCatalog::discover(root.path()).unwrap();
        assert!(catalog.packages.is_empty());
        assert_eq!(catalog.failures.len(), 3);
        assert!(
            catalog
                .failures
                .iter()
                .all(|failure| failure.reason.contains("native Rust UI"))
        );
    }

    #[test]
    fn declared_image_is_loaded_and_covered_by_package_approval() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("org.nickel.hello-panel");
        std::fs::create_dir(&directory).unwrap();
        let manifest = VALID.replace(
            "\"entry\": \"main.js\",",
            "\"entry\": \"main.js\", \"images\": [{\"id\":\"icon\",\"path\":\"icon.png\"}],",
        );
        std::fs::write(directory.join("plugin.json"), manifest).unwrap();
        std::fs::write(directory.join("main.js"), "function App() {}").unwrap();
        std::fs::write(directory.join("icon.png"), b"first image").unwrap();
        let catalog = PluginCatalog::discover(root.path()).unwrap();
        let descriptor = &catalog.packages["org.nickel.hello-panel"];
        assert_eq!(descriptor.load().unwrap().images["icon"], b"first image");
        std::fs::write(directory.join("icon.png"), b"changed image").unwrap();
        assert!(descriptor.load().unwrap_err().contains("content changed"));
    }

    #[test]
    fn declared_stylesheet_is_bounded_and_covered_by_package_approval() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("org.nickel.hello-panel");
        std::fs::create_dir(&directory).unwrap();
        let manifest = VALID.replace(
            "\"entry\": \"main.js\",",
            "\"entry\": \"main.js\", \"stylesheet\": \"ui.css\",",
        );
        std::fs::write(directory.join("plugin.json"), manifest).unwrap();
        std::fs::write(directory.join("main.js"), "function App() {}").unwrap();
        std::fs::write(directory.join("ui.css"), "button { padding: 8px; }").unwrap();
        let catalog = PluginCatalog::discover(root.path()).unwrap();
        let descriptor = &catalog.packages["org.nickel.hello-panel"];
        assert_eq!(
            descriptor.load().unwrap().stylesheet,
            "button { padding: 8px; }"
        );
        std::fs::write(directory.join("ui.css"), "button { padding: 9px; }").unwrap();
        assert!(descriptor.load().unwrap_err().contains("content changed"));
        std::fs::write(
            directory.join("ui.css"),
            vec![b'x'; MAX_PLUGIN_CSS_BYTES + 1],
        )
        .unwrap();
        assert!(PluginPackage::load(&directory).is_err());
    }

    #[test]
    fn image_free_approvals_keep_the_existing_fingerprint_shape() {
        let manifest = PluginManifest::from_json(VALID).unwrap();
        let approval = serde_json::to_value(PluginApproval::from_manifest(
            &manifest,
            &digest_source("function App() {}"),
        ))
        .unwrap();
        assert!(approval.get("images").is_none());
    }

    #[test]
    fn image_manifest_rejects_unsafe_paths_and_duplicate_ids() {
        let manifest = VALID.replace(
            "\"entry\": \"main.js\",",
            "\"entry\": \"main.js\", \"images\": [{\"id\":\"icon\",\"path\":\"../icon.png\"}],",
        );
        assert!(PluginManifest::from_json(&manifest).is_err());
        let manifest = manifest.replace("../icon.png", "icon.png").replace(
            "\"path\":\"icon.png\"}],",
            "\"path\":\"icon.png\"},{\"id\":\"icon\",\"path\":\"other.png\"}],",
        );
        assert!(PluginManifest::from_json(&manifest).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_an_image_link_outside_the_plugin_directory() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let manifest = VALID.replace(
            "\"entry\": \"main.js\",",
            "\"entry\": \"main.js\", \"images\": [{\"id\":\"icon\",\"path\":\"icon.png\"}],",
        );
        std::fs::write(directory.path().join("plugin.json"), manifest).unwrap();
        std::fs::write(directory.path().join("main.js"), "function App() {}").unwrap();
        std::fs::write(outside.path().join("icon.png"), b"outside").unwrap();
        symlink(
            outside.path().join("icon.png"),
            directory.path().join("icon.png"),
        )
        .unwrap();
        assert!(
            PluginPackage::load(directory.path())
                .unwrap_err()
                .contains("escapes")
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

    #[test]
    fn default_shell_manifest_declares_one_composed_package() {
        let manifest = PluginManifest::from_json(include_str!(
            "../../../assets/plugins/nickel-default/plugin.json"
        ))
        .unwrap();
        assert_eq!(manifest.entry, "src/Shell.js");
        assert_eq!(manifest.surfaces.len(), 5);
        assert!(
            manifest
                .composition
                .unwrap()
                .exports
                .contains_key("shell.settings")
        );
    }

    #[test]
    fn composition_is_bound_to_package_identity_and_approval() {
        let mut manifest = PluginManifest::from_json(VALID).unwrap();
        manifest.version = Some("0.2.0".into());
        let mut composition: crate::package_composition::ShellPackageComposition = serde_json::from_value(serde_json::json!({
            "api_version":1,"id":manifest.id,"version":"0.2.0","exports":{"shell.taskbar":"./taskbar.js#Taskbar"}
        })).unwrap();
        manifest.composition = Some(composition.clone());
        manifest.validate().unwrap();
        let approved = PluginApproval::fingerprint(&manifest, &digest_source("source"));
        composition
            .exports
            .insert("shell.taskbar".into(), "./replacement.js#Taskbar".into());
        manifest.composition = Some(composition.clone());
        assert_ne!(
            approved,
            PluginApproval::fingerprint(&manifest, &digest_source("source"))
        );
        composition.id = "other-shell".into();
        manifest.composition = Some(composition);
        assert!(manifest.validate().unwrap_err().contains("identity"));
    }

    #[test]
    fn discovered_package_rejects_changed_dependency_content() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("org.nickel.hello-panel");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("plugin.json"), VALID).unwrap();
        std::fs::write(directory.join("main.js"), "import './card.js';").unwrap();
        std::fs::write(directory.join("card.js"), "export const value = 1;").unwrap();
        let catalog = PluginCatalog::discover(root.path()).unwrap();
        let descriptor = &catalog.packages["org.nickel.hello-panel"];
        assert!(descriptor.load().is_ok());
        std::fs::write(directory.join("card.js"), "export const value = 2;").unwrap();
        assert!(descriptor.load().unwrap_err().contains("content changed"));
    }

    #[test]
    fn loads_bounded_package_module_sources_in_stable_order() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::create_dir(directory.path().join("ui")).unwrap();
        std::fs::write(directory.path().join("plugin.json"), VALID).unwrap();
        std::fs::write(directory.path().join("main.js"), "import './ui/card.js';").unwrap();
        std::fs::write(
            directory.path().join("ui/card.js"),
            "export const Card = 1;",
        )
        .unwrap();
        std::fs::write(directory.path().join("ui/card.css"), ".card {}").unwrap();
        std::fs::write(directory.path().join("ui/icon.png"), b"ignored").unwrap();

        let modules = PluginPackage::load_module_sources(directory.path()).unwrap();
        assert_eq!(
            modules
                .iter()
                .map(|module| module.path.as_str())
                .collect::<Vec<_>>(),
            ["main.js", "ui/card.css", "ui/card.js"]
        );
        assert_eq!(modules[2].source, "export const Card = 1;");
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
        manifest.composition = PluginManifest::from_json(include_str!(
            "../../../assets/plugins/nickel-default/plugin.json"
        ))
        .unwrap()
        .composition;
        assert!(!settings.approval_current(&manifest, &source_digest));
    }

    #[test]
    fn retired_slot_manifest_contracts_are_rejected() {
        for field in ["provides_slots", "contributes"] {
            let source = VALID.replace(
                "\"capabilities\": []",
                &format!("\"capabilities\": [], \"{field}\": []"),
            );
            assert!(
                PluginManifest::from_json(&source)
                    .unwrap_err()
                    .contains("unknown field")
            );
        }
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
    fn validation_data_requires_a_declared_surface_and_cannot_shadow_host_data() {
        let mut manifest = PluginManifest::from_json(VALID).unwrap();
        manifest
            .validation_data
            .insert("main".into(), serde_json::json!({"title": "Sample"}));
        manifest.validate().unwrap();

        manifest
            .validation_data
            .insert("missing".into(), serde_json::json!({}));
        assert!(
            manifest
                .validate()
                .unwrap_err()
                .contains("declared surface")
        );
        manifest.validation_data.remove("missing");

        manifest
            .validation_data
            .insert("main".into(), serde_json::json!("Sample"));
        assert!(
            manifest
                .validate()
                .unwrap_err()
                .contains("contain an object")
        );

        for reserved in ["settings", "slots", "surface"] {
            let mut sample = serde_json::Map::new();
            sample.insert(reserved.into(), serde_json::json!({}));
            manifest
                .validation_data
                .insert("main".into(), serde_json::Value::Object(sample));
            assert!(
                manifest
                    .validate()
                    .unwrap_err()
                    .contains("host-owned fields")
            );
        }
    }

    #[test]
    fn anchored_overlay_manifest_is_bounded_and_placed_inside_output() {
        let source = VALID
            .replace("\"kind\":\"panel\"", "\"kind\":\"overlay\"")
            .replace("\"bottom_offset\":24,", "")
            .replace(
                "\"height\":64,",
                "\"height\":64,\"anchor\":\"top-right\",\"offset_x\":-18,\"offset_y\":24,\"passive\":true,",
            );
        let manifest = PluginManifest::from_json(&source).unwrap();
        let surface = &manifest.surfaces[0];
        assert_eq!(surface.anchor, PluginSurfaceAnchor::TopRight);
        assert!(surface.passive);
        assert_eq!(
            surface.anchor.position(
                (100, 200, 800, 600),
                (360, 64),
                (surface.offset_x, surface.offset_y)
            ),
            (522, 224)
        );
        assert_eq!(
            surface
                .anchor
                .position((100, 200, 200, 60), (360, 64), (-18, 24)),
            (100, 200),
        );
        assert!(
            PluginManifest::from_json(&source.replace("\"offset_x\":-18", "\"offset_x\":-9000"))
                .is_err()
        );
        assert!(
            PluginManifest::from_json(
                &source.replace("\"kind\":\"overlay\"", "\"kind\":\"panel\"")
            )
            .is_err()
        );
        let window = source
            .replace("\"kind\":\"overlay\"", "\"kind\":\"window\"")
            .replace(
                "\"anchor\":\"top-right\",\"offset_x\":-18,\"offset_y\":24,",
                "",
            );
        assert!(PluginManifest::from_json(&window).is_err());
    }

    #[test]
    fn centered_edge_anchors_resolve_with_bounded_offsets() {
        let output = (100, 200, 800, 600);
        let size = (360, 64);
        assert_eq!(
            PluginSurfaceAnchor::TopCenter.position(output, size, (0, 24)),
            (320, 224)
        );
        assert_eq!(
            PluginSurfaceAnchor::BottomCenter.position(output, size, (0, -82)),
            (320, 654)
        );
        assert_eq!(
            PluginSurfaceAnchor::BottomCenter.position(output, size, (9000, -9000)),
            (540, 200)
        );
    }

    #[test]
    fn accepts_a_portable_panel_manifest() {
        let manifest = PluginManifest::from_json(VALID).unwrap();
        assert_eq!(manifest.surfaces[0].bottom_offset, 24);
        assert!(!manifest.surfaces[0].reserve_work_area);
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
    fn reserves_work_area_only_for_an_edge_panel() {
        let panel = VALID.replace("\"bottom_offset\":24,", "\"reserve_work_area\":true,");
        assert!(PluginManifest::from_json(&panel).unwrap().surfaces[0].reserve_work_area);
        for invalid in [
            VALID.replace(
                "\"output\":\"all\"",
                "\"output\":\"all\",\"reserve_work_area\":true",
            ),
            panel.replace("\"kind\":\"panel\"", "\"kind\":\"dock\""),
        ] {
            assert!(PluginManifest::from_json(&invalid).is_err());
        }
    }

    #[test]
    fn windows_have_centered_placement_without_a_dock_offset() {
        let source = VALID
            .replace("\"kind\":\"panel\"", "\"kind\":\"window\"")
            .replace("\"bottom_offset\":24,", "");
        let manifest = PluginManifest::from_json(&source).unwrap();
        assert_eq!(manifest.surfaces[0].kind, PluginSurfaceKind::Window);
        assert_eq!(manifest.surfaces[0].bottom_offset, 0);
        assert!(
            PluginManifest::from_json(&VALID.replace("\"kind\":\"panel\"", "\"kind\":\"window\""))
                .is_err()
        );
    }

    #[test]
    fn dialog_owner_must_be_a_window_on_the_same_output() {
        let mut manifest = PluginManifest::from_json(include_str!(
            "../../../assets/plugins/example-surface-dialog/plugin.json"
        ))
        .unwrap();
        assert_eq!(manifest.surfaces[1].owner.as_deref(), Some("home"));
        manifest.surfaces[1].owner = Some("missing".into());
        assert!(
            manifest
                .validate()
                .unwrap_err()
                .contains("must name a window")
        );
        manifest.surfaces[1].owner = Some("home".into());
        manifest.surfaces[0].output = PluginOutputScope::All;
        assert!(manifest.validate().unwrap_err().contains("same output"));
        manifest.surfaces[0].output = PluginOutputScope::Primary;
        manifest.surfaces[0].owner = Some("home".into());
        assert!(manifest.validate().unwrap_err().contains("only a dialog"));
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
                initially_open: true,
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

#[cfg(test)]
mod shell_selection_persistence_tests {
    use super::PluginActivationSettings;
    #[test]
    fn selected_shell_round_trips_without_changing_package_enable_state() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("activation.json");
        PluginActivationSettings::update(&path, "theme", true).unwrap();
        PluginActivationSettings::select_shell(&path, "theme").unwrap();
        let settings = PluginActivationSettings::load(&path).unwrap();
        assert_eq!(settings.selected_shell(), Some("theme"));
        assert!(settings.desired_enabled("theme", false));
        assert!(PluginActivationSettings::select_shell(&path, "../invalid").is_err());
    }
}
