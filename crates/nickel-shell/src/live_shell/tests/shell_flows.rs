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
        launcher_view::{LauncherAction, LauncherApplication},
        model::{ApplicationId, OpenWindow, TrayItem, WindowGroup, WindowId},
        notification::{NotificationAction, NotificationRequest, NotificationStore},
        window_preview::{MenuAction, build_preview_frame},
        winit_shell::SurfaceRole,
    };
    use nickel_core::launcher_preferences::LauncherPreferences;
    use nickel_core::theme::{Appearance, ThemeMode, ThemePalette};

    #[test]
    fn launcher_open_focuses_search_and_sequential_input_survives_mode_change() {
        let mut shell = LiveShell::new().unwrap();
        shell.apply_session_launcher_visibility(true);
        shell.launcher_host.step(HostBatch {
            surface_size: Some((920, 680)),
            ..HostBatch::default()
        });
        assert!(shell.launcher_host.inspect().keyboard_focus.is_some());
        shell.launcher_host.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::TextInput("a".into()))],
            ..HostBatch::default()
        });
        for action in shell.launcher_host.application_mut().take_effects() {
            shell.apply_launcher_action(action);
        }
        assert_eq!(shell.launcher.query(), "a");
        let status = shell.launcher_status_text();
        shell
            .launcher_host
            .application_mut()
            .sync(&shell.launcher, shell.palette, status);
        shell.launcher_host.step(HostBatch {
            events: vec![HostEvent::Ui(UiEvent::TextInput("b".into()))],
            ..HostBatch::default()
        });
        for action in shell.launcher_host.application_mut().take_effects() {
            shell.apply_launcher_action(action);
        }
        assert_eq!(shell.launcher.query(), "ab");
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

    fn launcher_application_menu_has_label(
        launcher: &crate::launcher::Launcher,
        palette: nickel_core::theme::ThemePalette,
        application_id: &str,
        expected_label: &str,
    ) -> bool {
        let mut host = UiHost::new(
            LauncherApplication::new(
                launcher.clone(),
                crate::launcher_view::LauncherViewState::default(),
                crate::launcher_view::LauncherIconCache::new(),
                palette,
            ),
            920,
            680,
        );
        let target = host
            .unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                application_id.to_owned(),
            ))
            .expect("application semantic target");
        let outcome = host.perform_accessibility_action(
            target.id.clone(),
            SemanticAction::Invoke(ActionKind::ContextMenu),
        );
        assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
        host.accessibility_nodes()
            .iter()
            .any(|node| node.label.as_deref() == Some(expected_label))
    }

    #[test]
    fn launcher_pin_persists_once_reopens_and_recovers_after_save_failure() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let preferences_path = directory.path().join("launcher-preferences");
        let mut shell = LiveShell::new().unwrap();
        let application_id = "org.nickel.Files".to_owned();
        shell.launcher = crate::launcher::Launcher::new(vec![crate::model::Application::new(
            application_id.clone(),
            "Files".into(),
            None,
            None,
            None,
        )]);
        shell.launcher_preferences_path = Some(preferences_path.clone());

        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        assert_eq!(shell.launcher_persistence_attempts, 1);
        assert!(shell.launcher.is_pinned(&application_id));
        let persisted = LauncherPreferences::load(&preferences_path).expect("persisted favorite");
        assert_eq!(persisted.favorites(), [application_id.as_str()]);

        let mut reopened =
            crate::launcher::Launcher::new(shell.launcher.applications().cloned().collect());
        reopened.set_preferences(persisted);
        assert!(reopened.is_pinned(&application_id));
        assert!(launcher_application_menu_has_label(
            &reopened,
            shell.palette,
            &application_id,
            "Unpin from Nickel Bar",
        ));

        shell.launcher_preferences_path = Some(directory.path().to_path_buf());
        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        assert_eq!(shell.launcher_persistence_attempts, 2);
        assert!(!shell.launcher.is_pinned(&application_id));
        assert!(
            shell.launcher_status.as_deref().is_some_and(
                |status| status.starts_with("Launcher preferences could not be saved:")
            )
        );
        assert!(launcher_application_menu_has_label(
            &shell.launcher,
            shell.palette,
            &application_id,
            "Pin to Nickel Bar",
        ));

        shell.launcher_preferences_path = Some(preferences_path.clone());
        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        assert_eq!(shell.launcher_persistence_attempts, 3);
        assert!(shell.launcher.is_pinned(&application_id));
        assert!(shell.launcher_status.is_none());
        assert_eq!(
            LauncherPreferences::load(preferences_path)
                .expect("recovered preferences")
                .favorites(),
            [application_id]
        );
    }

    fn launcher_scenario(
        launcher: &crate::launcher::Launcher,
        palette: nickel_core::theme::ThemePalette,
        status: Option<String>,
    ) -> Scenario<LauncherApplication> {
        let mut application = LauncherApplication::new(
            launcher.clone(),
            crate::launcher_view::LauncherViewState::default(),
            crate::launcher_view::LauncherIconCache::new(),
            palette,
        );
        application.sync(launcher, palette, status);
        Scenario::new(application, 920, 680)
    }

    fn application_context_target(
        scenario: &Scenario<LauncherApplication>,
        application_id: &str,
    ) -> Selector {
        let target = scenario
            .host()
            .unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                application_id.to_owned(),
            ))
            .expect("launcher application semantic target");
        Selector::id(target.id.as_str())
    }

    #[test]
    fn controller_scenario_pins_reopens_unpins_and_persists_each_action_once() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let preferences_path = directory.path().join("launcher-preferences");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::default();
        shell.launcher_preferences_path = Some(preferences_path.clone());
        let application_id = "firefox";

        let mut pin = launcher_scenario(&shell.launcher, shell.palette, None);
        let origin = application_context_target(&pin, application_id);
        pin.controller_semantic_action(&origin, ActionKind::ContextMenu)
            .expect("production controller context action opens application menu");
        pin.controller_activate(&Selector::role_name(
            SemanticRole::MenuItem,
            "Pin to Nickel Bar",
        ))
        .expect("controller reaches and confirms Pin");
        assert!(pin.host().inspect().open_overlay.is_none());
        assert_eq!(
            pin.host().inspect().controller_target.as_ref(),
            Some(
                &pin.host()
                    .query_unique(&SemanticSelector::Id(match &origin {
                        Selector::Id(id) => id.clone(),
                        _ => unreachable!(),
                    }))
                    .expect("origin remains present")
                    .id
            )
        );
        let effects = pin.host_mut().application_mut().take_effects();
        assert_eq!(effects, [LauncherAction::TogglePin(application_id.into())]);
        for effect in effects {
            shell.apply_launcher_action(effect);
        }
        assert_eq!(shell.launcher_persistence_attempts, 1);
        assert!(shell.launcher.is_pinned(application_id));
        assert_eq!(
            LauncherPreferences::load(&preferences_path)
                .expect("pin persisted")
                .favorites(),
            [application_id]
        );

        let mut unpin = launcher_scenario(&shell.launcher, shell.palette, None);
        let origin = application_context_target(&unpin, application_id);
        unpin
            .controller_semantic_action(&origin, ActionKind::ContextMenu)
            .expect("reopened menu uses authoritative favorite state");
        unpin
            .controller_activate(&Selector::role_name(
                SemanticRole::MenuItem,
                "Unpin from Nickel Bar",
            ))
            .expect("controller reaches and confirms Unpin");
        let effects = unpin.host_mut().application_mut().take_effects();
        assert_eq!(effects, [LauncherAction::TogglePin(application_id.into())]);
        for effect in effects {
            shell.apply_launcher_action(effect);
        }
        assert_eq!(shell.launcher_persistence_attempts, 2);
        assert!(!shell.launcher.is_pinned(application_id));
        assert!(
            LauncherPreferences::load(preferences_path)
                .expect("unpin persisted")
                .favorites()
                .is_empty()
        );
    }

    #[test]
    fn controller_scenario_logout_emits_only_the_typed_request() {
        let shell = LiveShell::new().unwrap();
        let mut scenario = launcher_scenario(&shell.launcher, shell.palette, None);
        let account = scenario
            .host()
            .unique_semantic_target_for_message(&LauncherAction::OpenAccount)
            .expect("account presentation semantic target");
        scenario
            .controller_semantic_action(&Selector::id(account.id.as_str()), ActionKind::ContextMenu)
            .expect("controller opens the shared account menu");
        scenario
            .controller_activate(&Selector::role_name(SemanticRole::MenuItem, "Log out"))
            .expect("controller reaches Logout");
        assert_eq!(
            scenario.host_mut().application_mut().take_effects(),
            [LauncherAction::RequestLogout]
        );
        assert!(scenario.host().inspect().open_overlay.is_none());
    }

    #[test]
    fn failed_pin_retry_is_idempotent_and_restores_origin_focus() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let valid_path = directory.path().join("launcher-preferences");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::default();
        let application_id = "firefox";
        shell.launcher_preferences_path = Some(directory.path().to_path_buf());

        shell.apply_launcher_action(LauncherAction::TogglePin(application_id.into()));
        assert_eq!(shell.launcher_persistence_attempts, 1);
        assert!(shell.launcher.is_pinned(application_id));
        let failure = shell
            .launcher_status
            .clone()
            .expect("truthful save failure");

        shell.launcher_preferences_path = Some(valid_path.clone());
        let mut retry = launcher_scenario(&shell.launcher, shell.palette, Some(failure));
        let origin = application_context_target(&retry, application_id);
        let origin_id = match &origin {
            Selector::Id(id) => id.clone(),
            _ => unreachable!(),
        };
        retry
            .controller_semantic_action(&origin, ActionKind::ContextMenu)
            .expect("failed menu remains controller-usable");
        retry
            .controller_activate(&Selector::role_name(
                SemanticRole::MenuItem,
                "Retry saving favorites",
            ))
            .expect("controller retries persistence without toggling state");
        assert!(retry.host().inspect().open_overlay.is_none());
        assert_eq!(
            retry.host().inspect().controller_target.as_ref(),
            Some(&origin_id),
            "closing the retry menu restores the originating application"
        );
        let effects = retry.host_mut().application_mut().take_effects();
        assert_eq!(effects, [LauncherAction::RetryPreferencePersistence]);
        for effect in effects {
            shell.apply_launcher_action(effect);
        }
        assert_eq!(shell.launcher_persistence_attempts, 2);
        assert!(shell.launcher.is_pinned(application_id));
        assert!(shell.launcher_status.is_none());
        assert_eq!(
            LauncherPreferences::load(valid_path)
                .expect("retry persisted unchanged authoritative state")
                .favorites(),
            [application_id]
        );
    }

    #[test]
    fn launcher_exposes_every_non_ready_secure_storage_state() {
        for (state, expected) in [
            (SecureStorageState::Starting, "Secure storage is starting…"),
            (SecureStorageState::Locked, "Secure storage is locked."),
            (
                SecureStorageState::PromptRequired,
                "Secure storage is waiting for its unlock prompt.",
            ),
            (
                SecureStorageState::Unavailable,
                "Secure storage is unavailable.",
            ),
            (
                SecureStorageState::UnavailableReason(
                    nickel_session_protocol::SecureStorageUnavailableReason::ProviderDisappeared,
                ),
                "The secure-storage provider disappeared.",
            ),
            (
                SecureStorageState::ControlUnavailable,
                "Nickel cannot reach the session service.",
            ),
        ] {
            assert_eq!(secure_storage_status_label(state), Some(expected));
        }
        assert_eq!(secure_storage_status_label(SecureStorageState::Ready), None);
    }

    #[test]
    fn session_feeds_start_loading_and_keep_failure_distinct_from_empty_ready() {
        let shell = LiveShell::new().unwrap();
        assert_eq!(shell.window_feed_status, FeedStatus::Loading);
        assert_eq!(shell.workspace_feed_status, FeedStatus::Loading);
        assert_eq!(
            session_feed_status_label(shell.window_feed_status, shell.workspace_feed_status),
            Some("Loading session data…")
        );

        assert_eq!(
            FeedState::<Vec<OpenWindow>>::Ready(Vec::new()).status(),
            FeedStatus::Ready
        );
        assert_eq!(
            FeedState::<Vec<OpenWindow>>::Disconnected.status(),
            FeedStatus::Disconnected
        );
        assert_eq!(
            FeedState::<Vec<OpenWindow>>::Failed.status(),
            FeedStatus::Failed
        );
        assert_eq!(
            session_feed_status_label(FeedStatus::Ready, FeedStatus::Ready),
            None
        );
        assert_eq!(
            session_feed_status_label(FeedStatus::Disconnected, FeedStatus::Ready),
            Some("Session window data is disconnected.")
        );
        assert_eq!(
            session_feed_status_label(FeedStatus::Failed, FeedStatus::Ready),
            Some("Session window data failed to load.")
        );
    }

    #[test]
    fn semantic_shell_targets_come_from_live_group_preview_and_menu_records() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .launcher
            .set_preferences(LauncherPreferences::default());
        let application_id = ApplicationId::new("org.kde.konsole");
        shell.windows = vec![
            OpenWindow {
                id: WindowId(4),
                application_id: Some(application_id.clone()),
                active: true,
                title: "one".into(),
                state: crate::model::WindowState::default(),
            },
            OpenWindow {
                id: WindowId(9),
                application_id: Some(application_id),
                active: false,
                title: "two".into(),
                state: crate::model::WindowState::default(),
            },
        ];
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);
        let panel = shell
            .resolve_semantic_target(&ShellSemanticTarget::PanelApplication {
                application_id: "org.kde.konsole".into(),
                output: Some("DP-1".into()),
                interaction: PointerInteraction::Hover,
            })
            .expect("live panel group resolves");
        assert_eq!(panel.role, ShellRole::Panel);
        assert_eq!(panel.output.as_deref(), Some("DP-1"));
        assert!(shell.panel_pointer_moved(panel.x as f32, 1280));
        assert_eq!(shell.panel_hover, Some(super::PanelHover::Task(0)));
        assert!(shell.preview_group.is_none());
        assert_eq!(shell.preview_pending.map(|(index, _)| index), Some(0));
        assert!(shell.preview_pending.unwrap().1 > Instant::now());

        let (preview_width, _) = super::preview_dimensions(2);
        assert_eq!(
            shell.preview_origin_x(0, preview_width),
            (panel.x - i32::try_from(preview_width / 2).unwrap()).max(shell.panel_origin_x)
        );

        let group = shell.launcher.group_windows(&shell.windows).remove(0);
        shell.preview_frame = Some(build_preview_frame(
            &group,
            &HashMap::new(),
            None,
            shell.semantic_theme(),
        ));
        let preview = shell
            .resolve_semantic_target(&ShellSemanticTarget::PreviewWindow {
                window: nickel_session_protocol::WindowId(9),
                action: PreviewTargetAction::Close,
            })
            .expect("live preview close target resolves");
        assert_eq!(preview.role, ShellRole::Preview);
        assert_eq!(preview.interaction, PointerInteraction::LeftClick);
        assert_eq!(
            shell.preview_frame.as_mut().unwrap().transition_pointer(
                Point {
                    x: preview.x as f32,
                    y: preview.y as f32,
                },
                false,
            ),
            Some(crate::window_preview::PreviewAction::Close(WindowId(9)))
        );

        shell.window_menu = Some(WindowId(9));
        let _ = shell.window_menu_scene();
        let menu = shell
            .resolve_semantic_target(&ShellSemanticTarget::WindowMenu {
                window: nickel_session_protocol::WindowId(9),
                action: WindowMenuTargetAction::Minimize,
            })
            .expect("live context-menu row resolves");
        assert_eq!(menu.role, ShellRole::ContextMenu);
        assert!(
            shell
                .window_menu_host
                .as_ref()
                .unwrap()
                .semantic_targets_for_message(&MenuAction::Minimize(WindowId(9)))
                .into_iter()
                .next()
                .is_some()
        );

        shell.screenshot.show(image::RgbaImage::new(400, 200));
        let _ = shell.scene(SurfaceRole::Screenshot, 800, 600);
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::SelectionStart));
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::SelectionEnd));
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::Confirm));
        assert!(shell.screenshot.confirmed());
    }

    #[test]
    fn taskbar_secondary_click_opens_menu_for_active_group_member_at_item_anchor() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .launcher
            .set_preferences(LauncherPreferences::default());
        let application_id = ApplicationId::new("org.kde.dolphin");
        shell.windows = vec![
            OpenWindow {
                id: WindowId(41),
                application_id: Some(application_id.clone()),
                active: false,
                title: "Files".into(),
                state: crate::model::WindowState::default(),
            },
            OpenWindow {
                id: WindowId(42),
                application_id: Some(application_id),
                active: true,
                title: "Downloads".into(),
                state: crate::model::WindowState::default(),
            },
        ];
        shell.panel_origin_x = 1_920;
        let _ = shell.scene(SurfaceRole::Panel, 1_280, 56);
        let target = shell
            .panel_host
            .unique_semantic_target_for_message(&super::PanelAction::Task(0))
            .expect("taskbar item");
        let expected_anchor = shell.panel_origin_x + target.bounds.origin.x.round() as i32;
        let center = target.bounds.origin.x + target.bounds.size.width / 2.0;

        assert!(shell.panel_click(center, 1_280, true));
        assert_eq!(shell.window_menu, Some(WindowId(42)));
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));
        assert!(shell.preview_group.is_none());
        assert_eq!(
            shell.window_menu_snapshot.as_ref().map(|window| window.id),
            Some(WindowId(42))
        );
        shell.windows[0].active = true;
        shell.windows[1].active = false;
        assert_eq!(
            shell.window_menu_snapshot.as_ref().map(|window| window.id),
            Some(WindowId(42)),
            "an open menu must not retarget when group activity changes"
        );
        shell.windows[0].active = false;
        shell.windows[1].active = true;

        shell.sync_transient_overlays();
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));

        shell.close_window_preview();
        let outcome = shell.panel_host.perform_accessibility_action(
            target.id.clone(),
            SemanticAction::Invoke(ActionKind::ContextMenu),
        );
        assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
        assert!(shell.apply_panel_effects());
        assert_eq!(shell.window_menu, Some(WindowId(42)));
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));

        for event in [
            HostEvent::Ui(UiEvent::KeyboardContextMenu),
            HostEvent::Controller(ControllerAction::ContextMenu),
        ] {
            shell.close_window_preview();
            shell.panel_host.step(HostBatch {
                events: vec![
                    HostEvent::Ui(UiEvent::AccessibilityFocus(target.id.clone())),
                    event,
                ],
                ..HostBatch::default()
            });
            assert!(shell.apply_panel_effects());
            assert_eq!(shell.window_menu, Some(WindowId(42)));
            assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));
        }
    }

    #[test]
    fn panel_popover_anchor_is_semantic_and_scoped_to_the_invoking_output() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Panel, 1_280, 56);
        shell.set_panel_output("left");
        let target = shell
            .panel_host
            .unique_semantic_target_for_message(&super::PanelAction::Control)
            .expect("control button");
        let expected = target.bounds;
        let outcome = shell
            .panel_host
            .perform_accessibility_action(target.id, SemanticAction::Invoke(ActionKind::Activate));
        assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
        assert!(shell.apply_panel_effects());
        let (role, first) = shell.popover_anchor(AnchorSide::Above).unwrap();
        assert_eq!(role, ShellRole::ControlCenter);
        assert_eq!(first.control, "panel-control");
        assert_eq!(first.output, "left");
        assert_eq!(first.bounds.x, expected.origin.x.floor() as i32);

        shell.set_panel_output("right");
        for _ in 0..2 {
            let target = shell
                .panel_host
                .unique_semantic_target_for_message(&super::PanelAction::Control)
                .unwrap();
            let outcome = shell.panel_host.perform_accessibility_action(
                target.id,
                SemanticAction::Invoke(ActionKind::Activate),
            );
            assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
            assert!(shell.apply_panel_effects());
        }
        let (_, reopened) = shell.popover_anchor(AnchorSide::Below).unwrap();
        assert_eq!(reopened.output, "right");
        assert_eq!(reopened.preferred, AnchorSide::Below);
        assert_eq!(reopened.bounds, first.bounds);
    }

    #[test]
    fn lock_application_transfers_password_to_a_typed_authentication_effect() {
        let mut application = super::LockApplication {
            password: zeroize::Zeroizing::new(String::new()),
            status: None,
            effects: Vec::new(),
        };
        nickel_ui::Application::update(
            &mut application,
            super::LockMessage::Password("secret".into()),
        );
        assert!(nickel_ui::Application::shortcut(
            &mut application,
            nickel_ui::Shortcut::Submit
        ));
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
    fn control_center_keyboard_navigation_uses_host_semantic_order() {
        let mut shell = LiveShell::new().unwrap();
        shell.control_visible = true;

        assert!(shell.control_key(Some(KeyCode::ArrowDown), 420, 600));
        assert!(shell.control_host.inspect().controller_target.is_some());
        assert!(shell.control_key(Some(KeyCode::ArrowUp), 420, 600));
        assert!(shell.control_host.inspect().controller_target.is_some());
        assert!(shell.control_key(Some(KeyCode::Escape), 420, 600));
        assert!(!shell.control_visible);
    }

    #[test]
    fn control_center_controller_dispatch_matches_keyboard_adapter() {
        let mut keyboard = LiveShell::new().unwrap();
        let mut controller = LiveShell::new().unwrap();
        keyboard.control_visible = true;
        controller.control_visible = true;

        assert!(controller.control_controller(nickel_ui::ControllerAction::Down, 420, 600));
        assert!(
            controller
                .control_host
                .inspect()
                .controller_target
                .is_some()
        );

        assert!(keyboard.control_key(Some(KeyCode::Escape), 420, 600));
        assert!(controller.control_controller(nickel_ui::ControllerAction::Cancel, 420, 600));
        assert_eq!(controller.control_visible, keyboard.control_visible);
    }

    #[test]
    fn project_displays_shortcut_opens_dedicated_projection_view() {
        let mut shell = LiveShell::new().unwrap();

        assert!(shell.global_shortcut(GlobalShortcut::ProjectDisplays));
        assert!(shell.control_visible);
        assert!(
            shell
                .control_host
                .semantic_targets_for_message(&ControlAction::ToggleShowDesktop)
                .is_empty(),
            "Super+P must not open the generic Control Center"
        );
    }

    #[test]
    fn transient_keyboard_navigation_uses_production_frame_order() {
        let mut shell = LiveShell::new().unwrap();
        let palette = nickel_core::theme::ThemePalette::from_appearance(Appearance::default());
        let group = WindowGroup {
            application_id: None,
            application_name: "Editor".into(),
            windows: vec![
                OpenWindow {
                    id: WindowId(4),
                    application_id: None,
                    active: true,
                    title: "one".into(),
                    state: crate::model::WindowState::default(),
                },
                OpenWindow {
                    id: WindowId(9),
                    application_id: None,
                    active: false,
                    title: "two".into(),
                    state: crate::model::WindowState::default(),
                },
            ],
        };
        shell.preview_group = Some(0);
        shell.preview_frame = Some(build_preview_frame(
            &group,
            &HashMap::new(),
            None,
            semantic_theme_from_palette(palette),
        ));

        assert!(shell.preview_key(Some(KeyCode::ArrowRight)));
        assert_eq!(shell.preview_hovered, Some(WindowId(9)));
        assert!(shell.preview_key(Some(KeyCode::ArrowLeft)));
        assert_eq!(shell.preview_hovered, Some(WindowId(4)));

        shell.window_menu = Some(WindowId(4));
        shell.window_menu_snapshot = Some(group.windows[0].clone());
        let _ = shell.window_menu_scene();
        assert!(shell.preview_key(Some(KeyCode::ArrowDown)));
        assert!(
            shell
                .window_menu_host
                .as_ref()
                .unwrap()
                .inspect()
                .controller_target
                .is_some()
        );
        let first_target = shell
            .window_menu_host
            .as_ref()
            .unwrap()
            .inspect()
            .controller_target
            .clone();
        assert!(!shell.window_menu_host_key(Some(KeyCode::ArrowUp)));
        assert_eq!(
            shell
                .window_menu_host
                .as_ref()
                .unwrap()
                .inspect()
                .controller_target,
            first_target
        );
        assert!(shell.window_menu_host_key(Some(KeyCode::ArrowDown)));
        assert_ne!(
            shell
                .window_menu_host
                .as_ref()
                .unwrap()
                .inspect()
                .controller_target,
            first_target
        );
    }

    #[test]
    fn notification_host_effects_stay_at_the_transport_boundary() {
        let mut shell = LiveShell::new().unwrap();
        let mut store = NotificationStore::default();
        store.notify(
            0,
            NotificationRequest {
                app_name: "Test".into(),
                summary: "Ready".into(),
                body: "Choose".into(),
                actions: vec![NotificationAction {
                    key: "open".into(),
                    label: "Open".into(),
                }],
                expire_timeout_ms: 0,
            },
            Instant::now(),
        );
        shell.notification = store.newest();
        let _ = shell.scene(SurfaceRole::Notification, 420, 180);
        let target = shell
            .notification_host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open".into(),
            })
            .unwrap();
        let point = Point {
            x: target.bounds.origin.x + target.bounds.size.width / 2.0,
            y: target.bounds.origin.y + target.bounds.size.height / 2.0,
        };

        assert!(shell.notification_click(point.x, point.y, 420, 180));
        assert!(shell.notification.is_none());
        assert!(
            shell
                .notification_host
                .query(&nickel_ui::SemanticSelector::Role(
                    nickel_ui::SemanticRole::Dialog
                ))
                .is_empty()
        );
    }

    #[test]
    fn notification_controller_cancel_uses_the_typed_host_effect() {
        let mut shell = LiveShell::new().unwrap();
        let mut store = NotificationStore::default();
        store.notify(
            0,
            NotificationRequest {
                app_name: "Test".into(),
                summary: "Ready".into(),
                body: "Choose".into(),
                actions: vec![],
                expire_timeout_ms: 0,
            },
            Instant::now(),
        );
        shell.notification = store.newest();
        let _ = shell.scene(SurfaceRole::Notification, 420, 180);

        assert!(shell.notification_controller(ControllerAction::Cancel));
        assert!(shell.notification.is_none());
        assert!(
            shell
                .notification_host
                .query(&nickel_ui::SemanticSelector::Role(
                    nickel_ui::SemanticRole::Dialog
                ))
                .is_empty()
        );
    }

