//! Isolated edit loop for an external surface or composition package.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use nickel_core::plugins::{MAX_PLUGIN_ENTRY_BYTES, PluginManifest, PluginPackage};

fn jsx_source(directory: &Path, entry: &str) -> Result<Option<PathBuf>, String> {
    let candidates = ["tsx", "jsx"]
        .map(|extension| Path::new(entry).with_extension(extension))
        .into_iter()
        .filter(|source| directory.join(source).exists())
        .collect::<Vec<_>>();
    let [source] = candidates.as_slice() else {
        return if candidates.is_empty() {
            Ok(None)
        } else {
            Err("both .jsx and .tsx sources exist for the declared entry".into())
        };
    };
    let metadata = std::fs::symlink_metadata(directory.join(source))
        .map_err(|error| format!("could not inspect JSX source: {error}"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_PLUGIN_ENTRY_BYTES as u64
    {
        return Err("JSX source must be an ordinary file of at most 2 MiB".into());
    }
    let resolved = std::fs::canonicalize(directory.join(source))
        .map_err(|error| format!("could not resolve JSX source: {error}"))?;
    let directory = std::fs::canonicalize(directory)
        .map_err(|error| format!("could not resolve plugin directory: {error}"))?;
    if !resolved.starts_with(directory) {
        return Err("JSX source escapes its plugin directory".into());
    }
    Ok(Some(source.clone()))
}

fn compile_jsx(directory: &Path, entry: &str, source: &Path) -> Result<String, String> {
    compile_jsx_modules(directory, entry, source).map(|(source, _)| source)
}

fn compile_jsx_modules(
    directory: &Path,
    entry: &str,
    source: &Path,
) -> Result<(String, Vec<nickel_core::plugins::PluginSourceFile>), String> {
    let output = tempfile::tempdir()
        .map_err(|error| format!("could not create JSX build directory: {error}"))?;
    // Manifest exports are module roots too; an exported JSX component need not
    // be imported by the entry merely to make the compiler emit its artifact.
    let jsx_roots = PluginPackage::load_module_sources(directory)?
        .into_iter()
        .filter(|module| module.path.ends_with(".jsx") && Path::new(&module.path) != source)
        .map(|module| module.path)
        .collect::<Vec<_>>();
    let compiler = directory.join("node_modules/.bin").join(tsc_executable());
    let compiler = if compiler.is_file() {
        compiler.into_os_string()
    } else {
        tsc_executable().into()
    };
    let status = Command::new(compiler)
        .current_dir(directory)
        .args([
            "--allowJs",
            // Every JSX module is an explicit root above. Avoid resolving its
            // checked-in .js artifact as a second input for the same output.
            "--noResolve",
            "--checkJs",
            "false",
            "--noCheck",
            "--noEmitOnError",
            "--jsx",
            "react",
            "--jsxFactory",
            "h",
            "--jsxFragmentFactory",
            "Fragment",
            "--target",
            "ES2020",
            "--lib",
            "ES2020",
            "--module",
            "ES2020",
            "--rootDir",
        ])
        .arg(".")
        .arg("--outDir")
        .arg(output.path())
        .arg(source)
        .args(jsx_roots)
        .status()
        .map_err(|error| format!("could not run TypeScript compiler (tsc): {error}"))?;
    if !status.success() {
        return Err(format!("JSX compilation failed with {status}"));
    }
    let compiled = output.path().join(entry);
    let metadata = std::fs::symlink_metadata(&compiled)
        .map_err(|error| format!("compiler did not produce {entry}: {error}"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_PLUGIN_ENTRY_BYTES as u64
    {
        return Err("compiled JavaScript must be an ordinary file of at most 2 MiB".into());
    }
    let source = std::fs::read_to_string(&compiled)
        .map_err(|error| format!("could not read compiled JavaScript: {error}"))?;
    let mut modules = PluginPackage::load_module_sources(directory)?;
    for compiled in PluginPackage::load_module_sources(output.path())? {
        if let Some(module) = modules
            .iter_mut()
            .find(|module| module.path == compiled.path)
        {
            *module = compiled;
        } else {
            modules.push(compiled);
        }
    }
    modules.sort_by(|left, right| left.path.cmp(&right.path));
    Ok((source, modules))
}

fn tsc_executable() -> &'static str {
    if cfg!(target_os = "windows") {
        "tsc.cmd"
    } else {
        "tsc"
    }
}

pub(super) fn load_package(directory: &Path) -> Result<PluginPackage, String> {
    let directory = std::fs::canonicalize(directory)
        .map_err(|error| format!("could not open plugin directory: {error}"))?;
    let manifest_bytes = std::fs::read(directory.join("plugin.json"))
        .map_err(|error| format!("could not read plugin.json: {error}"))?;
    let manifest_source = std::str::from_utf8(&manifest_bytes)
        .map_err(|error| format!("plugin.json is not UTF-8: {error}"))?;
    let manifest = PluginManifest::from_json(manifest_source)?;
    if let Some(source) = jsx_source(&directory, &manifest.entry)? {
        let (source, modules) = compile_jsx_modules(&directory, &manifest.entry, &source)?;
        Ok(PluginPackage {
            modules,
            source,
            stylesheet: PluginPackage::load_stylesheet(&directory, &manifest)?,
            images: PluginPackage::load_images(&directory, &manifest)?,
            manifest,
        })
    } else {
        PluginPackage::load(&directory)
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
mod platform {
    use std::{
        collections::{BTreeSet, HashSet, hash_map::DefaultHasher},
        hash::{Hash, Hasher},
        path::{Path, PathBuf},
        process::{Child, Command},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    #[cfg(test)]
    use super::tsc_executable;
    use super::{compile_jsx, jsx_source, load_package};
    use nickel_core::plugins::{
        MAX_PLUGIN_ENTRY_BYTES, PluginActivationSettings, PluginManifest, PluginPackage,
        PluginSurfaceKind,
    };
    use nickel_shell::plugin_panel::{PluginPanelApplication, manifest};

    fn bundled_manifest(id: &str) -> Option<&'static PluginManifest> {
        [manifest()].into_iter().find(|manifest| manifest.id == id)
    }

    fn staged_config_directory(root: &Path) -> PathBuf {
        #[cfg(target_os = "linux")]
        let directory = "nickel";
        #[cfg(target_os = "windows")]
        let directory = "Nickel";
        root.join(directory)
    }

    #[cfg(test)]
    fn load_dev_package(directory: &Path) -> Result<PluginPackage, String> {
        let package = load_dev_package_unvalidated(directory)?;
        PluginPanelApplication::validate_package(&package)?;
        Ok(package)
    }

    fn load_dev_package_unvalidated(directory: &Path) -> Result<PluginPackage, String> {
        let package = load_package(directory)?;
        if package.manifest.claims_native_shell_surface() {
            return Err("desktop and screenshot presentation are native Rust UI".into());
        }
        let bundled = if let Some(manifest) = bundled_manifest(&package.manifest.id) {
            if *manifest != package.manifest {
                return Err("bundled plugin dev requires its shipped manifest".into());
            }
            true
        } else {
            false
        };
        let panel = !package.manifest.surfaces.is_empty()
            && package.manifest.surfaces.iter().all(|surface| {
                matches!(
                    surface.kind,
                    PluginSurfaceKind::Panel
                        | PluginSurfaceKind::Dock
                        | PluginSurfaceKind::Window
                        | PluginSurfaceKind::Dialog
                        | PluginSurfaceKind::Overlay
                )
            })
            && package.manifest.surfaces.iter().any(|surface| {
                !matches!(
                    surface.kind,
                    PluginSurfaceKind::Dialog | PluginSurfaceKind::Overlay
                )
            });
        let composition = package.manifest.composition.is_some();
        if !bundled && !panel && !composition {
            return Err(
                "dev needs an unchanged bundled manifest, a panel, dock, or window that may declare dialogs, or a public composition package"
                    .into(),
            );
        }
        Ok(package)
    }

    fn source_fingerprint(directory: &Path, entry: &str) -> Result<u64, String> {
        let mut hasher = DefaultHasher::new();
        let manifest = std::fs::read(directory.join("plugin.json"))
            .map_err(|error| format!("could not read plugin.json: {error}"))?;
        manifest.hash(&mut hasher);
        let current_manifest = std::str::from_utf8(&manifest)
            .ok()
            .and_then(|source| PluginManifest::from_json(source).ok());
        let current_entry = current_manifest
            .as_ref()
            .map(|manifest| manifest.entry.as_str())
            .unwrap_or(entry);
        current_entry.hash(&mut hasher);
        // A missing new entry is still a distinct edit. Watch for its creation.
        let mut has_jsx = false;
        for extension in ["tsx", "jsx"] {
            let source = Path::new(&current_entry).with_extension(extension);
            has_jsx |= directory.join(&source).exists();
            let bytes = std::fs::read(directory.join(source)).unwrap_or_default();
            bytes.hash(&mut hasher);
        }
        if !has_jsx {
            std::fs::read(directory.join(current_entry))
                .unwrap_or_default()
                .hash(&mut hasher);
        }
        let mut siblings = std::fs::read_dir(directory)
            .map_err(|error| format!("could not watch plugin sources: {error}"))?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| {
                path.extension().is_some_and(|extension| {
                    matches!(extension.to_str(), Some("js" | "jsx" | "tsx"))
                })
            })
            .collect::<Vec<_>>();
        siblings.sort();
        for sibling in siblings {
            sibling.file_name().hash(&mut hasher);
            std::fs::read(sibling).unwrap_or_default().hash(&mut hasher);
        }
        if let Some(manifest) = current_manifest {
            if let Some(stylesheet) = manifest.stylesheet {
                stylesheet.hash(&mut hasher);
                std::fs::read(directory.join(stylesheet))
                    .unwrap_or_default()
                    .hash(&mut hasher);
            }
            for image in manifest.images {
                image.path.hash(&mut hasher);
                std::fs::read(directory.join(image.path))
                    .unwrap_or_default()
                    .hash(&mut hasher);
            }
        }
        for module in PluginPackage::load_module_sources(directory)? {
            module.path.hash(&mut hasher);
            module.source.hash(&mut hasher);
        }
        Ok(hasher.finish())
    }

    fn stage_images(package: &PluginPackage, target: &Path) -> Result<(), String> {
        for asset in &package.manifest.images {
            let bytes = package
                .images
                .get(&asset.id)
                .ok_or_else(|| format!("plugin image {:?} was not loaded", asset.id))?;
            let path = target.join(&asset.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not stage plugin image: {error}"))?;
            }
            std::fs::write(path, bytes)
                .map_err(|error| format!("could not stage plugin image: {error}"))?;
        }
        Ok(())
    }

    fn stage_modules(package: &PluginPackage, target: &Path) -> Result<(), String> {
        for module in &package.modules {
            let path = target.join(&module.path);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not stage module directory: {error}"))?;
            }
            std::fs::write(path, &module.source)
                .map_err(|error| format!("could not stage module: {error}"))?;
        }
        Ok(())
    }

    fn stage_stylesheet(package: &PluginPackage, target: &Path) -> Result<(), String> {
        let Some(relative) = &package.manifest.stylesheet else {
            return Ok(());
        };
        let path = target.join(relative);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not stage plugin stylesheet directory: {error}"))?;
        }
        std::fs::write(path, &package.stylesheet)
            .map_err(|error| format!("could not stage plugin stylesheet: {error}"))
    }

    fn stage(package: &PluginPackage, directory: &Path, root: &Path) -> Result<(), String> {
        if bundled_manifest(&package.manifest.id).is_some() {
            let target = root.join("bundled-source").join(&package.manifest.id);
            if target.exists() {
                std::fs::remove_dir_all(&target)
                    .map_err(|error| format!("could not replace staged bundled source: {error}"))?;
            }
            std::fs::create_dir_all(&target)
                .map_err(|error| format!("could not stage bundled source: {error}"))?;
            let mut siblings = BTreeSet::new();
            for entry in std::fs::read_dir(directory)
                .map_err(|error| format!("could not read bundled plugin directory: {error}"))?
            {
                let entry = entry.map_err(|error| error.to_string())?;
                let path = entry.path();
                if !path.extension().is_some_and(|extension| {
                    matches!(extension.to_str(), Some("js" | "jsx" | "tsx"))
                }) {
                    continue;
                }
                siblings.insert(path.with_extension("js").file_name().unwrap().to_owned());
            }
            for name in siblings {
                if Some(name.as_os_str()) == Path::new(&package.manifest.entry).file_name() {
                    continue;
                }
                let entry_name = name
                    .to_str()
                    .ok_or("bundled JavaScript file name is not UTF-8")?;
                if let Some(source) = jsx_source(directory, entry_name)? {
                    std::fs::write(
                        target.join(&name),
                        compile_jsx(directory, entry_name, &source)?,
                    )
                    .map_err(|error| format!("could not stage compiled JSX: {error}"))?;
                    continue;
                }
                let path = directory.join(&name);
                let metadata = std::fs::symlink_metadata(&path)
                    .map_err(|error| format!("could not inspect bundled JavaScript: {error}"))?;
                if !metadata.is_file()
                    || metadata.file_type().is_symlink()
                    || metadata.len() > MAX_PLUGIN_ENTRY_BYTES as u64
                {
                    return Err(
                        "bundled JavaScript must be an ordinary file of at most 2 MiB".into(),
                    );
                }
                std::fs::copy(&path, target.join(&name))
                    .map_err(|error| format!("could not stage bundled JavaScript: {error}"))?;
            }
            let entry = target.join(&package.manifest.entry);
            if let Some(parent) = entry.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| format!("could not stage bundled entry: {error}"))?;
            }
            std::fs::write(entry, &package.source)
                .map_err(|error| format!("could not stage bundled entry: {error}"))?;
            stage_stylesheet(package, &target)?;
            return stage_images(package, &target);
        }
        let manifest_bytes = std::fs::read(directory.join("plugin.json"))
            .map_err(|error| format!("could not read plugin manifest: {error}"))?;
        let manifest_source = std::str::from_utf8(&manifest_bytes)
            .map_err(|error| format!("plugin manifest is not UTF-8: {error}"))?;
        if PluginManifest::from_json(manifest_source)? != package.manifest {
            return Err("plugin manifest changed during staging".into());
        }
        let target = staged_config_directory(root)
            .join("plugins")
            .join(&package.manifest.id);
        if target.exists() {
            std::fs::remove_dir_all(&target)
                .map_err(|error| format!("could not replace staged plugin: {error}"))?;
        }
        std::fs::create_dir_all(&target)
            .map_err(|error| format!("could not stage plugin directory: {error}"))?;
        std::fs::write(target.join("plugin.json"), manifest_bytes)
            .map_err(|error| format!("could not stage manifest: {error}"))?;
        let entry = target.join(&package.manifest.entry);
        if let Some(parent) = entry.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|error| format!("could not stage entry directory: {error}"))?;
        }
        stage_modules(package, &target)?;
        std::fs::write(entry, &package.source)
            .map_err(|error| format!("could not stage JavaScript: {error}"))?;
        stage_stylesheet(package, &target)?;
        stage_images(package, &target)?;
        PluginActivationSettings::update_manifest(
            staged_config_directory(root).join("plugin-activation.json"),
            &package.manifest,
            &package.source_digest(),
            true,
        )
        .map_err(|error| format!("could not enable staged plugin: {error}"))
    }

    fn stop(child: &mut Child) {
        if child.try_wait().ok().flatten().is_none() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    fn launch(binary: &Path, config_root: &Path, bundled: bool) -> Result<Child, String> {
        let mut command = Command::new(binary);
        if bundled {
            command.env(
                "NICKEL_DEV_BUNDLED_PLUGIN_ROOT",
                config_root.join("bundled-source"),
            );
        }
        #[cfg(target_os = "linux")]
        {
            use std::os::unix::fs::PermissionsExt;

            let runtime = config_root.join("runtime");
            std::fs::create_dir_all(&runtime)
                .map_err(|error| format!("could not create nested runtime: {error}"))?;
            std::fs::set_permissions(&runtime, std::fs::Permissions::from_mode(0o700))
                .map_err(|error| format!("could not protect nested runtime: {error}"))?;
            let control_environment = config_root.join("test-control.env");
            if control_environment.exists() {
                std::fs::remove_file(&control_environment).map_err(|error| {
                    format!("could not replace test-control credentials: {error}")
                })?;
            }
            if let Some(display) = std::env::var_os("WAYLAND_DISPLAY") {
                let display = PathBuf::from(display);
                let display = if display.is_absolute() {
                    display
                } else {
                    std::env::var_os("XDG_RUNTIME_DIR")
                        .map(PathBuf::from)
                        .ok_or("WAYLAND_DISPLAY needs XDG_RUNTIME_DIR for nested plugin dev")?
                        .join(display)
                };
                command.env("WAYLAND_DISPLAY", display);
            }
            command
                .env_remove("WAYLAND_SOCKET")
                .env("XDG_CONFIG_HOME", config_root)
                .env("XDG_RUNTIME_DIR", runtime)
                .env("NICKEL_TEST_CONTROL_ENV_FILE", control_environment)
                .arg("--test-control");
        }
        #[cfg(target_os = "windows")]
        command
            .env("LOCALAPPDATA", config_root)
            .env("APPDATA", config_root)
            .arg("--no-desktop-windows");
        command
            .spawn()
            .map_err(|error| format!("could not start Nickel test shell: {error}"))
    }

    fn load_dev_packages(directories: &[PathBuf]) -> Result<Vec<PluginPackage>, String> {
        let packages = directories
            .iter()
            .map(|directory| load_dev_package_unvalidated(directory))
            .collect::<Result<Vec<_>, _>>()?;
        let ids = packages
            .iter()
            .map(|package| package.manifest.id.as_str())
            .collect::<HashSet<_>>();
        if ids.len() != packages.len() {
            return Err("dev package IDs must be distinct".into());
        }
        let mut catalog = nickel_shell::bundled_plugin_assets::validation_catalog()?;
        for package in &packages {
            catalog.insert(package.manifest.id.clone(), package.clone());
        }
        for package in &packages {
            PluginPanelApplication::validate_package_with_catalog(package, &catalog).map_err(
                |error| format!("plugin {} JavaScript failed: {error}", package.manifest.id),
            )?;
        }
        Ok(packages)
    }

    fn stage_all(
        packages: &[PluginPackage],
        directories: &[PathBuf],
        root: &Path,
    ) -> Result<(), String> {
        for (package, directory) in packages.iter().zip(directories) {
            stage(package, directory, root)?;
        }
        Ok(())
    }

    pub(super) fn run(directories: Vec<PathBuf>) -> Result<(), String> {
        let directories = directories
            .into_iter()
            .map(|directory| {
                std::fs::canonicalize(directory)
                    .map_err(|error| format!("could not open plugin directory: {error}"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let mut packages = load_dev_packages(&directories)?;
        let shell = std::env::current_exe()
            .map_err(|error| format!("could not locate nickel-plugin: {error}"))?
            .with_file_name(if cfg!(target_os = "windows") {
                "nickel.exe"
            } else {
                "nickel-nested"
            });
        if !shell.is_file() {
            return Err(format!(
                "{} is missing; build the Nickel shell binary beside nickel-plugin",
                shell.display()
            ));
        }
        let config = tempfile::tempdir()
            .map_err(|error| format!("could not create isolated plugin profile: {error}"))?;
        stage_all(&packages, &directories, config.path())?;
        let mut current_fingerprints = packages
            .iter()
            .zip(&directories)
            .map(|(package, directory)| source_fingerprint(directory, &package.manifest.entry))
            .collect::<Result<Vec<_>, _>>()?;
        let running = Arc::new(AtomicBool::new(true));
        let signal = running.clone();
        ctrlc::set_handler(move || signal.store(false, Ordering::SeqCst))
            .map_err(|error| format!("could not install interrupt handler: {error}"))?;
        println!(
            "Testing {} in isolated Nickel. Save a plugin manifest, source, or image to reload; Ctrl+C stops.",
            packages
                .iter()
                .map(|package| package.manifest.id.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
        #[cfg(target_os = "linux")]
        println!(
            "Nested input credentials: {}",
            config.path().join("test-control.env").display()
        );
        #[cfg(target_os = "windows")]
        println!("Isolated plugin profile: {}", config.path().display());
        let mut child = launch(
            &shell,
            config.path(),
            packages
                .iter()
                .any(|package| bundled_manifest(&package.manifest.id).is_some()),
        )?;
        let mut child_exited = false;
        while running.load(Ordering::SeqCst) {
            if !child_exited {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        eprintln!("Nickel test shell exited: {status}; waiting for a plugin edit");
                        child_exited = true;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        stop(&mut child);
                        return Err(format!("could not inspect Nickel test shell: {error}"));
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(500));
            let observed = match packages
                .iter()
                .zip(&directories)
                .map(|(package, directory)| source_fingerprint(directory, &package.manifest.entry))
                .collect::<Result<Vec<_>, _>>()
            {
                Ok(observed) => observed,
                Err(error) => {
                    eprintln!("plugin edit: {error}");
                    continue;
                }
            };
            if observed == current_fingerprints {
                continue;
            }
            current_fingerprints = observed;
            let next = match load_dev_packages(&directories) {
                Ok(next) => next,
                Err(error) => {
                    eprintln!("plugin edit: {error}");
                    continue;
                }
            };
            if next
                .iter()
                .zip(&packages)
                .any(|(next, old)| next.manifest.id != old.manifest.id)
            {
                eprintln!("plugin ID changed; restart nickel-plugin dev for a new identity");
                continue;
            }
            stop(&mut child);
            stage_all(&next, &directories, config.path())?;
            packages = next;
            current_fingerprints = packages
                .iter()
                .zip(&directories)
                .map(|(package, directory)| source_fingerprint(directory, &package.manifest.entry))
                .collect::<Result<Vec<_>, _>>()?;
            child = launch(
                &shell,
                config.path(),
                packages
                    .iter()
                    .any(|package| bundled_manifest(&package.manifest.id).is_some()),
            )?;
            child_exited = false;
            println!(
                "Reloaded {}",
                packages
                    .iter()
                    .map(|package| package.manifest.id.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
        }
        stop(&mut child);
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn connectivity_settings_use_native_controls_and_revision_bound_public_clients() {
            // Boa evaluation plus native view construction needs more than the test thread default.
            std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
            use nickel_plugin_runtime::{JsxModuleGraph, ModuleSource};
            use nickel_ui::SemanticRole;
            use nickel_ui::{SemanticAction, SemanticSelector, UiHost};
            let directory = tempfile::tempdir().unwrap();
            std::fs::create_dir(directory.path().join("styles")).unwrap();
            std::fs::write(
                directory.path().join("Wifi.jsx"),
                include_str!("../../../../assets/plugins/nickel-default/src/Wifi.jsx"),
            )
            .unwrap();
            std::fs::write(
                directory.path().join("Bluetooth.jsx"),
                include_str!("../../../../assets/plugins/nickel-default/src/Bluetooth.jsx"),
            )
            .unwrap();
            std::fs::write(
                directory.path().join("styles/connectivity.css"),
                include_str!(
                    "../../../../assets/plugins/nickel-default/src/styles/connectivity.css"
                ),
            )
            .unwrap();
            std::fs::write(directory.path().join("main.jsx"), "import {Wifi} from './Wifi.js'; import {Bluetooth} from './Bluetooth.js'; export default function App() { return <Window id='main' width={520} height={340}><Column><Wifi/><Bluetooth/></Column></Window>; }").unwrap();
            let (source, modules) = super::super::compile_jsx_modules(
                directory.path(),
                "main.js",
                Path::new("main.jsx"),
            )
            .unwrap();
            let graph = JsxModuleGraph::new(
                "main.js",
                modules.iter().map(|module| ModuleSource {
                    path: &module.path,
                    source: &module.source,
                }),
            )
            .unwrap();
            let mut manifest = PluginManifest::from_json(include_str!(
                "../../../../assets/plugins/example-window/plugin.json"
            ))
            .unwrap();
            manifest.capabilities.extend([
                nickel_core::plugins::PluginCapability::NetworkRead,
                nickel_core::plugins::PluginCapability::NetworkControl,
                nickel_core::plugins::PluginCapability::BluetoothRead,
                nickel_core::plugins::PluginCapability::BluetoothControl,
            ]);
            let data = serde_json::json!({
                "wifi":{"available":true,"enabled":true,"revision":"0123456789abcdef","operations":{"connect":true,"setEnabled":true},"networks":[
                    {"id":"profile-stable","name":"Cafe","signalPercent":80,"saved":true,"connected":false,"canConnect":true},
                    {"id":"profile-unsaved","name":"Cafe","signalPercent":60,"saved":false,"connected":false,"canConnect":false}]},
                "bluetooth":{"available":true,"powered":true,"discovering":false,"revision":"fedcba9876543210","operations":{"setPowered":true,"pair":true,"connect":false,"disconnect":false,"setDiscovery":false},"devices":[{"id":"device-stable","name":"Headset","paired":false,"connected":false}]}
            });
            let mut runtime =
                nickel_plugin_runtime::JsxRuntime::new_modules(&graph, Some(&data.to_string()))
                    .unwrap();
            let metadata: serde_json::Value =
                runtime.eval_json("__nickelSettingsMetadata()").unwrap();
            assert!(
                metadata["pages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|page| page["id"] == "wifi")
            );
            assert!(
                metadata["pages"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|page| page["id"] == "bluetooth")
            );
            let package = PluginPackage {
                manifest: manifest.clone(),
                source: source.clone(),
                stylesheet: String::new(),
                modules: modules.clone(),
                images: Default::default(),
            };
            let mut app = PluginPanelApplication::from_package(&package).unwrap();
            app.sync_data(&data).unwrap();
            let mut host = UiHost::new(app, 520, 340);
            let connect = host
                .query_unique(&SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Connect".into(),
                })
                .unwrap();
            host.perform_semantic_action(
                connect.id,
                SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert!(matches!(
                host.application_mut().take_effects().as_slice(),
                [nickel_shell::plugin_panel::PluginEffect::Connectivity { .. }]
            ));
            // Capture the actual public handler effect: the profile ID and observed revision survive rendering.
            let tree = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            fn find_action(node: &serde_json::Value, label: &str) -> Option<u64> {
                if node["kind"] == "button"
                    && node["children"].as_array().is_some_and(|children| {
                        children.iter().any(|child| {
                            child.as_str() == Some(label) || child["text"].as_str() == Some(label)
                        })
                    })
                {
                    return node["action"].as_u64();
                }
                node["children"]
                    .as_array()?
                    .iter()
                    .find_map(|child| find_action(child, label))
            }
            let action = find_action(&tree, "Connect").unwrap();
            runtime
                .eval(&format!("__nickelDispatch({action})"))
                .unwrap();
            let effects = runtime.take_effects().unwrap();
            assert_eq!(
                effects[0],
                serde_json::json!({"type":"wifi.connect","id":"profile-stable","revision":"0123456789abcdef"})
            );
            runtime.eval("__nickelCommitRender()").unwrap();
            runtime.finish_event(true).unwrap();
            let tree = runtime
                .render("__nickelRender()", |node| Ok(node.clone()))
                .unwrap();
            let pair = find_action(&tree, "Pair").unwrap();
            runtime.eval(&format!("__nickelDispatch({pair})")).unwrap();
            assert_eq!(
                runtime.take_effects().unwrap()[0],
                serde_json::json!({"type":"bluetooth.pair","id":"device-stable","revision":"fedcba9876543210"})
            );
            let unsupported = serde_json::json!({"wifi":{"available":false,"reason":"Adapter absent","networks":[],"operations":{}},"bluetooth":{"available":true,"powered":true,"devices":[{"id":"paired","name":"Headset","paired":true,"connected":false}],"operations":{}}});
            let mut app = PluginPanelApplication::from_package(&package).unwrap();
            app.sync_data(&unsupported).unwrap();
            let host = UiHost::new(app, 520, 340);
            assert!(
                host.query_unique(&SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Connect".into()
                })
                .is_err()
            );
            assert!(
                host.query_unique(&SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Pair".into()
                })
                .is_err()
            );
            nickel_plugin_presentation::css::StyleSheet::compile(include_str!(
                "../../../../assets/plugins/nickel-default/src/styles/connectivity.css"
            ))
            .unwrap();
            }).unwrap().join().unwrap();
        }

        #[test]
        fn ordinary_keyboard_module_uses_public_generation_bound_native_requests() {
            use nickel_plugin_runtime::{JsxModuleGraph, ModuleSource};
            let directory = tempfile::tempdir().unwrap();
            std::fs::create_dir(directory.path().join("styles")).unwrap();
            std::fs::write(
                directory.path().join("OnScreenKeyboard.jsx"),
                include_str!("../../../../assets/plugins/nickel-default/src/OnScreenKeyboard.jsx"),
            )
            .unwrap();
            std::fs::write(
                directory.path().join("styles/keyboard.css"),
                include_str!("../../../../assets/plugins/nickel-default/src/styles/keyboard.css"),
            )
            .unwrap();
            std::fs::write(directory.path().join("main.jsx"),"import {OnScreenKeyboard} from './OnScreenKeyboard.js'; export default OnScreenKeyboard;").unwrap();
            let (source, modules) = super::super::compile_jsx_modules(
                directory.path(),
                "main.js",
                Path::new("main.jsx"),
            )
            .unwrap();
            let graph = JsxModuleGraph::new(
                "main.js",
                modules.iter().map(|module| ModuleSource {
                    path: &module.path,
                    source: &module.source,
                }),
            )
            .unwrap();
            drop(graph);
            let snapshot = serde_json::json!({"available":true,"generation":7,"height":368,"recipientAvailable":true,"operations":{"press":true,"hide":true,"toggleDock":true,"holdModifiers":true,"resize":true},"rows":[[{"id":"key-a","label":"A","quarters":4,"enabled":true}]]});
            let mut manifest = PluginManifest::from_json(include_str!(
                "../../../../assets/plugins/example-window/plugin.json"
            ))
            .unwrap();
            manifest.surfaces[0].id = "keyboard".into();
            manifest.surfaces[0].width = 1056;
            manifest.surfaces[0].height = 640;
            manifest.surfaces[0].anchor = nickel_core::plugins::PluginSurfaceAnchor::BottomLeft;
            manifest.surfaces[0].kind = PluginSurfaceKind::Overlay;
            manifest.surfaces[0].passive = true;
            manifest.capabilities.extend([
                nickel_core::plugins::PluginCapability::OnScreenKeyboardRead,
                nickel_core::plugins::PluginCapability::OnScreenKeyboardInput,
            ]);
            let package = PluginPackage {
                manifest,
                source,
                modules,
                stylesheet: String::new(),
                images: Default::default(),
            };
            let mut app = PluginPanelApplication::from_package(&package).unwrap();
            app.sync_data(&serde_json::json!({"keyboard":snapshot}))
                .unwrap();
            let mut host = nickel_ui::UiHost::new(app, 1056, 368);
            fn invoke(host: &mut nickel_ui::UiHost<PluginPanelApplication>, label: &str) {
                let target = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: nickel_ui::SemanticRole::Button,
                        name: label.into(),
                    })
                    .unwrap();
                host.perform_semantic_action(
                    target.id,
                    nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                );
            }
            invoke(&mut host, "A");
            assert!(
                matches!(host.application_mut().take_effects().as_slice(),[nickel_shell::plugin_panel::PluginEffect::Keyboard {effect,..}] if effect.operation == "keyboard.press" && effect.id.as_deref() == Some("key-a") && effect.generation == 7)
            );
            invoke(&mut host, "Move up");
            assert!(
                matches!(host.application_mut().take_effects().as_slice(),[nickel_shell::plugin_panel::PluginEffect::Keyboard {effect,..}] if effect.operation == "keyboard.toggleDock")
            );
            let mut expanded = snapshot.clone();
            expanded["height"] = 640.into();
            expanded["dockTop"] = true.into();
            expanded["generation"] = 8.into();
            let mut resized = PluginPanelApplication::from_package(&package).unwrap();
            resized
                .sync_data(&serde_json::json!({"keyboard":expanded}))
                .unwrap();
            let mut resized = nickel_ui::UiHost::new(resized, 1056, 640);
            invoke(&mut resized, "Move down");
            assert!(
                matches!(resized.application_mut().take_effects().as_slice(),[nickel_shell::plugin_panel::PluginEffect::Keyboard{effect,..}] if effect.operation == "keyboard.toggleDock" && effect.generation == 8)
            );
            invoke(&mut resized, "Smaller");
            assert!(
                matches!(resized.application_mut().take_effects().as_slice(),[nickel_shell::plugin_panel::PluginEffect::Keyboard{effect,..}] if effect.operation == "keyboard.resize" && effect.delta == Some(-32) && effect.generation == 8)
            );
            let mut ungranted = package.clone();
            ungranted.manifest.capabilities.retain(|cap| {
                *cap != nickel_core::plugins::PluginCapability::OnScreenKeyboardInput
            });
            let mut denied = PluginPanelApplication::from_package(&ungranted).unwrap();
            denied
                .sync_data(&serde_json::json!({"keyboard":snapshot}))
                .unwrap();
            let mut denied = nickel_ui::UiHost::new(denied, 1056, 368);
            invoke(&mut denied, "A");
            assert!(denied.application_mut().take_effects().is_empty());
            assert!(
                denied
                    .application()
                    .last_error()
                    .unwrap()
                    .contains("not granted")
            );
        }

        #[test]
        fn quick_settings_uses_public_revision_bound_workspace_and_display_clients() {
            std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
                use nickel_plugin_runtime::{JsxModuleGraph, ModuleSource, JsxRuntime};
                let directory=tempfile::tempdir().unwrap();
                std::fs::create_dir(directory.path().join("styles")).unwrap();
                std::fs::write(directory.path().join("QuickSettings.jsx"),include_str!("../../../../assets/plugins/nickel-default/src/QuickSettings.jsx")).unwrap();
                std::fs::write(directory.path().join("styles/quick-settings.css"),include_str!("../../../../assets/plugins/nickel-default/src/styles/quick-settings.css")).unwrap();
                std::fs::write(directory.path().join("main.jsx"),"import {QuickSettings} from './QuickSettings.js'; export default QuickSettings;").unwrap();
                let (source,modules)=super::super::compile_jsx_modules(directory.path(),"main.js",Path::new("main.jsx")).unwrap();
                let graph=JsxModuleGraph::new("main.js",modules.iter().map(|module|ModuleSource{path:&module.path,source:&module.source})).unwrap();
                let revision="a".repeat(64);
                let data=serde_json::json!({"workspaces":{"available":true,"revision":revision,"workspaces":[{"id":"9","active":true},{"id":"44","active":false}],"activeWorkspace":"9","operations":{"switch":true,"create":true,"remove":true}},"desktop":{"operations":{"toggleShowDesktop":true}},"displays":{"available":true,"revision":"0123456789abcdef","projectionModes":[{"id":"extend","label":"Extend"}],"outputs":[]}});
                let mut runtime=JsxRuntime::new_modules(&graph,Some(&data.to_string())).unwrap();
                fn action(node:&serde_json::Value,id:&str)->Option<u64>{if node["id"]==id {return node["action"].as_u64();}node["children"].as_array()?.iter().find_map(|child|action(child,id))}
                for (id,expected) in [("workspace-44",serde_json::json!({"type":"workspaces.switch","id":"44","revision":revision})),("projection-extend",serde_json::json!({"type":"displays.previewProjection","mode":"extend","revision":"0123456789abcdef"})),("show-desktop",serde_json::json!({"type":"desktop.toggleShowDesktop"}))] {
                    let tree=runtime.render("__nickelRender()",|node|Ok(node.clone())).unwrap();
                    let action=action(&tree,id).unwrap();
                    runtime.render(&format!("__nickelDispatch({action})"),|node|Ok(node.clone())).unwrap(); runtime.finish_event(true).unwrap();
                    assert_eq!(runtime.take_effects().unwrap(),vec![expected]);
                }
                let mut manifest=PluginManifest::from_json(include_str!("../../../../assets/plugins/nickel-default/plugin.json")).unwrap(); manifest.composition=None; manifest.entry="main.js".into(); manifest.surfaces.retain(|surface|surface.id=="quick-settings");
                let package=PluginPackage{manifest,source,modules,stylesheet:String::new(),images:Default::default()};
                let mut app=PluginPanelApplication::from_package(&package).unwrap(); app.sync_data(&data).unwrap();
                let mut host=nickel_ui::UiHost::new(app,420,600);
                let target=host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {role:nickel_ui::SemanticRole::Button,name:"2".into()}).unwrap();
                host.perform_semantic_action(target.id,nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate));
                assert!(matches!(host.application_mut().take_effects().as_slice(),[nickel_shell::plugin_panel::PluginEffect::Workspace{..}]));
            }).unwrap().join().unwrap();
        }

        #[test]
        fn feature_settings_confirm_destructive_preferences_and_render_public_shortcuts() {
            std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
                use nickel_plugin_runtime::{JsxModuleGraph, ModuleSource, JsxRuntime};
                let directory = tempfile::tempdir().unwrap();
                std::fs::create_dir(directory.path().join("styles")).unwrap();
                std::fs::write(directory.path().join("OptionalFeatures.jsx"), include_str!("../../../../assets/plugins/nickel-default/src/OptionalFeatures.jsx")).unwrap();
                std::fs::write(directory.path().join("KeyboardShortcuts.jsx"), include_str!("../../../../assets/plugins/nickel-default/src/KeyboardShortcuts.jsx")).unwrap();
                std::fs::write(directory.path().join("styles/features.css"), include_str!("../../../../assets/plugins/nickel-default/src/styles/features.css")).unwrap();
                std::fs::write(directory.path().join("main.jsx"), "import {OptionalFeatures} from './OptionalFeatures.js'; import {KeyboardShortcuts} from './KeyboardShortcuts.js'; export default function App() { return <Window id='main' width={520} height={340}><Column><OptionalFeatures/><KeyboardShortcuts/></Column></Window>; }").unwrap();
                let (source, modules) = super::super::compile_jsx_modules(directory.path(), "main.js", Path::new("main.jsx")).unwrap();
                let graph = JsxModuleGraph::new("main.js", modules.iter().map(|module|ModuleSource {path:&module.path,source:&module.source})).unwrap();
                let revision = "a".repeat(64);
                let data = serde_json::json!({"features":{"available":true,"revision":revision,"operations":{"setKeyboardMode":true,"setCodexEnabled":true,"retryCodex":true},"keyboard":{"mode":"automatic","runtimeAvailable":true},"codex":{"requestedEnabled":true,"disableConfirmationRequired":true,"state":"enabled","policy":"editable","runtimeCountersAvailable":false}},"shortcuts":{"available":true,"editable":false,"reason":"Shortcut remapping is unsupported","globalAvailable":false,"globalReason":"Native shortcuts unavailable","shortcuts":[{"id":"launcher","action":"Open launcher","keys":"Super","scope":"Global","available":false}]}});
                let mut runtime = JsxRuntime::new_modules(&graph,Some(&data.to_string())).unwrap();
                let metadata:serde_json::Value = runtime.eval_json("__nickelSettingsMetadata()").unwrap();
                for id in ["optional-features","keyboard-shortcuts"] { assert!(metadata["pages"].as_array().unwrap().iter().any(|page|page["id"]==id)); }
                fn action(node:&serde_json::Value,id:&str)->Option<u64> { if node["id"].as_str()==Some(id) {return node["action"].as_u64();} node["children"].as_array()?.iter().find_map(|child|action(child,id)) }
                let tree = runtime.render("__nickelRender()",|node|Ok(node.clone())).unwrap();
                let switch = action(&tree,"feature-codex-enabled").unwrap();
                let confirm = runtime.render(&format!("__nickelDispatch({switch})"),|node|Ok(node.clone())).unwrap();
                runtime.finish_event(true).unwrap();
                assert!(runtime.take_effects().unwrap().is_empty());
                let disable = action(&confirm,"feature-codex-confirm").unwrap();
                runtime.render(&format!("__nickelDispatch({disable})"),|node|Ok(node.clone())).unwrap();
                runtime.finish_event(true).unwrap();
                assert_eq!(runtime.take_effects().unwrap(),vec![serde_json::json!({"type":"features.setCodexEnabled","revision":revision,"enabled":false,"confirmed":true})]);
                let mut manifest = PluginManifest::from_json(include_str!("../../../../assets/plugins/example-window/plugin.json")).unwrap();
                manifest.capabilities.extend([nickel_core::plugins::PluginCapability::FeaturesRead,nickel_core::plugins::PluginCapability::FeaturesControl,nickel_core::plugins::PluginCapability::ShortcutsRead]);
                let package = PluginPackage {manifest,source,modules,stylesheet:String::new(),images:Default::default()};
                let mut app = PluginPanelApplication::from_package(&package).unwrap();
                app.sync_data(&data).unwrap();
                let host = nickel_ui::UiHost::new(app,520,340);
                assert!(host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {role:nickel_ui::SemanticRole::Switch,name:"Enable Codex integration".into()}).is_ok());
                nickel_plugin_presentation::css::StyleSheet::compile(include_str!("../../../../assets/plugins/nickel-default/src/styles/features.css")).unwrap();
            }).unwrap().join().unwrap();
        }

        #[test]
        fn default_launcher_owns_paging_query_and_launches_stable_search_identities() {
            std::thread::Builder::new().stack_size(16 * 1024 * 1024).spawn(|| {
                use nickel_plugin_runtime::{JsxModuleGraph, ModuleSource, JsxRuntime};
                let directory = tempfile::tempdir().unwrap();
                std::fs::create_dir(directory.path().join("styles")).unwrap();
                std::fs::write(directory.path().join("Launcher.jsx"), include_str!("../../../../assets/plugins/nickel-default/src/Launcher.jsx")).unwrap();
                std::fs::write(directory.path().join("styles/launcher.css"), include_str!("../../../../assets/plugins/nickel-default/src/styles/launcher.css")).unwrap();
                std::fs::write(directory.path().join("main.jsx"), "import {Launcher} from './Launcher.js'; export default Launcher;").unwrap();
                let (source, modules) = super::super::compile_jsx_modules(directory.path(), "main.js", Path::new("main.jsx")).unwrap();
                let graph = JsxModuleGraph::new("main.js", modules.iter().map(|module|ModuleSource {path:&module.path,source:&module.source})).unwrap();
                let apps = (0..14).map(|index|serde_json::json!({"id":format!("application-{index}"),"name":format!("Editor {index}"),"icon":format!("application:icon-{index}"),"pinned":false,"pinOrder":null,"recentOrder":null,"kind":"application"})).collect::<Vec<_>>();
                let mut data = serde_json::json!({"applications":apps,"applicationSearch":{"available":true,"query":"","results":[],"total":0}});
                let mut runtime = JsxRuntime::new_modules(&graph, Some(&data.to_string())).unwrap();
                fn action(node: &serde_json::Value, id: &str) -> Option<u64> {
                    if node["id"].as_str() == Some(id) {return node["action"].as_u64();}
                    node["children"].as_array()?.iter().find_map(|child|action(child,id))
                }
                let tree = runtime.render("__nickelRender()", |node|Ok(node.clone())).unwrap();
                let next = action(&tree,"launcher-dashboard-next").unwrap();
                let page = runtime.render(&format!("__nickelDispatch({next})"), |node|Ok(node.clone())).unwrap();
                runtime.finish_event(true).unwrap();
                assert!(runtime.take_effects().unwrap().is_empty());
                assert!(action(&page,"launcher-dashboard-application:icon-12").is_some());
                assert!(action(&page,"launcher-dashboard-application:icon-0").is_none());
                let input = action(&page,"launcher-query").unwrap();
                runtime.render(&format!("__nickelDispatch({input}, 'ed')"), |node|Ok(node.clone())).unwrap();
                runtime.finish_event(true).unwrap();
                assert_eq!(runtime.take_effects().unwrap(),vec![serde_json::json!({"type":"applications.search","query":"ed"})]);
                data["applicationSearch"] = serde_json::json!({"available":true,"query":"ed","total":1,"results":[{"id":"native-editor","name":"Native Editor","icon":"application:native-editor","pinned":false}]});
                runtime.set_data(&data.to_string()).unwrap();
                let results = runtime.render("__nickelRender()", |node|Ok(node.clone())).unwrap();
                let launch = action(&results,"launcher-result-application:native-editor").unwrap();
                runtime.render(&format!("__nickelDispatch({launch})"), |node|Ok(node.clone())).unwrap();
                runtime.finish_event(true).unwrap();
                assert_eq!(runtime.take_effects().unwrap(),vec![serde_json::json!({"type":"applications.launch","id":"native-editor"}),serde_json::json!({"type":"surface.hide","surfaceId":"launcher"})]);
                let mut manifest = PluginManifest::from_json(include_str!("../../../../assets/plugins/nickel-default/plugin.json")).unwrap();
                manifest.entry = "main.js".into();
                manifest.composition = None;
                manifest.surfaces.retain(|surface|surface.id=="launcher");
                let package = PluginPackage {manifest,source,modules,stylesheet:String::new(),images:Default::default()};
                let mut app = PluginPanelApplication::from_package(&package).unwrap();
                app.sync_data(&data).unwrap();
                let host = nickel_ui::UiHost::new(app,620,548);
                assert!(host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {role:nickel_ui::SemanticRole::Button,name:"Pinned & recent".into()}).is_ok());
            }).unwrap().join().unwrap();
        }

        #[test]
        fn development_compiles_and_stages_imported_jsx_modules() {
            if Command::new(tsc_executable())
                .arg("--version")
                .output()
                .is_err()
            {
                return;
            }
            let directory = tempfile::tempdir().unwrap();
            let fixture = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/example-window"
            ));
            std::fs::copy(
                fixture.join("plugin.json"),
                directory.path().join("plugin.json"),
            )
            .unwrap();
            std::fs::copy(fixture.join("ui.css"), directory.path().join("ui.css")).unwrap();
            std::fs::copy(fixture.join("icon.png"), directory.path().join("icon.png")).unwrap();
            std::fs::write(directory.path().join("main.jsx"), "import Card from './card.js';\nexport default function App() { return <Window id='main' width={520} height={340}><Card /></Window>; }").unwrap();
            std::fs::write(
                directory.path().join("card.jsx"),
                "export default function Card() { return <Text>Imported card</Text>; }",
            )
            .unwrap();
            let package = load_dev_package(directory.path()).unwrap();
            assert!(
                package
                    .modules
                    .iter()
                    .any(|module| module.path == "card.js" && module.source.contains("h(Text"))
            );
            let profile = tempfile::tempdir().unwrap();
            stage(&package, directory.path(), profile.path()).unwrap();
            let staged = staged_config_directory(profile.path())
                .join("plugins")
                .join(&package.manifest.id);
            let loaded = PluginPackage::load(&staged).unwrap();
            assert_eq!(loaded.source_digest(), package.source_digest());
            PluginPanelApplication::validate_package(&loaded).unwrap();
        }

        #[test]
        fn hello_panel_uses_the_bundled_dev_path() {
            let root = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/hello-panel"
            ));
            let package = load_dev_package(root).unwrap();
            assert_eq!(package.manifest.id, "org.nickel.hello-panel");
            assert_eq!(package.manifest.surfaces.len(), 1);
        }

        #[test]
        fn dev_validates_supplied_composition_dependencies_in_any_order() {
            let base = tempfile::tempdir().unwrap();
            let child = tempfile::tempdir().unwrap();
            std::fs::write(base.path().join("plugin.json"), r#"{"api_version":1,"id":"org.example.dev-base","name":"Base","version":"1.0.0","entry":"main.js","composition":{"api_version":1,"id":"org.example.dev-base","version":"1.0.0","exports":{"shell":"./main.js#Shell"}},"surfaces":[{"id":"main","kind":"panel","width":300,"height":48}]}"#).unwrap();
            std::fs::write(base.path().join("main.js"), "export function Shell(){return h(Panel,{},h(Text,{},'Base'));}\nexport default Shell;").unwrap();
            std::fs::write(child.path().join("plugin.json"), r#"{"api_version":1,"id":"org.example.dev-child","name":"Child","version":"1.0.0","entry":"main.js","composition":{"api_version":1,"id":"org.example.dev-child","version":"1.0.0","extends":"org.example.dev-base","requires":{"org.example.dev-base":"^1"}},"surfaces":[{"id":"main","kind":"panel","width":300,"height":48}]}"#).unwrap();
            std::fs::write(
                child.path().join("main.js"),
                "export function Unused(){return h(Text,{},'Child');}\nexport default Unused;",
            )
            .unwrap();
            let packages =
                super::load_dev_packages(&[child.path().into(), base.path().into()]).unwrap();
            assert_eq!(packages[0].manifest.id, "org.example.dev-child");
            assert!(
                super::load_dev_packages(&[child.path().into()])
                    .unwrap_err()
                    .contains("org.example.dev-base")
            );
        }

        #[test]
        fn bundled_dev_requires_the_shipped_manifest() {
            let directory = tempfile::tempdir().unwrap();
            let root = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/hello-panel"
            ));
            let manifest = std::fs::read_to_string(root.join("plugin.json")).unwrap();
            std::fs::write(
                directory.path().join("plugin.json"),
                manifest.replace("Hello Panel", "Renamed Panel"),
            )
            .unwrap();
            std::fs::copy(root.join("main.js"), directory.path().join("main.js")).unwrap();
            std::fs::copy(root.join("ui.css"), directory.path().join("ui.css")).unwrap();
            assert!(
                load_dev_package(directory.path())
                    .unwrap_err()
                    .contains("shipped manifest")
            );
        }

        #[test]
        fn stages_a_valid_panel_in_an_isolated_enabled_profile() {
            let source = tempfile::tempdir().unwrap();
            std::fs::write(
                source.path().join("plugin.json"),
                r#"{"api_version":1,"id":"org.example.clock","name":"Clock","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":300,"height":48}]}"#,
            )
            .unwrap();
            std::fs::write(
                source.path().join("main.js"),
                "function App() { return h(Panel, {}, h(Text, null, 'Clock')); }",
            )
            .unwrap();
            let package = load_dev_package(source.path()).unwrap();
            let profile = tempfile::tempdir().unwrap();
            stage(&package, source.path(), profile.path()).unwrap();
            let staged = PluginPackage::load(
                staged_config_directory(profile.path()).join("plugins/org.example.clock"),
            )
            .unwrap();
            assert_eq!(staged.source, package.source);
            let activation = PluginActivationSettings::load(
                staged_config_directory(profile.path()).join("plugin-activation.json"),
            )
            .unwrap();
            assert!(activation.desired_enabled("org.example.clock", false));
            assert!(activation.approval_current(&staged.manifest, &staged.source_digest()));
        }

        #[test]
        fn stages_and_watches_declared_plugin_images() {
            let source = tempfile::tempdir().unwrap();
            let example = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/example-window"
            ));
            for name in ["plugin.json", "main.js", "ui.css", "icon.png"] {
                std::fs::copy(example.join(name), source.path().join(name)).unwrap();
            }
            let package = load_dev_package(source.path()).unwrap();
            let fingerprint = source_fingerprint(source.path(), &package.manifest.entry).unwrap();
            let profile = tempfile::tempdir().unwrap();
            stage(&package, source.path(), profile.path()).unwrap();
            let staged = PluginPackage::load(
                staged_config_directory(profile.path())
                    .join("plugins/org.example.component-window"),
            )
            .unwrap();
            assert_eq!(staged.images, package.images);
            assert_eq!(staged.source_digest(), package.source_digest());
            std::fs::write(source.path().join("icon.png"), b"changed image").unwrap();
            assert_ne!(
                fingerprint,
                source_fingerprint(source.path(), &package.manifest.entry).unwrap()
            );
        }

        #[test]
        fn stages_and_watches_declared_plugin_stylesheet() {
            let source = tempfile::tempdir().unwrap();
            std::fs::write(
                source.path().join("plugin.json"),
                r#"{"api_version":1,"id":"org.example.css","name":"CSS","entry":"main.js","stylesheet":"ui.css","surfaces":[{"id":"main","kind":"panel","width":300,"height":48}]}"#,
            ).unwrap();
            std::fs::write(
                source.path().join("main.js"),
                "function App() { return h(Panel, {}, h(Text, null, 'CSS')); }",
            )
            .unwrap();
            std::fs::write(source.path().join("ui.css"), "text { color: #fff; }").unwrap();
            let package = load_dev_package(source.path()).unwrap();
            let fingerprint = source_fingerprint(source.path(), &package.manifest.entry).unwrap();
            let profile = tempfile::tempdir().unwrap();
            stage(&package, source.path(), profile.path()).unwrap();
            let staged = PluginPackage::load(
                staged_config_directory(profile.path()).join("plugins/org.example.css"),
            )
            .unwrap();
            assert_eq!(staged.stylesheet, package.stylesheet);
            assert_eq!(staged.source_digest(), package.source_digest());
            std::fs::write(source.path().join("ui.css"), "text { color: #000; }").unwrap();
            assert_ne!(
                fingerprint,
                source_fingerprint(source.path(), &package.manifest.entry).unwrap()
            );
        }

        #[test]
        fn accepts_a_translucent_dock_package() {
            let source = tempfile::tempdir().unwrap();
            std::fs::write(
                source.path().join("plugin.json"),
                r#"{"api_version":1,"id":"org.example.dock","name":"Dock","entry":"main.js","surfaces":[{"id":"main","kind":"dock","width":400,"height":72,"bottom_offset":24}]}"#,
            )
            .unwrap();
            std::fs::write(
                source.path().join("main.js"),
                "function App() { return h(Panel, {background: 0x80202020}, h(Text, {}, 'Dock')); }",
            )
            .unwrap();
            let package = load_dev_package(source.path()).unwrap();
            assert_eq!(package.manifest.surfaces[0].kind, PluginSurfaceKind::Dock);
            let profile = tempfile::tempdir().unwrap();
            stage(&package, source.path(), profile.path()).unwrap();
            let staged = PluginPackage::load(
                staged_config_directory(profile.path()).join("plugins/org.example.dock"),
            )
            .unwrap();
            let activation = PluginActivationSettings::load(
                staged_config_directory(profile.path()).join("plugin-activation.json"),
            )
            .unwrap();
            assert!(activation.approval_current(&staged.manifest, &staged.source_digest()));
        }

        #[test]
        fn validates_each_surface_of_one_package() {
            let source = tempfile::tempdir().unwrap();
            std::fs::write(
                source.path().join("plugin.json"),
                r#"{"api_version":1,"id":"org.example.two","name":"Two","entry":"main.js","surfaces":[{"id":"clock","kind":"panel","width":300,"height":48},{"id":"dock","kind":"dock","width":400,"height":72}]}"#,
            )
            .unwrap();
            std::fs::write(
                source.path().join("main.js"),
                "function App() { return h(Panel, {height: nickel.data.surface.height}, h(Text, {}, nickel.data.surface.id)); }",
            )
            .unwrap();
            let package = load_dev_package(source.path()).unwrap();
            assert_eq!(package.manifest.surfaces.len(), 2);
            std::fs::write(
                source.path().join("main.js"),
                "function App() { return nickel.data.surface.id === 'dock' ? h('unknown-component', {}) : h(Panel, {}, h(Text, {}, 'Clock')); }",
            )
            .unwrap();
            assert!(
                load_dev_package(source.path())
                    .unwrap_err()
                    .contains("surface \"dock\"")
            );
        }

        #[test]
        fn compiles_public_jsx_exports_without_entry_imports() {
            if Command::new(tsc_executable())
                .arg("--version")
                .output()
                .is_err()
            {
                return;
            }
            let source = tempfile::tempdir().unwrap();
            std::fs::write(source.path().join("plugin.json"),
                r#"{"api_version":1,"id":"org.example.exports","name":"Exports","version":"0.1.0","entry":"main.js","composition":{"api_version":1,"id":"org.example.exports","version":"0.1.0","exports":{"shell.widget":"./widget.js#Widget"}},"surfaces":[{"id":"main","kind":"panel","width":300,"height":48}]}"#,
            ).unwrap();
            std::fs::write(source.path().join("main.jsx"), "export function App() { const Widget = nickel.component('shell.widget'); return <FixedWindow width={300} height={48}><Widget /></FixedWindow>; }").unwrap();
            std::fs::write(
                source.path().join("widget.jsx"),
                "export function Widget() { return <Text>Public widget</Text>; }",
            )
            .unwrap();
            let package = load_dev_package(source.path()).unwrap();
            assert!(package.modules.iter().any(
                |module| module.path == "widget.js" && module.source.contains("Public widget")
            ));
            assert!(!source.path().join("widget.js").exists());
            // Shipped packages keep compiled JS alongside editable JSX. It must
            // neither collide in tsc nor override a fresh JSX compilation.
            std::fs::write(
                source.path().join("widget.js"),
                "throw Error('stale artifact');",
            )
            .unwrap();
            std::fs::write(source.path().join("main.jsx"), "import './widget.js';\nexport function App() { const Widget = nickel.component('shell.widget'); return <FixedWindow width={300} height={48}><Widget /></FixedWindow>; }").unwrap();
            let package = load_dev_package(source.path()).unwrap();
            assert!(package.modules.iter().any(
                |module| module.path == "widget.js" && module.source.contains("Public widget")
            ));
            assert_eq!(
                std::fs::read_to_string(source.path().join("widget.js")).unwrap(),
                "throw Error('stale artifact');"
            );
        }

        #[test]
        fn compiles_jsx_without_overwriting_the_entry_and_rejects_invalid_edits() {
            if Command::new(tsc_executable())
                .arg("--version")
                .output()
                .is_err()
            {
                return;
            }
            let source = tempfile::tempdir().unwrap();
            std::fs::write(source.path().join("plugin.json"),
                r#"{"api_version":1,"id":"org.example.jsx","name":"JSX","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":300,"height":48}]}"#,
            ).unwrap();
            let jsx = source.path().join("main.jsx");
            std::fs::write(
                &jsx,
                "function App() { return <Panel><Text>From JSX</Text></Panel>; }",
            )
            .unwrap();
            let package = load_dev_package(source.path()).unwrap();
            assert!(package.source.contains("h(Panel"));
            assert!(!source.path().join("main.js").exists());
            let previous = source_fingerprint(source.path(), "main.js").unwrap();
            std::fs::write(&jsx, "function App() { return <Panel>").unwrap();
            assert_ne!(
                source_fingerprint(source.path(), "main.js").unwrap(),
                previous
            );
            assert!(load_dev_package(source.path()).is_err());
            assert!(!source.path().join("main.js").exists());
        }

        #[test]
        fn compiles_tsx_with_type_annotations() {
            if Command::new(tsc_executable())
                .arg("--version")
                .output()
                .is_err()
            {
                return;
            }
            let source = tempfile::tempdir().unwrap();
            std::fs::write(
                source.path().join("plugin.json"),
                r#"{"api_version":1,"id":"org.example.tsx","name":"TSX","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":300,"height":48}]}"#,
            )
            .unwrap();
            std::fs::write(
                source.path().join("main.tsx"),
                "function App() { const label: string = 'From TSX'; return <Panel><Text>{label}</Text></Panel>; }",
            )
            .unwrap();
            let package = load_dev_package(source.path()).unwrap();
            assert!(package.source.contains("From TSX"));
            assert!(!package.source.contains(": string"));
            assert!(!source.path().join("main.js").exists());
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(super) fn run(directories: Vec<std::path::PathBuf>) -> Result<(), String> {
    platform::run(directories)
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(super) fn run(_directories: Vec<std::path::PathBuf>) -> Result<(), String> {
    Err("nickel-plugin dev currently requires Linux or Windows".into())
}
