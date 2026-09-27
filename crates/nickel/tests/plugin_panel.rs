use nickel_core::plugins::PluginPackage;
use nickel_shell::plugin_panel::{
    LauncherPluginProject, LauncherPluginProjection, LauncherPluginResult, LauncherView,
    NotificationPluginAction, NotificationPluginItem, NotificationPluginProjection, PluginEffect,
    PluginMessage, PluginPanelApplication, TaskbarMenuPluginProjection, TaskbarPluginItem,
    TaskbarPluginProjection, TaskbarPluginTrayItem, surface,
};

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
use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    ActionKind, HostBatch, HostEvent, Point, SemanticAction, SemanticRole, SemanticSelector,
    SemanticValueSnapshot, Shortcut, UiEvent, UiHost,
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
fn bundled_launcher_renders_host_results_and_requests_typed_actions() {
    let projection = LauncherPluginProjection {
        query: String::new(),
        status: None,
        dashboard_visible: true,
        view: LauncherView::Favorites,
        result_page: 0,
        result_page_count: 1,
        dashboard_page: 0,
        dashboard_page_count: 1,
        results: vec![LauncherPluginResult {
            index: 0,
            id: "calculator".into(),
            name: "Calculator".into(),
            pinned: false,
        }],
        dashboard: vec![LauncherPluginResult {
            index: 0,
            id: "editor".into(),
            name: "Editor".into(),
            pinned: false,
        }],
        places: vec![],
        projects: vec![],
        codex_available: false,
        account_name: "Local session".into(),
        logout_available: false,
    };
    let mut application = PluginPanelApplication::launcher_with_projection(&projection)
        .expect("launcher script loads");
    let icon = std::sync::Arc::new(image::RgbaImage::from_pixel(
        16,
        16,
        image::Rgba([40, 140, 240, 255]),
    ));
    application.sync_images(
        [
            ("dashboard:0".into(), (0x4000, std::sync::Arc::clone(&icon))),
            ("search:0".into(), (0x4000, icon)),
        ]
        .into(),
    );
    let mut host = UiHost::new(application, 920, 680);
    assert!(
        host.commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::Image { id: 0x4000, .. }))
    );
    let dashboard = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Editor".into(),
        })
        .expect("dashboard application")
        .id;
    host.perform_semantic_action(dashboard, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::LaunchDashboardApplication {
            id: "editor".into()
        }]
    );
    let pin = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Pin Editor".into(),
        })
        .expect("pin action")
        .id;
    host.perform_semantic_action(pin, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ToggleLauncherPin {
            id: "editor".into()
        }]
    );
    let editor = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Editor".into(),
        })
        .expect("application context target")
        .id;
    host.perform_semantic_action(editor, SemanticAction::Invoke(ActionKind::ContextMenu));
    assert!(host.inspect().open_overlay.is_some());
    let menu_pin = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::MenuItem,
            name: "Pin to Nickel Bar".into(),
        })
        .expect("JSX app menu pin item")
        .id;
    host.perform_semantic_action(menu_pin, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ToggleLauncherPin {
            id: "editor".into()
        }]
    );
    let field = host
        .query_unique(&SemanticSelector::Role(SemanticRole::TextField))
        .expect("launcher search field")
        .id;
    host.request_focus(field.clone());
    host.handle_event(UiEvent::TextInput("calc".into()));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::SetLauncherQuery("calc".into())]
    );

    let mut updated = projection.clone();
    updated.query = "calc".into();
    updated.dashboard_visible = false;
    updated.results[0].pinned = true;
    assert!(
        host.application_mut()
            .sync_launcher_projection(&updated)
            .expect("host projection updates")
    );
    host.step(nickel_ui::HostBatch {
        application_changed: true,
        ..nickel_ui::HostBatch::default()
    });
    assert_eq!(host.inspect().keyboard_focus, Some(field));
    host.query_unique(&SemanticSelector::RoleAndName {
        role: SemanticRole::Button,
        name: "Unpin Calculator".into(),
    })
    .expect("updated pin state");
    host.step(HostBatch {
        events: vec![HostEvent::Shortcut(Shortcut::Submit)],
        ..HostBatch::default()
    });
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ActivateLauncherResult {
            index: 0,
            id: "calculator".into()
        }]
    );
    host.step(HostBatch {
        events: vec![HostEvent::Shortcut(Shortcut::Escape)],
        ..HostBatch::default()
    });
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::SetLauncherQuery(String::new())]
    );
    let result = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Calculator".into(),
        })
        .expect("launcher result")
        .id;
    host.perform_semantic_action(result, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ActivateLauncherResult {
            index: 0,
            id: "calculator".into(),
        }]
    );
}

