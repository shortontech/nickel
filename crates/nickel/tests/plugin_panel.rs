use nickel_shell::plugin_panel::{HEIGHT, PluginPanelApplication, WIDTH};
use nickel_ui::{ActionKind, SemanticAction, SemanticRole, SemanticSelector, UiHost};

#[test]
fn bundled_jsx_panel_click_updates_visible_state() {
    let application = PluginPanelApplication::bundled().expect("bundled JavaScript loads");
    let mut host = UiHost::new(application, WIDTH, HEIGHT);
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
}
