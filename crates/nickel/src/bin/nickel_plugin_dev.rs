//! Isolated edit loop for an external panel or executable extension package.

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
    let output = tempfile::tempdir()
        .map_err(|error| format!("could not create JSX build directory: {error}"))?;
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
            "--checkJs",
            "false",
            "--noCheck",
            "--noEmitOnError",
            "--jsx",
            "react",
            "--jsxFactory",
            "h",
            "--target",
            "ES2020",
            "--lib",
            "ES2020",
            "--module",
            "none",
            "--rootDir",
        ])
        .arg(".")
        .arg("--outDir")
        .arg(output.path())
        .arg(source)
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
    std::fs::read_to_string(&compiled)
        .map_err(|error| format!("could not read compiled JavaScript: {error}"))
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
        Ok(PluginPackage {
            source: compile_jsx(&directory, &manifest.entry, &source)?,
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
        MAX_PLUGIN_ENTRY_BYTES, PluginActivationSettings, PluginContributionMode, PluginManifest,
        PluginPackage, PluginSlotContract, PluginSurfaceKind,
    };
    use nickel_shell::plugin_panel::{
        PluginPanelApplication, codex_projects_manifest, control_center_manifest,
        launcher_manifest, manifest, notification_manifest, on_screen_keyboard_manifest,
        run_manifest, taskbar_manifest, volume_osd_manifest, window_preview_manifest,
    };

    fn bundled_manifest(id: &str) -> Option<&'static PluginManifest> {
        [
            manifest(),
            taskbar_manifest(),
            launcher_manifest(),
            notification_manifest(),
            run_manifest(),
            control_center_manifest(),
            codex_projects_manifest(),
            on_screen_keyboard_manifest(),
            window_preview_manifest(),
            volume_osd_manifest(),
        ]
        .into_iter()
        .find(|manifest| manifest.id == id)
    }

    fn staged_config_directory(root: &Path) -> PathBuf {
        #[cfg(target_os = "linux")]
        let directory = "nickel";
        #[cfg(target_os = "windows")]
        let directory = "Nickel";
        root.join(directory)
    }

    fn load_dev_package(directory: &Path) -> Result<PluginPackage, String> {
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
            })
            && package.manifest.contributes.is_empty();
        let extension = package.manifest.surfaces.is_empty()
            && matches!(package.manifest.contributes.as_slice(), [contribution]
                if matches!((contribution.target_plugin.as_str(), contribution.target_slot.as_str(), contribution.contract),
                    ("org.nickel.taskbar", "task-badge", PluginSlotContract::Badge)
                    | ("org.nickel.taskbar", "task-action", PluginSlotContract::Action)
                    | ("org.nickel.control-center", "control-section", PluginSlotContract::Section)
                    | (_, _, PluginSlotContract::Widget)
                    | (_, _, PluginSlotContract::Action))
                    && matches!(contribution.mode, PluginContributionMode::Add | PluginContributionMode::Replace));
        if !bundled && !panel && !extension {
            return Err(
                "dev needs an unchanged bundled manifest, a panel, dock, or window that may declare dialogs, or one supported surface-free contribution"
                    .into(),
            );
        }
        PluginPanelApplication::validate_package(&package)
            .map_err(|error| format!("plugin JavaScript failed: {error}"))?;
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
            .map(|directory| load_dev_package(directory))
            .collect::<Result<Vec<_>, _>>()?;
        let ids = packages
            .iter()
            .map(|package| package.manifest.id.as_str())
            .collect::<HashSet<_>>();
        if ids.len() != packages.len() {
            return Err("dev package IDs must be distinct".into());
        }
        for package in &packages {
            for contribution in &package.manifest.contributes {
                if bundled_manifest(&contribution.target_plugin).is_none()
                    && !ids.contains(contribution.target_plugin.as_str())
                {
                    return Err(format!(
                        "dev needs the target package directory for {}",
                        contribution.target_plugin
                    ));
                }
            }
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
        fn accepts_each_executable_surface_free_extension() {
            let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins"));
            for name in [
                "example-task-badge",
                "example-task-action",
                "example-control-section",
            ] {
                let package = load_dev_package(&root.join(name)).unwrap();
                assert!(package.manifest.surfaces.is_empty());
                assert_eq!(package.manifest.contributes.len(), 1);
            }
        }

        #[test]
        fn codex_project_menu_uses_the_bundled_dev_path() {
            let root = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/codex-projects"
            ));
            let package = load_dev_package(root).unwrap();
            assert_eq!(package.manifest.id, "org.nickel.codex-projects");
            assert_eq!(package.manifest.surfaces.len(), 1);
        }

        #[test]
        fn keyboard_uses_the_bundled_dev_path() {
            let root = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/on-screen-keyboard"
            ));
            let package = load_dev_package(root).unwrap();
            assert_eq!(package.manifest.id, "org.nickel.on-screen-keyboard");
            assert_eq!(package.manifest.surfaces.len(), 1);
        }

        #[test]
        fn stages_a_provider_and_multiple_contributors_together() {
            let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins"));
            let host = root.join("example-widget-host");
            let contributor = root.join("example-widget-contributor");
            let action = root.join("example-action-contributor");
            assert!(
                load_dev_packages(&[contributor.clone()])
                    .unwrap_err()
                    .contains("target package directory")
            );
            assert!(
                load_dev_packages(&[action.clone()])
                    .unwrap_err()
                    .contains("target package directory")
            );
            let directories = vec![host, contributor, action];
            let packages = load_dev_packages(&directories).unwrap();
            let profile = tempfile::tempdir().unwrap();
            stage_all(&packages, &directories, profile.path()).unwrap();
            for package in packages {
                let staged = PluginPackage::load(
                    staged_config_directory(profile.path())
                        .join("plugins")
                        .join(&package.manifest.id),
                )
                .unwrap();
                let activation = PluginActivationSettings::load(
                    staged_config_directory(profile.path()).join("plugin-activation.json"),
                )
                .unwrap();
                assert!(activation.approval_current(&staged.manifest, &staged.source_digest()));
                assert!(activation.desired_enabled(&staged.manifest.id, false));
            }
        }

        #[test]
        fn stages_bundled_shell_sources_without_installing_duplicate_packages() {
            let root = Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins"));
            for name in ["launcher", "taskbar"] {
                let directory = root.join(name);
                let package = PluginPackage::load(&directory).unwrap();
                let profile = tempfile::tempdir().unwrap();
                stage(&package, &directory, profile.path()).unwrap();
                let staged = profile
                    .path()
                    .join("bundled-source")
                    .join(&package.manifest.id);
                assert_eq!(
                    std::fs::read_to_string(staged.join(&package.manifest.entry)).unwrap(),
                    package.source
                );
                assert!(
                    !staged_config_directory(profile.path())
                        .join("plugins")
                        .join(&package.manifest.id)
                        .exists()
                );
                if name == "taskbar" {
                    assert!(staged.join("menu.js").is_file());
                    assert!(staged.join("window-menu.js").is_file());
                }
            }
        }

        #[test]
        fn stages_and_watches_bundled_auxiliary_jsx() {
            if Command::new(tsc_executable())
                .arg("--version")
                .output()
                .is_err()
            {
                return;
            }
            let source = tempfile::tempdir().unwrap();
            let taskbar = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/taskbar"
            ));
            for name in ["plugin.json", "main.js", "ui.css"] {
                std::fs::copy(taskbar.join(name), source.path().join(name)).unwrap();
            }
            std::fs::write(source.path().join("menu.js"), "stale menu").unwrap();
            std::fs::write(
                source.path().join("menu.jsx"),
                "function App() { return <FixedWindow width=\"100%\" height=\"100%\"><Text>Fresh menu</Text></FixedWindow>; }",
            )
            .unwrap();
            let package = load_dev_package(source.path()).unwrap();
            let profile = tempfile::tempdir().unwrap();
            stage(&package, source.path(), profile.path()).unwrap();
            let staged = profile
                .path()
                .join("bundled-source/org.nickel.taskbar/menu.js");
            let compiled = std::fs::read_to_string(staged).unwrap();
            assert!(compiled.contains("Fresh menu"));
            assert!(!compiled.contains("stale menu"));
            let before = source_fingerprint(source.path(), "main.js").unwrap();
            std::fs::write(
                source.path().join("menu.jsx"),
                "function App() { return <FixedWindow width=\"100%\" height=\"100%\"><Text>Updated menu</Text></FixedWindow>; }",
            )
            .unwrap();
            assert_ne!(
                source_fingerprint(source.path(), "main.js").unwrap(),
                before
            );
        }

        #[test]
        fn reload_fingerprint_tracks_bundled_sibling_javascript() {
            let directory = tempfile::tempdir().unwrap();
            let root = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/taskbar"
            ));
            std::fs::copy(
                root.join("plugin.json"),
                directory.path().join("plugin.json"),
            )
            .unwrap();
            std::fs::copy(root.join("main.js"), directory.path().join("main.js")).unwrap();
            std::fs::write(directory.path().join("menu.js"), "first").unwrap();
            let first = source_fingerprint(directory.path(), "main.js").unwrap();
            std::fs::write(directory.path().join("menu.js"), "second").unwrap();
            assert_ne!(
                first,
                source_fingerprint(directory.path(), "main.js").unwrap()
            );
        }

        #[test]
        fn bundled_dev_requires_the_shipped_manifest() {
            let directory = tempfile::tempdir().unwrap();
            let root = Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/launcher"
            ));
            let manifest = std::fs::read_to_string(root.join("plugin.json")).unwrap();
            std::fs::write(
                directory.path().join("plugin.json"),
                manifest.replace("Nickel Launcher", "Renamed Launcher"),
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
