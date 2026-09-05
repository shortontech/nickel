#![cfg(feature = "workbench-fixtures")]

use nickel_shell::ShellFixtureProvider;
use nickel_ui::{ActionKind, SemanticRole, Size};
use nickel_ui_testkit::{ActivationVia, FixtureProvider, FixtureRegistry};

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
            "shell.launcher-dashboard",
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
                    node.id.as_str().ends_with("launcher-search-focus")
                        && node.label.as_deref() == Some("Focus application search")
                }));
            }
            if entry.metadata.id == "shell.launcher-dashboard" {
                assert!(session.accessibility_nodes().iter().any(|node| {
                    node.id.as_str().ends_with("launcher-search-focus")
                        && node.label.as_deref() == Some("Focus application search")
                }));
            }
            if entry.metadata.id == "shell.notification" {
                let has_activate = session
                    .semantic_nodes()
                    .iter()
                    .any(|node| node.actions.contains(&nickel_ui::ActionKind::Activate));
                assert!(
                    has_activate,
                    "notification action reachability drifted for {}",
                    variant.id
                );
                assert!(session.accessibility_nodes().iter().any(|node| {
                    node.id.as_str().ends_with("notification-dismiss")
                        && node.label.as_deref() == Some("Dismiss")
                }));
            }
            if entry.metadata.id == "shell.panel" && variant.id == "status-items" {
                for label in [
                    "Codex projects",
                    "Fixture notification icon",
                    "Fixture Browser",
                    "Fixture Editor",
                ] {
                    assert!(
                        session
                            .accessibility_nodes()
                            .iter()
                            .any(|node| node.label.as_deref() == Some(label)),
                        "populated panel fixture omitted {label}"
                    );
                }
                let semantic_nodes = session.semantic_nodes();
                let tray = semantic_nodes
                    .iter()
                    .find(|node| node.name.as_deref() == Some("Fixture notification icon"))
                    .expect("populated panel fixture tray target");
                assert!(tray.actions.contains(&nickel_ui::ActionKind::Activate));
                assert!(tray.actions.contains(&nickel_ui::ActionKind::ContextMenu));
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

#[test]
fn desktop_variants_expose_named_context_interactive_presentation() {
    let mut registry = FixtureRegistry::new();
    ShellFixtureProvider.register(&mut registry).unwrap();
    let entry = registry
        .finish()
        .into_iter()
        .find(|entry| entry.metadata.id == "shell.desktop")
        .expect("desktop fixture");

    assert_eq!(
        entry
            .metadata
            .variants
            .iter()
            .map(|variant| variant.id)
            .collect::<Vec<_>>(),
        ["solid", "wallpaper"]
    );

    for variant in entry.metadata.variants {
        let session = entry.open_configuration(*variant);
        let semantic = session.semantic_nodes();
        assert_eq!(semantic.len(), 1, "{} semantic nodes", variant.id);
        assert_eq!(
            semantic[0].role,
            Some(SemanticRole::ApplicationPresentation)
        );
        assert_eq!(semantic[0].name.as_deref(), Some("Desktop"));
        assert_eq!(semantic[0].bounds.size, Size::new(960.0, 540.0));
        assert_eq!(semantic[0].actions, vec![ActionKind::ContextMenu]);

        let accessibility = session.accessibility_nodes();
        let accessibility = accessibility
            .iter()
            .filter(|node| node.semantic_role.is_some())
            .collect::<Vec<_>>();
        assert_eq!(accessibility.len(), 1, "{} accessibility nodes", variant.id);
        assert_eq!(
            accessibility[0].semantic_role,
            Some(SemanticRole::ApplicationPresentation)
        );
        assert_eq!(accessibility[0].role.as_deref(), Some("application"));
        assert_eq!(accessibility[0].label.as_deref(), Some("Desktop"));
        assert_eq!(accessibility[0].rect.size, Size::new(960.0, 540.0));
        assert!(accessibility[0].interactive);
        assert_eq!(accessibility[0].actions, vec![ActionKind::ContextMenu]);

        for via in [
            ActivationVia::Semantic,
            ActivationVia::Pointer,
            ActivationVia::Touch,
            ActivationVia::Keyboard,
            ActivationVia::Controller,
            ActivationVia::Accessibility,
        ] {
            let mut session = entry.open_configuration(*variant);
            session
                .activate(via)
                .unwrap_or_else(|error| panic!("{} {via:?}: {error}", variant.id));
        }
    }
}

#[test]
fn launcher_dashboard_matrix_covers_every_required_axis() {
    let mut registry = FixtureRegistry::new();
    ShellFixtureProvider.register(&mut registry).unwrap();
    let entry = registry
        .finish()
        .into_iter()
        .find(|entry| entry.metadata.id == "shell.launcher-dashboard")
        .unwrap();
    let ids = entry
        .metadata
        .variants
        .iter()
        .map(|variant| variant.id)
        .collect::<Vec<_>>()
        .join("\n");
    for required in [
        "populated",
        "empty",
        "loading",
        "partial-failure",
        "wide",
        "narrow",
        "ltr",
        "rtl",
        "dark",
        "light",
        "high-contrast",
        "1x",
        "2x",
        "pointer",
        "keyboard",
        "controller",
        "a11y",
    ] {
        assert!(
            ids.contains(required),
            "launcher matrix does not cover {required}"
        );
    }
}
