    #[test]
    fn reopening_launcher_restores_default_dashboard_view() {
        let mut shell = LiveShell::new().unwrap();
        shell.apply_session_launcher_visibility(true);
        shell.apply_launcher_action(crate::launcher_view::LauncherAction::SetView(
            crate::launcher::LauncherView::Applications,
        ));
        assert_eq!(shell.launcher.view(), crate::launcher::LauncherView::Applications);

        shell.apply_session_launcher_visibility(false);
        assert_eq!(shell.launcher.view(), crate::launcher::LauncherView::Favorites);
        assert_eq!(
            shell.launcher_view.dashboard_narrow_page,
            crate::launcher_view::DashboardNarrowPage::Primary
        );

        shell.apply_session_launcher_visibility(true);
        assert_eq!(shell.launcher.view(), crate::launcher::LauncherView::Favorites);
    }

    #[test]
    fn disabling_plugin_retires_host_and_clears_reported_memory() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.plugin_panel_host.is_some());
        shell.scene(
            super::SurfaceRole::Panel,
            crate::plugin_panel::surface().width,
            crate::plugin_panel::surface().height,
        );
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.plugin_panel_host.is_none());
        let entry = shell.plugin_registry().get(id).unwrap();
        assert_eq!(entry.health, nickel_core::plugins::PluginHealth::Disabled);
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
    }

    #[test]
    fn notification_plugin_can_start_render_and_retire() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::notification_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.plugin_notification_host.is_some());
        shell.scene(super::SurfaceRole::Notification, 420, 180);
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.plugin_notification_host.is_none());
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
    }

    #[test]
    fn window_preview_plugin_reports_memory_and_can_be_disabled_or_enabled() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(Vec::new());
        shell.windows = vec![OpenWindow {
            id: WindowId(71),
            application_id: None,
            active: true,
            title: "Document".into(),
            state: Default::default(),
        }];
        let id = &crate::plugin_panel::window_preview_manifest().id;
        shell.open_window_preview(0);
        assert!(!shell.scene(SurfaceRole::WindowPreview, 300, 214).is_empty());
        assert!(shell.preview_plugin_active());
        let status = shell
            .plugin_status_snapshot()
            .plugins
            .into_iter()
            .find(|plugin| &plugin.id == id)
            .unwrap();
        assert!(status.desired_enabled);
        assert!(status.memory.native_ui_bytes.is_some_and(|bytes| bytes > 0));

        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(!shell.preview_plugin_active());
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
        assert!(!shell.scene(SurfaceRole::WindowPreview, 300, 214).is_empty());
        assert!(shell.preview_frame.is_some());

        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.preview_plugin_active());
        assert!(!shell.scene(SurfaceRole::WindowPreview, 300, 214).is_empty());
        assert!(shell.preview_frame.is_none());
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
        assert!(shell.preview_frame.is_none());
        let group = shell.task_switcher_group.clone().unwrap();
        let (projection, _) = shell.preview_plugin_projection(&group);
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
            .set_plugin_enabled(&crate::plugin_panel::window_preview_manifest().id, false)
            .unwrap();
        shell.task_switcher.apply(
            nickel_core::hotkeys::HotkeyAction::SwitchNext,
            &candidates,
        );
        shell.rebuild_task_switcher_preview();
        assert!(!shell.scene(SurfaceRole::WindowPreview, 474, 214).is_empty());
        assert!(!shell.preview_plugin_active());
        assert!(shell.preview_frame.is_some());
    }

    #[test]
    fn volume_osd_plugin_can_retire_and_restore_native_fallback() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::volume_osd_manifest().id;
        assert!(shell.plugin_volume_osd_host.is_some());
        shell.scene(SurfaceRole::VolumeOsd, 420, 96);
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.plugin_volume_osd_host.is_none());
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
        shell.scene(SurfaceRole::VolumeOsd, 420, 96);
        assert!(!shell.volume_osd_host.commands().is_empty());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.plugin_volume_osd_host.is_some());
    }

    #[test]
    fn control_center_plugin_renders_and_dispatches_typed_desktop_action() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.control_visible = true;
        assert!(
            shell.plugin_control_host.is_some(),
            "{:?}",
            shell.plugin_registry()
                .get(&crate::plugin_panel::control_center_manifest().id)
        );
        assert!(!shell.scene(SurfaceRole::ControlCenter, 420, 720).is_empty());
        assert!(shell
            .plugin_registry()
            .get(&crate::plugin_panel::control_center_manifest().id)
            .unwrap()
            .memory
            .native_ui_bytes
            .is_some());
        let button = shell
            .plugin_control_host
            .as_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Show desktop".into(),
            })
            .unwrap();
        assert!(shell
            .control_host_event(
                HostEvent::Ui(UiEvent::AccessibilityActivate(button.id)),
                (420, 720),
                None,
            )
            .changed);
        assert!(host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::ToggleShowDesktop
        )));
    }

    #[test]
    fn control_section_extension_renders_and_invokes_its_own_callback() {
        use nickel_core::plugins::{PluginPackage, PluginPackageDescriptor};
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-control-section"
        );
        let package = PluginPackage::load(directory).unwrap();
        let mut shell = LiveShell::new().unwrap();
        shell.plugin_registry.register(package.manifest.clone()).unwrap();
        shell.external_plugin_packages.insert(
            package.manifest.id.clone(),
            PluginPackageDescriptor {
                directory: directory.into(),
                manifest: package.manifest.clone(),
                source_digest: package.source_digest(),
            },
        );
        shell.set_plugin_enabled(&package.manifest.id, true).unwrap();
        assert!(shell
            .plugin_registry
            .get(&package.manifest.id)
            .unwrap()
            .memory
            .native_ui_bytes
            .unwrap()
            > 0);
        shell.control_visible = true;
        shell.scene(SurfaceRole::ControlCenter, 420, 720);
        let button = shell
            .plugin_control_host
            .as_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open".into(),
            })
            .unwrap();
        assert!(shell
            .control_host_event(
                HostEvent::Ui(UiEvent::AccessibilityActivate(button.id)),
                (420, 720),
                None,
            )
            .changed);
        assert!(shell.launcher_visible);
        shell.set_plugin_enabled(&package.manifest.id, false).unwrap();
        assert!(!shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::InvokeControlExtensionSection {
                plugin_id: package.manifest.id,
                id: "find-apps".into(),
            },
        ]));
    }

    #[test]
    fn control_center_plugin_rejects_stale_workspace_id() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.control_visible = true;
        shell.workspaces = vec![crate::platform::WorkspaceSummary { id: 12, active: true }];
        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::Control(
            ControlAction::SwitchWorkspace(99),
        )]);
        assert!(!host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::SwitchWorkspace(99)
        )));
    }

    #[test]
    fn control_center_plugin_can_be_disabled_and_reenabled() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::control_center_manifest().id;
        assert!(shell.plugin_control_host.is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.plugin_control_host.is_none());
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
        assert!(!shell.scene(SurfaceRole::ControlCenter, 420, 720).is_empty());
        assert!(!shell.control_host.commands().is_empty());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.plugin_control_host.is_some());
    }

    #[test]
    fn control_center_plugin_confirms_session_action_in_component_dialog() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.control_visible = true;
        shell.scene(SurfaceRole::ControlCenter, 420, 720);
        let suspend = shell
            .plugin_control_host
            .as_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Suspend".into(),
            })
            .unwrap();
        assert!(shell
            .control_host_event(
                HostEvent::Ui(UiEvent::AccessibilityActivate(suspend.id)),
                (420, 720),
                None,
            )
            .changed);
        assert!(!host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::SessionAction(
                crate::platform::SessionAction::Suspend
            )
        )));
        assert_eq!(
            shell.control_host.application().view_state().pending_session_action,
            Some(crate::platform::SessionAction::Suspend)
        );
        shell.scene(SurfaceRole::ControlCenter, 420, 720);
        let confirm = shell
            .plugin_control_host
            .as_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Confirm".into(),
            })
            .unwrap();
        assert!(shell
            .control_host_event(
                HostEvent::Ui(UiEvent::AccessibilityActivate(confirm.id)),
                (420, 720),
                None,
            )
            .changed);
        let commands = host.take_commands();
        assert!(commands.iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::SessionAction(
                crate::platform::SessionAction::Suspend
            )
        )), "command count: {}; pending: {:?}; plugin error: {:?}", commands.len(),
            shell.control_host.application().view_state().pending_session_action,
            shell.plugin_control_host.as_ref().unwrap().application().last_error());
    }

    #[test]
    fn notification_plugin_action_uses_the_host_reducer() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::notification_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        shell.set_plugin_enabled(id, true).unwrap();
        shell.notification_feed.notify_internal(NotificationRequest {
            app_name: "Test".into(),
            summary: "Ready".into(),
            body: "Choose".into(),
            actions: vec![NotificationAction {
                key: "open".into(),
                label: "Open".into(),
            }],
            expire_timeout_ms: 0,
        });
        shell.notification = shell.notification_feed.snapshot();
        shell.scene(SurfaceRole::Notification, 420, 180);
        let target = shell
            .plugin_notification_host
            .as_ref()
            .unwrap()
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
    }

    #[test]
    fn notification_plugin_cancel_dismisses_the_visible_notification() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::notification_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        shell.set_plugin_enabled(id, true).unwrap();
        shell.notification_feed.notify_internal(NotificationRequest {
            app_name: "Test".into(),
            summary: "Ready".into(),
            body: "Choose".into(),
            actions: Vec::new(),
            expire_timeout_ms: 0,
        });
        shell.notification = shell.notification_feed.snapshot();
        shell.scene(SurfaceRole::Notification, 420, 180);
        assert!(shell.notification_controller(ControllerAction::Cancel));
        assert!(shell.notification.is_none());
    }

    #[test]
    fn notification_plugin_closes_history_from_its_component() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::notification_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        shell.set_plugin_enabled(id, true).unwrap();
        shell.notification_history_visible = true;
        shell.scene(SurfaceRole::Notification, 420, 180);
        let target = shell
            .plugin_notification_host
            .as_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Close".into(),
            })
            .unwrap();
        let point = Point {
            x: target.bounds.origin.x + target.bounds.size.width / 2.0,
            y: target.bounds.origin.y + target.bounds.size.height / 2.0,
        };
        assert!(shell.notification_click(point.x, point.y, 420, 180));
        assert!(!shell.notification_history_visible);
    }

    #[test]
    fn run_plugin_can_start_render_and_retire() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::run_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.plugin_run_host.is_some());
        shell.apply_session_launcher_visibility(true);
        assert!(shell.set_run_visible(true));
        assert!(shell.plugin_run_host.as_ref().unwrap().inspect().keyboard_focus.is_some());
        shell.scene(super::SurfaceRole::Launcher, 620, 180);
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.plugin_run_host.is_none());
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
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
        launcher_view::{LauncherAction, LauncherApplication},
        model::{ApplicationId, OpenWindow, TrayItem, WindowGroup, WindowId},
        notification::{NotificationAction, NotificationRequest, NotificationStore},
        window_preview::{MenuAction, build_preview_frame},
        winit_shell::SurfaceRole,
    };
    use nickel_core::launcher_preferences::LauncherPreferences;
    use nickel_core::theme::{Appearance, ThemeMode, ThemePalette};

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
    fn launcher_open_focuses_search_and_sequential_input_survives_mode_change() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::launcher_manifest().id, false)
            .unwrap();
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
    fn first_launcher_open_accepts_typing_after_native_scene_layout() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::launcher_manifest().id, false)
            .unwrap();
        shell.launcher.set_codex_available(true);
        shell.apply_session_launcher_visibility(true);
        shell.scene(SurfaceRole::Launcher, 960, 720);
        shell.launcher_host_ui(UiEvent::TextInput("konsole".into()), 960, 720);
        assert_eq!(shell.launcher.query(), "konsole");
    }

    #[test]
    fn reopening_launcher_replaces_retained_child_focus_with_search() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::launcher_manifest().id, false)
            .unwrap();
        shell.apply_session_launcher_visibility(true);
        shell.scene(SurfaceRole::Launcher, 920, 680);
        let non_search = shell
            .launcher_host
            .unique_semantic_target_for_message(&LauncherAction::SetView(
                crate::launcher::LauncherView::Applications,
            ))
            .expect("launcher navigation target");
        assert!(shell.launcher_host.request_focus(non_search.id).changed);
        shell.apply_session_launcher_visibility(false);
        shell.apply_session_launcher_visibility(true);
        let search = shell
            .launcher_host
            .query_unique(&nickel_ui::SemanticSelector::Role(
                nickel_ui::SemanticRole::TextField,
            ))
            .expect("launcher search field");
        assert_eq!(
            shell.launcher_host.inspect().keyboard_focus,
            Some(search.id)
        );
        shell.launcher_host_ui(UiEvent::TextInput("files".into()), 920, 680);
        assert_eq!(shell.launcher.query(), "files");
    }

    #[test]
    fn launcher_submit_opens_the_keyboard_focused_dashboard_project() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::launcher_manifest().id, false)
            .unwrap();
        shell.launcher.set_codex_available(true);
        shell.set_dashboard_projects(crate::launcher::DashboardSection::Ready(vec![
            crate::launcher::DashboardProject {
                id: "nickel".into(),
                name: "Nickel".into(),
                roots: Vec::new(),
                chat_count: Some(1),
                activity: crate::launcher::ProjectActivity::Idle,
                // Recent-project navigation only exposes projects with activity.
                last_used_at: Some(1),
            },
        ]));
        shell.apply_session_launcher_visibility(true);
        shell.launcher_host_event_with_clipboard_limit(HostEvent::Poll, 920, 680, None);
        let target = shell
            .launcher_host
            .unique_semantic_target_for_message(&LauncherAction::OpenProject("nickel".into()))
            .expect("Nickel project row");
        for event in [UiEvent::KeyboardNavigateDown, UiEvent::KeyboardNavigateLeft] {
            shell.launcher_host_ui(event, 920, 680);
        }
        for _ in 0..7 {
            shell.launcher_host_ui(UiEvent::KeyboardNavigateDown, 920, 680);
        }
        assert_eq!(shell.launcher_host.inspect().controller_target, Some(target.id));
        assert!(shell.take_requested_codex_project().is_none());
        shell.shell_role_host_shortcut(SurfaceRole::Launcher, Shortcut::Submit, 920, 680);
        assert_eq!(shell.take_requested_codex_project().as_deref(), Some("nickel"));
    }

    #[test]
    fn launcher_submit_dispatches_the_keyboard_focused_dashboard_application() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::launcher_manifest().id, false)
            .unwrap();
        shell.launcher = crate::launcher::Launcher::new(vec![crate::model::Application::new(
            "org.kde.konsole".into(),
            "Konsole".into(),
            None,
            None,
            Some(vec!["nickel-test-command-that-does-not-exist".into()]),
        )]);
        shell.apply_session_launcher_visibility(true);
        shell.launcher_host_event_with_clipboard_limit(HostEvent::Poll, 920, 680, None);
        let target = shell
            .launcher_host
            .unique_semantic_target_for_message(&LauncherAction::LaunchApplication(
                "org.kde.konsole".into(),
            ))
            .expect("Konsole application row");
        for event in [
            UiEvent::KeyboardNavigateDown,
            UiEvent::KeyboardNavigateRight,
        ] {
            shell.launcher_host_ui(event, 920, 680);
        }
        assert_eq!(shell.launcher_host.inspect().controller_target, Some(target.id));
        assert!(!shell.launcher_status.as_deref().unwrap_or_default().starts_with("Could not launch Konsole: "));
        shell.shell_role_host_shortcut(SurfaceRole::Launcher, Shortcut::Submit, 920, 680);
        // An intentionally unavailable executable proves the production launch
        // action ran without spawning a real application during this test.
        assert!(
            shell.launcher_status.as_deref().unwrap_or_default()
                .starts_with("Could not launch Konsole: ")
        );
    }

    #[test]
    fn plugin_launcher_submit_activates_the_focused_search_result() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(vec![
            crate::model::Application::new(
                "org.nickel.demo-one".into(),
                "Demo One".into(),
                None,
                None,
                Some(vec!["nickel-test-command-one-does-not-exist".into()]),
            ),
            crate::model::Application::new(
                "org.nickel.demo-two".into(),
                "Demo Two".into(),
                None,
                None,
                Some(vec!["nickel-test-command-two-does-not-exist".into()]),
            ),
        ]);
        shell.launcher.set_query("demo");
        let id = &crate::plugin_panel::launcher_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        shell.set_plugin_enabled(id, true).unwrap();
        shell.apply_session_launcher_visibility(true);
        shell.scene(SurfaceRole::Launcher, 920, 680);
        let host = shell.plugin_launcher_host.as_mut().unwrap();
        let target = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Demo Two".into(),
            })
            .unwrap();
        host.request_focus(target.id.clone());
        shell.shell_role_host_shortcut(SurfaceRole::Launcher, Shortcut::Submit, 920, 680);
        assert!(
            shell
                .launcher_status
                .as_deref()
                .unwrap_or_default()
                .starts_with("Could not launch Demo Two: "),
            "launcher status: {:?}",
            shell.launcher_status
        );
    }

    #[test]
    fn plugin_launcher_focuses_search_and_accepts_first_input() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::launcher_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        shell.set_plugin_enabled(id, true).unwrap();
        shell.apply_session_launcher_visibility(true);
        shell.scene(SurfaceRole::Launcher, 920, 680);
        let host = shell.plugin_launcher_host.as_ref().unwrap();
        let search = host
            .query_unique(&nickel_ui::SemanticSelector::Role(
                nickel_ui::SemanticRole::TextField,
            ))
            .unwrap();
        assert_eq!(host.inspect().keyboard_focus, Some(search.id));
        shell.launcher_host_ui(UiEvent::TextInput("konsole".into()), 920, 680);
        assert_eq!(shell.launcher.query(), "konsole");
    }

    #[test]
    fn plugin_launcher_submit_activates_the_focused_dashboard_application() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(vec![
            crate::model::Application::new(
                "org.nickel.demo-one".into(),
                "Demo One".into(),
                None,
                None,
                Some(vec!["nickel-test-command-one-does-not-exist".into()]),
            ),
            crate::model::Application::new(
                "org.nickel.demo-two".into(),
                "Demo Two".into(),
                None,
                None,
                Some(vec!["nickel-test-command-two-does-not-exist".into()]),
            ),
        ]);
        let id = &crate::plugin_panel::launcher_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        shell.set_plugin_enabled(id, true).unwrap();
        shell.apply_session_launcher_visibility(true);
        shell.scene(SurfaceRole::Launcher, 920, 680);
        let host = shell.plugin_launcher_host.as_mut().unwrap();
        let target = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Demo Two".into(),
            })
            .unwrap();
        host.request_focus(target.id);
        shell.shell_role_host_shortcut(SurfaceRole::Launcher, Shortcut::Submit, 920, 680);
        assert!(
            shell
                .launcher_status
                .as_deref()
                .unwrap_or_default()
                .starts_with("Could not launch Demo Two: "),
            "launcher status: {:?}",
            shell.launcher_status
        );
    }

    #[test]
    fn plugin_launcher_displays_live_host_status() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::launcher_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        shell.set_plugin_enabled(id, true).unwrap();
        shell.launcher_status = Some("Could not launch Demo".into());
        shell.scene(SurfaceRole::Launcher, 920, 680);
        let projection = shell.current_plugin_launcher_projection();
        assert_eq!(projection.status.as_deref(), Some("Could not launch Demo"));
        assert!(!shell
            .plugin_launcher_host
            .as_ref()
            .unwrap()
            .query(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Text,
                name: "Could not launch Demo".into(),
            })
            .is_empty());
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
        preferences_fixture(&mut shell, preferences_path.clone());

        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        finish_preference_write(&mut shell);
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

        preferences_fixture(&mut shell, directory.path().to_path_buf());
        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        finish_preference_write(&mut shell);
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

        preferences_fixture(&mut shell, preferences_path.clone());
        shell.apply_launcher_action(crate::launcher_view::LauncherAction::TogglePin(
            application_id.clone(),
        ));
        finish_preference_write(&mut shell);
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
        preferences_fixture(&mut shell, preferences_path.clone());
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
        finish_preference_write(&mut shell);
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
        finish_preference_write(&mut shell);
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
        preferences_fixture(&mut shell, directory.path().to_path_buf());

        shell.apply_launcher_action(LauncherAction::TogglePin(application_id.into()));
        finish_preference_write(&mut shell);
        assert_eq!(shell.launcher_persistence_attempts, 1);
        assert!(shell.launcher.is_pinned(application_id));
        let failure = shell
            .launcher_status
            .clone()
            .expect("truthful save failure");

        preferences_fixture(&mut shell, valid_path.clone());
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
        finish_preference_write(&mut shell);
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
            .set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, false)
            .unwrap();
        shell
            .launcher
            .set_preferences(LauncherPreferences::default());
        let application_id = ApplicationId::new("org.nickel.Terminal");
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
        let _ = shell.scene(SurfaceRole::Taskbar, 1280, 56);
        let panel = shell
            .resolve_semantic_target(&ShellSemanticTarget::PanelApplication {
                application_id: "org.nickel.Terminal".into(),
                output: Some("DP-1".into()),
                interaction: PointerInteraction::Hover,
            })
            .expect("live panel group resolves");
        assert_eq!(panel.role, ShellRole::Panel);
        assert_eq!(panel.output.as_deref(), Some("DP-1"));
        assert!(shell.panel_pointer_moved(panel.x as f32, 1280));
        assert_eq!(shell.panel_hover, Some(super::TaskbarHover::Task(0)));
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
    fn plugin_taskbar_context_menu_uses_the_jsx_item_anchor() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::taskbar_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        shell.set_plugin_enabled(id, true).unwrap();
        shell.windows = vec![OpenWindow {
            id: WindowId(41),
            application_id: Some(ApplicationId::new("org.kde.dolphin")),
            active: true,
            title: "Files".into(),
            state: crate::model::WindowState::default(),
        }];
        shell.panel_origin_x = 1_920;
        shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        let index = shell
            .panel_groups()
            .iter()
            .position(|group| group.windows.iter().any(|window| window.id == WindowId(41)))
            .unwrap();
        let item = super::taskbar_plugin_control_bounds(
            shell.plugin_taskbar_host.as_ref().unwrap(),
            &format!("taskbar-item-{index}"),
        )
        .unwrap();
        let expected_x = shell.panel_origin_x + item.origin.x.round() as i32;
        let center = item.origin.x + item.size.width / 2.0;
        assert!(shell.panel_pointer_moved(center, 1_280));
        assert_eq!(shell.panel_hover, Some(super::TaskbarHover::Task(index)));
        assert!(shell.panel_click(center, 1_280, true));
        assert_eq!(shell.window_menu_anchor_x, Some(expected_x));
        assert_eq!(
            shell.application_menu_target.as_ref().map(|target| target.windows.clone()),
            Some(vec![WindowId(41)])
        );
    }

    #[test]
    fn taskbar_action_extension_renders_and_invokes_its_plugin_callback() {
        use nickel_core::plugins::{PluginPackage, PluginPackageDescriptor};
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-task-action"
        );
        let package = PluginPackage::load(directory).unwrap();
        let mut shell = LiveShell::new().unwrap();
        shell.plugin_registry.register(package.manifest.clone()).unwrap();
        shell.external_plugin_packages.insert(
            package.manifest.id.clone(),
            PluginPackageDescriptor {
                directory: directory.into(),
                manifest: package.manifest.clone(),
                source_digest: package.source_digest(),
            },
        );
        shell.set_plugin_enabled(&package.manifest.id, true).unwrap();
        let active = shell
            .plugin_status_snapshot()
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == package.manifest.id)
            .unwrap();
        assert!(active.memory.native_ui_bytes.unwrap() > 0);
        shell.windows = vec![OpenWindow {
            id: WindowId(81),
            application_id: Some(ApplicationId::new("org.nickel.mail")),
            active: true,
            title: "Mail".into(),
            state: crate::model::WindowState::default(),
        }];
        shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        let index = shell.panel_groups().iter().position(|group|
            group.windows.iter().any(|window| window.id == WindowId(81))
        ).unwrap();
        let bounds = super::taskbar_plugin_control_bounds(
            shell.plugin_taskbar_host.as_ref().unwrap(),
            &format!("taskbar-item-{index}"),
        ).unwrap();
        assert!(shell.panel_click(bounds.origin.x + bounds.size.width / 2.0, 1_280, true));
        let height = shell.window_context_menu_height() as u32;
        shell.scene(SurfaceRole::WindowContextMenu, super::MENU_WIDTH as u32, height);
        let action = shell.application_menu_plugin_host.as_ref().unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Find apps".into(),
            }).unwrap();
        assert!(action.bounds.origin.y + action.bounds.size.height <= height as f32);
        assert!(shell.window_menu_host_event(
            HostEvent::Ui(UiEvent::AccessibilityActivate(action.id)),
            super::MENU_WIDTH as u32,
            height,
        ));
        assert!(shell.launcher_visible);
        assert!(shell.application_menu_target.is_none());
        shell.set_plugin_enabled(&package.manifest.id, false).unwrap();
        let disabled = shell
            .plugin_status_snapshot()
            .plugins
            .into_iter()
            .find(|plugin| plugin.id == package.manifest.id)
            .unwrap();
        assert!(disabled.memory.native_ui_bytes.is_none());
        assert!(!shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::InvokeTaskbarExtensionAction {
                plugin_id: package.manifest.id.clone(),
                id: "find-apps".into(),
                application_id: Some("org.nickel.mail".into()),
            },
        ]));
    }

    #[test]
    fn plugin_taskbar_menu_pins_the_captured_application_and_retires() {
        let directory = tempfile::tempdir().unwrap();
        let mut shell = LiveShell::new().unwrap();
        preferences_fixture(&mut shell, directory.path().join("launcher.json"));
        let application_id = ApplicationId::new("org.nickel.menu-test");
        shell.windows = vec![OpenWindow {
            id: WindowId(41),
            application_id: Some(application_id.clone()),
            active: true,
            title: "Menu test".into(),
            state: crate::model::WindowState::default(),
        }];
        shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        let index = shell
            .panel_groups()
            .iter()
            .position(|group| group.windows.iter().any(|window| window.id == WindowId(41)))
            .unwrap();
        let bounds = super::taskbar_plugin_control_bounds(
            shell.plugin_taskbar_host.as_ref().unwrap(),
            &format!("taskbar-item-{index}"),
        )
        .unwrap();
        assert!(shell.panel_click(bounds.origin.x + bounds.size.width / 2.0, 1_280, true));
        let menu_height = shell.window_context_menu_height() as u32;
        assert!(!shell
            .scene(SurfaceRole::WindowContextMenu, super::MENU_WIDTH as u32, menu_height)
            .is_empty());
        assert!(shell.application_menu_host.is_none());
        let taskbar_only = shell.plugin_taskbar_memory.values().copied().sum::<u64>();
        assert!(
            shell
                .plugin_registry
                .get(&crate::plugin_panel::taskbar_manifest().id)
                .unwrap()
                .memory
                .native_ui_bytes
                .unwrap()
                > taskbar_only
        );
        let menu = shell.application_menu_plugin_host.as_ref().unwrap();
        let pin = menu
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Pin to Nickel Bar".into(),
            })
            .unwrap();
        assert!(shell.window_menu_host_event(
            HostEvent::Ui(UiEvent::AccessibilityActivate(pin.id)),
            super::MENU_WIDTH as u32,
            menu_height,
        ));
        assert!(shell.launcher.is_pinned(application_id.as_str()));
        assert!(shell.application_menu_target.is_none());
        assert!(shell.application_menu_plugin_host.is_none());
        assert_eq!(
            shell
                .plugin_registry
                .get(&crate::plugin_panel::taskbar_manifest().id)
                .unwrap()
                .memory
                .native_ui_bytes,
            Some(taskbar_only)
        );
    }

    #[test]
    fn plugin_taskbar_menu_close_all_uses_captured_window_authority() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.windows = vec![OpenWindow {
            id: WindowId(41),
            application_id: Some(ApplicationId::new("org.nickel.menu-close")),
            active: true,
            title: "Close test".into(),
            state: crate::model::WindowState::default(),
        }];
        shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        let index = shell
            .panel_groups()
            .iter()
            .position(|group| group.windows.iter().any(|window| window.id == WindowId(41)))
            .unwrap();
        let bounds = super::taskbar_plugin_control_bounds(
            shell.plugin_taskbar_host.as_ref().unwrap(),
            &format!("taskbar-item-{index}"),
        )
        .unwrap();
        shell.panel_click(bounds.origin.x + bounds.size.width / 2.0, 1_280, true);
        let menu_height = shell.window_context_menu_height() as u32;
        shell.scene(
            SurfaceRole::WindowContextMenu,
            super::MENU_WIDTH as u32,
            menu_height,
        );
        let close = shell
            .application_menu_plugin_host
            .as_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Close all windows".into(),
            })
            .unwrap();
        assert!(close.bounds.origin.y + close.bounds.size.height <= menu_height as f32);
        host.take_commands();
        shell.window_menu_host_event(
            HostEvent::Ui(UiEvent::AccessibilityActivate(close.id)),
            super::MENU_WIDTH as u32,
            menu_height,
        );
        assert!(host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::WindowAction {
                window: WindowId(41),
                action: crate::platform::WindowAction::Close,
            }
        )));
    }

    #[test]
    fn plugin_taskbar_window_menu_dispatches_validated_close() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        let window = OpenWindow {
            id: WindowId(71),
            application_id: Some(ApplicationId::new("org.nickel.window-menu")),
            active: true,
            title: "Window menu test".into(),
            state: crate::model::WindowState::default(),
        };
        shell.windows = vec![window.clone()];
        shell.window_menu = Some(window.id);
        shell.window_menu_snapshot = Some(window);
        let height = shell.window_context_menu_height() as u32;
        assert!(!shell.window_menu_scene().is_empty());
        assert!(shell.window_menu_host.is_none());
        let menu = shell.window_menu_plugin_host.as_ref().unwrap();
        let target = shell
            .resolve_semantic_target(&ShellSemanticTarget::WindowMenu {
                window: nickel_session_protocol::WindowId(71),
                action: WindowMenuTargetAction::Close,
            })
            .expect("JSX window menu close target");
        assert_eq!(target.role, ShellRole::ContextMenu);
        let close = menu
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Close Window".into(),
            })
            .unwrap();
        assert!(shell.window_menu_host_event(
            HostEvent::Ui(UiEvent::AccessibilityActivate(close.id)),
            super::MENU_WIDTH as u32,
            height,
        ));
        assert!(host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::WindowAction {
                window: WindowId(71),
                action: crate::platform::WindowAction::Close,
            }
        )));
        assert!(shell.window_menu_plugin_host.is_none());
    }

    #[test]
    fn plugin_taskbar_window_menu_rejects_reused_window_identity() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        let captured = OpenWindow {
            id: WindowId(71),
            application_id: Some(ApplicationId::new("org.nickel.original")),
            active: true,
            title: "Original".into(),
            state: crate::model::WindowState::default(),
        };
        shell.window_menu = Some(captured.id);
        shell.window_menu_snapshot = Some(captured.clone());
        shell.windows = vec![OpenWindow {
            application_id: Some(ApplicationId::new("org.nickel.replacement")),
            ..captured
        }];
        let close_index = crate::window_preview::window_menu_entries(
            shell.window_menu_snapshot.as_ref().unwrap(),
            &shell.workspaces,
            &shell.window_feed.outputs(),
        )
        .iter()
        .position(|(_, action)| matches!(action, MenuAction::Close(_)))
        .unwrap();
        shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::InvokeTaskbarWindowMenu {
                page: "root".into(),
                index: close_index,
            },
        ]);
        assert!(!host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::WindowAction { .. }
        )));
    }

    #[test]
    fn plugin_taskbar_window_menu_navigates_to_workspace_action() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        let mut window = OpenWindow {
            id: WindowId(72),
            application_id: Some(ApplicationId::new("org.nickel.workspace-menu")),
            active: true,
            title: "Workspace menu test".into(),
            state: crate::model::WindowState::default(),
        };
        window.state.capabilities.move_workspace = true;
        shell.workspaces = vec![
            crate::platform::WorkspaceSummary { id: 10, active: true },
            crate::platform::WorkspaceSummary { id: 20, active: false },
        ];
        shell.windows = vec![window.clone()];
        shell.window_menu = Some(window.id);
        shell.window_menu_snapshot = Some(window);
        let height = shell.window_context_menu_height() as u32;
        shell.window_menu_scene();
        let menu = shell.window_menu_plugin_host.as_ref().unwrap();
        let navigate = menu
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Move to Workspace ›".into(),
            })
            .unwrap();
        assert!(shell.window_menu_host_event(
            HostEvent::Ui(UiEvent::AccessibilityActivate(navigate.id)),
            super::MENU_WIDTH as u32,
            height,
        ));
        let menu = shell.window_menu_plugin_host.as_ref().unwrap();
        let destination = menu
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Workspace 2".into(),
            })
            .unwrap();
        assert!(shell.window_menu_host_event(
            HostEvent::Ui(UiEvent::AccessibilityActivate(destination.id)),
            super::MENU_WIDTH as u32,
            height,
        ));
        assert!(host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::MoveWindowToWorkspace {
                window: WindowId(72),
                workspace: 20,
            }
        )));
    }

    #[test]
    fn taskbar_secondary_click_opens_application_menu_for_captured_group_at_item_anchor() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, false)
            .unwrap();
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
        let _ = shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        let target = shell
            .panel_host
            .unique_semantic_target_for_message(&super::TaskbarAction::Task(0))
            .expect("taskbar item");
        let expected_anchor = shell.panel_origin_x + target.bounds.origin.x.round() as i32;
        let center = target.bounds.origin.x + target.bounds.size.width / 2.0;

        assert!(shell.panel_click(center, 1_280, true));
        assert!(shell.window_menu.is_none());
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));
        assert!(shell.preview_group.is_none());
        assert_eq!(
            shell
                .application_menu_target
                .as_ref()
                .map(|target| target.windows.clone()),
            Some(vec![WindowId(41), WindowId(42)])
        );
        shell.windows[0].active = true;
        shell.windows[1].active = false;
        assert_eq!(
            shell
                .application_menu_target
                .as_ref()
                .map(|target| target.windows.clone()),
            Some(vec![WindowId(41), WindowId(42)]),
            "an open menu must not recapture membership when group activity changes"
        );
        shell.windows[0].active = false;
        shell.windows[1].active = true;

        shell.sync_transient_overlays();
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));
        assert_eq!(
            shell.window_menu_geometry(),
            Some((
                expected_anchor,
                shell.panel_origin_y,
                super::MENU_WIDTH.ceil() as u32,
                shell.window_context_menu_height() as u32,
            ))
        );

        shell.close_window_preview();
        let outcome = shell.panel_host.perform_accessibility_action(
            target.id.clone(),
            SemanticAction::Invoke(ActionKind::ContextMenu),
        );
        assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
        assert!(shell.apply_panel_effects());
        assert!(shell.application_menu_target.is_some());
        assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));

        for event in [
            HostEvent::Ui(UiEvent::KeyboardContextMenu),
            HostEvent::Controller(ControllerAction::ContextMenu),
            HostEvent::Ui(UiEvent::TouchLongPress(Point {
                x: center,
                y: target.bounds.origin.y + target.bounds.size.height / 2.0,
            })),
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
            assert!(shell.application_menu_target.is_some());
            assert_eq!(shell.window_menu_anchor_x, Some(expected_anchor));
        }
    }

    #[test]
    fn taskbar_primary_click_activates_the_topmost_group_window_without_opening_previews() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, false)
            .unwrap();
        shell
            .launcher
            .set_preferences(LauncherPreferences::default());
        let application_id = ApplicationId::new("org.kde.konsole");
        shell.windows = [WindowId(41), WindowId(42)]
            .into_iter()
            .map(|id| OpenWindow {
                id,
                application_id: Some(application_id.clone()),
                active: id == WindowId(42),
                title: format!("Terminal {}", id.0),
                state: crate::model::WindowState::default(),
            })
            .collect();
        let _ = shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        let target = shell
            .panel_host
            .unique_semantic_target_for_message(&super::TaskbarAction::Task(0))
            .expect("taskbar item");
        let center = target.bounds.origin.x + target.bounds.size.width / 2.0;

        assert!(shell.panel_click(center, 1_280, false));
        assert!(shell.preview_group.is_none());
        let commands = host.take_commands();
        assert!(commands.iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::WindowAction {
                window: WindowId(42),
                action: crate::platform::WindowAction::Activate,
            }
        )));
        assert!(!commands.iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::ShowPreview { .. }
        )));
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
    fn panel_popover_anchor_is_semantic_and_scoped_to_the_invoking_output() {
        let mut shell = LiveShell::new().unwrap();
        shell.set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, false).unwrap();
        let _ = shell.scene(SurfaceRole::Taskbar, 1_280, 56);
        shell.set_panel_output("left");
        let target = shell
            .panel_host
            .unique_semantic_target_for_message(&super::TaskbarAction::Control)
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
                .unique_semantic_target_for_message(&super::TaskbarAction::Control)
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
    fn control_center_keyboard_navigation_uses_host_semantic_order() {
        let mut shell = LiveShell::new().unwrap();
        shell.control_visible = true;

        assert!(shell.control_key(Some(KeyCode::ArrowDown), 420, 600));
        assert!(shell.plugin_control_host.as_ref().unwrap().inspect().controller_target.is_some());
        assert!(shell.control_key(Some(KeyCode::ArrowDown), 420, 600));
        assert!(shell.control_key(Some(KeyCode::ArrowUp), 420, 600));
        assert!(shell.plugin_control_host.as_ref().unwrap().inspect().controller_target.is_some());
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
                .plugin_control_host
                .as_ref()
                .unwrap()
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
        shell.plugin_preview_host = None;
        shell
            .set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, false)
            .unwrap();
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
        shell
            .set_plugin_enabled(&crate::plugin_panel::notification_manifest().id, false)
            .unwrap();
        shell.notification_feed.notify_internal(NotificationRequest {
            app_name: "Test".into(),
            summary: "Ready".into(),
            body: "Choose".into(),
            actions: vec![NotificationAction {
                key: "open".into(),
                label: "Open".into(),
            }],
            expire_timeout_ms: 0,
        });
        shell.notification = shell.notification_feed.snapshot();
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
        shell
            .set_plugin_enabled(&crate::plugin_panel::notification_manifest().id, false)
            .unwrap();
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

    #[test]
    fn compositor_owned_notification_ui_uses_production_effect_reducer() {
        let mut shell = LiveShell::new().unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::notification_manifest().id, false)
            .unwrap();
        shell.notification_feed.notify_internal(NotificationRequest {
            app_name: "Test".into(),
            summary: "Ready".into(),
            body: "Choose".into(),
            actions: vec![NotificationAction {
                key: "open".into(),
                label: "Open".into(),
            }],
            expire_timeout_ms: 0,
        });
        shell.notification = shell.notification_feed.snapshot();
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

        assert!(shell.shell_role_host_ui(
            SurfaceRole::Notification,
            UiEvent::PointerPressed(point),
            420,
            180,
        ));
        assert!(shell.shell_role_host_ui(
            SurfaceRole::Notification,
            UiEvent::PointerReleased(point),
            420,
            180,
        ));
        assert!(shell.notification.is_none());
    }

    #[test]
    fn compositor_owned_control_center_ui_updates_the_production_host() {
        let mut shell = LiveShell::new().unwrap();
        shell.control_visible = true;
        let _ = shell.scene(SurfaceRole::ControlCenter, 420, 600);
        let target = shell
            .plugin_control_host
            .as_ref()
            .unwrap()
            .query(&nickel_ui::SemanticSelector::Role(
                nickel_ui::SemanticRole::Button,
            ))
            .into_iter()
            .find(|target| target.name.as_deref() == Some("More"))
            .expect("visible control center section button");
        let point = Point {
            x: target.bounds.origin.x + target.bounds.size.width / 2.0,
            y: target.bounds.origin.y + target.bounds.size.height / 2.0,
        };

        assert!(shell.shell_role_host_ui(
            SurfaceRole::ControlCenter,
            UiEvent::PointerPressed(point),
            420,
            600,
        ));
        assert!(shell.shell_role_host_ui(
            SurfaceRole::ControlCenter,
            UiEvent::PointerReleased(point),
            420,
            600,
        ));
    }
