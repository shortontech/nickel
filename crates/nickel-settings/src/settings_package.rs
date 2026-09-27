//! Embedded first-party Settings plugin package and its page sources.

use std::sync::OnceLock;

use nickel_core::plugins::{PluginManifest, PluginSurfaceKind};
use nickel_session_protocol::{
    PluginMemorySnapshot, PluginRuntimeHealth, PluginStatus, PluginStatusSnapshot,
};

pub(super) const ID: &str = "org.nickel.settings";
const MANIFEST_SOURCE: &str = include_str!("../../../assets/plugins/settings/plugin.json");
static MANIFEST: OnceLock<Result<PluginManifest, String>> = OnceLock::new();

#[derive(Clone, Copy)]
pub(super) enum Script {
    Navigation,
    Plugins,
    OrdinaryPages,
    Bar,
    OptionalFeatures,
    Network,
}

pub(super) fn manifest() -> Result<&'static PluginManifest, String> {
    MANIFEST
        .get_or_init(|| {
            let manifest = PluginManifest::from_json(MANIFEST_SOURCE)?;
            if manifest.id != ID
                || manifest.entry != "settings-navigation.js"
                || manifest.surfaces.len() != 1
                || manifest.surfaces[0].id != "main"
                || manifest.surfaces[0].kind != PluginSurfaceKind::Window
            {
                return Err("bundled Settings package has an invalid host contract".into());
            }
            Ok(manifest)
        })
        .as_ref()
        .map_err(Clone::clone)
}

pub(super) fn status(enabled: bool) -> Result<PluginStatus, String> {
    let manifest = manifest()?;
    Ok(PluginStatus {
        id: manifest.id.clone(),
        name: manifest.name.clone(),
        author: manifest.author.clone(),
        version: manifest.version.clone(),
        desired_enabled: enabled,
        health: if enabled {
            PluginRuntimeHealth::Running
        } else {
            PluginRuntimeHealth::Disabled
        },
        capabilities: manifest
            .capabilities
            .iter()
            .map(|grant| grant.as_str().into())
            .collect(),
        surfaces: manifest
            .surfaces
            .iter()
            .map(|surface| format!("{}: {}", surface.id, surface.kind.as_str()))
            .collect(),
        composition: Vec::new(),
        settings: Vec::new(),
        memory: PluginMemorySnapshot::default(),
    })
}

pub(super) fn local_snapshot(enabled: bool) -> Result<PluginStatusSnapshot, String> {
    Ok(PluginStatusSnapshot {
        activation_generation: 0,
        plugins: vec![status(enabled)?],
    })
}

pub(super) fn source(script: Script) -> Result<&'static str, String> {
    manifest()?;
    Ok(match script {
        Script::Navigation => {
            include_str!("../../../assets/plugins/settings/settings-navigation.js")
        }
        Script::Plugins => include_str!("../../../assets/plugins/settings/settings-plugins.js"),
        Script::OrdinaryPages => {
            include_str!("../../../assets/plugins/settings/settings-pages.js")
        }
        Script::Bar => include_str!("../../../assets/plugins/settings/settings-bar.js"),
        Script::OptionalFeatures => {
            include_str!("../../../assets/plugins/settings/settings-optional-features.js")
        }
        Script::Network => include_str!("../../../assets/plugins/settings/settings-network.js"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::plugins::PluginPackage;
    use nickel_plugin_runtime::JsxRuntime;

    #[test]
    fn bundled_settings_package_loads_and_each_page_script_evaluates() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/plugins/settings");
        let package = PluginPackage::load(path).unwrap();
        assert_eq!(package.manifest, *manifest().unwrap());
        assert_eq!(package.source, source(Script::Navigation).unwrap());
        for script in [
            Script::Navigation,
            Script::Plugins,
            Script::OrdinaryPages,
            Script::Bar,
            Script::OptionalFeatures,
            Script::Network,
        ] {
            JsxRuntime::new(source(script).unwrap(), None).unwrap();
        }
    }
}