#[test]
fn launcher_dashboard_scrolls_without_dispatching_a_plugin_handler() {
    let dashboard = (0..12)
        .map(|index| LauncherPluginResult {
            index,
            id: format!("app-{index}"),
            name: format!("Application {index}"),
            pinned: false,
        })
        .collect();
    let application = PluginPanelApplication::launcher_with_projection(&LauncherPluginProjection {
        query: String::new(),
        status: None,
        dashboard_visible: true,
        view: LauncherView::Favorites,
        result_page: 0,
        result_page_count: 1,
        dashboard_page: 0,
        dashboard_page_count: 1,
        results: vec![],
        dashboard,
        places: vec![],
        projects: vec![],
        codex_available: false,
        account_name: "Local session".into(),
        logout_available: false,
    })
    .expect("launcher script loads");
    let mut host = UiHost::new(application, 920, 680);
    let extent = host
        .scroll_extent(&PluginMessage::Scroll)
        .expect("dashboard scroll surface");
    assert!(extent.can_scroll());
    host.handle_event(UiEvent::Scroll {
        point: Point { x: 100.0, y: 150.0 },
        delta_y: 240.0,
    });
    assert!(host.scroll_extent(&PluginMessage::Scroll).unwrap().offset > 0.0);
    assert!(host.application_mut().take_effects().is_empty());
}

#[test]
fn launcher_dashboard_requests_projects_settings_account_and_logout() {
    let application = PluginPanelApplication::launcher_with_projection(&LauncherPluginProjection {
        query: String::new(),
        status: None,
        dashboard_visible: true,
        view: LauncherView::Favorites,
        result_page: 0,
        result_page_count: 1,
        dashboard_page: 0,
        dashboard_page_count: 1,
        results: vec![],
        dashboard: vec![],
        places: vec![],
        projects: vec![LauncherPluginProject {
            id: "project-1".into(),
            name: "Nickel source".into(),
        }],
        codex_available: true,
        account_name: "Ada".into(),
        logout_available: true,
    })
    .expect("launcher script loads");
    let mut host = UiHost::new(application, 920, 680);
    let invoke = |host: &mut UiHost<PluginPanelApplication>, name: &str| {
        let id = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            })
            .expect("dashboard button")
            .id;
        host.perform_semantic_action(id, SemanticAction::Invoke(ActionKind::Activate));
    };
    invoke(&mut host, "Nickel source");
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::LauncherOpenProject {
            id: "project-1".into()
        }]
    );
    invoke(&mut host, "All projects");
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::LauncherSeeAllProjects]
    );
    invoke(&mut host, "Ada");
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::LauncherOpenAccount]
    );
    invoke(&mut host, "Settings");
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::LauncherOpenSettings]
    );
    invoke(&mut host, "Log out");
    assert!(host.inspect().open_overlay.is_some());
    assert!(host.application_mut().take_effects().is_empty());
    host.application_mut().set_overlay_open(true);
    host.step(HostBatch {
        events: vec![HostEvent::Shortcut(Shortcut::Escape)],
        ..HostBatch::default()
    });
    assert!(host.application_mut().take_effects().is_empty());
    invoke(&mut host, "Cancel");
    assert!(host.inspect().open_overlay.is_none());
    host.application_mut().set_overlay_open(false);
    invoke(&mut host, "Log out");
    invoke(&mut host, "Confirm log out");
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::LauncherRequestLogout]
    );
    host.step(HostBatch {
        events: vec![HostEvent::Shortcut(Shortcut::Escape)],
        ..HostBatch::default()
    });
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::DismissLauncher]
    );
}

