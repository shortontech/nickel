use nickel_shell::plugin_panel::{PluginEffect, PluginPanelApplication, surface};
use nickel_ui::{ActionKind, SemanticAction, SemanticRole, SemanticSelector, UiEvent, UiHost};

#[test]
fn bundled_jsx_panel_click_updates_visible_state() {
    let application = PluginPanelApplication::bundled().expect("bundled JavaScript loads");
    let mut host = UiHost::new(application, surface().width, surface().height);
    let button = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Count: 0".into(),
        })
        .expect("initial count button")
        .id;
    let outcome =
        host.perform_semantic_action(button, SemanticAction::Invoke(ActionKind::Activate));
    assert!(outcome.changed);
    host.query_unique(&SemanticSelector::RoleAndName {
        role: SemanticRole::Button,
        name: "Count: 1".into(),
    })
    .expect("JavaScript state updated the native tree");
    assert!(host.application().last_error().is_none());

    let open_dialog = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Open dialog".into(),
        })
        .expect("dialog anchor button")
        .id;
    assert!(
        host.perform_semantic_action(open_dialog, SemanticAction::Invoke(ActionKind::Activate))
            .changed
    );
    assert!(host.inspect().open_overlay.is_some());
    let show = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Show".into(),
        })
        .expect("dialog action button")
        .id;
    assert!(
        host.perform_semantic_action(show, SemanticAction::Invoke(ActionKind::Activate))
            .changed
    );
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ShowLauncher]
    );
    host.handle_event(UiEvent::ControllerBack);
    assert!(host.inspect().open_overlay.is_none());
    let open_dialog = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Open dialog".into(),
        })
        .expect("dialog can be reopened after dismissal")
        .id;
    host.perform_semantic_action(open_dialog, SemanticAction::Invoke(ActionKind::Activate));
    assert!(host.inspect().open_overlay.is_some());
}

#[test]
fn ungranted_effect_prevents_the_entire_action_batch() {
    let script = r#"
        function App() {
            return h(Panel, null, h(Button, {
                onClick: () => {
                    nickel.request('show-launcher');
                    nickel.request('focus-window');
                }
            }, 'Request'));
        }
    "#;
    let mut host = UiHost::new(
        PluginPanelApplication::new(script).expect("script loads"),
        surface().width,
        surface().height,
    );
    let button = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Request".into(),
        })
        .expect("request button")
        .id;
    host.perform_semantic_action(button, SemanticAction::Invoke(ActionKind::Activate));
    assert!(host.application().last_error().is_some());
    assert!(host.application_mut().take_effects().is_empty());
}
