//! Small development tool for inspecting and validating JavaScript packages.

use std::{ffi::OsString, path::PathBuf};

use nickel_core::plugins::PluginCatalog;
use nickel_shell::plugin_panel::PluginPanelApplication;

fn usage() -> &'static str {
    "usage: nickel-plugin list [plugin-root] | validate <plugin-directory> | dev <plugin-directory>"
}

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    match args.next().as_deref() {
        Some(command) if command == "list" => {
            let root = args.next();
            if args.next().is_some() {
                return Err(usage().into());
            }
            let catalog = match root {
                Some(root) => PluginCatalog::discover(PathBuf::from(root))?,
                None => PluginCatalog::discover_default()?,
            };
            for (id, descriptor) in &catalog.packages {
                println!("{id}\t{}", descriptor.manifest.name);
            }
            for failure in &catalog.failures {
                eprintln!("{}: {}", failure.directory, failure.reason);
            }
            if catalog.failures.is_empty() {
                Ok(())
            } else {
                Err(format!(
                    "{} plugin package(s) failed validation",
                    catalog.failures.len()
                ))
            }
        }
        Some(command) if command == "validate" => {
            let directory: OsString = args.next().ok_or_else(|| usage().to_owned())?;
            if args.next().is_some() {
                return Err(usage().into());
            }
            let package = dev::load_package(&PathBuf::from(directory))?;
            PluginPanelApplication::from_package(&package)
                .map_err(|error| format!("plugin JavaScript failed: {error}"))?;
            println!("{} ({})", package.manifest.name, package.manifest.id);
            for surface in &package.manifest.surfaces {
                println!(
                    "surface {}: {} {}x{}",
                    surface.id,
                    surface.kind.as_str(),
                    surface.width,
                    surface.height
                );
            }
            for capability in &package.manifest.capabilities {
                println!("access: {}", capability.as_str());
            }
            for slot in &package.manifest.provides_slots {
                println!("provides: {} ({})", slot.id, slot.contract.as_str());
            }
            for contribution in &package.manifest.contributes {
                println!(
                    "contributes: {} {}/{} ({})",
                    contribution.mode.as_str(),
                    contribution.target_plugin,
                    contribution.target_slot,
                    contribution.contract.as_str()
                );
            }
            Ok(())
        }
        Some(command) if command == "dev" => {
            let directory: OsString = args.next().ok_or_else(|| usage().to_owned())?;
            if args.next().is_some() {
                return Err(usage().into());
            }
            dev::run(PathBuf::from(directory))
        }
        Some(command) if command == "--help" || command == "-h" => {
            println!("{}", usage());
            Ok(())
        }
        _ => Err(usage().into()),
    }
}

#[path = "nickel_plugin_dev.rs"]
mod dev;