#[test]
fn launcher_plugin_switches_dashboard_views_from_host_projection() {
    let mut projection = LauncherPluginProjection {
        query: String::new(),
        status: None,
        dashboard_visible: true,
        view: LauncherView::Favorites,
        result_page: 0,
        result_page_count: 1,
        dashboard_page: 0,
        dashboard_page_count: 1,
        results: vec![],
        dashboard: vec![LauncherPluginResult {
            index: 0,
            id: "favorite".into(),
            name: "Favorite app".into(),
            pinned: true,
        }],
        places: vec![],
        projects: vec![],
        codex_available: false,
        account_name: "Local session".into(),
        logout_available: false,
    };
    let mut host = UiHost::new(
        PluginPanelApplication::launcher_with_projection(&projection)
            .expect("launcher script loads"),
        920,
        680,
    );
    let switch = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "All applications".into(),
        })
        .expect("view switch")
        .id;
    host.perform_semantic_action(switch, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::SetLauncherView(LauncherView::Applications)]
    );
    projection.view = LauncherView::Applications;
    projection.dashboard = vec![LauncherPluginResult {
        index: 0,
        id: "calculator".into(),
        name: "Calculator".into(),
        pinned: false,
    }];
    assert!(
        host.application_mut()
            .sync_launcher_projection(&projection)
            .unwrap()
    );
    host.step(nickel_ui::HostBatch {
        application_changed: true,
        ..Default::default()
    });
    host.query_unique(&SemanticSelector::RoleAndName {
        role: SemanticRole::Button,
        name: "Calculator".into(),
    })
    .expect("host supplied application list");
    assert!(
        host.query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Favorite app".into(),
        })
        .is_err()
    );
}

#[test]
fn launcher_plugin_pages_search_results_through_typed_requests() {
    let mut projection = LauncherPluginProjection {
        query: "app".into(),
        status: None,
        dashboard_visible: false,
        view: LauncherView::Applications,
        result_page: 0,
        result_page_count: 2,
        dashboard_page: 0,
        dashboard_page_count: 1,
        results: vec![LauncherPluginResult {
            index: 0,
            id: "app-0".into(),
            name: "First app".into(),
            pinned: false,
        }],
        dashboard: vec![],
        places: vec![],
        projects: vec![],
        codex_available: false,
        account_name: "Local session".into(),
        logout_available: false,
    };
    let mut host = UiHost::new(
        PluginPanelApplication::launcher_with_projection(&projection).unwrap(),
        920,
        680,
    );
    let next = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Next".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(next, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::SetLauncherPage {
            dashboard: false,
            page: 1,
        }]
    );
    projection.result_page = 1;
    projection.results[0] = LauncherPluginResult {
        index: 12,
        id: "app-12".into(),
        name: "Later app".into(),
        pinned: false,
    };
    assert!(
        host.application_mut()
            .sync_launcher_projection(&projection)
            .unwrap()
    );
    host.step(HostBatch {
        application_changed: true,
        ..HostBatch::default()
    });
    let later = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Later app".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(later, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ActivateLauncherResult {
            index: 12,
            id: "app-12".into(),
        }]
    );
}

#[test]
fn launcher_plugin_pages_dashboard_applications() {
    let projection = LauncherPluginProjection {
        query: String::new(),
        status: None,
        dashboard_visible: true,
        view: LauncherView::Applications,
        result_page: 0,
        result_page_count: 1,
        dashboard_page: 0,
        dashboard_page_count: 2,
        results: vec![],
        dashboard: vec![LauncherPluginResult {
            index: 0,
            id: "app-0".into(),
            name: "First app".into(),
            pinned: false,
        }],
        places: vec![],
        projects: vec![],
        codex_available: false,
        account_name: "Local session".into(),
        logout_available: false,
    };
    let mut host = UiHost::new(
        PluginPanelApplication::launcher_with_projection(&projection).unwrap(),
        920,
        680,
    );
    let next = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Next".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(next, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::SetLauncherPage {
            dashboard: true,
            page: 1,
        }]
    );
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

