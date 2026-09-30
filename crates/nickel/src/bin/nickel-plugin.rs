//! Small development tool for inspecting and validating JavaScript packages.

use std::{ffi::OsString, path::PathBuf};

use nickel_core::plugins::PluginCatalog;
use nickel_shell::plugin_panel::PluginPanelApplication;

fn usage() -> &'static str {
    "usage: nickel-plugin list [plugin-root] | validate <plugin-directory> | dev <plugin-directory> [more-plugin-directories...]"
}

fn main() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    return std::thread::Builder::new()
        .name("nickel-plugin-command".into())
        .stack_size(8 * 1024 * 1024)
        .spawn(run_command)
        .map_err(|error| format!("could not start plugin command: {error}"))?
        .join()
        .map_err(|_| "plugin command panicked".to_owned())?;

    #[cfg(not(target_os = "windows"))]
    run_command()
}

fn run_command() -> Result<(), String> {
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
            if package.manifest.claims_native_shell_surface() {
                return Err("desktop and screenshot presentation are native Rust UI".into());
            }
            PluginPanelApplication::validate_package(&package)
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
                if !surface.anchor.is_center() || surface.offset_x != 0 || surface.offset_y != 0 {
                    println!(
                        "  placement: {} ({:+}, {:+})",
                        surface.anchor.as_str(),
                        surface.offset_x,
                        surface.offset_y
                    );
                }
                if surface.passive {
                    println!("  passive overlay: opens without activating the window");
                }
            }
            for capability in &package.manifest.capabilities {
                println!("access: {}", capability.as_str());
            }
            if let Some(composition) = &package.manifest.composition {
                for contribution in &composition.contributions {
                    println!(
                        "contributes: {} to {}",
                        contribution.id, contribution.collection
                    );
                }
            }
            Ok(())
        }
        Some(command) if command == "dev" => {
            let directories = args.map(PathBuf::from).collect::<Vec<_>>();
            if directories.is_empty() {
                return Err(usage().into());
            }
            dev::run(directories)
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
