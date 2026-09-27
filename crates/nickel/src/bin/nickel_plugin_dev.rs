//! Isolated edit loop for an external panel or taskbar badge package.

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
    if !resolved.starts_with(directory) {
        return Err("JSX source escapes its plugin directory".into());
    }
    Ok(Some(source.clone()))
}

fn compile_jsx(
    directory: &Path,
    manifest: &PluginManifest,
    source: &Path,
) -> Result<String, String> {
    let output = tempfile::tempdir()
        .map_err(|error| format!("could not create JSX build directory: {error}"))?;
    let compiler = directory.join("node_modules/.bin/tsc");
    let compiler = if compiler.is_file() {
        compiler.into_os_string()
    } else {
        "tsc".into()
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
        .arg(directory)
        .arg("--outDir")
        .arg(output.path())
        .arg(source)
        .status()
        .map_err(|error| format!("could not run TypeScript compiler (tsc): {error}"))?;
    if !status.success() {
        return Err(format!("JSX compilation failed with {status}"));
    }
    let compiled = output.path().join(&manifest.entry);
    let metadata = std::fs::symlink_metadata(&compiled)
        .map_err(|error| format!("compiler did not produce {}: {error}", manifest.entry))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > MAX_PLUGIN_ENTRY_BYTES as u64
    {
        return Err("compiled JavaScript must be an ordinary file of at most 2 MiB".into());
    }
    std::fs::read_to_string(&compiled)
        .map_err(|error| format!("could not read compiled JavaScript: {error}"))
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
            source: compile_jsx(&directory, &manifest, &source)?,
            manifest,
        })
    } else {
        PluginPackage::load(&directory)
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use std::{
        collections::hash_map::DefaultHasher,
        hash::{Hash, Hasher},
        path::{Path, PathBuf},
        process::{Child, Command},
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        time::Duration,
    };

    use super::load_package;
    use nickel_core::plugins::{
        PluginActivationSettings, PluginContributionMode, PluginManifest, PluginPackage,
        PluginSlotContract, PluginSurfaceKind,
    };
    use nickel_shell::plugin_panel::PluginPanelApplication;

    fn load_panel(directory: &Path) -> Result<PluginPackage, String> {
        let package = load_package(directory)?;
        let panel = package.manifest.surfaces.len() == 1
            && package.manifest.surfaces[0].kind == PluginSurfaceKind::Panel
            && package.manifest.contributes.is_empty();
        let badge = package.manifest.surfaces.is_empty()
            && matches!(package.manifest.contributes.as_slice(), [contribution]
                if contribution.target_plugin == "org.nickel.taskbar"
                    && contribution.target_slot == "task-badge"
                    && contribution.contract == PluginSlotContract::Badge
                    && matches!(contribution.mode, PluginContributionMode::Add | PluginContributionMode::Replace));
        if !panel && !badge {
            return Err(
                "dev currently needs one panel or one surface-free badge contribution".into(),
            );
        }
        PluginPanelApplication::from_package(&package)
            .map_err(|error| format!("plugin JavaScript failed: {error}"))?;
        Ok(package)
    }

    fn source_fingerprint(directory: &Path, entry: &str) -> Result<u64, String> {
        let mut hasher = DefaultHasher::new();
        let manifest = std::fs::read(directory.join("plugin.json"))
            .map_err(|error| format!("could not read plugin.json: {error}"))?;
        manifest.hash(&mut hasher);
        let current_entry = std::str::from_utf8(&manifest)
            .ok()
            .and_then(|source| PluginManifest::from_json(source).ok())
            .map(|manifest| manifest.entry)
            .unwrap_or_else(|| entry.to_owned());
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
        Ok(hasher.finish())
    }

    fn stage(package: &PluginPackage, directory: &Path, root: &Path) -> Result<(), String> {
        let manifest_bytes = std::fs::read(directory.join("plugin.json"))
            .map_err(|error| format!("could not read plugin manifest: {error}"))?;
        let manifest_source = std::str::from_utf8(&manifest_bytes)
            .map_err(|error| format!("plugin manifest is not UTF-8: {error}"))?;
        if PluginManifest::from_json(manifest_source)? != package.manifest {
            return Err("plugin manifest changed during staging".into());
        }
        let target = root
            .join("nickel")
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
        PluginActivationSettings::update(
            root.join("nickel").join("plugin-activation.json"),
            &package.manifest.id,
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

    fn launch(binary: &Path, config_root: &Path) -> Result<Child, String> {
        Command::new(binary)
            .env("XDG_CONFIG_HOME", config_root)
            .spawn()
            .map_err(|error| format!("could not start nested Nickel: {error}"))
    }

    pub(super) fn run(directory: PathBuf) -> Result<(), String> {
        let directory = std::fs::canonicalize(directory)
            .map_err(|error| format!("could not open plugin directory: {error}"))?;
        let mut package = load_panel(&directory)?;
        let nested = std::env::current_exe()
            .map_err(|error| format!("could not locate nickel-plugin: {error}"))?
            .with_file_name("nickel-nested");
        if !nested.is_file() {
            return Err(format!(
                "{} is missing; build it with cargo build -p nickel --bin nickel-nested --features backend-winit",
                nested.display()
            ));
        }
        let config = tempfile::tempdir()
            .map_err(|error| format!("could not create isolated plugin profile: {error}"))?;
        stage(&package, &directory, config.path())?;
        let mut current_fingerprint = source_fingerprint(&directory, &package.manifest.entry)?;
        let running = Arc::new(AtomicBool::new(true));
        let signal = running.clone();
        ctrlc::set_handler(move || signal.store(false, Ordering::SeqCst))
            .map_err(|error| format!("could not install interrupt handler: {error}"))?;
        println!(
            "Testing {} in nested Nickel. Save plugin.json, JSX source, or {} to reload; Ctrl+C stops.",
            package.manifest.id, package.manifest.entry
        );
        let mut child = launch(&nested, config.path())?;
        let mut child_exited = false;
        while running.load(Ordering::SeqCst) {
            if !child_exited {
                match child.try_wait() {
                    Ok(Some(status)) => {
                        eprintln!("nested Nickel exited: {status}; waiting for a plugin edit");
                        child_exited = true;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        stop(&mut child);
                        return Err(format!("could not inspect nested Nickel: {error}"));
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(500));
            let observed = match source_fingerprint(&directory, &package.manifest.entry) {
                Ok(observed) => observed,
                Err(error) => {
                    eprintln!("plugin edit: {error}");
                    continue;
                }
            };
            if observed == current_fingerprint {
                continue;
            }
            current_fingerprint = observed;
            let next = match load_panel(&directory) {
                Ok(next) => next,
                Err(error) => {
                    eprintln!("plugin edit: {error}");
                    continue;
                }
            };
            if next.manifest.id != package.manifest.id {
                eprintln!("plugin ID changed; restart nickel-plugin dev for a new identity");
                continue;
            }
            stop(&mut child);
            stage(&next, &directory, config.path())?;
            package = next;
            current_fingerprint = source_fingerprint(&directory, &package.manifest.entry)?;
            child = launch(&nested, config.path())?;
            child_exited = false;
            println!("Reloaded {}", package.manifest.id);
        }
        stop(&mut child);
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

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
            let package = load_panel(source.path()).unwrap();
            let profile = tempfile::tempdir().unwrap();
            stage(&package, source.path(), profile.path()).unwrap();
            let staged =
                PluginPackage::load(profile.path().join("nickel/plugins/org.example.clock"))
                    .unwrap();
            assert_eq!(staged.source, package.source);
            let activation = PluginActivationSettings::load(
                profile.path().join("nickel/plugin-activation.json"),
            )
            .unwrap();
            assert!(activation.desired_enabled("org.example.clock", false));
        }

        #[test]
        fn compiles_jsx_without_overwriting_the_entry_and_rejects_invalid_edits() {
            if Command::new("tsc").arg("--version").output().is_err() {
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
            let package = load_panel(source.path()).unwrap();
            assert!(package.source.contains("h(Panel"));
            assert!(!source.path().join("main.js").exists());
            let previous = source_fingerprint(source.path(), "main.js").unwrap();
            std::fs::write(&jsx, "function App() { return <Panel>").unwrap();
            assert_ne!(
                source_fingerprint(source.path(), "main.js").unwrap(),
                previous
            );
            assert!(load_panel(source.path()).is_err());
            assert!(!source.path().join("main.js").exists());
        }

        #[test]
        fn compiles_tsx_with_type_annotations() {
            if Command::new("tsc").arg("--version").output().is_err() {
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
            let package = load_panel(source.path()).unwrap();
            assert!(package.source.contains("From TSX"));
            assert!(!package.source.contains(": string"));
            assert!(!source.path().join("main.js").exists());
        }
    }
}

#[cfg(target_os = "linux")]
pub(super) fn run(directory: std::path::PathBuf) -> Result<(), String> {
    linux::run(directory)
}

#[cfg(not(target_os = "linux"))]
pub(super) fn run(_directory: std::path::PathBuf) -> Result<(), String> {
    Err("nickel-plugin dev currently requires Linux nested Nickel".into())
}
