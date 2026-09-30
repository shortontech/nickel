//! Compile-time source and stylesheet assets for Nickel's shipped plugins.
//! The JSX host itself only receives a manifest, entry, and data.

pub(crate) fn resolve(
    plugin_id: &str,
    entry: &str,
) -> Result<(&'static str, &'static str), String> {
    let assets = match (plugin_id, entry) {
        ("org.nickel.taskbar", "main.js") => (
            include_str!("../../../assets/plugins/taskbar/main.js"),
            include_str!("../../../assets/plugins/taskbar/ui.css"),
        ),
        ("org.nickel.taskbar", "menu.js") => (
            include_str!("../../../assets/plugins/taskbar/menu.js"),
            include_str!("../../../assets/plugins/taskbar/ui.css"),
        ),
        ("org.nickel.taskbar", "window-menu.js") => (
            include_str!("../../../assets/plugins/taskbar/window-menu.js"),
            include_str!("../../../assets/plugins/taskbar/ui.css"),
        ),
        ("org.nickel.notification", "main.js") => (
            include_str!("../../../assets/plugins/notification/main.js"),
            include_str!("../../../assets/plugins/notification/ui.css"),
        ),
        ("org.nickel.volume-osd", "main.js") => (
            include_str!("../../../assets/plugins/volume-osd/main.js"),
            include_str!("../../../assets/plugins/volume-osd/ui.css"),
        ),
        ("org.nickel.control-center", "main.js") => (
            include_str!("../../../assets/plugins/control-center/main.js"),
            include_str!("../../../assets/plugins/control-center/ui.css"),
        ),
        ("org.nickel.codex-projects", "main.js") => (
            include_str!("../../../assets/plugins/codex-projects/main.js"),
            include_str!("../../../assets/plugins/codex-projects/ui.css"),
        ),
        ("org.nickel.on-screen-keyboard", "main.js") => (
            include_str!("../../../assets/plugins/on-screen-keyboard/main.js"),
            include_str!("../../../assets/plugins/on-screen-keyboard/ui.css"),
        ),
        ("org.nickel.window-preview", "main.js") => (
            include_str!("../../../assets/plugins/window-preview/main.js"),
            include_str!("../../../assets/plugins/window-preview/ui.css"),
        ),
        ("org.nickel.run", "main.js") => (
            include_str!("../../../assets/plugins/run/main.js"),
            include_str!("../../../assets/plugins/run/ui.css"),
        ),
        _ => {
            return Err(format!(
                "bundled plugin {plugin_id:?} entry {entry:?} is unavailable"
            ));
        }
    };
    Ok(assets)
}

include!(concat!(env!("OUT_DIR"), "/bundled_packages.rs"));

/// Load an ordinary shipped package from the compile-time catalog. No disk,
/// JSX compiler, or package manager is needed by the running shell.
pub fn load_package(id: &str) -> Result<nickel_core::plugins::PluginPackage, String> {
    for files in BUNDLED_PACKAGES {
        let manifest = files
            .iter()
            .find(|(path, _)| *path == "plugin.json")
            .ok_or("bundled package manifest is missing")?;
        let manifest = nickel_core::plugins::PluginManifest::from_json(
            std::str::from_utf8(manifest.1).map_err(|error| error.to_string())?,
        )?;
        if manifest.id == id {
            return nickel_core::plugins::PluginPackage::from_embedded(files);
        }
    }
    Err(format!("bundled package {id:?} is unavailable"))
}
