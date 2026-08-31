#![cfg(feature = "workbench-fixtures")]

use nickel_shell::ShellFixtureProvider;
use nickel_ui_testkit::{FixtureProvider, FixtureRegistry};

#[test]
fn registers_every_shell_surface_fixture() {
    let mut registry = FixtureRegistry::new();
    ShellFixtureProvider.register(&mut registry).unwrap();
    let entries = registry.finish();
    let ids = entries
        .iter()
        .map(|entry| entry.metadata.id)
        .collect::<Vec<_>>();
    assert_eq!(
        ids,
        [
            "shell.codex-project-menu",
            "shell.control-center",
            "shell.desktop",
            "shell.launcher-search",
            "shell.lock",
            "shell.notification",
            "shell.panel",
            "shell.runtime",
            "shell.screenshot",
            "shell.window-preview",
        ]
    );
    for entry in entries {
        for variant in entry.metadata.variants {
            let session = entry.open_configuration(*variant);
            let raster = session.render(variant.scale.factor);
            let repeated = session.render(variant.scale.factor);
            assert_eq!(
                raster, repeated,
                "{} / {} rendered nondeterministically",
                entry.metadata.id, variant.id
            );
            assert_eq!(
                raster.rgba.len(),
                (raster.width * raster.height * 4) as usize
            );
            assert!(
                !session.semantic_nodes().is_empty() || !session.accessibility_nodes().is_empty(),
                "{} / {} emitted no semantic or accessibility nodes",
                entry.metadata.id,
                variant.id
            );
            if entry.metadata.id == "shell.control-center" {
                assert!(session.accessibility_nodes().iter().any(|node| {
                    node.id.as_str().ends_with("audio-volume")
                        && node.label.as_deref() == Some("Audio volume")
                }));
            }
            if entry.metadata.id == "shell.launcher-search" {
                assert!(session.accessibility_nodes().iter().any(|node| {
                    (node.id.as_str().ends_with("launcher-search-focus")
                        || node
                            .id
                            .as_str()
                            .ends_with("launcher-dashboard-search-focus"))
                        && node.label.as_deref() == Some("Focus application search")
                }));
            }
            if entry.metadata.id == "shell.notification" {
                let has_activate = session
                    .semantic_nodes()
                    .iter()
                    .any(|node| node.actions.contains(&nickel_ui::ActionKind::Activate));
                assert_eq!(
                    has_activate,
                    variant.id != "no-actions",
                    "notification action reachability drifted for {}",
                    variant.id
                );
            }
            if entry.metadata.id == "shell.screenshot" {
                let has_activate = session
                    .semantic_nodes()
                    .iter()
                    .any(|node| node.actions.contains(&nickel_ui::ActionKind::Activate));
                assert_eq!(
                    has_activate,
                    variant.id == "confirmed",
                    "screenshot action reachability drifted for {}",
                    variant.id
                );
            }
            if entry.metadata.id == "shell.window-preview" && variant.id != "empty" {
                assert!(session.accessibility_nodes().iter().any(|node| {
                    node.interactive && node.label.as_deref() == Some("Workbench window 1")
                }));
            }
        }
    }
}
