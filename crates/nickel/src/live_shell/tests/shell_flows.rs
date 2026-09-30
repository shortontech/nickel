    #[test]
    fn reopening_launcher_restores_default_dashboard_view() {
        let mut shell = LiveShell::new().unwrap();
        shell.apply_session_launcher_visibility(true);
        shell.apply_launcher_action(crate::launcher_actions::LauncherAction::SetView(
            crate::launcher::LauncherView::Applications,
        ));
        assert_eq!(shell.launcher.view(), crate::launcher::LauncherView::Applications);

        shell.apply_session_launcher_visibility(false);
        assert_eq!(shell.launcher.view(), crate::launcher::LauncherView::Favorites);

        shell.apply_session_launcher_visibility(true);
        assert_eq!(shell.launcher.view(), crate::launcher::LauncherView::Favorites);
    }

    #[test]
    fn disabling_plugin_retires_host_and_clears_reported_memory() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.primary_panel_host_ref().is_some());
        let surface = crate::plugin_panel::surface();
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: id.clone(),
            surface_id: surface.id.clone(),
        };
        shell.plugin_panel_scene(&key, surface.width, surface.height);
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.primary_panel_host_ref().is_none());
        let entry = shell.plugin_registry().get(id).unwrap();
        assert_eq!(entry.health, nickel_core::plugins::PluginHealth::Disabled);
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
    }

    #[test]
    fn installed_provider_projection_failure_retires_primary_panel() {
        use nickel_core::plugins::{PluginPackage, PluginPackageDescriptor};

        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-widget-host"
        );
        let package = PluginPackage::load(directory).unwrap();
        let mut shell = LiveShell::new().unwrap();
        shell
            .set_plugin_enabled(&crate::plugin_panel::manifest().id, false)
            .unwrap();
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
        assert_eq!(shell.plugin_panel_owner, package.manifest.id);

        let mut failing = package.clone();
        failing.source = "function App() { if (nickel.data.slots.metrics) throw Error('provider projection exploded'); return h(Panel, {}, h(Text, {}, 'Ready')); }".into();
        let surface = &failing.manifest.surfaces[0];
        let application = crate::plugin_panel::PluginPanelApplication::from_package_surface(
            &failing,
            &Default::default(),
            surface,
        )
        .unwrap();
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: shell.plugin_panel_owner.clone(),
            surface_id: surface.id.clone(),
        };
        shell.plugin_surface_hosts.insert(
            key.clone(),
            (
                surface.clone(),
                nickel_ui::UiHost::new(application, surface.width, surface.height),
            ),
        );
        assert!(shell
            .plugin_panel_scene(&key, surface.width, surface.height)
            .is_none());
        let entry = shell.plugin_registry().get(&package.manifest.id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("provider projection exploded")));
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
        assert!(shell.primary_panel_host_ref().is_none());
    }

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

    #[test]
    fn installed_audio_resource_requires_audio_read_and_redacts_while_locked() {
        use nickel_core::plugins::{PluginCapability, PluginPackage, PluginPackageDescriptor};

        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        );
        let mut package = PluginPackage::load(directory).unwrap();
        package.source = "function App() { return h(Window, {id:'main',width:520,height:340}, h(Text, null, nickel.data.audio ? 'Volume ' + nickel.data.audio.percent : 'Waiting')); }".into();
        let id = package.manifest.id.clone();
        let mut shell = LiveShell::new().unwrap();
        shell.external_plugin_packages.insert(
            id.clone(),
            PluginPackageDescriptor {
                directory: directory.into(),
                manifest: package.manifest.clone(),
                source_digest: package.source_digest(),
            },
        );
        assert!(shell.plugin_audio(&id).is_none());
        shell
            .external_plugin_packages
            .get_mut(&id)
            .unwrap()
            .manifest
            .capabilities
            .push(PluginCapability::AudioRead);
        package.manifest.capabilities.push(PluginCapability::AudioRead);
        let surface = &package.manifest.surfaces[0];
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: id.clone(),
            surface_id: surface.id.clone(),
        };
        let application = crate::plugin_panel::PluginPanelApplication::from_package_surface(
            &package,
            &Default::default(),
            surface,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            key.clone(),
            (
                surface.clone(),
                nickel_ui::UiHost::new(application, surface.width, surface.height),
            ),
        );
        assert!(shell
            .plugin_panel_scene(&key, surface.width, surface.height)
            .is_some());
        let expected = format!("Volume {}", shell.audio.volume_percent.min(100));
        assert!(shell
            .plugin_surface_hosts
            .get(&key)
            .unwrap()
            .1
            .accessibility_nodes()
            .iter()
            .any(|node| node.label.as_deref() == Some(expected.as_str())));
        let audio = shell.plugin_audio(&id).unwrap();
        assert_eq!(audio["percent"], shell.audio.volume_percent.min(100));
        shell.locked = true;
        assert_eq!(
            shell.plugin_audio(&id).unwrap()["outputName"],
            "Audio output"
        );
    }

    #[test]
    fn settings_activation_is_registered_without_an_in_process_window() {
        let mut shell = LiveShell::new().unwrap();
        let id = crate::settings_plugin_report::ID;
        let entry = shell.plugin_registry().get(id).unwrap();
        assert!(entry.desired_enabled);
        assert_eq!(entry.health, nickel_core::plugins::PluginHealth::Starting);
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        let disabled = shell.plugin_registry().get(id).unwrap();
        assert!(!disabled.desired_enabled);
        assert_eq!(disabled.health, nickel_core::plugins::PluginHealth::Disabled);
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert_eq!(shell.plugin_registry().get(id).unwrap().health, nickel_core::plugins::PluginHealth::Starting);
    }

    #[test]
    fn notification_plugin_can_start_render_and_retire() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::notification_manifest().id;
        shell.set_plugin_enabled(id, false).unwrap();
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.notification_plugin_host_ref().is_some());
        shell.scene(super::SurfaceRole::Notification, 420, 180);
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.is_some());
        shell.notification_feed.notify_internal(NotificationRequest {
            app_name: "Test".into(),
            summary: "Ready".into(),
            body: "Ordinary notification".into(),
            actions: vec![],
            expire_timeout_ms: 0,
        });
        shell.notification = shell.notification_feed.snapshot();
        assert!(shell.surface_visible(SurfaceRole::Notification));
        let key = crate::plugin_panel::notification_surface_key();
        assert!(shell.shell_panel_surfaces().iter().any(|(candidate, surface)| {
            candidate == &key && surface.passive
        }));
        assert!(shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        assert!(!shell.native_surface_visible(SurfaceRole::Notification, None));
        assert!(shell
            .plugin_surface_scene_for_output(&key, Some("primary"), 420, 180)
            .is_some());
        assert!(shell.plugin_surface_change_token(&key).is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.notification_plugin_host_ref().is_none());
        assert!(!shell.surface_visible(SurfaceRole::Notification));
        assert!(!shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        assert!(!shell.plugin_surface_matches(&key));
        assert!(shell.scene(SurfaceRole::Notification, 420, 180).is_empty());
        assert!(!shell.notification_click(20.0, 20.0, 420, 180));
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
    }

    #[test]
    fn notification_callback_failure_retires_ordinary_surface() {
        let mut shell = LiveShell::new().unwrap();
        let projection = shell.notification_plugin_projection();
        let application = crate::plugin_panel::PluginPanelApplication::notification_with_test_source(
            "function App() { return h(Panel, {}, h(Button, {id: 'notification-fail', onClick: () => { throw Error('notification callback exploded'); }}, 'Break Notifications')); }",
            &projection,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::notification_surface_key(),
            (
                crate::plugin_panel::notification_surface().clone(),
                nickel_ui::UiHost::new(application, 420, 180),
            ),
        );
        let target = shell.notification_plugin_host_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Break Notifications".into(),
            })
            .unwrap();
        assert!(shell
            .step_notification_plugin(nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(UiEvent::AccessibilityActivate(target.id))],
                ..Default::default()
            })
            .is_none());
        let id = &crate::plugin_panel::notification_manifest().id;
        let entry = shell.plugin_registry().get(id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("notification callback exploded")));
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
        assert!(!shell.plugin_surface_matches(&crate::plugin_panel::notification_surface_key()));
        assert!(shell.scene(SurfaceRole::Notification, 420, 180).is_empty());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.notification_plugin_host_ref().is_some());
    }

    #[test]
    fn notification_projection_failure_retires_ordinary_surface() {
        let mut shell = LiveShell::new().unwrap();
        let projection = shell.notification_plugin_projection();
        let application = crate::plugin_panel::PluginPanelApplication::notification_with_test_source(
            "function App() { if (nickel.data.notification !== null) throw Error('notification projection exploded'); return h(Panel, {}, h(Text, {}, 'Notifications ready')); }",
            &projection,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::notification_surface_key(),
            (
                crate::plugin_panel::notification_surface().clone(),
                nickel_ui::UiHost::new(application, 420, 180),
            ),
        );
        shell.notification_feed.notify_internal(NotificationRequest {
            app_name: "Test".into(),
            summary: "Ready".into(),
            body: "Ordinary notification".into(),
            actions: vec![],
            expire_timeout_ms: 0,
        });
        shell.notification = shell.notification_feed.snapshot();
        assert!(shell.scene(SurfaceRole::Notification, 420, 180).is_empty());
        let entry = shell
            .plugin_registry()
            .get(&crate::plugin_panel::notification_manifest().id)
            .unwrap();
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("notification projection exploded")));
        assert!(!shell.surface_visible(SurfaceRole::Notification));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn opening_plugin_notification_history_does_not_reopen_the_trusted_surface() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        host.take_commands();
        assert!(shell.global_shortcut(crate::platform::GlobalShortcut::ShowNotifications));
        assert!(shell.native_surface_visible(
            SurfaceRole::Panel,
            Some(&crate::plugin_panel::notification_surface_key()),
        ));
        assert!(!shell.native_surface_visible(SurfaceRole::Notification, None));
        assert!(!host.take_commands().iter().any(|command| matches!(
            command,
            crate::platform::ShellCommand::SetShellRoleVisible {
                role: nickel_session_protocol::ShellRole::Notification,
                visible: true,
            }
        )));
    }

    #[test]
    fn trusted_remote_approval_uses_host_even_when_notification_plugin_is_enabled() {
        use nickel_session_protocol::{
            RemoteLeaseRequest, RemoteLeaseRequestChanges, RemotePendingLease,
            RemoteResourceScope,
        };

        let mut shell = LiveShell::new().unwrap();
        assert!(shell.notification_plugin_host_ref().is_some());
        shell.sync_remote_lease_notifications_from(vec![RemotePendingLease {
            pending_generation: 1,
            client_id: "test-client".into(),
            client_label: "Requester".into(),
            request: RemoteLeaseRequest {
                renewal: None,
                scope: RemoteResourceScope::FullSession,
                duration_seconds: Some(300),
                allow_resumption: false,
                full_debug: false,
            },
            resource_label: None,
            changes: RemoteLeaseRequestChanges::default(),
        }]);
        shell.notification = shell.notification_feed.snapshot();
        let id = shell.notification.as_ref().unwrap().id;
        assert!(shell.trusted_notification_visible());
        assert!(shell.native_surface_visible(SurfaceRole::Notification, None));
        assert!(!shell.native_surface_visible(
            SurfaceRole::Panel,
            Some(&crate::plugin_panel::notification_surface_key()),
        ));
        assert!(shell
            .plugin_surface_scene_for_output(
                &crate::plugin_panel::notification_surface_key(),
                None,
                420,
                180,
            )
            .is_none());
        assert!(shell
            .plugin_surface_change_token(&crate::plugin_panel::notification_surface_key())
            .is_none());
        assert!(shell.notification_plugin_projection().notification.is_none());
        let plugin_frame = shell.notification_plugin_host_ref().unwrap().inspect().frame_generation;
        assert!(!shell.scene(SurfaceRole::Notification, 420, 180).is_empty());
        assert_eq!(shell.notification_plugin_host_ref().unwrap().inspect().frame_generation, plugin_frame);
        let trusted_token = shell.scene_change_token(SurfaceRole::Notification).unwrap();
        let approve = shell.notification_host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
            role: nickel_ui::SemanticRole::Button,
            name: "Approve".into(),
        }).unwrap();
        let x = approve.bounds.origin.x + approve.bounds.size.width / 2.0;
        let y = approve.bounds.origin.y + approve.bounds.size.height / 2.0;
        assert!(shell.notification_click(x, y, 420, 180));
        assert!(shell.remote_lease_submitting.contains(&id));

        shell.notification_history_visible = true;
        assert!(shell.notification_plugin_projection().history.is_empty());
        shell.notification_history_visible = false;
        shell.notification = None;
        assert!(!shell.trusted_notification_visible());
        assert_ne!(shell.scene_change_token(SurfaceRole::Notification).unwrap(), trusted_token);
    }

    #[test]
    fn notification_plugin_secure_field_protects_its_surface() {
        let mut shell = LiveShell::new().unwrap();
        let source = "function App() { return h(Panel, {}, h(TextField, {id: 'private', value: 'secret', secure: true, onChange: value => {}})); }";
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::notification_surface_key(),
            (
                crate::plugin_panel::notification_surface().clone(),
                nickel_ui::UiHost::new(
                    crate::plugin_panel::PluginPanelApplication::new(source).unwrap(),
                    420,
                    180,
                ),
            ),
        );
        assert!(shell.surface_remote_access_protected(SurfaceRole::Notification));
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
        let key = crate::plugin_panel::window_preview_surface_key();
        shell.open_window_preview(0);
        shell.preview_images.insert(
            WindowId(71),
            Arc::new(RgbaImage::from_pixel(8, 8, Rgba([20, 40, 60, 255]))),
        );
        assert!(!shell.scene(SurfaceRole::WindowPreview, 300, 214).is_empty());
        assert!(shell.plugin_surface_matches(&key));
        assert!(shell
            .plugin_surface_scene_for_output(&key, None, 300, 214)
            .is_some());
        assert!(shell.plugin_surface_change_token(&key).is_some());
        assert!(shell.preview_plugin_active());
        let preview_host = shell.preview_plugin_host_mut().unwrap();
        let image_bytes = preview_host.application().retained_image_bytes();
        assert!(image_bytes > 0);
        let frame_bytes = preview_host
            .step(HostBatch::default())
            .telemetry
            .retained_frame_bytes as u64;
        let status = shell
            .plugin_status_snapshot()
            .plugins
            .into_iter()
            .find(|plugin| &plugin.id == id)
            .unwrap();
        assert!(status.desired_enabled);
        assert_eq!(
            status.memory.native_ui_bytes,
            Some(frame_bytes.saturating_add(image_bytes))
        );

        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(!shell.preview_plugin_active());
        assert!(!shell.plugin_surface_matches(&key));
        assert!(shell.plugin_surface_change_token(&key).is_none());
        assert!(shell.preview_group.is_none());
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
        assert!(!shell.surface_visible(SurfaceRole::WindowPreview));
        assert!(shell.scene(SurfaceRole::WindowPreview, 300, 214).is_empty());
        shell.open_window_preview(0);
        assert!(!shell.surface_visible(SurfaceRole::WindowPreview));

        assert!(shell.set_plugin_enabled(id, true).unwrap());
        shell.open_window_preview(0);
        assert!(shell.preview_plugin_active());
        assert!(!shell.scene(SurfaceRole::WindowPreview, 300, 214).is_empty());
    }

    #[test]
    fn window_preview_projection_failure_closes_its_surface() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(Vec::new());
        shell.windows = vec![OpenWindow {
            id: WindowId(71),
            application_id: None,
            active: true,
            title: "Document".into(),
            state: Default::default(),
        }];
        let application = crate::plugin_panel::PluginPanelApplication::window_preview_with_test_source(
            "function App() { if (nickel.data.windows.length > 0) throw Error('preview projection exploded'); return h(Panel, {}, h(Text, {}, 'Preview ready')); }",
            &serde_json::json!({"windows": []}),
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::window_preview_surface_key(),
            (
                crate::plugin_panel::window_preview_surface().clone(),
                nickel_ui::UiHost::new(application, 300, 214),
            ),
        );
        shell.open_window_preview(0);
        assert!(shell.window_preview_scene().is_empty());
        let id = &crate::plugin_panel::window_preview_manifest().id;
        let entry = shell.plugin_registry().get(id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("preview projection exploded")));
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
        assert!(shell.preview_group.is_none());
        assert!(!shell.plugin_surface_matches(&crate::plugin_panel::window_preview_surface_key()));
        assert!(!shell.surface_visible(SurfaceRole::WindowPreview));
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.preview_plugin_host_ref().is_some());
    }

    #[test]
    fn window_preview_callback_failure_closes_its_surface() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(Vec::new());
        shell.windows = vec![OpenWindow {
            id: WindowId(71),
            application_id: None,
            active: true,
            title: "Document".into(),
            state: Default::default(),
        }];
        shell.open_window_preview(0);
        let group = shell.preview_plugin_group().unwrap();
        let (data, _) = shell.preview_plugin_projection(&group);
        let application = crate::plugin_panel::PluginPanelApplication::window_preview_with_test_source(
            "function App() { return h(Panel, {}, h(Button, {id: 'preview-fail', onClick: () => { throw Error('preview callback exploded'); }}, 'Break Preview')); }",
            &data,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::window_preview_surface_key(),
            (
                crate::plugin_panel::window_preview_surface().clone(),
                nickel_ui::UiHost::new(application, 300, 214),
            ),
        );
        let target = shell.preview_plugin_host_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Break Preview".into(),
            })
            .unwrap();
        assert!(!shell
            .preview_plugin_event(
                nickel_ui::HostEvent::Ui(UiEvent::AccessibilityActivate(target.id)),
                (300, 214),
                None,
            )
            .changed);
        let id = &crate::plugin_panel::window_preview_manifest().id;
        let entry = shell.plugin_registry().get(id).unwrap();
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("preview callback exploded")));
        assert!(shell.preview_group.is_none());
        assert!(!shell.surface_visible(SurfaceRole::WindowPreview));
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
            .set_plugin_enabled(&crate::plugin_panel::window_preview_manifest().id, false)
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
    fn volume_osd_plugin_retires_without_native_fallback() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::volume_osd_manifest().id;
        let key = crate::plugin_panel::volume_osd_surface_key();
        assert!(shell.plugin_panel_host_ref(&key).is_some());
        assert!(shell.plugin_surface_matches(&key));
        assert!(shell.shell_fixed_surface_keys().contains(&key));
        assert!(!shell.plugin_panels().iter().any(|(panel, _)| panel == &key));
        assert!(shell
            .plugin_surface_scene_for_output(&key, None, 420, 96)
            .is_some());
        assert!(shell.plugin_surface_change_token(&key).is_some());
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.plugin_panel_host_ref(&key).is_none());
        assert!(!shell.plugin_surface_matches(&key));
        assert!(!shell.shell_fixed_surface_keys().contains(&key));
        assert!(shell
            .plugin_surface_scene_for_output(&key, None, 420, 96)
            .is_none());
        assert!(shell.plugin_surface_change_token(&key).is_none());
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
        assert!(shell.scene(SurfaceRole::VolumeOsd, 420, 96).is_empty());
        assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.plugin_panel_host_ref(&key).is_some());
        assert!(shell.plugin_surface_matches(&key));
        assert!(shell.plugin_surface_change_token(&key).is_some());
    }

    #[test]
    fn volume_osd_projection_failure_retires_its_overlay() {
        let mut shell = LiveShell::new().unwrap();
        let mut data = serde_json::json!({"audio": shell.audio_plugin_data()});
        data["audio"]["label"] = "fixture-label".into();
        let application = crate::plugin_panel::PluginPanelApplication::volume_osd_with_test_source(
            "function App() { if (nickel.data.audio.label !== 'fixture-label') throw Error('volume projection exploded'); return h(Panel, {}, h(Text, {}, 'Volume ready')); }",
            &data,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::volume_osd_surface_key(),
            (
                crate::plugin_panel::volume_osd_surface().clone(),
                nickel_ui::UiHost::new(application, 420, 96),
            ),
        );
        shell.volume_osd_until = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
        assert!(shell.scene(SurfaceRole::VolumeOsd, 420, 96).is_empty());
        let id = &crate::plugin_panel::volume_osd_manifest().id;
        let entry = shell.plugin_registry().get(id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("volume projection exploded")));
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
        assert!(shell.volume_osd_until.is_none());
        assert!(!shell.plugin_surface_matches(&crate::plugin_panel::volume_osd_surface_key()));
        assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell
            .plugin_panel_host_ref(&crate::plugin_panel::volume_osd_surface_key())
            .is_some());
    }

    #[test]
    fn control_center_plugin_renders_and_dispatches_typed_desktop_action() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.control_visible = true;
        assert!(
            shell.control_plugin_host_ref().is_some(),
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
        let button = shell.control_plugin_host_ref()
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
        let button = shell.control_plugin_host_ref()
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
            crate::plugin_panel::PluginEffect::InvokePluginSlotSection {
                target_plugin: crate::plugin_panel::control_center_manifest().id.clone(),
                slot_id: "control-section".into(),
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
        let key = crate::plugin_panel::control_center_surface_key();
        assert!(shell.control_plugin_host_ref().is_some());
        shell.apply_control_visibility(true);
        assert!(shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        assert!(!shell.native_surface_visible(SurfaceRole::ControlCenter, None));
        assert!(shell
            .plugin_surface_scene_for_output(&key, Some("primary"), 420, 600)
            .is_some());
        assert!(shell.plugin_surface_change_token(&key).is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.control_plugin_host_ref().is_none());
        assert!(!shell.plugin_surface_matches(&key));
        assert!(!shell.surface_visible(SurfaceRole::ControlCenter));
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
        assert!(shell.scene(SurfaceRole::ControlCenter, 420, 720).is_empty());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.control_plugin_host_ref().is_some());
    }

    #[test]
    fn control_center_callback_failure_retires_its_plugin_surface() {
        let mut shell = LiveShell::new().unwrap();
        let data = shell.control_plugin_data(720);
        let application = crate::plugin_panel::PluginPanelApplication::control_center_with_test_source(
            "function App() { return h(Panel, {}, h(Button, {id: 'control-fail', onClick: () => { throw Error('control callback exploded'); }}, 'Break Quick Settings')); }",
            &data,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::control_center_surface_key(),
            (
                crate::plugin_panel::control_center_surface().clone(),
                nickel_ui::UiHost::new(application, 420, 720),
            ),
        );
        shell.apply_control_visibility(true);
        let target = shell.control_plugin_host_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Break Quick Settings".into(),
            })
            .unwrap();
        let outcome = shell.control_host_event(
            nickel_ui::HostEvent::Ui(UiEvent::AccessibilityActivate(target.id)),
            (420, 720),
            None,
        );
        assert!(!outcome.changed);
        let id = &crate::plugin_panel::control_center_manifest().id;
        let key = crate::plugin_panel::control_center_surface_key();
        let entry = shell.plugin_registry().get(id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("control callback exploded")));
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
        assert!(!shell.plugin_surface_matches(&key));
        assert!(!shell.control_visible);
        assert!(shell.scene(SurfaceRole::ControlCenter, 420, 720).is_empty());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.control_plugin_host_ref().is_some());
    }

    #[test]
    fn control_center_projection_failure_retires_its_plugin_surface() {
        let mut shell = LiveShell::new().unwrap();
        let data = shell.control_plugin_data(720);
        let application = crate::plugin_panel::PluginPanelApplication::control_center_with_test_source(
            "function App() { if (nickel.data.height !== 720) throw Error('control projection exploded'); return h(Panel, {}, h(Text, {}, 'Quick Settings ready')); }",
            &data,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::control_center_surface_key(),
            (
                crate::plugin_panel::control_center_surface().clone(),
                nickel_ui::UiHost::new(application, 420, 720),
            ),
        );
        shell.apply_control_visibility(true);
        assert!(shell
            .plugin_panel_scene(&crate::plugin_panel::control_center_surface_key(), 420, 600)
            .is_none());
        let entry = shell
            .plugin_registry()
            .get(&crate::plugin_panel::control_center_manifest().id)
            .unwrap();
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("control projection exploded")));
        assert!(!shell.control_visible);
        assert!(!shell.control_surface_available());
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
        shell.apply_panel_action(super::TaskbarAction::Codex);
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
    fn codex_menu_callback_failure_retires_its_plugin_surface() {
        let mut shell = LiveShell::new().unwrap();
        let key = crate::plugin_panel::codex_projects_surface_key();
        let initial = nickel_codex_ui::ProjectMenuProjection::from_state(
            &nickel_codex_ui::ChatState::default(),
        );
        let application = crate::plugin_panel::PluginPanelApplication::codex_projects_with_test_source(
            "function App() { return h(Panel, {}, h(Button, {id:'explode', onClick: () => { throw Error('Codex callback exploded'); }}, 'Open project')); }",
            &initial,
        )
        .unwrap();
        let surface = crate::plugin_panel::codex_projects_manifest().surfaces[0].clone();
        let host = nickel_ui::UiHost::new(application, surface.width, surface.height);
        let target = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open project".into(),
            })
            .unwrap();
        shell.plugin_surface_hosts.insert(key.clone(), (surface.clone(), host));
        assert!(shell.plugin_panel_host_ui_for(
            &key,
            UiEvent::AccessibilityActivate(target.id),
            surface.width,
            surface.height,
        ));
        let entry = shell.plugin_registry().get(&key.plugin_id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("Codex callback exploded")));
        assert!(!shell.plugin_surface_hosts.contains_key(&key));
        assert!(!shell.plugin_surface_matches(&key));
    }

    #[test]
    fn keyboard_projection_failure_retires_its_plugin_surface() {
        let mut shell = LiveShell::new().unwrap();
        let key = crate::plugin_panel::on_screen_keyboard_surface_key();
        let data = shell.keyboard_plugin_data();
        let application = crate::plugin_panel::PluginPanelApplication::on_screen_keyboard_with_test_source(
            "let renders = 0; function App() { if (++renders > 1) throw Error('keyboard projection exploded'); return h(Panel, {}, h(Text, {}, 'Ready')); }",
            &data,
        )
        .unwrap();
        let surface = crate::plugin_panel::on_screen_keyboard_manifest().surfaces[0].clone();
        shell.plugin_surface_hosts.insert(
            key.clone(),
            (surface.clone(), nickel_ui::UiHost::new(application, surface.width, surface.height)),
        );
        shell.keyboard_visible = !shell.keyboard_visible;
        assert!(shell.plugin_panel_scene(&key, surface.width, surface.height).is_none());
        let entry = shell.plugin_registry().get(&key.plugin_id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("keyboard projection exploded")));
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
    fn display_projection_recovery_survives_control_plugin_disablement() {
        let mut shell = LiveShell::new().unwrap();
        shell.control_host.application_mut().show_projection_chooser();
        shell.apply_control_visibility(true);
        let key = crate::plugin_panel::control_center_surface_key();
        assert!(shell.native_surface_visible(SurfaceRole::ControlCenter, None));
        assert!(!shell.native_surface_visible(SurfaceRole::Panel, Some(&key)));
        assert!(shell
            .plugin_surface_scene_for_output(&key, Some("primary"), 420, 600)
            .is_none());
        shell
            .set_plugin_enabled(&crate::plugin_panel::control_center_manifest().id, false)
            .unwrap();
        assert!(shell.surface_visible(SurfaceRole::ControlCenter));
        assert!(shell.native_surface_visible(SurfaceRole::ControlCenter, None));
        assert!(!shell.native_surface_visible(
            SurfaceRole::Panel,
            Some(&crate::plugin_panel::control_center_surface_key()),
        ));
        assert!(!shell.scene(SurfaceRole::ControlCenter, 420, 720).is_empty());
    }

    #[test]
    fn control_center_plugin_confirms_session_action_in_component_dialog() {
        let host = std::sync::Arc::new(crate::session_host::StagedSessionHost::new(
            crate::session_host::default_session_host(),
        ));
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.control_visible = true;
        shell.scene(SurfaceRole::ControlCenter, 420, 720);
        let suspend = shell.control_plugin_host_ref()
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
        let confirm = shell.control_plugin_host_ref()
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
            shell.control_plugin_host_ref().unwrap().application().last_error());
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
        let target = shell.notification_plugin_host_ref()
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
        let target = shell.notification_plugin_host_ref()
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
        let run_key = crate::plugin_panel::run_surface_key();
        let launcher_key = crate::plugin_panel::launcher_surface_key();
        shell.set_plugin_enabled(id, false).unwrap();
        assert!(!shell.set_run_visible(true));
        assert!(!shell.run_visible);
        assert!(!shell.launcher_visible);
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.run_host_ref().is_some());
        let launcher_surface = crate::plugin_panel::launcher_surface();
        assert_eq!(
            shell.launcher_preferred_surface_size((960, 720)),
            (launcher_surface.width, launcher_surface.height)
        );
        shell.apply_session_launcher_visibility(true);
        assert!(shell.set_run_visible(true));
        let run_surface = crate::plugin_panel::run_surface();
        assert_eq!(
            shell.launcher_preferred_surface_size((960, 720)),
            (run_surface.width, run_surface.height)
        );
        assert_eq!(shell.launcher_preferred_surface_size((480, 120)), (480, 120));
        assert_eq!(shell.active_launcher_surface_key(), Some(run_key.clone()));
        assert!(shell.run_host_ref().unwrap().inspect().keyboard_focus.is_some());
        assert!(shell
            .plugin_surface_scene_for_output(&run_key, None, 620, 180)
            .is_some());
        assert_ne!(
            shell.plugin_surface_change_token(&run_key),
            shell.plugin_surface_change_token(&launcher_key)
        );
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.is_some());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.run_host_ref().is_none());
        assert!(!shell.run_visible);
        assert_eq!(shell.active_launcher_surface_key(), Some(launcher_key));
        assert!(shell.plugin_surface_change_token(&run_key).is_none());
        assert!(shell.plugin_surface_scene_for_output(&run_key, None, 620, 180).is_none());
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
    }

    #[test]
    fn run_callback_failure_retires_only_run_and_restores_launcher() {
        let mut shell = LiveShell::new().unwrap();
        let application = crate::plugin_panel::PluginPanelApplication::run_with_test_source(
            "function App() { return h(Panel, {}, h(Button, {id: 'run-fail', onClick: () => { throw Error('Run callback exploded'); }}, 'Break Run')); }",
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::run_surface_key(),
            (
                crate::plugin_panel::run_surface().clone(),
                nickel_ui::UiHost::new(application, 620, 180),
            ),
        );
        shell.apply_session_launcher_visibility(true);
        assert!(shell.set_run_visible(true));
        let target = shell
            .run_host_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Break Run".into(),
            })
            .unwrap();
        assert!(!shell.launcher_host_ui(
            UiEvent::AccessibilityActivate(target.id),
            620,
            180,
        ));
        let id = &crate::plugin_panel::run_manifest().id;
        let entry = shell.plugin_registry().get(id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("Run callback exploded")));
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
        assert!(!shell.run_visible);
        assert!(!shell.launcher_visible);
        assert_eq!(
            shell.active_launcher_surface_key(),
            Some(crate::plugin_panel::launcher_surface_key())
        );
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.set_run_visible(true));
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
        launcher_actions::LauncherAction,
        model::{ApplicationId, OpenWindow, TrayItem, WindowId},
        notification::{NotificationAction, NotificationRequest},
        window_preview::MenuAction,
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
    fn disabled_launcher_retires_surface_without_native_fallback() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::launcher_manifest().id;
        let key = crate::plugin_panel::launcher_surface_key();
        assert!(shell.can_show_launcher());
        assert_eq!(shell.active_launcher_surface_key(), Some(key.clone()));
        shell.apply_session_launcher_visibility(true);
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(!shell.can_show_launcher());
        assert!(shell.active_launcher_surface_key().is_none());
        assert!(shell.plugin_surface_change_token(&key).is_none());
        assert!(!shell.launcher_visible);
        assert!(shell.scene(SurfaceRole::Launcher, 920, 680).is_empty());
        assert!(!shell.request_launcher_toggle());
        assert!(!shell.launcher_host_ui(UiEvent::TextInput("ignored".into()), 920, 680));
        assert_eq!(shell.launcher.query(), "");
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.can_show_launcher());
        assert_eq!(shell.active_launcher_surface_key(), Some(key));
        assert!(!shell.scene(SurfaceRole::Launcher, 920, 680).is_empty());
    }

    #[test]
    fn launcher_callback_failure_retires_its_surface_and_can_restart() {
        let mut shell = LiveShell::new().unwrap();
        let projection = shell.current_plugin_launcher_projection();
        let application = crate::plugin_panel::PluginPanelApplication::launcher_with_test_source(
            "function App() { return h(Panel, {}, h(Button, {id: 'launcher-fail', onClick: () => { throw Error('launcher callback exploded'); }}, 'Break launcher')); }",
            &projection,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::launcher_surface_key(),
            (
                crate::plugin_panel::launcher_surface().clone(),
                nickel_ui::UiHost::new(application, 920, 680),
            ),
        );
        shell.apply_session_launcher_visibility(true);
        let target = shell.launcher_host_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Break launcher".into(),
            })
            .unwrap();
        assert!(!shell.launcher_host_ui(
            UiEvent::AccessibilityActivate(target.id),
            920,
            680,
        ));
        let id = &crate::plugin_panel::launcher_manifest().id;
        let entry = shell.plugin_registry().get(id).unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("launcher callback exploded")));
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
        assert!(!shell.launcher_visible);
        assert!(!shell.can_show_launcher());
        assert!(shell.scene(SurfaceRole::Launcher, 920, 680).is_empty());
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.can_show_launcher());
        assert!(!shell.scene(SurfaceRole::Launcher, 920, 680).is_empty());
    }

    #[test]
    fn launcher_projection_failure_retires_its_surface() {
        let mut shell = LiveShell::new().unwrap();
        let projection = shell.current_plugin_launcher_projection();
        let application = crate::plugin_panel::PluginPanelApplication::launcher_with_test_source(
            "function App() { if (nickel.data.query !== '') throw Error('launcher projection exploded'); return h(Panel, {}, h(Text, {}, 'Launcher ready')); }",
            &projection,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::launcher_surface_key(),
            (
                crate::plugin_panel::launcher_surface().clone(),
                nickel_ui::UiHost::new(application, 920, 680),
            ),
        );
        shell.launcher.set_query("trigger");
        assert!(shell.scene(SurfaceRole::Launcher, 920, 680).is_empty());
        let entry = shell
            .plugin_registry()
            .get(&crate::plugin_panel::launcher_manifest().id)
            .unwrap();
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("launcher projection exploded")));
        assert!(!shell.can_show_launcher());
    }

    #[test]
    fn launcher_failure_preserves_the_active_run_plugin() {
        let mut shell = LiveShell::new().unwrap();
        let projection = shell.current_plugin_launcher_projection();
        let application = crate::plugin_panel::PluginPanelApplication::launcher_with_test_source(
            "function App() { if (nickel.data.query !== '') throw Error('launcher projection exploded'); return h(Panel, {}, h(Text, {}, 'Launcher ready')); }",
            &projection,
        )
        .unwrap();
        shell.plugin_surface_hosts.insert(
            crate::plugin_panel::launcher_surface_key(),
            (
                crate::plugin_panel::launcher_surface().clone(),
                nickel_ui::UiHost::new(application, 920, 680),
            ),
        );
        shell.apply_session_launcher_visibility(true);
        assert!(shell.set_run_visible(true));
        shell.launcher.set_query("trigger");
        assert!(shell.sync_plugin_launcher().is_none());
        assert!(shell.run_visible);
        assert!(shell.launcher_visible);
        assert_eq!(
            shell.active_launcher_surface_key(),
            Some(crate::plugin_panel::run_surface_key())
        );
        assert!(!shell.scene(SurfaceRole::Launcher, 920, 680).is_empty());
    }

    #[test]
    fn disabled_taskbar_retires_its_visible_role() {
        let mut shell = LiveShell::new().unwrap();
        let id = &crate::plugin_panel::taskbar_manifest().id;
        assert!(shell.surface_visible(SurfaceRole::Taskbar));
        assert_eq!(shell.taskbar_reservation_height(), 56);
        let window = OpenWindow {
            id: WindowId(71),
            application_id: Some(ApplicationId::new("org.example.menu")),
            active: true,
            title: "Menu owner".into(),
            state: Default::default(),
        };
        shell.windows = vec![window.clone()];
        shell.window_menu = Some(window.id);
        shell.window_menu_snapshot = Some(window);
        assert!(!shell.window_menu_scene().is_empty());
        assert!(shell.surface_visible(SurfaceRole::WindowContextMenu));
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(!shell.surface_visible(SurfaceRole::Taskbar));
        assert_eq!(shell.taskbar_reservation_height(), 0);
        assert!(shell.scene(SurfaceRole::Taskbar, 800, 56).is_empty());
        assert!(!shell.panel_click(20.0, 800, false));
        assert!(shell
            .resolve_semantic_target(&ShellSemanticTarget::OnScreenKeyboardToggle)
            .is_none());
        assert!(shell
            .resolve_semantic_target(&ShellSemanticTarget::PanelControlCenter { output: None })
            .is_none());
        assert!(!shell.surface_visible(SurfaceRole::WindowContextMenu));
        assert!(shell.window_menu_scene().is_empty());
        assert!(shell.window_menu_plugin_host.is_none());
        assert!(!shell.open_window_menu_at(71, 10, 10));
        shell.preview_group = Some(0);
        assert!(!shell.preview_plugin_action_allowed(crate::window_preview::PreviewAction::OpenMenu(WindowId(71))));
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.surface_visible(SurfaceRole::Taskbar));
        assert_eq!(shell.taskbar_reservation_height(), 56);
    }

    #[test]
    fn taskbar_callback_failure_retires_only_its_plugin_surface() {
        let mut shell = LiveShell::new().unwrap();
        let (projection, _) = shell.taskbar_plugin_projection("12:00");
        let application = crate::plugin_panel::PluginPanelApplication::taskbar_with_test_source(
            "function App() { return h(Panel, {}, h(Button, {id: 'taskbar-launcher', onClick: () => { throw Error('taskbar callback exploded'); }}, 'Break taskbar')); }",
            &projection,
        )
        .unwrap();
        shell.plugin_taskbar_host = Some(nickel_ui::UiHost::new(application, 800, 56));
        let target = shell
            .plugin_taskbar_host
            .as_ref()
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Break taskbar".into(),
            })
            .unwrap();
        assert!(shell
            .step_taskbar_plugin_batch(
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(
                        nickel_ui::UiEvent::AccessibilityActivate(target.id),
                    )],
                    ..Default::default()
                },
                800,
                56,
            )
            .is_none());
        let entry = shell
            .plugin_registry()
            .get(&crate::plugin_panel::taskbar_manifest().id)
            .unwrap();
        assert!(entry.desired_enabled);
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("taskbar callback exploded")));
        assert_eq!(entry.memory, nickel_core::plugins::PluginMemory::default());
        assert!(!shell.surface_visible(SurfaceRole::Taskbar));
        assert_eq!(shell.taskbar_reservation_height(), 0);
        assert!(shell.scene(SurfaceRole::Taskbar, 800, 56).is_empty());
        assert!(!shell.panel_click(20.0, 800, false));
        assert!(shell.launcher_host_ref().is_some());
        let id = &crate::plugin_panel::taskbar_manifest().id;
        assert!(shell.set_plugin_enabled(id, false).unwrap());
        assert!(shell.set_plugin_enabled(id, true).unwrap());
        assert!(shell.surface_visible(SurfaceRole::Taskbar));
    }

    #[test]
    fn taskbar_projection_failure_retires_its_plugin_surface() {
        let mut shell = LiveShell::new().unwrap();
        let (projection, _) = shell.taskbar_plugin_projection("fixture-clock");
        let application = crate::plugin_panel::PluginPanelApplication::taskbar_with_test_source(
            "function App() { if (nickel.data.clock !== 'fixture-clock') throw Error('taskbar projection exploded'); return h(Panel, {}, h(Text, {}, 'Clock ready')); }",
            &projection,
        )
        .unwrap();
        shell.plugin_taskbar_host = Some(nickel_ui::UiHost::new(application, 800, 56));
        assert!(shell
            .step_taskbar_plugin_batch(Default::default(), 800, 56)
            .is_none());
        let entry = shell
            .plugin_registry()
            .get(&crate::plugin_panel::taskbar_manifest().id)
            .unwrap();
        assert!(matches!(&entry.health, nickel_core::plugins::PluginHealth::Failed(error) if error.contains("taskbar projection exploded")));
        assert!(!shell.surface_visible(SurfaceRole::Taskbar));
        assert_eq!(shell.taskbar_reservation_height(), 0);
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
        let host = shell.launcher_host_mut().unwrap();
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
        let host = shell.launcher_host_ref().unwrap();
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
        let host = shell.launcher_host_mut().unwrap();
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
        assert!(!shell.launcher_host_ref()
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
        let mut application = crate::plugin_panel::PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::launcher_manifest(),
            "main.js",
            crate::plugin_panel::LauncherPluginProjection::from_launcher(launcher).to_json(),
        )
        .expect("bundled launcher");
        application.sync_theme_palette(palette).expect("launcher palette");
        let mut host = UiHost::new(application, 920, 680);
        let application_name = launcher
            .applications()
            .find(|application| application.id() == application_id)
            .expect("catalog application")
            .name();
        let target = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: application_name.into(),
            })
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

        shell.apply_launcher_action(crate::launcher_actions::LauncherAction::TogglePin(
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
        shell.apply_launcher_action(crate::launcher_actions::LauncherAction::TogglePin(
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
        shell.apply_launcher_action(crate::launcher_actions::LauncherAction::TogglePin(
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

    #[test]
    fn plugin_retry_saves_failed_launcher_preferences_once() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let preferences_path = directory.path().join("launcher-preferences");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::default();
        preferences_fixture(&mut shell, directory.path().to_path_buf());
        shell.apply_launcher_action(LauncherAction::TogglePin("firefox".into()));
        finish_preference_write(&mut shell);
        assert!(shell.launcher_status.as_deref().is_some_and(|status| {
            status.starts_with("Launcher preferences could not be saved:")
        }));
        let projection = shell.current_plugin_launcher_projection();
        assert!(projection.pin_save_failed);
        assert!(projection.to_json().contains("\"pinSaveFailed\":true"));

        preferences_fixture(&mut shell, preferences_path.clone());
        assert!(shell.apply_plugin_effects(vec![
            crate::plugin_panel::PluginEffect::RetryApplicationPinSave
        ]));
        finish_preference_write(&mut shell);
        assert_eq!(shell.launcher_persistence_attempts, 2);
        assert!(shell.launcher_status.is_none());
        assert!(!shell.current_plugin_launcher_projection().pin_save_failed);
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
        assert!(shell.current_plugin_launcher_projection().results.is_empty());

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
                output: None,
                interaction: PointerInteraction::Hover,
            })
            .expect("live panel group resolves");
        assert_eq!(panel.role, ShellRole::Panel);
        assert_eq!(panel.output, None);
        let key = shell.taskbar_surface_key().expect("active taskbar surface");
        assert!(shell.plugin_panel_host_input_for(
            &key,
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(1),
                position: nickel_input::Point {
                    x: f64::from(panel.x),
                    y: 28.0,
                },
                delta: None,
            }),
            1280,
            56,
        ));
        assert_eq!(shell.panel_hover, Some(super::TaskbarHover::Task(0)));
        assert!(shell.preview_group.is_none());
        assert_eq!(shell.preview_pending.map(|(index, _)| index), Some(0));
        assert!(shell.preview_pending.unwrap().1 > Instant::now());

        let (preview_width, _) = super::preview_dimensions(2);
        assert_eq!(
            shell.preview_origin_x(0, preview_width),
            (panel.x - i32::try_from(preview_width / 2).unwrap()).max(shell.panel_origin_x)
        );

        shell.open_window_preview(0);
        assert!(!shell.scene(SurfaceRole::WindowPreview, preview_width, 214).is_empty());
        let preview = shell
            .resolve_semantic_target(&ShellSemanticTarget::PreviewWindow {
                window: nickel_session_protocol::WindowId(9),
                action: PreviewTargetAction::Close,
            })
            .expect("live preview close target resolves");
        assert_eq!(preview.role, ShellRole::Preview);
        assert_eq!(preview.interaction, PointerInteraction::LeftClick);
        assert!(shell
            .preview_plugin_bounds(crate::window_preview::PreviewAction::Close(WindowId(9)))
            .is_some());

        shell.window_menu = Some(WindowId(9));
        shell.window_menu_snapshot = shell.windows.iter().find(|window| window.id == WindowId(9)).cloned();
        let _ = shell.window_menu_scene();
        let menu = shell
            .resolve_semantic_target(&ShellSemanticTarget::WindowMenu {
                window: nickel_session_protocol::WindowId(9),
                action: WindowMenuTargetAction::Minimize,
            })
            .expect("live context-menu row resolves");
        assert_eq!(menu.role, ShellRole::ContextMenu);
        assert!(shell.window_menu_plugin_host.is_some());

        shell.screenshot.show(image::RgbaImage::new(400, 200));
        let _ = shell.scene(SurfaceRole::Screenshot, 800, 600);
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::SelectionStart));
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::SelectionEnd));
        assert!(shell.perform_screenshot_semantic_action(ScreenshotTargetAction::Confirm));
        assert!(shell.screenshot.confirmed());
    }

    #[test]
    fn jsx_taskbar_semantic_targets_use_live_groups_and_controls() {
        let mut shell = LiveShell::new().unwrap();
        assert!(shell.plugin_taskbar_host.is_some());
        shell.windows = vec![OpenWindow {
            id: WindowId(41),
            application_id: Some(ApplicationId::new("org.kde.dolphin")),
            active: true,
            title: "Files".into(),
            state: crate::model::WindowState::default(),
        }];
        shell.keyboard_enabled = true;
        shell.scene(SurfaceRole::Taskbar, 1280, 56);

        let application = shell
            .resolve_semantic_target(&ShellSemanticTarget::PanelApplication {
                application_id: "org.kde.dolphin".into(),
                output: None,
                interaction: PointerInteraction::Hover,
            })
            .expect("JSX task target resolves without native groups");
        assert_eq!(application.role, ShellRole::Panel);
        assert_eq!(application.interaction, PointerInteraction::Hover);

        let keyboard = shell
            .resolve_semantic_target(&ShellSemanticTarget::OnScreenKeyboardToggle)
            .expect("JSX keyboard control resolves");
        assert_eq!(keyboard.role, ShellRole::Panel);
        assert_eq!(keyboard.interaction, PointerInteraction::LeftClick);

        shell.windows[0].application_id = Some(ApplicationId::new("org.example.Changed"));
        assert!(
            shell
                .resolve_semantic_target(&ShellSemanticTarget::PanelApplication {
                    application_id: "org.example.Changed".into(),
                    output: None,
                    interaction: PointerInteraction::LeftClick,
                })
                .is_none(),
            "a changed group cannot reuse a stale JSX button"
        );
    }

    #[test]
    fn unnamed_taskbar_semantic_target_uses_a_rendered_output() {
        let mut shell = LiveShell::new().unwrap();
        shell.panel_scene_for_output(Some("winit"), 1200, 56);
        let target = shell
            .resolve_semantic_target(&ShellSemanticTarget::PanelControlCenter { output: None })
            .expect("rendered taskbar control exists");
        assert_eq!(target.output.as_deref(), Some("winit"));
        assert!((0..1200).contains(&target.x));
    }

    #[test]
    fn jsx_taskbar_anchors_previews_and_codex_menu_to_its_controls() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher.set_codex_available(true);
        shell.windows = vec![OpenWindow {
            id: WindowId(42),
            application_id: Some(ApplicationId::new("org.kde.dolphin")),
            active: true,
            title: "Files".into(),
            state: crate::model::WindowState::default(),
        }];
        shell.set_panel_output("left");
        shell.scene(SurfaceRole::Taskbar, 1280, 56);
        let host = shell.plugin_taskbar_host.as_ref().unwrap();
        let item = super::taskbar_plugin_control_bounds(host, "taskbar-item-0").unwrap();
        let codex = super::taskbar_plugin_control_bounds(host, "taskbar-codex").unwrap();
        let preview_width = 320;
        assert_eq!(
            shell.preview_origin_x(0, preview_width),
            super::TaskbarPreviewAnchor::new(shell.panel_origin_x, item)
                .preview_origin_x(preview_width)
        );

        shell.apply_panel_action(super::TaskbarAction::Codex);
        let anchor = shell.pending_popover_anchor.as_ref().unwrap();
        assert_eq!(anchor.control, "taskbar-codex");
        assert_eq!(anchor.output, "left");
        assert_eq!(anchor.bounds, codex);
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
        let center = item.origin.x + item.size.width / 2.0;
        assert!(shell.panel_pointer_moved(center, 1_280));
        assert_eq!(shell.panel_hover, Some(super::TaskbarHover::Task(index)));
        assert!(shell.panel_click(center, 1_280, true));
        let current_item = super::taskbar_plugin_control_bounds(
            shell.plugin_taskbar_host.as_ref().unwrap(),
            &format!("taskbar-item-{index}"),
        )
        .unwrap();
        let expected_x = shell.panel_origin_x + current_item.origin.x.round() as i32;
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
            crate::plugin_panel::PluginEffect::InvokePluginSlotAction {
                target_plugin: crate::plugin_panel::taskbar_manifest().id.clone(),
                slot_id: "task-action".into(),
                plugin_id: package.manifest.id.clone(),
                id: "find-apps".into(),
                item: Some("org.nickel.mail".into()),
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
        let taskbar_only = shell
            .plugin_registry
            .get(&crate::plugin_panel::taskbar_manifest().id)
            .unwrap()
            .memory
            .native_ui_bytes
            .unwrap();
        assert!(shell.panel_click(bounds.origin.x + bounds.size.width / 2.0, 1_280, true));
        let menu_height = shell.window_context_menu_height() as u32;
        assert!(!shell
            .scene(SurfaceRole::WindowContextMenu, super::MENU_WIDTH as u32, menu_height)
            .is_empty());
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
    fn control_center_keyboard_navigation_uses_host_semantic_order() {
        let mut shell = LiveShell::new().unwrap();
        shell.control_visible = true;

        assert!(shell.control_key(Some(KeyCode::ArrowDown), 420, 600));
        assert!(shell.control_plugin_host_ref().unwrap().inspect().controller_target.is_some());
        assert!(shell.control_key(Some(KeyCode::ArrowDown), 420, 600));
        assert!(shell.control_key(Some(KeyCode::ArrowUp), 420, 600));
        assert!(shell.control_plugin_host_ref().unwrap().inspect().controller_target.is_some());
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
                .control_plugin_host_ref()
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
    fn compositor_owned_control_center_ui_updates_the_production_host() {
        let mut shell = LiveShell::new().unwrap();
        shell.control_visible = true;
        let _ = shell.scene(SurfaceRole::ControlCenter, 420, 600);
        let target = shell.control_plugin_host_ref()
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
