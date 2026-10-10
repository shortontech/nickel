use nickel_core::plugins::PluginPackage;
use nickel_shell::plugin_panel::{PluginEffect, PluginPanelApplication, surface};

#[test]
fn generic_color_swatch_renders_accessible_radio_and_custom_action() {
    let script = "function App() { return h(Panel, {}, h(ColorSwatch, {id: 'blue', color: '#336699', selected: true, accessibilityLabel: 'Blue accent', onClick: () => nickel.request('show-launcher')}), h(ColorSwatch, {id: 'custom', accessibilityLabel: 'Custom color', onClick: () => nickel.request('show-launcher')})); }";
    let mut package = PluginPackage::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/plugins/hello-panel"
    ))
    .unwrap();
    package.source = script.into();
    // Control geometry belongs to package CSS, not native hard-coded defaults.
    package.stylesheet.push_str(include_str!(
        "../../../assets/plugins/nickel-default/src/styles/controls.css"
    ));
    let mut host = UiHost::new(
        PluginPanelApplication::from_package(&package).unwrap(),
        320,
        80,
    );
    assert!(
        host.query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Radio,
            name: "Blue accent".into(),
        })
        .is_ok()
    );
    assert!(
        host.query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Custom color".into(),
        })
        .is_ok()
    );
    assert!(host.accessibility_nodes().iter().any(|node| {
        node.semantic_role == Some(SemanticRole::Radio)
            && node.label.as_deref() == Some("Blue accent")
            && node.state.as_deref() == Some("selected")
    }));
    assert!(host.commands().iter().any(|command| matches!(
        command,
        PaintCommand::RoundedFill {
            color: 0xff336699,
            ..
        }
    )));
    for (role, name) in [
        (SemanticRole::Radio, "Blue accent"),
        (SemanticRole::Button, "Custom color"),
    ] {
        let target = host
            .query_unique(&SemanticSelector::RoleAndName {
                role,
                name: name.into(),
            })
            .unwrap();
        host.perform_semantic_action(target.id, SemanticAction::Invoke(ActionKind::Activate));
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowLauncher]
        );
    }
}

#[test]
fn clickable_div_composes_an_accessible_preview_choice() {
    let script = "function App() { return h(Panel, {}, h(Div, {id: 'dark-choice', role: 'radio', 'aria-label': 'Dark mode', 'aria-checked': true, onClick: () => nickel.request('show-launcher')}, h(Div, {className: 'preview'}, h(Text, {}, 'Preview')), h(Text, {}, 'Dark'))); }";
    let mut host = UiHost::new(PluginPanelApplication::new(script).unwrap(), 320, 180);
    let choice = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Radio,
            name: "Dark mode".into(),
        })
        .expect("composed div exposes one radio target");
    assert!(host.accessibility_nodes().iter().any(|node| {
        node.semantic_role == Some(SemanticRole::Radio)
            && node.label.as_deref() == Some("Dark mode")
            && node.state.as_deref() == Some("selected")
    }));
    assert!(
        host.perform_semantic_action(choice.id, SemanticAction::Invoke(ActionKind::Activate),)
            .changed
    );
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ShowLauncher]
    );
}

#[test]
fn directory_package_runs_in_the_same_jsx_host() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::write(
        directory.path().join("plugin.json"),
        r#"{"api_version":1,"id":"org.example.panel","name":"Example Panel","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":320,"height":80}],"capabilities":["launcher-show"]}"#,
    )
    .unwrap();
    std::fs::write(
        directory.path().join("main.js"),
        r#"function App() { return h(Panel, {},
            h(Button, {id: 'example', onClick: () => nickel.openDialog('example-dialog')}, 'Example'),
            h(Dialog, {id: 'example-dialog', anchor: 'example', open: true},
                h(Button, {id: 'show-launcher', onClick: () => nickel.request('show-launcher')}, 'Show launcher'))); }"#,
    )
    .unwrap();
    let package = PluginPackage::load(directory.path()).unwrap();
    let mut host = UiHost::new(
        PluginPanelApplication::from_package(&package).unwrap(),
        320,
        80,
    );
    let button = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Example".into(),
        })
        .expect("external package rendered by the native JSX host")
        .id;
    assert!(
        host.perform_semantic_action(button, SemanticAction::Invoke(ActionKind::Activate))
            .changed
    );
    assert!(host.inspect().open_overlay.is_some());
    let action = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Show launcher".into(),
        })
        .expect("external package dialog action")
        .id;
    assert!(
        host.perform_semantic_action(action, SemanticAction::Invoke(ActionKind::Activate))
            .changed
    );
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ShowLauncher]
    );
}