#[test]
fn bundled_taskbar_renders_grouped_items_and_emits_typed_actions() {
    let projection = TaskbarPluginProjection {
        items: vec![TaskbarPluginItem {
            index: 0,
            id: "org.example.editor".into(),
            name: "Editor".into(),
            active: true,
            pinned: true,
            icon: true,
            badges: Vec::new(),
        }],
        tray: vec![TaskbarPluginTrayItem {
            id: "mail".into(),
            title: "Mail".into(),
            icon: true,
        }],
        clock: "4:20 PM".into(),
        keyboard_enabled: true,
        codex_available: true,
    };
    let icon = std::sync::Arc::new(image::RgbaImage::from_pixel(
        16,
        16,
        image::Rgba([40, 140, 240, 255]),
    ));
    let mut application = PluginPanelApplication::taskbar_with_projection(&projection).unwrap();
    application.sync_images(
        [
            ("logo".into(), (2, std::sync::Arc::clone(&icon))),
            ("codex".into(), (0x5000, std::sync::Arc::clone(&icon))),
            ("task:0".into(), (3, std::sync::Arc::clone(&icon))),
            ("tray:mail".into(), (4, std::sync::Arc::clone(&icon))),
        ]
        .into(),
    );
    let mut host = UiHost::new(application, 900, 56);
    assert!(
        host.commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::Image { id: 3, .. }))
    );
    let editor = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Editor".into(),
        })
        .expect("task group button")
        .id;
    host.perform_semantic_action(editor, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ActivateTaskbarItem {
            index: 0,
            id: "org.example.editor".into(),
        }]
    );
    let editor = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Editor".into(),
        })
        .expect("task group context target")
        .id;
    host.perform_semantic_action(editor, SemanticAction::Invoke(ActionKind::ContextMenu));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ContextTaskbarItem {
            index: 0,
            id: "org.example.editor".into()
        }]
    );
    let launcher = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Open Nickel Start".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(launcher, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ToggleLauncher]
    );
    let control = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "4:20 PM".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(control, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ToggleControlCenter]
    );
    for (name, expected) in [
        ("On-screen keyboard", PluginEffect::ToggleOnScreenKeyboard),
        ("Codex projects", PluginEffect::ToggleCodexProjects),
    ] {
        let button = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            })
            .unwrap()
            .id;
        host.perform_semantic_action(button, SemanticAction::Invoke(ActionKind::Activate));
        assert_eq!(host.application_mut().take_effects(), vec![expected]);
    }
    let tray = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Mail".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(tray, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ActivateTrayItem { id: "mail".into() }]
    );
    let tray = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Mail".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(tray, SemanticAction::Invoke(ActionKind::ContextMenu));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::ContextTrayItem { id: "mail".into() }]
    );
}

#[test]
fn bundled_taskbar_menu_requests_validated_pin_and_close_actions() {
    let application =
        PluginPanelApplication::taskbar_menu_with_projection(&TaskbarMenuPluginProjection {
            application_id: Some("org.example.editor".into()),
            pinned: false,
            close_all: true,
            actions: Vec::new(),
        })
        .unwrap();
    let mut host = UiHost::new(application, 248, 112);
    for (label, expected) in [
        (
            "Pin to Nickel Bar",
            PluginEffect::ToggleTaskbarMenuPin {
                id: "org.example.editor".into(),
            },
        ),
        ("Close all windows", PluginEffect::CloseTaskbarMenuWindows),
    ] {
        let button = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: label.into(),
            })
            .unwrap();
        host.perform_semantic_action(button.id, SemanticAction::Invoke(ActionKind::Activate));
        assert_eq!(host.application_mut().take_effects(), vec![expected]);
    }
}

#[test]
fn bundled_notification_invokes_current_actions_and_closes_history() {
    let mut projection = NotificationPluginProjection {
        notification: Some(NotificationPluginItem {
            id: 41,
            app_name: "Mail".into(),
            summary: "New message".into(),
            body: "A message arrived".into(),
            actions: vec![NotificationPluginAction {
                key: "open".into(),
                label: "Open".into(),
            }],
        }),
        history: vec![],
        history_visible: false,
    };
    let mut host = UiHost::new(
        PluginPanelApplication::notification_with_projection(&projection).unwrap(),
        420,
        180,
    );
    let open = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Open".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(open, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::InvokeNotification {
            id: 41,
            key: "open".into(),
        }]
    );
    let dismiss = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Dismiss".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(dismiss, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::DismissNotification { id: 41 }]
    );
    assert!(host.shortcut(Shortcut::Escape));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::DismissNotification { id: 41 }]
    );
    projection.history = vec![projection.notification.clone().unwrap()];
    projection.history_visible = true;
    assert!(
        host.application_mut()
            .sync_notification_projection(&projection)
            .unwrap()
    );
    host.step(HostBatch {
        application_changed: true,
        ..HostBatch::default()
    });
    let close = host
        .query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Close".into(),
        })
        .unwrap()
        .id;
    host.perform_semantic_action(close, SemanticAction::Invoke(ActionKind::Activate));
    assert_eq!(
        host.application_mut().take_effects(),
        vec![PluginEffect::CloseNotificationHistory]
    );
}
