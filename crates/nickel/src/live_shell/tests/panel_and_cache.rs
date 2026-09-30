    #[test]
    fn full_preview_refresh_moves_provider_pixels_and_preserves_unchanged_identity() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(Vec::new());
        let id = WindowId(42);
        shell.windows = vec![OpenWindow {
            id,
            application_id: None,
            active: true,
            title: "Preview fixture".into(),
            state: Default::default(),
        }];
        shell.preview_group = Some(0);
        for (width, color) in [(240, 10), (240, 10), (240, 11), (135, 11)] {
            let frame = RgbaImage::from_pixel(width, 135, Rgba([color, 20, 30, 255]));
            let allocation = frame.as_ptr();
            let previous = shell.preview_images.get(&id).map(Arc::downgrade);
            let unchanged = shell.preview_images.get(&id).is_some_and(|old| **old == frame);
            let mut supplied = Some(frame);
            shell.preview_refresh_deadline = None;
            shell.refresh_fast_changes_with_preview_source(|_, requested| {
                assert_eq!(requested, id);
                Some(crate::model::WindowPreview { window: id, image: supplied.take().unwrap() })
            });
            assert!(supplied.is_none());
            let current = &shell.preview_images[&id];
            if unchanged {
                assert!(Arc::ptr_eq(&previous.unwrap().upgrade().unwrap(), current));
            } else {
                assert_eq!(current.as_ptr(), allocation);
                assert!(previous.is_none_or(|previous| previous.upgrade().is_none()));
            }
        }
        let last = Arc::downgrade(&shell.preview_images[&id]);
        shell.close_window_preview();
        assert!(shell.preview_images.is_empty());
        assert!(last.upgrade().is_none());
    }

    #[test]
    fn owned_preview_refresh_moves_pixels_preserves_identity_and_retires_old_images() {
        let mut cache = HashMap::new();
        let id = WindowId(42);
        let source = RgbaImage::from_pixel(240, 135, Rgba([10, 20, 30, 255]));
        let pixels = source.as_ptr();
        assert!(super::update_preview_image(&mut cache, id, source));
        assert_eq!(cache[&id].as_ptr(), pixels, "the incoming pixel allocation is moved");
        let first = Arc::downgrade(&cache[&id]);
        let unchanged = (*cache[&id]).clone();
        assert!(!super::update_preview_image(&mut cache, id, unchanged));
        assert!(Arc::ptr_eq(&first.upgrade().unwrap(), &cache[&id]));
        for source in [
            RgbaImage::from_pixel(240, 135, Rgba([11, 20, 30, 255])),
            RgbaImage::from_pixel(135, 240, Rgba([11, 20, 30, 255])),
        ] {
            let pixels = source.as_ptr();
            assert!(super::update_preview_image(&mut cache, id, source));
            assert_eq!(cache[&id].as_ptr(), pixels);
        }
        assert!(first.upgrade().is_none());
        let last = Arc::downgrade(&cache[&id]);
        super::retain_preview_generation(&mut cache, &[]);
        assert!(cache.is_empty());
        assert!(last.upgrade().is_none());
    }

    #[test]
    #[ignore = "release-only owned preview refresh comparison"]
    fn owned_preview_refresh_release_evidence() {
        use crate::allocation_counter::thread_allocation_operations;
        use std::hint::black_box;
        for (width, height, changing) in [(240, 135, false), (240, 135, true), (480, 270, false), (480, 270, true)] {
            let mut legacy = HashMap::new();
            let mut moved = HashMap::new();
            // Warm admission and map capacity before sampling refresh work.
            // Changing frames start at a different color from this cached seed.
            let seed = RgbaImage::from_pixel(width, height, Rgba([if changing { 0 } else { 17 }, 20, 30, 255]));
            legacy.insert(WindowId(1), Arc::new(seed.clone()));
            moved.insert(WindowId(1), Arc::new(seed));
            let mut legacy_time = Duration::ZERO;
            let mut moved_time = Duration::ZERO;
            let mut cloned_payload = 0;
            let mut peak_extra_pixel_capacity = 0;
            let mut legacy_allocations = 0;
            let mut moved_allocations = 0;
            for index in 0..1000 {
                let color = if changing { ((index + 1) % 251) as u8 } else { 17 };
                let source = RgbaImage::from_pixel(width, height, Rgba([color, 20, 30, 255]));
                let incoming = source.clone(); // provider allocations excluded from timing
                let allocations_before = thread_allocation_operations();
                let started = Instant::now();
                let copy = Arc::new(super::legacy_preview_copy(black_box(&source)));
                assert_ne!(source.as_ptr(), copy.as_ptr());
                cloned_payload += source.as_raw().len();
                // Both allocations are alive here: this measures the clone's
                // additional pixel storage, not whole-process peak memory.
                peak_extra_pixel_capacity = peak_extra_pixel_capacity.max(copy.as_raw().capacity());
                if legacy.get(&WindowId(1)).is_none_or(|current: &Arc<RgbaImage>| **current != *copy) {
                    legacy.insert(WindowId(1), copy);
                }
                legacy_time += started.elapsed();
                legacy_allocations += thread_allocation_operations() - allocations_before;
                let pixels = incoming.as_ptr();
                let allocations_before = thread_allocation_operations();
                let started = Instant::now();
                let changed = super::update_preview_image(&mut moved, WindowId(1), black_box(incoming));
                moved_time += started.elapsed();
                moved_allocations += thread_allocation_operations() - allocations_before;
                if changed {
                    assert_eq!(moved[&WindowId(1)].as_ptr(), pixels);
                }
                assert_eq!(legacy, moved);
            }
            assert!(legacy_allocations >= moved_allocations + 1000);
            assert!(peak_extra_pixel_capacity >= width as usize * height as usize * 4);
            println!("owned-preview changing={changing} dimensions={width}x{height} frames=1000 legacy={legacy_time:?} moved={moved_time:?} eliminated_pixel_copy_bytes={cloned_payload} legacy_peak_extra_pixel_capacity_bytes={peak_extra_pixel_capacity} legacy_allocation_calls={legacy_allocations} moved_allocation_calls={moved_allocations}; allocation calls include Arc/map storage; extra capacity measures only the redundant clone, not process peak memory; payload excludes Arc headers; provider allocations, RSS and GPU storage excluded");
        }
    }

    #[test]
    fn rendering_a_sibling_panel_preserves_pointer_capture_and_keyboard_focus() {
        let mut shell = LiveShell::new().unwrap();
        shell.set_panel_output("left");
        shell.scene(SurfaceRole::Taskbar, 1280, 56);
        let target = super::taskbar_plugin_control_bounds(
            shell.plugin_taskbar_host.as_ref().unwrap(),
            "taskbar-launcher",
        )
        .unwrap();
        let point = Point { x: target.origin.x + target.size.width / 2.0, y: target.origin.y + target.size.height / 2.0 };
        shell.panel_host_ui(UiEvent::FocusNext, 1280);
        shell.panel_host_ui(UiEvent::PointerMoved(point), 1280);
        shell.panel_host_ui(UiEvent::PointerPressed(point), 1280);
        let before = shell.plugin_taskbar_host.as_ref().unwrap().inspect();
        shell.panel_scene_for_output(Some("right"), 800, 56);
        assert!(std::rc::Rc::ptr_eq(
            &shell
                .plugin_taskbar_host
                .as_ref()
                .unwrap()
                .application()
                .shared_runtime(),
            &shell.plugin_taskbar_hosts[&Some("right".into())]
                .application()
                .shared_runtime(),
        ));
        let after = shell.plugin_taskbar_host.as_ref().unwrap().inspect();
        assert_eq!(before.pointer_capture, after.pointer_capture);
        assert_eq!(before.pointer_hover, after.pointer_hover);
        assert_eq!(before.keyboard_focus, after.keyboard_focus);
        assert_eq!(shell.panel_output.as_deref(), Some("left"));
        assert!(shell.plugin_taskbar_hosts[&Some("right".into())].inspect().pointer_hover.is_none());
        assert!(shell.plugin_taskbar_hosts[&Some("right".into())].inspect().pointer_capture.is_none());
        shell.panel_host_ui(UiEvent::PointerReleased(point), 1280);
        assert!(shell.launcher_visible);
    }

    #[test]
    fn taskbar_uses_keyed_panel_scene_and_ui_routes() {
        let mut shell = LiveShell::new().unwrap();
        let key = shell.taskbar_surface_key().unwrap();
        shell.set_panel_output("left");
        shell.scene(SurfaceRole::Taskbar, 1280, 56);
        assert!(!shell
            .plugin_panel_scene_for_output(&key, Some("right"), 1280, 56)
            .unwrap()
            .is_empty());
        assert_eq!(shell.panel_output.as_deref(), Some("left"));

        let target = super::taskbar_plugin_control_bounds(
            shell.plugin_taskbar_host.as_ref().unwrap(),
            "taskbar-launcher",
        )
        .unwrap();
        let point = Point {
            x: target.origin.x + target.size.width / 2.0,
            y: target.origin.y + target.size.height / 2.0,
        };
        assert!(shell.plugin_panel_host_ui_for(&key, UiEvent::PointerPressed(point), 1280, 56));
        assert!(shell.plugin_panel_host_ui_for(&key, UiEvent::PointerReleased(point), 1280, 56));
        assert!(shell.launcher_visible);

        shell
            .set_plugin_enabled(&crate::plugin_panel::taskbar_manifest().id, false)
            .unwrap();
        assert!(shell.plugin_panel_scene_for_output(&key, Some("left"), 1280, 56).is_none());
        assert!(!shell.plugin_panel_host_ui_for(&key, UiEvent::PointerPressed(point), 1280, 56));
    }

    #[test]
    fn bundled_panel_reconciles_its_jsx_root_geometry() {
        let mut shell = LiveShell::new().unwrap();
        let source = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/hello-panel/main.js"
        ))
        .unwrap()
        .replace("bottomOffset: 24", "bottomOffset: 12");
        let application = crate::plugin_panel::PluginPanelApplication::new(&source).unwrap();
        let grant = crate::plugin_panel::surface();
        shell.plugin_surface_hosts.insert(
            shell.primary_panel_key(),
            (
                grant.clone(),
                nickel_ui::UiHost::new(application, grant.width, grant.height),
            ),
        );
        assert_eq!(shell.plugin_panel_surface().bottom_offset, 24);
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: shell.primary_panel_key.plugin_id.clone(),
            surface_id: grant.id.clone(),
        };
        shell.plugin_panel_scene(&key, grant.width, grant.height);
        assert_eq!(shell.plugin_panel_surface().bottom_offset, 12);
    }

    #[test]
    fn primary_panel_events_use_the_keyed_plugin_host() {
        let mut shell = LiveShell::new().unwrap();
        let grant = crate::plugin_panel::surface();
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: shell.primary_panel_key.plugin_id.clone(),
            surface_id: grant.id.clone(),
        };
        shell.plugin_surface_hosts.insert(
            key.clone(),
            (
                grant.clone(),
                nickel_ui::UiHost::new(
                    crate::plugin_panel::PluginPanelApplication::bundled().unwrap(),
                    grant.width,
                    grant.height,
                ),
            ),
        );
        let target = shell
            .plugin_panel_host_ref(&key)
            .unwrap()
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open dialog".into(),
            })
            .unwrap();
        let event = UiEvent::AccessibilityActivate(target.id);
        assert!(!shell.shell_role_host_ui(
            SurfaceRole::Panel,
            event.clone(),
            grant.width,
            grant.height,
        ));
        assert!(shell.plugin_panel_host_ui_for(&key, event, grant.width, grant.height));
        assert!(shell
            .plugin_panel_host_ref(&key)
            .unwrap()
            .inspect()
            .open_overlay
            .is_some());
    }

    #[test]
    fn taskbar_keyed_controller_activates_focused_launcher_button() {
        let mut shell = LiveShell::new().unwrap();
        let key = shell.taskbar_surface_key().unwrap();
        shell.scene(SurfaceRole::Taskbar, 1280, 56);
        let host = shell.plugin_taskbar_host.as_mut().unwrap();
        let target = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Open Nickel Start".into(),
            })
            .unwrap();
        assert!(host.request_focus(target.id).changed);

        assert!(shell.plugin_panel_host_controller_for(
            &key,
            nickel_ui::ControllerAction::Confirm,
            1280,
            56,
        ));
        assert!(shell.launcher_visible);
    }

    #[test]
    fn taskbar_keyed_normalized_pointer_opens_launcher() {
        let mut shell = LiveShell::new().unwrap();
        let key = shell.taskbar_surface_key().unwrap();
        shell.scene(SurfaceRole::Taskbar, 1280, 56);
        let bounds = super::taskbar_plugin_control_bounds(
            shell.plugin_taskbar_host.as_ref().unwrap(),
            "taskbar-launcher",
        )
        .unwrap();
        let point = nickel_input::Point {
            x: f64::from(bounds.origin.x + bounds.size.width / 2.0),
            y: f64::from(bounds.origin.y + bounds.size.height / 2.0),
        };
        let pointer = |edge, order| {
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(order),
                position: Some(point),
                button: nickel_input::PointerButton::Primary,
                edge,
            })
        };
        shell.plugin_panel_host_input_for(&key, pointer(nickel_input::KeyEdge::Pressed, 1), 1280, 56);
        assert!(shell.plugin_panel_host_input_for(
            &key,
            pointer(nickel_input::KeyEdge::Released, 2),
            1280,
            56,
        ));
        assert!(shell.launcher_visible);
    }

    #[test]
    fn taskbar_memory_counts_shared_images_once_across_outputs() {
        let mut shell = LiveShell::new().unwrap();
        shell.set_panel_output("left");
        shell.scene(SurfaceRole::Taskbar, 1280, 56);
        shell.panel_scene_for_output(Some("right"), 1280, 56);

        let allocations = shell
            .plugin_taskbar_host
            .iter()
            .chain(shell.plugin_taskbar_hosts.values())
            .flat_map(|host| host.application().retained_image_allocations())
            .collect::<Vec<_>>();
        let mut seen = std::collections::HashSet::new();
        let unique_image_bytes = allocations
            .iter()
            .filter(|(address, _)| seen.insert(*address))
            .map(|(_, bytes)| *bytes)
            .sum::<u64>();
        assert!(allocations.iter().map(|(_, bytes)| bytes).sum::<u64>() > unique_image_bytes);
        let frame_bytes = shell.plugin_taskbar_memory.values().copied().sum::<u64>();
        assert_eq!(
            shell
                .plugin_registry()
                .get(&crate::plugin_panel::taskbar_manifest().id)
                .unwrap()
                .memory
                .native_ui_bytes,
            Some(frame_bytes + unique_image_bytes)
        );

        #[cfg(target_os = "linux")]
        {
            let runtime = shell
                .plugin_taskbar_host
                .as_ref()
                .unwrap()
                .application()
                .shared_runtime();
            assert!(runtime
                .borrow_mut()
                .eval_json::<bool>("__activeSurface === 'taskbar-output:right' || __surfaceStates.has('taskbar-output:right')")
                .unwrap());
            shell.retain_panel_outputs(&[crate::internal_shell::InternalOutput {
                x: 0,
                y: 0,
                name: "left".into(),
                width: 1280,
                height: 720,
                scale: 1.0,
            }]);
            assert!(!shell.plugin_taskbar_hosts.contains_key(&Some("right".into())));
            assert!(!runtime
                .borrow_mut()
                .eval_json::<bool>("__surfaceStates.has('taskbar-output:right')")
                .unwrap());
            assert!(runtime
                .borrow_mut()
                .eval_json::<bool>("__activeSurface !== 'taskbar-output:right' || __componentHooks.size === 0")
                .unwrap());
            assert_eq!(
                shell
                    .plugin_registry()
                    .get(&crate::plugin_panel::taskbar_manifest().id)
                    .unwrap()
                    .memory
                    .native_ui_bytes,
                Some(shell.plugin_taskbar_memory.values().copied().sum::<u64>() + unique_image_bytes)
            );
        }
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
    fn panel_scene_rebuilds_when_a_window_feed_adds_an_application() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Taskbar, 1280, 56);
        let before = shell.panel_change_token;
        shell.windows.push(OpenWindow {
            id: WindowId(77),
            application_id: Some(ApplicationId::new("google-chrome")),
            active: true,
            title: "Chrome".into(),
            state: crate::model::WindowState::default(),
        });

        let _ = shell.scene(SurfaceRole::Taskbar, 1280, 56);

        assert_ne!(shell.panel_change_token, before);
        assert!(shell
            .plugin_taskbar_host
            .as_ref()
            .unwrap()
            .application()
            .rendered_taskbar_item_matches(0, "google-chrome"));
    }

    #[test]
    fn jsx_taskbar_animates_project_faces() {
        let mut shell = LiveShell::new().unwrap();
        assert!(shell.plugin_taskbar_host.is_some());
        shell.launcher = crate::launcher::Launcher::new(Vec::new());
        shell.windows = vec![OpenWindow {
            id: WindowId(1),
            application_id: Some(ApplicationId::new("io.nickel.codex.project.alpha")),
            active: true,
            title: "Codex".into(),
            state: Default::default(),
        }];

        shell.scene(SurfaceRole::Taskbar, 1280, 56);
        let (clock, _) = super::panel_clock_text();
        let initial = shell.taskbar_plugin_projection(&clock).1["task:0"].0;
        let due = shell.panel_pet_deadline.expect("JSX pet animation deadline");
        assert!(due <= Instant::now() + Duration::from_millis(400));

        assert!(shell.poll_host_deadlines(due).contains(&SurfaceRole::Taskbar));
        let animated = shell.taskbar_plugin_projection(&clock).1["task:0"].0;
        assert_ne!(initial, animated);
        assert!(shell.panel_pet_deadline.is_some_and(|deadline| deadline > due));
    }

    #[test]
    fn jsx_taskbar_pin_drag_reorders_only_the_current_pinned_item() {
        let directory = tempfile::tempdir().unwrap();
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(vec![
            crate::model::Application::new("first".into(), "First".into(), None, None, Some(vec!["first".into()])),
            crate::model::Application::new("second".into(), "Second".into(), None, None, Some(vec!["second".into()])),
        ]);
        shell.launcher.set_pins(vec![("first".into(), 0), ("second".into(), 1)]);
        shell.launcher_preferences_path = Some(directory.path().join("launcher-preferences"));
        shell.windows.clear();

        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::MoveTaskbarPin {
            index: 0,
            id: "stale".into(),
            direction: 1,
        }]);
        assert_eq!(shell.panel_groups()[0].application_id.as_ref().unwrap().as_str(), "first");

        shell.apply_plugin_effects(vec![crate::plugin_panel::PluginEffect::MoveTaskbarPin {
            index: 0,
            id: "first".into(),
            direction: 1,
        }]);
        assert_eq!(shell.panel_groups()[0].application_id.as_ref().unwrap().as_str(), "second");
    }

    #[test]
    fn internal_file_window_keeps_its_icon_when_titled_after_a_folder() {
        let mut shell = LiveShell::new().unwrap();
        shell.launcher = crate::launcher::Launcher::new(Vec::new());
        shell.windows = vec![OpenWindow {
            id: WindowId(1),
            application_id: Some(ApplicationId::new("nickel-file")),
            active: true,
            title: "Music".into(),
            state: Default::default(),
        }];

        let _ = shell.scene(SurfaceRole::Taskbar, 1280, 56);

        let (clock, _) = super::panel_clock_text();
        let (projection, images) = shell.taskbar_plugin_projection(&clock);
        assert_eq!(projection.items[0].name, "Music");
        assert_eq!(images["task:0"].0, 0x3001);
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

        shell.apply_panel_action(super::TaskbarAction::TaskContext(0));

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
            .application_menu_plugin_host
            .as_ref()
            .expect("JSX application-only task menu host");
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
        shell.apply_panel_action(super::TaskbarAction::TaskContext(0));
        assert!(shell.application_menu_target.is_some());

        shell.apply_application_menu_action(
            crate::window_preview::ApplicationMenuAction::CloseAll,
        );

        assert!(shell.application_menu_target.is_none());
    }

    #[test]
    fn every_advertised_shell_deadline_is_consumed_when_due() {
        let mut shell = LiveShell::new().unwrap();
        let _ = shell.scene(SurfaceRole::Desktop, 1280, 720);
        let _ = shell.scene(SurfaceRole::Taskbar, 1280, 56);
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
    fn panel_hover_is_projected_only_on_the_output_that_received_pointer_input() {
        let mut shell = LiveShell::new().unwrap();
        shell.panel_hover = Some(super::TaskbarHover::Launcher);
        shell.panel_hover_output = Some("DP-1".into());

        shell.set_panel_output("DP-1");
        assert_eq!(shell.panel_hover_output, shell.panel_output);
        shell.set_panel_output("HDMI-A-1");
        assert_ne!(shell.panel_hover_output, shell.panel_output);
        assert!(!shell.panel_pointer_left());
        assert_eq!(shell.panel_hover, Some(super::TaskbarHover::Launcher));

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
        shell.apply_panel_action(super::TaskbarAction::Codex);
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

        let scene = shell.desktop_scene(1920, 1080);
        let rebuilt = shell.desktop_host.inspect();

        assert_eq!(rebuilt.frame_generation, initial.frame_generation + 1);
        assert!(
            scene.iter().any(|command| matches!(command,
                nickel_ui::backend::PaintCommand::Image { id: 1, .. })),
            "the native desktop must paint the arriving wallpaper"
        );
    }

    #[test]
    fn preview_cache_retains_authoritative_source_aspect_for_ui_containment() {
        let source = RgbaImage::from_pixel(240, 135, Rgba([10, 20, 30, 255]));
        let expected = source.clone();
        let mut cache = HashMap::new();
        super::update_preview_image(&mut cache, WindowId(1), source);
        let retained = &cache[&WindowId(1)];

        assert_eq!(retained.dimensions(), expected.dimensions());
        assert_eq!(retained.as_raw(), expected.as_raw());
        assert_eq!(
            super::PREVIEW_CACHE_CAPACITY * retained.as_raw().len(),
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
        shell.preview_images.insert(WindowId(2), Arc::new(RgbaImage::new(320, 200)));
        let restricted = shell.image_cache_diagnostics_for_previews(|id| id == WindowId(1));
        assert_eq!(restricted, diagnostics, "unprojected preview pixels must not contribute even to byte counts");
        let empty = shell.image_cache_diagnostics_for_previews(|_| false);
        assert_eq!(empty.preview_entries, 0);
        assert_eq!(empty.preview_bytes, 0);
        assert_eq!(empty.wallpaper_bytes, diagnostics.wallpaper_bytes);
    }

    #[test]
    fn preview_cache_churn_releases_previous_group_pixels_and_stays_bounded() {
        let normalized = Arc::new(super::legacy_preview_copy(&RgbaImage::from_pixel(
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
                let _ = super::legacy_preview_copy(&preview_source);
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
            let image = Arc::new(super::legacy_preview_copy(source));
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
            .map(super::legacy_preview_copy)
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
            let bypass = super::legacy_preview_copy(black_box(source));
            cold_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "cold cache changed preview pixels");

            let started = Instant::now();
            let cached = cached_preview(&mut cache, id, black_box(source));
            warm_cached.push(started.elapsed());
            let started = Instant::now();
            let bypass = super::legacy_preview_copy(black_box(source));
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
            let mut bypass = super::legacy_preview_copy(black_box(source));
            for _ in 1..CHURN_REUSES {
                bypass = super::legacy_preview_copy(black_box(source));
            }
            churn_bypass.push(started.elapsed());
            assert_eq!(&*cached, &bypass, "generation churn changed preview pixels");

            let unique_id = WindowId(20_000 + index as u64);
            let started = Instant::now();
            let cached = cached_preview(&mut cache, unique_id, black_box(source));
            low_reuse_cached.push(started.elapsed());
            let started = Instant::now();
            let bypass = super::legacy_preview_copy(black_box(source));
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
        assert!(shell
            .plugin_panel_host_ref(&crate::plugin_panel::volume_osd_surface_key())
            .is_some());
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
        shell.scene(SurfaceRole::VolumeOsd, 320, 88);
        assert!(shell.audio_plugin_data()["label"].as_str().unwrap().starts_with("Volume 47%"));
        assert!(
            shell
                .plugin_panel_host_ref(&crate::plugin_panel::volume_osd_surface_key())
                .unwrap()
                .accessibility_nodes()
                .iter()
                .any(|node| {
                    node.semantic_role == Some(SemanticRole::Text)
                        && node
                            .label
                            .as_deref()
                            .is_some_and(|label| label.starts_with("Volume 47%"))
                })
        );
        assert!(shell
            .plugin_registry()
            .get(&crate::plugin_panel::volume_osd_manifest().id)
            .unwrap()
            .memory
            .native_ui_bytes
            .is_some());

        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: true,
            volume_percent: 47,
            muted: true,
            output_name: None,
        });
        assert!(shell.volume_osd_until.unwrap() >= first_deadline);
        shell.scene(SurfaceRole::VolumeOsd, 320, 88);
        assert!(shell.audio_plugin_data()["label"].as_str().unwrap().starts_with("Muted"));

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

        // Windows routes this shortcut to the real LockWorkStation API. The
        // protected-state assertion belongs to the Linux shell reducer here.
        #[cfg(not(target_os = "windows"))]
        shell.global_shortcut(GlobalShortcut::LockState { locked: true });
        #[cfg(target_os = "windows")]
        {
            shell.locked = true;
        }
        shell.global_shortcut(GlobalShortcut::AudioChanged {
            available: true,
            volume_percent: 47,
            muted: false,
            output_name: Some("Private Bluetooth Headset".into()),
        });
        shell.scene(SurfaceRole::VolumeOsd, 320, 88);
        assert_eq!(shell.audio_plugin_data()["label"], "Volume 47% · Audio output");
        assert!(
            !shell
                .plugin_panel_host_ref(&crate::plugin_panel::volume_osd_surface_key())
                .unwrap()
                .accessibility_nodes()
                .iter()
                .any(|node| node.label.as_deref().is_some_and(|label| label.contains("Private")))
        );
    }