#[test]
fn installed_jsx_dialog_requests_settings_only_with_its_grant() {
    let package = PluginPackage::load(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../assets/plugins/example-dialog"
    ))
    .unwrap();
    PluginPanelApplication::validate_package(&package).unwrap();
    let activate = |host: &mut UiHost<PluginPanelApplication>, name: &str| {
        let target = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            })
            .unwrap()
            .id;
        host.perform_semantic_action(target, SemanticAction::Invoke(ActionKind::Activate));
    };
    let mut host = UiHost::new(
        PluginPanelApplication::from_package(&package).unwrap(),
        320,
        120,
    );
    activate(&mut host, "Open a dialog");
    assert!(host.inspect().open_overlay.is_some());
    activate(&mut host, "Open Settings");
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ShowSettings(None)]
    );

    let mut ungranted = package;
    ungranted
        .manifest
        .capabilities
        .retain(|capability| *capability != nickel_core::plugins::PluginCapability::SettingsShow);
    let mut host = UiHost::new(
        PluginPanelApplication::from_package(&ungranted).unwrap(),
        320,
        120,
    );
    activate(&mut host, "Open a dialog");
    activate(&mut host, "Open Settings");
    assert!(host.application_mut().take_effects().is_empty());
}

#[test]
fn failed_jsx_render_keeps_previous_handlers_and_recovers() {
    let script = r#"
        function App() {
            const [broken, setBroken] = useState(false);
            const [count, setCount] = useState(0);
            if (!broken) useRef(null);
            return h(Panel, {},
                h(Button, {id: 'break', onClick: () => setBroken(true)}, 'Break'),
                h(Button, {id: 'reset', onClick: () => setBroken(false)}, 'Reset'),
                h(Button, {id: 'count', onClick: () => setCount(count + 1)}, 'Count: ' + count));
        }
    "#;
    let mut host = UiHost::new(PluginPanelApplication::new(script).unwrap(), 320, 140);
    let break_button = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Break".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(break_button, SemanticAction::Invoke(ActionKind::Activate));
    assert!(
        host.application()
            .last_error()
            .is_some_and(|error| error.contains("hook order changed"))
    );
    let reset = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Reset".into(),
        })
        .expect("previous valid tree remains visible")
        .id;
    host.perform_semantic_action(reset, SemanticAction::Invoke(ActionKind::Activate));
    assert!(host.application().last_error().is_none());
    let count = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Count: 0".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(count, SemanticAction::Invoke(ActionKind::Activate));
    host.query_unique(&SemanticSelector::RoleAndName {
        role: SemanticRole::Button,
        name: "Count: 1".into(),
    })
    .expect("handlers still match the recovered tree");
}

#[test]
fn rejected_native_tree_keeps_previous_jsx_handlers() {
    let script = r#"
        function App() {
            const [invalid, setInvalid] = useState(false);
            return h(Panel, {},
                h(Button, {id: 'break', onClick: () => setInvalid(true)}, 'Break'),
                h(Button, {id: 'reset', onClick: () => setInvalid(false)}, 'Reset'),
                invalid ? h('unsupported-component', {}, 'Bad') : h(Text, {}, 'Valid'));
        }
    "#;
    let mut host = UiHost::new(PluginPanelApplication::new(script).unwrap(), 320, 140);
    let button = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Break".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(button, SemanticAction::Invoke(ActionKind::Activate));
    assert!(host.application().last_error().is_some());
    let reset = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Reset".into(),
        })
        .expect("previous tree is still visible")
        .id;
    host.perform_semantic_action(reset, SemanticAction::Invoke(ActionKind::Activate));
    assert!(host.application().last_error().is_none());
}
use twinkle::backend::PaintCommand;
use twinkle::{
    ActionKind, SemanticAction, SemanticRole, SemanticSelector, SemanticValueSnapshot, UiEvent,
    UiHost,
};

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
fn jsx_menu_opens_and_dispatches_a_typed_item_action() {
    let script = r#"
        function App() {
            return h(Panel, {height: 96},
                h(Button, {id: 'menu-anchor', onClick: () => nickel.openMenu('actions')}, 'Actions'),
                h(Menu, {id: 'actions', anchor: 'menu-anchor', open: true},
                    h(MenuItem, {id: 'show', onClick: () => nickel.request('show-launcher')}, 'Show launcher')));
        }
    "#;
    let mut host = UiHost::new(
        PluginPanelApplication::new(script).expect("menu script loads"),
        700,
        280,
    );
    let anchor = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Actions".into(),
        })
        .expect("menu anchor")
        .id;
    host.perform_semantic_action(anchor, SemanticAction::Invoke(ActionKind::Activate));
    assert!(host.inspect().open_overlay.is_some());
    let item = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::MenuItem,
            name: "Show launcher".into(),
        })
        .expect("menu item")
        .id;
    host.perform_semantic_action(item, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ShowLauncher]
    );
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

