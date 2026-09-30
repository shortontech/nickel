


    #[test]
    fn ordinary_plugin_window_title_comes_from_its_jsx_root() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-surface-dialog"
        );
        let mut package = nickel_core::plugins::PluginPackage::load(directory).unwrap();
        package.source = "function App() { const surface = nickel.data.surface; return h(Window, {width: surface.width, height: surface.height, title: surface.id === 'confirm' ? 'Confirm' : 'Home'}, h(Text, {}, 'Example')); }".into();
        let surface = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "home")
            .unwrap();
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: package.manifest.id.clone(),
            surface_id: surface.id.clone(),
        };
        let application = crate::plugin_panel::PluginPanelApplication::from_package_surface(
            &package,
            &Default::default(),
            surface,
        )
        .unwrap();
        let mut shell = LiveShell::new().unwrap();
        shell.plugin_surface_hosts.insert(
            key.clone(),
            (
                surface.clone(),
                nickel_ui::UiHost::new(application, surface.width, surface.height),
            ),
        );
        assert_eq!(shell.plugin_panel_title(&key), Some("Home"));
        shell.plugin_surface_hosts.remove(&key);
        assert_eq!(shell.plugin_panel_title(&key), None);
    }






    #[cfg(target_os = "linux")]




    #[test]
    fn window_preview_shared_surface_retires_and_public_requests_recheck_revision_lock() {
        let host = Arc::new(crate::session_host::StagedSessionHost::new(crate::session_host::default_session_host()));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.launcher = crate::launcher::Launcher::new(Vec::new());
        shell.windows = vec![OpenWindow {id:WindowId(71),application_id:None,active:true,title:"Document".into(),state:Default::default()}];
        shell.open_window_preview(0);
        assert!(shell.preview_plugin_active());
        assert!(!shell.window_preview_scene().is_empty());
        assert!(shell.plugin_registry.get("org.nickel.window-preview").is_none());
        let revision = shell.plugin_window_previews("nickel-default").unwrap()["revision"].as_str().unwrap().to_owned();
        let request = |revision:String| crate::plugin_panel::PluginEffect::WindowPreviewRequest {plugin_id:"nickel-default".into(),revision,action:crate::window_preview::PreviewAction::Close(WindowId(71))};
        host.take_commands();
        assert!(!shell.apply_plugin_effects(vec![request("stale".into())]));
        shell.close_window_preview();
        shell.open_window_preview(0);
        assert!(!shell.apply_plugin_effects(vec![request(revision.clone())]));
        let revision = shell.plugin_window_previews("nickel-default").unwrap()["revision"].as_str().unwrap().to_owned();
        shell.locked=true;
        assert_eq!(shell.plugin_window_previews("nickel-default").unwrap()["available"],false);
        assert!(!shell.apply_plugin_effects(vec![request(revision.clone())]));
        shell.locked=false;
        assert!(shell.apply_plugin_effects(vec![request(revision)]));
        assert!(host.take_commands().iter().any(|command|matches!(command,crate::platform::ShellCommand::WindowAction{window:WindowId(71),action:crate::platform::WindowAction::Close})));
        shell.set_plugin_enabled("nickel-default",false).unwrap();
        assert!(!shell.preview_plugin_active());
        assert!(shell.preview_group.is_none());
        assert!(shell.plugin_window_previews("nickel-default").is_none());
    }

    #[test]
    fn task_switcher_cards_render_in_jsx_and_activate_through_the_host() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.windows = [WindowId(71), WindowId(72)]
            .into_iter()
            .map(|id| OpenWindow {
                id,
                application_id: None,
                active: id == WindowId(71),
                title: format!("Window {}", id.0),
                state: Default::default(),
            })
            .collect();
        let candidates = shell
            .windows
            .iter()
            .map(|window| nickel_core::task_switcher::SwitchWindow {
                id: window.id,
                application_id: window.title.clone(),
                active: window.active,
            })
            .collect::<Vec<_>>();
        shell.task_switcher.apply(
            nickel_core::hotkeys::HotkeyAction::SwitchNext,
            &candidates,
        );
        shell.rebuild_task_switcher_preview();
        assert!(!shell.scene(SurfaceRole::WindowPreview, 474, 214).is_empty());
        assert!(shell.preview_plugin_active());
        let projection = shell.plugin_window_previews("nickel-default").unwrap();
        assert_eq!(projection["taskSwitcher"], true);
        assert_eq!(
            projection["windows"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|window| window["selected"] == true)
                .count(),
            1
        );

        let action = crate::window_preview::PreviewAction::Activate(WindowId(71));
        let bounds = shell.preview_plugin_bounds(action).unwrap();
        host.take_commands();
        shell.sync_transient_overlays();
        let thumbnails = host
            .take_commands()
            .into_iter()
            .find_map(|command| match command {
                crate::platform::ShellCommand::ShowTaskSwitcher {
                    thumbnail_bounds, ..
                } => Some(thumbnail_bounds),
                _ => None,
            })
            .expect("task switcher command includes live JSX thumbnail bounds");
        assert_eq!(thumbnails.len(), 2);
        assert_eq!(thumbnails[0].left, bounds.origin.x.floor() as i32);
        assert!(shell.preview_click(
            bounds.origin.x + bounds.size.width / 2.0,
            bounds.origin.y + bounds.size.height / 2.0,
            false,
        ));
        assert!(host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::WindowAction {
                window: WindowId(71),
                action: crate::platform::WindowAction::Activate,
            }
        )));
        assert!(shell.task_switcher.session().is_none());
        assert!(shell.task_switcher_group.is_none());

        shell
            .set_plugin_enabled("nickel-default", false)
            .unwrap();
        assert!(shell.task_switcher.session().is_none());
        assert!(shell.task_switcher_group.is_none());
        shell.task_switcher.apply(
            nickel_core::hotkeys::HotkeyAction::SwitchNext,
            &candidates,
        );
        shell.rebuild_task_switcher_preview();
        assert!(shell.scene(SurfaceRole::WindowPreview, 474, 214).is_empty());
        assert!(!shell.surface_visible(SurfaceRole::WindowPreview));
        assert!(!shell.preview_plugin_active());
    }



    #[test]
    fn codex_project_menu_uses_a_plugin_surface_and_retires_on_disable() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::codex_projects_manifest().id;
        let key = crate::plugin_panel::codex_projects_surface_key();
        shell.launcher.set_codex_available(true);
        assert!(shell.plugin_surface_matches(&key));
        assert!(shell.shell_panel_surfaces().iter().any(|(surface, _)| surface == &key));
        assert!(shell.plugin_panels().iter().all(|(surface, _)| surface != &key));
        assert!(!shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        assert!(shell.show_projects_menu(true));
        assert!(shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        assert!(!shell.native_surface_visible(SurfaceRole::CodexProjectMenu, None));
        let mut state = nickel_codex_ui::ChatState::default();
        state.status = nickel_codex_ui::ConnectionStatus::Ready;
        state.account.authenticated = true;
        state.projects.push(nickel_codex::Project {
            id: "private-backend-id".into(),
            name: "Example project".into(),
            roots: vec![std::path::PathBuf::from("/private/work")],
        });
        let projection = nickel_codex_ui::ProjectMenuProjection::from_state(&state);
        assert!(shell.apply_codex_menu_projection(&projection));
        let scene = shell
            .plugin_surface_scene_for_output(&key, Some("primary"), 520, 680)
            .unwrap();
        assert!(format!("{scene:?}").contains("Example project"));
        assert!(shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::CodexProjectOpen {
                token: "0".into(),
                revision: projection.revision,
            }
        ]));
        assert_eq!(
            shell.take_codex_menu_requests(),
            vec![super::CodexMenuRequest::Open {
                token: "0".into(),
                revision: projection.revision,
            }]
        );
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(!shell.plugin_surface_matches(&key));
        assert!(!shell.codex_project_menu_visible);
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.plugin_surface_matches(&key));
        assert!(!shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
    }

    #[test]
    fn codex_menu_projection_failure_retires_its_plugin_surface() {
        let mut shell = LiveShell::new().unwrap();
        let key = crate::plugin_panel::codex_projects_surface_key();
        let initial = nickel_codex_ui::ProjectMenuProjection::from_state(
            &nickel_codex_ui::ChatState::default(),
        );
        let application = crate::plugin_panel::PluginPanelApplication::codex_projects_with_test_source(
            "let renders = 0; function App() { if (++renders > 1) throw Error('Codex projection exploded'); return h(Panel, {}, h(Text, {}, 'Ready')); }",
            &initial,
        )
        .unwrap();
        let surface = crate::plugin_panel::codex_projects_manifest().surfaces[0].clone();
        shell.plugin_surface_hosts.insert(
            key.clone(),
            (surface.clone(), nickel_ui::UiHost::new(application, surface.width, surface.height)),
        );
        let mut state = nickel_codex_ui::ChatState::default();
        state.status = nickel_codex_ui::ConnectionStatus::Ready;
        let projection = nickel_codex_ui::ProjectMenuProjection::from_state(&state);
        assert!(!shell.apply_codex_menu_projection(&projection));
        let entry = shell.plugin_registry().get(&key.plugin_id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("Codex projection exploded")));
        assert!(!shell.plugin_surface_matches(&key));
        assert!(!shell.plugin_surface_hosts.contains_key(&key));
    }



    #[test]
    #[cfg(target_os = "linux")]
    fn keyboard_plugin_owns_ordinary_presentation_and_retires_to_host_fallback() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::on_screen_keyboard_manifest().id;
        let key = crate::plugin_panel::on_screen_keyboard_surface_key();
        assert!(shell.plugin_surface_matches(&key));
        assert!(shell.shell_panel_surfaces().iter().any(|(surface, _)| surface == &key));
        assert!(shell.plugin_panels().iter().all(|(surface, _)| surface != &key));
        assert!(!shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        shell.keyboard_enabled = true;
        shell.keyboard_visible = true;
        assert!(shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        assert!(!shell.native_surface_visible(SurfaceRole::OnScreenKeyboard, None));
        assert!(shell
            .plugin_surface_scene_for_output(&key, Some("primary"), 1056, 368)
            .is_some_and(|scene| !scene.is_empty()));
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(!shell.plugin_surface_matches(&key));
        assert!(!shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        assert!(shell.native_surface_visible(SurfaceRole::OnScreenKeyboard, None));
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.plugin_surface_matches(&key));
        assert!(shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
    }

    #[test]
    fn shortcut_capability_failures_have_visible_classified_status() {
        use nickel_input::global::{ShortcutCapability, UnavailableReason};

        assert_eq!(
            shortcut_capability_status(&ShortcutCapability::Available),
            None
        );
        for (reason, detail) in [
            (UnavailableReason::UnsupportedPlatform, "unsupported"),
            (UnavailableReason::MissingRuntime, "runtime"),
            (UnavailableReason::PermissionDenied, "permission"),
            (UnavailableReason::SessionLocked, "locked"),
            (
                UnavailableReason::Backend("registration conflict".into()),
                "registration conflict",
            ),
        ] {
            let status = shortcut_capability_status(&ShortcutCapability::Unavailable(reason))
                .expect("an unavailable shortcut adapter must remain visible");
            assert!(status.starts_with("Global shortcuts unavailable:"));
            assert!(status.contains(detail), "{status:?}");
        }
    }

    #[test]
    fn per_display_panel_projection_keeps_owned_and_unresolved_windows_only() {
        assert!(window_belongs_to_panel(false, Some("DP-1"), Some("DP-1")));
        assert!(!window_belongs_to_panel(
            false,
            Some("DP-1"),
            Some("HDMI-A-1")
        ));
        assert!(window_belongs_to_panel(false, Some("DP-1"), None));
        assert!(window_belongs_to_panel(
            true,
            Some("DP-1"),
            Some("HDMI-A-1")
        ));
    }

    #[test]
    fn host_runtime_phase_samples_are_bounded() {
        let mut samples = HostRuntimeSamples::default();
        for value in 0..70 {
            samples.record(HostTelemetry {
                input_to_message_us: value,
                input_to_frame_us: value,
                layout_us: value,
                paint_list_us: value,
                scheduled_wakeups: 1,
                ..HostTelemetry::default()
            });
        }
        assert_eq!(samples.input_to_frame_us.len(), 64);
        assert_eq!(samples.input_to_frame_us.front(), Some(&6));
        assert_eq!(samples.scheduled_wakeups, 70);
    }
    use crate::{
        model::{ApplicationId, OpenWindow, TrayItem, WindowId},
        winit_shell::SurfaceRole,
    };
    use nickel_core::launcher_preferences::LauncherPreferences;

    fn preferences_fixture(shell: &mut LiveShell, path: std::path::PathBuf) {
        let preferences = LauncherPreferences::load(&path).unwrap_or_default();
        shell.launcher_preference_persistence = super::preference_persistence::PreferencePersistence::new(preferences);
        shell.launcher_preferences_path = Some(path);
    }

    fn finish_preference_write(shell: &mut LiveShell) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while shell.launcher_preferences_busy() {
            shell.poll_launcher_preferences();
            assert!(std::time::Instant::now() < deadline, "local preference worker did not finish");
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
    }







    #[test]
    fn controller_cancel_closes_nested_overlay_before_requesting_launcher_dismissal() {
        assert!(matches!(
            super::launcher_controller_host_event(ControllerAction::Cancel, true),
            HostEvent::Controller(ControllerAction::Cancel)
        ));
        assert!(matches!(
            super::launcher_controller_host_event(ControllerAction::Cancel, false),
            HostEvent::Shortcut(Shortcut::Escape)
        ));
        assert!(matches!(
            super::launcher_controller_host_event(ControllerAction::Down, false),
            HostEvent::Controller(ControllerAction::Down)
        ));
    }

    #[test]
    fn failed_application_launch_keeps_launcher_open_and_reports_error() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher_visible = true;
        let application = crate::model::Application::new(
            "org.example.missing".into(),
            "Missing application".into(),
            None,
            None,
            Some(vec!["nickel-test-command-that-does-not-exist".into()]),
        );

        shell.launch_application(application);

        assert!(shell.launcher_visible);
        let status = shell.launcher_status.as_deref().unwrap_or_default();
        assert!(status.starts_with("Could not launch Missing application: "));
        assert!(status.contains("No such file") || status.contains("not found"));
    }

    #[test]
    fn unavailable_shortcut_application_is_a_visible_typed_failure() {
        let mut shell = LiveShell::new().unwrap();

        assert!(!shell.launch_named_application("Missing Nickel Tool"));
        assert_eq!(
            shell.shortcut_action_status.as_deref(),
            Some("Missing Nickel Tool is unavailable.")
        );
    }



    #[test]
    fn plugin_retry_saves_failed_launcher_preferences_once() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let preferences_path = directory.path().join("launcher-preferences");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::default();
        preferences_fixture(&mut shell, directory.path().to_path_buf());
        shell.toggle_application_pin("firefox");
        finish_preference_write(&mut shell);
        assert!(shell.launcher_status.as_deref().is_some_and(|status| {
            status.starts_with("Launcher preferences could not be saved:")
        }));


        preferences_fixture(&mut shell, preferences_path.clone());
        assert!(shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::RetryApplicationPinSave
        ]));
        finish_preference_write(&mut shell);
        assert_eq!(shell.launcher_persistence_attempts, 2);
        assert!(shell.launcher_status.is_none());

        assert_eq!(
            LauncherPreferences::load(preferences_path)
                .expect("retried preferences")
                .favorites(),
            ["firefox"]
        );
        assert!(!shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::RetryApplicationPinSave
        ]));
        assert_eq!(shell.launcher_persistence_attempts, 2);
    }

    #[test]
    fn granted_plugin_can_pin_catalog_app_and_unpin_unavailable_app() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::default();
        shell.launcher.set_query("no-such-application");
        preferences_fixture(&mut shell, directory.path().join("launcher-preferences"));
        assert_eq!(shell.launcher.result_count(), 0);

        assert!(shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::ToggleApplicationPin {
                id: "firefox".into(),
            }
        ]));
        assert!(shell.launcher.is_pinned("firefox"));
        assert_eq!(shell.launcher_persistence_attempts, 1);
        assert!(!shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::ToggleApplicationPin {
                id: "org.example.missing".into(),
            }
        ]));
        assert_eq!(shell.launcher_persistence_attempts, 1);

        shell.launcher.toggle_pin("org.example.unavailable");
        assert!(shell.launcher.is_pinned("org.example.unavailable"));
        assert!(shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::ToggleApplicationPin {
                id: "org.example.unavailable".into(),
            }
        ]));
        assert!(!shell.launcher.is_pinned("org.example.unavailable"));
    }











    #[test]
    fn preview_window_menus_anchor_to_their_distinct_cards() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher.set_preferences(LauncherPreferences::default());
        let application = ApplicationId::new("org.example.Editor");
        shell.windows = [WindowId(71), WindowId(72)]
            .into_iter()
            .map(|id| OpenWindow {
                id,
                application_id: Some(application.clone()),
                active: id == WindowId(71),
                title: format!("Document {}", id.0),
                state: crate::model::WindowState::default(),
            })
            .collect();
        shell.panel_origin_x = 300;
        let _ = shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        shell.open_window_preview(0);
        let _ = shell.scene(SurfaceRole::WindowPreview, 640, 240);
        assert!(shell.preview_plugin_active());
        assert!(shell
            .preview_plugin_bounds(crate::window_preview::PreviewAction::Activate(WindowId(71)))
            .is_some());
        let thumbnails = shell
            .preview_thumbnail_bounds(&[WindowId(71), WindowId(72)])
            .expect("JSX preview provides thumbnail bounds");
        assert_eq!(thumbnails.len(), 2);
        let first_image = shell
            .preview_plugin_bounds(crate::window_preview::PreviewAction::Activate(WindowId(71)))
            .unwrap();
        assert_eq!(thumbnails[0].left, first_image.origin.x.floor() as i32);
        assert_eq!(thumbnails[0].top, first_image.origin.y.floor() as i32);
        assert!(thumbnails[1].left > thumbnails[0].right);
        let (preview_width, preview_height) = super::preview_dimensions(2);
        assert_eq!(
            shell.preview_geometry(),
            Some((
                shell.preview_origin_x(0, preview_width),
                shell.panel_origin_y,
                preview_width,
                preview_height,
            ))
        );

        shell.apply_preview_action(crate::window_preview::PreviewAction::OpenMenu(WindowId(71)));
        let first = shell.window_menu_anchor_x.expect("first card anchor");
        shell.apply_preview_action(crate::window_preview::PreviewAction::OpenMenu(WindowId(72)));
        let second = shell.window_menu_anchor_x.expect("second card anchor");

        assert!(second > first + 200, "each card must retain its own anchor");

        shell.window_menu = None;
        let card = shell
            .preview_plugin_bounds(crate::window_preview::PreviewAction::Activate(WindowId(71)))
            .expect("first preview card");
        let touch = Point {
            x: card.origin.x + card.size.width / 2.0,
            y: card.origin.y + card.size.height / 2.0,
        };
        shell.preview_plugin_event(
            HostEvent::Ui(UiEvent::TouchLongPress(touch)),
            (preview_width, preview_height),
            None,
        );
        assert_eq!(shell.window_menu, Some(WindowId(71)));
        assert_eq!(shell.window_menu_anchor_x, Some(first));
    }

    #[test]
    fn preview_geometry_and_thumbnail_bounds_follow_the_projected_card_limit() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher.set_preferences(LauncherPreferences::default());
        shell.windows = (1..=13)
            .map(|index| OpenWindow {
                id: WindowId(index),
                application_id: Some(ApplicationId::new("org.example.Editor")),
                active: index == 1,
                title: format!("Document {index}"),
                state: crate::model::WindowState::default(),
            })
            .collect();
        let _ = shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        shell.open_window_preview(0);
        let (_, _, width, height) = shell.preview_geometry().expect("preview is open");
        assert_eq!((width, height), super::preview_dimensions(12));
        let windows = (1..=12).map(WindowId).collect::<Vec<_>>();
        assert_eq!(shell.preview_thumbnail_bounds(&windows).unwrap().len(), 12);
        assert!(shell.preview_plugin_bounds(crate::window_preview::PreviewAction::Activate(WindowId(13))).is_none());
    }

    #[test]
    fn successful_window_command_consumes_and_dismisses_the_preview_menu() {
        let mut shell = LiveShell::new().unwrap();
        let window = OpenWindow {
            id: WindowId(73),
            application_id: Some(ApplicationId::new("org.example.Editor")),
            active: true,
            title: "Document".into(),
            state: crate::model::WindowState::default(),
        };
        shell.windows = vec![window.clone()];
        shell.preview_group = Some(0);
        shell.window_menu = Some(window.id);
        shell.window_menu_snapshot = Some(window.clone());

        shell.apply_window_menu_action(crate::window_preview::MenuAction::Close(window.id));

        assert!(shell.window_menu.is_none());
        assert!(shell.window_menu_snapshot.is_none());
        assert!(shell.preview_group.is_none());
    }

    #[test]
    fn lock_application_transfers_password_to_a_typed_authentication_effect() {
        let mut application = super::LockApplication {
            password: zeroize::Zeroizing::new(String::new()),
            status: None,
            palette: nickel_core::theme::ThemePalette::from_appearance(
                nickel_core::theme::Appearance::default(),
            ),
            effects: Vec::new(),
        };
        nickel_ui::Application::update(
            &mut application,
            super::LockMessage::Password("secret".into()),
        );
        assert!(nickel_ui::Application::shortcut_outcome(
            &mut application,
            nickel_ui::Shortcut::Submit
        )
        .changed);
        assert!(application.password.is_empty());
        let super::LockEffect::Authenticate(password) = application.effects.pop().unwrap();
        assert_eq!(&**password, "secret");
    }

    #[test]
    fn lock_password_is_a_named_protected_textbox_through_the_production_host() {
        let mut scenario = Scenario::new(super::LockApplication::fixture("nickel", None), 960, 540);
        let selector = Selector::RoleAndName {
            role: SemanticRole::TextField,
            name: "Password".into(),
        };
        let target = scenario
            .host()
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::TextField,
                name: "Password".into(),
            })
            .expect("lock password semantic target");

        assert_eq!(target.role, Some(SemanticRole::TextField));
        assert_eq!(target.name.as_deref(), Some("Password"));
        assert_eq!(target.actions, vec![ActionKind::SetValue]);
        scenario
            .assert_value(
                &selector,
                &SemanticValueSnapshot::ProtectedText { character_count: 6 },
            )
            .unwrap();
        scenario
            .set_value(&selector, SemanticValueInput::Text("new secret".into()))
            .unwrap()
            .assert_value(
                &selector,
                &SemanticValueSnapshot::ProtectedText {
                    character_count: 10,
                },
            )
            .unwrap();
    }

    #[test]
    fn project_displays_shortcut_opens_dedicated_projection_view() {
        let mut shell = LiveShell::new().unwrap();

        assert!(shell.global_shortcut(GlobalShortcut::ProjectDisplays));
        assert!(shell.control_visible);
        assert!(shell.control_host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Show desktop".into(),
        }).is_err());
    }
