#![cfg(feature = "workbench-fixtures")]

use nickel_shell::ShellFixtureProvider;
use twinkle::{ActionKind, SemanticRole, Size};
use twinkle_testkit::{ActivationVia, FixtureProvider, FixtureRegistry};

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
            "shell.lock",
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

            if entry.metadata.id == "shell.screenshot" {
                let has_activate = session
                    .semantic_nodes()
                    .iter()
                    .any(|node| node.actions.contains(&twinkle::ActionKind::Activate));
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