#[test]
fn javascript_text_field_updates_state_from_native_input() {
    let script = r#"
        function App() {
            const [query, setQuery] = useState('');
            return h(Panel, null,
                h(TextField, {id: 'query', value: query, placeholder: 'Search', onChange: setQuery}),
                h(Text, null, query));
        }
    "#;
    let mut host = UiHost::new(
        PluginPanelApplication::new(script).expect("script loads"),
        surface().width,
        surface().height,
    );
    let field = host
        .query_unique(&SemanticSelector::Role(SemanticRole::TextField))
        .expect("native text field")
        .id;
    assert!(host.request_focus(field).changed);
    assert!(
        host.handle_event(UiEvent::TextInput("nickel".into()))
            .changed
    );
    assert!(host.application().last_error().is_none());
    let field = host
        .query_unique(&SemanticSelector::Role(SemanticRole::TextField))
        .expect("updated text field");
    assert_eq!(
        field.value,
        Some(SemanticValueSnapshot::Text("nickel".into()))
    );
}

#[test]
fn javascript_text_fields_route_to_their_own_handlers() {
    let script = r#"
        function App() {
            const [first, setFirst] = useState('');
            const [second, setSecond] = useState('');
            return h(Column, null,
                h(TextField, {id: 'first', value: first, onChange: setFirst}),
                h(TextField, {id: 'second', value: second, onChange: setSecond}));
        }
    "#;
    let mut host = UiHost::new(
        PluginPanelApplication::new(script).expect("script loads"),
        surface().width,
        surface().height,
    );
    let fields = host
        .semantic_nodes()
        .into_iter()
        .filter(|node| node.role == Some(SemanticRole::TextField))
        .map(|node| node.id)
        .collect::<Vec<_>>();
    assert_eq!(fields.len(), 2);
    host.request_focus(fields[1].clone());
    host.handle_event(UiEvent::TextInput("second".into()));
    let fields = host
        .semantic_nodes()
        .into_iter()
        .filter(|node| node.role == Some(SemanticRole::TextField))
        .collect::<Vec<_>>();
    assert_eq!(
        fields[0].value,
        Some(SemanticValueSnapshot::Text("".into()))
    );
    assert_eq!(
        fields[1].value,
        Some(SemanticValueSnapshot::Text("second".into()))
    );
    assert!(host.application().last_error().is_none());
}

#[test]
fn keyed_function_components_keep_state_across_conditional_siblings() {
    let script = r#"
        function Counter(props) {
            const [count, setCount] = useState(0);
            return h(Button, {id: props.id, onClick: () => setCount(count + 1)}, props.id + ':' + count);
        }
        function App() {
            const [showFirst, setShowFirst] = useState(true);
            return h(Panel, null,
                h(Button, {id: 'toggle', onClick: () => setShowFirst(!showFirst)}, 'Toggle'),
                showFirst ? h(Counter, {key: 'first', id: 'first'}) : null,
                h(Counter, {key: 'second', id: 'second'}));
        }
    "#;
    let mut host = UiHost::new(
        PluginPanelApplication::new(script).expect("script loads"),
        surface().width,
        surface().height,
    );
    let invoke = |host: &mut UiHost<PluginPanelApplication>, name: &str| {
        let id = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            })
            .unwrap()
            .id;
        assert!(
            host.perform_semantic_action(id, SemanticAction::Invoke(ActionKind::Activate))
                .changed
        );
    };
    invoke(&mut host, "second:0");
    invoke(&mut host, "Toggle");
    assert!(
        host.query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "first:0".into(),
        })
        .is_err()
    );
    invoke(&mut host, "Toggle");
    host.query_unique(&SemanticSelector::RoleAndName {
        role: SemanticRole::Button,
        name: "first:0".into(),
    })
    .expect("remounted component starts with fresh state");
    host.query_unique(&SemanticSelector::RoleAndName {
        role: SemanticRole::Button,
        name: "second:1".into(),
    })
    .expect("stable keyed sibling retains its state");
}
