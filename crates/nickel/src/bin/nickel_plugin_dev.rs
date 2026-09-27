//! Isolated edit loop for one external panel package.

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

    use nickel_core::plugins::{
        PluginActivationSettings, PluginManifest, PluginPackage, PluginSurfaceKind,
    };
    use nickel_shell::plugin_panel::PluginPanelApplication;

    fn load_panel(directory: &Path) -> Result<PluginPackage, String> {
        let package = PluginPackage::load(directory)?;
        if package.manifest.surfaces.len() != 1
            || package.manifest.surfaces[0].kind != PluginSurfaceKind::Panel
        {
            return Err("dev currently needs exactly one panel surface".into());
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
        std::fs::read(directory.join(current_entry))
            .unwrap_or_default()
            .hash(&mut hasher);
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
            "Testing {} in nested Nickel. Save plugin.json or {} to reload; Ctrl+C stops.",
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
