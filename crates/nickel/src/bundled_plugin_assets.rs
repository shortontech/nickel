//! Compile-time catalog of ordinary shipped plugin packages.

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

/// Authoring catalog; caller packages override these installed or shipped entries.
pub fn validation_catalog()
-> Result<std::collections::BTreeMap<String, nickel_core::plugins::PluginPackage>, String> {
    let mut catalog = std::collections::BTreeMap::new();
    for files in BUNDLED_PACKAGES {
        let package = nickel_core::plugins::PluginPackage::from_embedded(files)?;
        catalog.insert(package.manifest.id.clone(), package);
    }
    for (id, descriptor) in nickel_core::plugins::PluginCatalog::discover_default()?.packages {
        catalog.insert(id, descriptor.load()?);
    }
    Ok(catalog)
}
