    #[test]
    fn warm_panel_hover_reuses_task_projection_and_builtin_images() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new((0..10_000).map(|index| crate::model::Application::new(format!("app.{index}"), format!("App {index}"), None, None, None)).collect());
        shell.windows = vec![OpenWindow { id: WindowId(77), application_id: None, active: true, title: "Nickel Settings".into(), state: Default::default() }];
        shell.scene(SurfaceRole::Panel, 1280, 56);
        let groups = Arc::clone(&shell.panel_host.application().groups);
        let image = Arc::clone(&shell.panel_host.application().task_icons[0].as_ref().unwrap().1);
        let target = shell.panel_host.query_unique(&SemanticSelector::RoleAndName { role: SemanticRole::Button, name: "Open Nickel Start".into() }).unwrap();
        for _ in 0..20 {
            shell.panel_pointer_moved(target.bounds.origin.x + target.bounds.size.width / 2.0, 1280);
            shell.scene(SurfaceRole::Panel, 1280, 56);
            assert!(Arc::ptr_eq(&groups, &shell.panel_host.application().groups));
            assert!(Arc::ptr_eq(&image, &shell.panel_host.application().task_icons[0].as_ref().unwrap().1));
            assert!(!shell.sync_panel_host());
        }
        shell.launcher.toggle_pin("app.1");
        assert!(shell.sync_panel_host());
        assert!(!Arc::ptr_eq(&groups, &shell.panel_host.application().groups));
        assert!(shell.panel_host.application().groups[0].pinned);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn panel_render_context_preserves_input_output_and_reuses_each_output_projection() {
        use nickel_session_protocol::{Geometry, OutputSnapshot, OutputTransform, Snapshot, WindowSnapshot, WorkspaceId};
        let output = |name: &str, x| OutputSnapshot { name: name.into(), model: name.into(), geometry: Geometry { x, y: 0, width: 1000, height: 800 }, work_area: Geometry { x, y: 0, width: 1000, height: 744 }, scale_120: 120, transform: OutputTransform::Normal, physical_width_mm: 1, physical_height_mm: 1, primary: x == 0, enabled: true };
        let window = |id, x, title: &str| WindowSnapshot { id: nickel_session_protocol::WindowId(id), application_id: format!("app.{id}"), title: title.into(), active: id == 1, minimized: false, maximized: false, fullscreen: false, geometry: Some(Geometry { x, y: 0, width: 400, height: 400 }), workspace: WorkspaceId(1) };
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(Vec::new());
        shell.all_windows_on_every_bar = false;
        shell.apply_internal_session_snapshot(Snapshot { outputs: vec![output("left", 0), output("right", 1000)], windows: vec![window(1, 0, "Left task"), window(2, 1000, "Right task")], ..Default::default() });
        shell.refresh_fast();
        shell.set_panel_output("left");
        shell.panel_scene_for_output(Some("right"), 1000, 56);
        assert_eq!(shell.panel_output.as_deref(), Some("left"));
        assert_eq!(shell.panel_host.application().groups[0].application_name, "Right task");
        let right = Arc::clone(&shell.panel_host.application().groups);
        shell.panel_scene_for_output(Some("left"), 1000, 56);
        assert_eq!(shell.panel_host.application().groups[0].application_name, "Left task");
        shell.panel_scene_for_output(Some("right"), 1000, 56);
        assert!(Arc::ptr_eq(&right, &shell.panel_host.application().groups));
        shell.panel_host_ui(UiEvent::PointerMoved(Point { x: 0.0, y: 0.0 }), 1000);
        assert_eq!(shell.panel_host.application().groups[0].application_name, "Left task");
        shell.all_windows_on_every_bar = true;
        shell.panel_scene_for_output(Some("left"), 1000, 56);
        assert_eq!(shell.panel_host.application().groups.len(), 2);
        shell.panel_scene_for_output(Some("right"), 1000, 56);
        assert_eq!(shell.panel_host.application().groups.len(), 2);
        shell.retain_panel_outputs(&[]);
        assert!(shell.panel_projections.is_empty());
    }

    #[test]
    fn right_panel_cluster_is_compact_and_grouped() {
        let layout = panel_status_layout(1920, 3, true);
        assert_eq!(layout.control_start, 1816.0);
        assert_eq!(layout.tray_start, 1732.0);
        assert_eq!(layout.codex_start, 1696.0);
        assert_eq!(
            layout.codex_icon_bounds(),
            Rect::new(1700.0, 14.0, 28.0, 28.0)
        );
    }

    #[test]
    fn panel_host_owns_pointer_and_accessibility_targets() {
        let mut shell = LiveShell::new().unwrap();
        let commands = shell.scene(SurfaceRole::Panel, 1280, 56);
        assert!(!commands.is_empty());

        let launcher = shell
            .panel_host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open Nickel Start".into(),
            })
            .unwrap();
        let center = Point {
            x: launcher.bounds.origin.x + launcher.bounds.size.width / 2.0,
            y: launcher.bounds.origin.y + launcher.bounds.size.height / 2.0,
        };
        assert!(shell.panel_pointer_moved(center.x, 1280));
        assert_eq!(shell.panel_hover, Some(super::PanelHover::Launcher));
        assert!(
            shell
                .panel_host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Button,
                    name: "Open Quick Settings".into(),
                })
                .is_ok()
        );
    }

    #[test]
    fn panel_scene_rebuilds_when_persisted_appearance_changes() {
        let mut shell = LiveShell::new().unwrap();
        let before_commands = shell.scene(SurfaceRole::Panel, 1280, 56);
        let before = shell.panel_change_token;
        let light = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Light,
            accent: nickel_core::theme::accent_from_hue(167),
            intensity: 100,
        });
        let dark = ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Dark,
            accent: nickel_core::theme::accent_from_hue(167),
            intensity: 100,
        });
        shell.palette = if shell.palette == light { dark } else { light };

        let commands = shell.scene(SurfaceRole::Panel, 1280, 56);

        assert_ne!(shell.panel_change_token, before);
        assert_ne!(commands, before_commands);
    }

    #[test]
    fn panel_scene_rebuilds_when_a_window_feed_adds_an_application() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);
        let before = shell.panel_change_token;
        shell.windows.push(OpenWindow {
            id: WindowId(77),
            application_id: Some(ApplicationId::new("google-chrome")),
            active: true,
            title: "Chrome".into(),
            state: crate::model::WindowState::default(),
        });

        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);

        assert_ne!(shell.panel_change_token, before);
        assert_eq!(
            shell
                .panel_host
                .semantic_targets_for_message(&super::PanelAction::Task(0))
                .len(),
            1
        );
    }

    #[test]
    fn taskbar_drag_reorders_a_pin_without_emitting_activation() {
        let mut launcher = crate::launcher::Launcher::new(vec![
            crate::model::Application::new(
                "first".into(),
                "First".into(),
                None,
                None,
                Some(vec!["first".into()]),
            ),
            crate::model::Application::new(
                "second".into(),
                "Second".into(),
                None,
                None,
                Some(vec!["second".into()]),
            ),
        ]);
        launcher.set_pins(vec![("first".into(), 0), ("second".into(), 1)]);
        let mut panel = super::PanelApplication::fixture(
            launcher,
            ThemePalette::from_appearance(Appearance::default()),
        );
        let bounds = Rect::new(100.0, 0.0, 48.0, 48.0);

        nickel_ui::Application::update(
            &mut panel,
            super::PanelAction::TaskDrag(
                0,
                nickel_ui::DragGesture {
                    phase: nickel_ui::DragPhase::Moved,
                    position: Point { x: 170.0, y: 20.0 },
                    bounds,
                },
            ),
        );
        nickel_ui::Application::update(
            &mut panel,
            super::PanelAction::TaskDrag(
                0,
                nickel_ui::DragGesture {
                    phase: nickel_ui::DragPhase::Ended,
                    position: Point { x: 170.0, y: 20.0 },
                    bounds,
                },
            ),
        );

        assert_eq!(
            panel.effects,
            [super::PanelAction::MoveTaskPinRight("first".into())]
        );
    }

    #[test]
    fn closed_taskbar_pin_opens_application_actions_and_unpins_once() {
        let directory = tempfile::tempdir().expect("temporary preferences directory");
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(vec![crate::model::Application::new(
            "org.example.pinned".into(),
            "Pinned Example".into(),
            None,
            None,
            Some(vec!["example".into()]),
        )]);
        shell
            .launcher
            .set_pins(vec![("org.example.pinned".into(), 0)]);
        shell.launcher_preferences_path = Some(directory.path().join("launcher-preferences"));
        shell.windows.clear();
        shell.sync_panel_host();

        shell.apply_panel_action(super::PanelAction::TaskContext(0));

        assert!(
            shell.window_menu.is_none(),
            "closed pins have no WindowId target"
        );
        assert_eq!(
            shell
                .application_menu_target
                .as_ref()
                .and_then(|target| target.application_id.as_ref())
                .map(crate::model::ApplicationId::as_str),
            Some("org.example.pinned")
        );
        let _ = shell.window_menu_scene();
        let host = shell
            .application_menu_host
            .as_ref()
            .expect("application-only task menu host");
        assert!(!host.accessibility_nodes().iter().any(|node| {
            node.label.as_deref() == Some("New Window")
        }));
        assert!(host.accessibility_nodes().iter().any(|node| {
            node.label.as_deref() == Some("Unpin from Nickel Bar")
                && node.semantic_role == Some(SemanticRole::Button)
        }));

        shell.apply_application_menu_action(crate::window_preview::ApplicationMenuAction::TogglePin(
            crate::model::ApplicationId::new("org.example.unrelated"),
        ));
        assert!(shell.launcher.is_pinned("org.example.pinned"));
        assert!(!shell.launcher.is_pinned("org.example.unrelated"));

        shell.apply_application_menu_action(crate::window_preview::ApplicationMenuAction::TogglePin(
            crate::model::ApplicationId::new("org.example.pinned"),
        ));
        assert!(!shell.launcher.is_pinned("org.example.pinned"));
        assert!(
            shell.application_menu_target.is_none(),
            "a successful application command consumes and dismisses its menu"
        );
        shell.apply_application_menu_action(crate::window_preview::ApplicationMenuAction::TogglePin(
            crate::model::ApplicationId::new("org.example.pinned"),
        ));
        assert!(
            !shell.launcher.is_pinned("org.example.pinned"),
            "a stale closed-pin menu cannot recreate its removed target"
        );
    }

    #[test]
    fn successful_close_all_consumes_and_dismisses_the_application_menu() {
        let mut shell = LiveShell::new().unwrap();
        let application = ApplicationId::new("org.example.group");
        shell.launcher = crate::launcher::Launcher::new(vec![crate::model::Application::new(
            application.as_str().into(),
            "Grouped Example".into(),
            None,
            None,
            Some(vec!["example".into()]),
        )]);
        shell.windows = [81, 82]
            .into_iter()
            .map(|id| OpenWindow {
                id: WindowId(id),
                application_id: Some(application.clone()),
                active: id == 81,
                title: format!("Window {id}"),
                state: crate::model::WindowState::default(),
            })
            .collect();
        shell.sync_panel_host();
        shell.apply_panel_action(super::PanelAction::TaskContext(0));
        assert!(shell.application_menu_target.is_some());

        shell.apply_application_menu_action(
            crate::window_preview::ApplicationMenuAction::CloseAll,
        );

        assert!(shell.application_menu_target.is_none());
        assert!(shell.application_menu_host.is_none());
    }

    #[test]
    fn due_panel_clock_deadline_rebuilds_only_when_the_minute_changes() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);
        shell.panel_host.application_mut().clock = "stale".into();
        shell.panel_host.application_mut().date = "stale".into();
        let now = Instant::now();
        shell.panel_deadline = Some(now);

        assert!(shell.poll_host_deadlines(now).contains(&SurfaceRole::Panel));
        assert_ne!(shell.panel_host.application().clock, "stale");
        assert!(shell.panel_deadline.is_some_and(|deadline| deadline > now));
    }

    #[test]
    fn every_advertised_shell_deadline_is_consumed_when_due() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Desktop, 1280, 720);
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);
        let _ = shell.scene(SurfaceRole::Lock, 1280, 720);
        let _ = shell.scene(SurfaceRole::ControlCenter, 420, 640);
        shell.screenshot.request_capture();
        shell.screenshot.queue_pointer_moved(4.0, 5.0, 800, 600);
        let now = Instant::now();
        shell.desktop_deadline = Some(now);
        shell.panel_deadline = Some(now);
        shell.lock_deadline = Some(now);
        shell.control_deadline = Some(now);
        shell.preview_pending = Some((usize::MAX, now));
        shell.preview_leave_deadline = Some(now);
        let due = shell
            .next_host_deadline()
            .expect("the shell advertises its earliest wakeup")
            .max(now + Duration::from_millis(100));

        let outcome = shell.poll_deadlines(due);

        assert!(outcome.capture_screenshot);
        assert!(shell.desktop_deadline.is_none_or(|deadline| deadline > due));
        assert!(shell.panel_deadline.is_none_or(|deadline| deadline > due));
        assert!(shell.lock_deadline.is_none_or(|deadline| deadline > due));
        assert!(shell.control_deadline.is_none_or(|deadline| deadline > due));
        assert!(
            shell
                .next_host_deadline()
                .is_none_or(|deadline| deadline > due)
        );
    }

    #[test]
    fn panel_hover_treats_semantic_ids_as_opaque() {
        let mut shell = LiveShell::new().unwrap();
        shell.tray = vec![TrayItem {
            id: "opaque/panel-task-999".into(),
            title: "Opaque tray target".into(),
            icon: RgbaImage::new(18, 18),
        }];
        shell.tray_icons = panel_tray_icons(&shell.tray);
        let _ = shell.scene(SurfaceRole::Panel, 1280, 56);

        let tray = shell
            .panel_host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Opaque tray target".into(),
            })
            .unwrap();
        let center = Point {
            x: tray.bounds.origin.x + tray.bounds.size.width / 2.0,
            y: tray.bounds.origin.y + tray.bounds.size.height / 2.0,
        };

        assert!(shell.panel_pointer_moved(center.x, 1280));
        assert_eq!(shell.panel_hover, Some(super::PanelHover::Tray(0)));
    }

    #[test]
    fn panel_hover_is_projected_only_on_the_output_that_received_pointer_input() {
        let mut shell = LiveShell::new().unwrap();
        shell.panel_hover = Some(super::PanelHover::Launcher);
        shell.panel_hover_output = Some("DP-1".into());

        shell.set_panel_output("DP-1");
        assert_eq!(
            shell.visible_panel_hover(),
            Some(super::PanelHover::Launcher)
        );
        shell.set_panel_output("HDMI-A-1");
        assert_eq!(shell.visible_panel_hover(), None);
        assert!(!shell.panel_pointer_left());
        assert_eq!(shell.panel_hover, Some(super::PanelHover::Launcher));

        shell.set_panel_output("DP-1");
        assert!(shell.panel_pointer_left());
        assert_eq!(shell.panel_hover, None);
    }

    #[test]
    fn right_panel_omits_codex_space_until_codex_is_available() {
        let layout = panel_status_layout(1920, 3, false);
        assert_eq!(layout.codex_start, layout.tray_start);
    }

    #[test]
    fn stale_codex_actions_cannot_restore_hidden_chrome_or_project_requests() {
        use nickel_core::optional_features::{
            CodexAvailabilityProjection, FeatureHealth, FeatureInstallation, FeatureSupport,
        };
        let mut shell = LiveShell::new().unwrap();
        shell.requested_codex_project = Some("stale-private-project".into());
        shell.codex_project_menu_visible = true;
        shell.apply_codex_projection(CodexAvailabilityProjection::new(
            FeatureSupport::Supported,
            FeatureInstallation::Installed,
            false,
            FeatureHealth::Unknown,
            9,
            None,
        ));
        assert_eq!(shell.take_requested_codex_project(), None);
        shell.apply_panel_action(super::PanelAction::Codex);
        assert!(!shell.codex_project_menu_visible);
    }

    #[test]
    fn panel_reentry_cancels_a_stale_preview_leave_deadline() {
        let mut shell = LiveShell::new().unwrap();
        shell.preview_leave_deadline = Some(Instant::now());

        assert!(shell.panel_pointer_entered());
        assert!(shell.preview_leave_deadline.is_none());
        assert!(!shell.panel_pointer_entered());
    }

    #[test]
    fn preview_refresh_has_a_hard_temporal_bound() {
        let now = Instant::now();
        assert!(preview_refresh_due(None, now));
        assert!(!preview_refresh_due(
            Some(now + super::PREVIEW_REFRESH_INTERVAL),
            now
        ));
        assert!(preview_refresh_due(Some(now), now));
    }

    #[test]
    fn wallpaper_cache_caps_growth_and_reclaims_four_x_area_reductions() {
        assert_eq!(
            super::wallpaper_cache_target((0, 0), (16_000, 9_000)),
            Some((7680, 4320))
        );
        assert_eq!(
            super::wallpaper_cache_target((3840, 2160), (2560, 1440)),
            None,
            "minor topology changes reuse the existing thumbnail"
        );
        assert_eq!(
            super::wallpaper_cache_target((3840, 2160), (1920, 1080)),
            Some((1920, 1080)),
            "4K to FHD releases three quarters of retained pixels"
        );
    }

    #[test]
    fn desktop_scene_rebuilds_when_wallpaper_arrives_at_the_initial_host_size() {
        let mut shell = LiveShell::new().unwrap();
        shell.wallpaper = None;
        shell.wallpaper_size = (0, 0);
        shell.desktop_host.application_mut().wallpaper = None;
        shell.desktop_host.step(HostBatch {
            application_changed: true,
            surface_size: Some((1920, 1080)),
            ..HostBatch::default()
        });
        shell.wallpaper = Some(Arc::new(RgbaImage::from_pixel(
            1920,
            1080,
            Rgba([10, 20, 30, 255]),
        )));
        shell.wallpaper_size = (1920, 1080);
        let initial = shell.desktop_host.inspect();

        shell.desktop_scene(1920, 1080);
        let rebuilt = shell.desktop_host.inspect();

        assert_eq!(rebuilt.frame_generation, initial.frame_generation + 1);
        assert!(
            rebuilt.resources.paint_primitive_count > initial.resources.paint_primitive_count,
            "the declarative wallpaper image enters the rebuilt desktop frame"
        );
    }

    #[test]
    fn preview_cache_retains_authoritative_source_aspect_for_ui_containment() {
        let source = RgbaImage::from_pixel(240, 135, Rgba([10, 20, 30, 255]));
        let normalized = super::normalize_preview_image(&source);

        assert_eq!(normalized.dimensions(), source.dimensions());
        assert_eq!(normalized.as_raw(), source.as_raw());
        assert_eq!(
            super::PREVIEW_CACHE_CAPACITY * normalized.as_raw().len(),
            4_147_200
        );
    }

    #[test]
    fn shell_image_diagnostics_account_owned_caches_without_process_rss() {
        let mut shell = LiveShell::new().unwrap();
        shell.wallpaper = Some(Arc::new(RgbaImage::new(10, 10)));
        shell.tray = vec![TrayItem {
            id: "fixture".into(),
            title: "Fixture".into(),
            icon: RgbaImage::new(18, 18),
        }];
        shell.tray_icons = panel_tray_icons(&shell.tray);
        shell
            .preview_images
            .insert(WindowId(1), Arc::new(RgbaImage::new(240, 135)));

        let diagnostics = shell.image_cache_diagnostics();
        assert_eq!(diagnostics.wallpaper_entries, 1);
        assert_eq!(diagnostics.wallpaper_bytes, 400);
        assert_eq!(diagnostics.tray_entries, 2);
        assert_eq!(diagnostics.tray_bytes, 18 * 18 * 4 * 2);
        assert_eq!(diagnostics.preview_entries, 1);
        assert_eq!(diagnostics.preview_bytes, 240 * 135 * 4);
    }

    #[test]
    fn preview_cache_churn_releases_previous_group_pixels_and_stays_bounded() {
        let normalized = Arc::new(super::normalize_preview_image(&RgbaImage::from_pixel(
            240,
            135,
            Rgba([10, 20, 30, 255]),
        )));
        let mut cache = HashMap::new();
        for generation in 0..20_u64 {
            let windows = (0..64_u64)
                .map(|offset| OpenWindow {
                    id: WindowId(generation * 100 + offset),
                    application_id: None,
                    active: false,
                    title: String::new(),
                    state: crate::model::WindowState::default(),
                })
                .collect::<Vec<_>>();
            super::retain_preview_generation(&mut cache, &windows);
            for window in windows.iter().take(super::PREVIEW_CACHE_CAPACITY) {
                cache.insert(window.id, Arc::new((*normalized).clone()));
            }
            assert_eq!(cache.len(), super::PREVIEW_CACHE_CAPACITY);
            assert!(cache.keys().all(|id| id.0 / 100 == generation));
            assert_eq!(
                cache
                    .values()
                    .map(|image| image.as_raw().len())
                    .sum::<usize>(),
                super::PREVIEW_CACHE_CAPACITY * normalized.as_raw().len()
            );
        }
        cache.clear();
        assert!(cache.is_empty());
        assert_eq!(Arc::strong_count(&normalized), 1);
    }

    #[test]
    fn tray_icons_are_normalized_without_letter_fallbacks() {
        let item = TrayItem {
            id: "status".into(),
            title: "Status Application".into(),
            icon: RgbaImage::from_pixel(32, 16, Rgba([10, 20, 30, 255])),
        };
        let icons = panel_tray_icons(&[item]);

        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].dimensions(), (18, 18));
        assert!(icons[0].pixels().any(|pixel| pixel.0[3] != 0));
    }

    #[test]
    fn tray_source_keeps_only_four_items_without_discarding_source_quality() {
        let items = (0..7)
            .map(|index| TrayItem {
                id: index.to_string(),
                title: format!("Item {index}"),
                icon: RgbaImage::from_pixel(128, 64, Rgba([index, 0, 0, 255])),
            })
            .collect();
        let normalized = super::normalize_tray_items(items);

        assert_eq!(
            normalized
                .iter()
                .map(|item| item.id.as_str())
                .collect::<Vec<_>>(),
            ["3", "4", "5", "6"]
        );
        assert!(
            normalized
                .iter()
                .all(|item| item.icon.dimensions() == (128, 64))
        );
        assert_eq!(
            normalized
                .iter()
                .map(|item| item.icon.as_raw().len())
                .sum::<usize>(),
            131_072
        );
    }

    #[test]
    #[ignore = "release-only cache timing evidence; debug resampling is intentionally slow"]
    fn shell_image_cache_warm_and_churn_costs_are_measured() {
        use std::time::Duration;

        fn p95(mut samples: Vec<Duration>) -> Duration {
            samples.sort_unstable();
            samples[samples.len() * 95 / 100]
        }

        let wallpaper_warm = p95((0..31)
            .map(|_| {
                let started = Instant::now();
                assert_eq!(
                    super::wallpaper_cache_target((3840, 2160), (2560, 1440)),
                    None
                );
                started.elapsed()
            })
            .collect());
        let wallpaper_source = RgbaImage::from_pixel(2560, 1440, Rgba([10, 20, 30, 255]));
        let wallpaper_churn = p95((0..7)
            .map(|_| {
                let started = Instant::now();
                let _ = crate::icons::resized(&wallpaper_source, 1920, 1080);
                started.elapsed()
            })
            .collect());
        let preview_source = RgbaImage::from_pixel(1280, 720, Rgba([10, 20, 30, 255]));
        let preview_churn = p95((0..11)
            .map(|_| {
                let started = Instant::now();
                let _ = super::normalize_preview_image(&preview_source);
                started.elapsed()
            })
            .collect());
        let tray_source = (0..7)
            .map(|index| TrayItem {
                id: index.to_string(),
                title: String::new(),
                icon: RgbaImage::from_pixel(512, 512, Rgba([index, 0, 0, 255])),
            })
            .collect::<Vec<_>>();
        let tray_churn = p95((0..11)
            .map(|_| {
                let started = Instant::now();
                let _ = super::normalize_tray_items(tray_source.clone());
                started.elapsed()
            })
            .collect());

        println!(
            "shell image caches wallpaper-warm-p95={}ns wallpaper-rebuild-p95={}us preview-normalize-p95={}us tray-generation-p95={}us",
            wallpaper_warm.as_nanos(),
            wallpaper_churn.as_micros(),
            preview_churn.as_micros(),
            tray_churn.as_micros()
        );
        assert!(wallpaper_churn > wallpaper_warm);
        assert!(preview_churn > wallpaper_warm);
        assert!(tray_churn > wallpaper_warm);
    }

    #[test]
    #[ignore = "release-only cache timing evidence; debug resampling is intentionally slow"]
    fn preview_image_cache_cold_warm_churn_and_low_reuse_are_measured() {
        use std::{hint::black_box, time::Duration};

        const SAMPLES: usize = 51;
        const CHURN_REUSES: usize = 8;
        const LOW_REUSE_P95_ADDITION: Duration = Duration::from_micros(500);

        fn percentile(samples: &[Duration], percentile: usize) -> Duration {
            assert!(
                samples.len() >= 20,
                "tail latency needs a useful sample set"
            );
            let mut sorted = samples.to_vec();
            sorted.sort_unstable();
            let rank = (sorted.len() * percentile).div_ceil(100);
            sorted[rank.saturating_sub(1)]
        }

        fn cached_preview(
            cache: &mut HashMap<WindowId, Arc<RgbaImage>>,
            id: WindowId,
            source: &RgbaImage,
        ) -> Arc<RgbaImage> {
            if let Some(image) = cache.get(&id) {
                return Arc::clone(image);
            }
            let image = Arc::new(super::normalize_preview_image(source));
            cache.insert(id, Arc::clone(&image));
            image
        }

        let sources = (0..SAMPLES)
            .map(|index| {
                RgbaImage::from_pixel(
                    640 + (index % 3) as u32,
                    360 + (index % 5) as u32,
                    Rgba([index as u8, 20, 30, 255]),
                )
            })
            .collect::<Vec<_>>();
        let expected = sources
            .iter()
            .map(super::normalize_preview_image)
            .collect::<Vec<_>>();

        let mut cold_cached = Vec::with_capacity(SAMPLES);
        let mut cold_bypass = Vec::with_capacity(SAMPLES);
        let mut warm_cached = Vec::with_capacity(SAMPLES);
        let mut warm_bypass = Vec::with_capacity(SAMPLES);
        let mut churn_cached = Vec::with_capacity(SAMPLES);
        let mut churn_bypass = Vec::with_capacity(SAMPLES);
        let mut low_reuse_cached = Vec::with_capacity(SAMPLES);
        let mut low_reuse_bypass = Vec::with_capacity(SAMPLES);

        for (index, source) in sources.iter().enumerate() {
            let id = WindowId(index as u64);
            let mut cache = HashMap::new();
            let started = Instant::now();
            let cached = cached_preview(&mut cache, id, black_box(source));
            cold_cached.push(started.elapsed());
            let started = Instant::now();
            let bypass = super::normalize_preview_image(black_box(source));
            cold_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "cold cache changed preview pixels");

            let started = Instant::now();
            let cached = cached_preview(&mut cache, id, black_box(source));
            warm_cached.push(started.elapsed());
            let started = Instant::now();
            let bypass = super::normalize_preview_image(black_box(source));
            warm_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "warm cache changed preview pixels");

            let churn_id = WindowId(10_000 + index as u64);
            cache.clear();
            let started = Instant::now();
            let mut cached = cached_preview(&mut cache, churn_id, black_box(source));
            for _ in 1..CHURN_REUSES {
                cached = cached_preview(&mut cache, churn_id, black_box(source));
            }
            churn_cached.push(started.elapsed());
            let started = Instant::now();
            let mut bypass = super::normalize_preview_image(black_box(source));
            for _ in 1..CHURN_REUSES {
                bypass = super::normalize_preview_image(black_box(source));
            }
            churn_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "generation churn changed preview pixels");

            let unique_id = WindowId(20_000 + index as u64);
            let started = Instant::now();
            let cached = cached_preview(&mut cache, unique_id, black_box(source));
            low_reuse_cached.push(started.elapsed());
            let started = Instant::now();
            let bypass = super::normalize_preview_image(black_box(source));
            low_reuse_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "low-reuse cache changed preview pixels");
            assert_eq!(&*cached, &expected[index]);
        }

        let cold_cached_p95 = percentile(&cold_cached, 95);
        let cold_bypass_p95 = percentile(&cold_bypass, 95);
        let warm_cached_p95 = percentile(&warm_cached, 95);
        let warm_bypass_p95 = percentile(&warm_bypass, 95);
        let churn_cached_p95 = percentile(&churn_cached, 95);
        let churn_bypass_p95 = percentile(&churn_bypass, 95);
        let low_reuse_cached_p95 = percentile(&low_reuse_cached, 95);
        let low_reuse_bypass_p95 = percentile(&low_reuse_bypass, 95);

        println!(
            "preview image cache samples={SAMPLES} cold-median={:?}/{:?} cold-p95={cold_cached_p95:?}/{cold_bypass_p95:?} warm-median={:?}/{:?} warm-p95={warm_cached_p95:?}/{warm_bypass_p95:?} churn-reuses={CHURN_REUSES} churn-median={:?}/{:?} churn-p95={churn_cached_p95:?}/{churn_bypass_p95:?} low-reuse-median={:?}/{:?} low-reuse-p95={low_reuse_cached_p95:?}/{low_reuse_bypass_p95:?}",
            percentile(&cold_cached, 50),
            percentile(&cold_bypass, 50),
            percentile(&warm_cached, 50),
            percentile(&warm_bypass, 50),
            percentile(&churn_cached, 50),
            percentile(&churn_bypass, 50),
            percentile(&low_reuse_cached, 50),
            percentile(&low_reuse_bypass, 50),
        );

        assert!(warm_cached_p95 < warm_bypass_p95);
        assert!(churn_cached_p95 < churn_bypass_p95);
        assert!(
            cold_cached_p95 <= cold_bypass_p95 + LOW_REUSE_P95_ADDITION,
            "cold insertion exceeds the predeclared 0.5 ms frame-work allowance"
        );
        assert!(
            low_reuse_cached_p95 <= low_reuse_bypass_p95 + LOW_REUSE_P95_ADDITION,
            "low-reuse insertion exceeds the predeclared 0.5 ms frame-work allowance"
        );
    }

    #[test]
    fn visible_tray_indices_address_the_last_four_items() {
        let items = (0..5)
            .map(|index| TrayItem {
                id: index.to_string(),
                title: String::new(),
                icon: RgbaImage::new(1, 1),
            })
            .collect::<Vec<_>>();

        assert_eq!(visible_tray_item(&items, 0).unwrap().id, "1");
        assert_eq!(visible_tray_item(&items, 3).unwrap().id, "4");
        assert!(visible_tray_item(&items, 4).is_none());
    }

    #[test]
    fn confirmed_audio_changes_coalesce_one_bounded_volume_osd() {
        let mut shell = LiveShell::new().unwrap();
        // The production constructor may discover the developer machine's live
        // default output. Keep this state-machine test independent of that
        // ambient device while the explicit-output case below covers labeling.
        shell.audio = AudioStatus::default();
        assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));

        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: true,
            volume_percent: 47,
            muted: false,
            output_name: None,
        });
        let first_deadline = shell.volume_osd_until.unwrap();
        assert!(shell.surface_visible(SurfaceRole::VolumeOsd));
        shell.volume_osd_scene(320, 88);
        assert!(
            shell
                .volume_osd_host
                .application()
                .label
                .starts_with("Volume 47%")
        );
        assert!(
            shell
                .volume_osd_host
                .accessibility_nodes()
                .iter()
                .any(|node| {
                    node.semantic_role == Some(SemanticRole::Status)
                        && node
                            .label
                            .as_deref()
                            .is_some_and(|label| label.starts_with("Volume 47%"))
                })
        );

        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: true,
            volume_percent: 47,
            muted: true,
            output_name: None,
        });
        assert!(shell.volume_osd_until.unwrap() >= first_deadline);
        shell.volume_osd_scene(320, 88);
        assert!(
            shell
                .volume_osd_host
                .application()
                .label
                .starts_with("Muted")
        );

        let outcome = shell.poll_deadlines(Instant::now() + Duration::from_secs(2));
        assert!(outcome.visibility_changed);
        assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));

        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: false,
            volume_percent: 0,
            muted: false,
            output_name: None,
        });
        assert!(!shell.surface_visible(SurfaceRole::VolumeOsd));

        shell.global_shortcut(GlobalShortcut::LockState { locked: true });
        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: true,
            volume_percent: 47,
            muted: false,
            output_name: Some("Private Bluetooth Headset".into()),
        });
        shell.volume_osd_scene(320, 88);
        assert_eq!(
            shell.volume_osd_host.application().label,
            "Volume 47% · Audio output"
        );
        assert!(
            !shell
                .volume_osd_host
                .application()
                .label
                .contains("Private")
        );
    }
