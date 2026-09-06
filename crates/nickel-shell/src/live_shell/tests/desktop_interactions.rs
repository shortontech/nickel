    #[test]
    fn desktop_surface_projects_only_its_output_and_has_no_idle_tile_backgrounds() {
        use std::{ffi::OsString, path::PathBuf};
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.set_outputs(vec![
            nickel_file::desktop::DesktopOutput {
                id: "left".into(),
                work_area: nickel_file::desktop::Rect {
                    x: -400.0,
                    y: 0.0,
                    width: 400.0,
                    height: 600.0,
                },
                scale: 1.0,
            },
            nickel_file::desktop::DesktopOutput {
                id: "right".into(),
                work_area: nickel_file::desktop::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 400.0,
                    height: 600.0,
                },
                scale: 1.5,
            },
        ]);
        desktop.layout.reconcile(vec![(
            nickel_file::FileIdentity(1, 2),
            nickel_file::FileEntry {
                display_name_override: None,
                name: OsString::from("document.txt"),
                path: PathBuf::from("/desktop/document.txt"),
                is_directory: false,
                size: Some(12),
                modified: None,
            },
        )]);
        desktop.set_active_output(
            "left".into(),
            nickel_file::desktop::Point { x: -400.0, y: 0.0 },
            1.0,
        );
        let _ = desktop.prepare_icons();
        let mut host = UiHost::new(desktop, 400, 600);
        let entry = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::GridCell))
            .expect("desktop entry belongs to the active output");
        assert_eq!(entry.label.as_deref(), Some("document.txt"));
        host.application_mut().set_active_output(
            "right".into(),
            nickel_file::desktop::Point { x: 0.0, y: 0.0 },
            1.5,
        );
        host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        assert!(
            host.accessibility_nodes()
                .iter()
                .all(|node| node.semantic_role != Some(SemanticRole::GridCell))
        );
        host.application_mut().set_active_output(
            "left".into(),
            nickel_file::desktop::Point { x: -400.0, y: 0.0 },
            1.0,
        );
        let _ = host.application_mut().prepare_icons();
        host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        let entry = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::GridCell))
            .expect("desktop entry belongs to Nickel UI semantic authority");
        assert_eq!(entry.label.as_deref(), Some("document.txt"));
        assert_eq!(entry.rect.size.width, 96.0);
        assert!(entry.actions.contains(&ActionKind::Activate));
        assert!(entry.actions.contains(&ActionKind::ContextMenu));
    }

    #[test]
    fn desktop_live_input_rebuilds_selection_and_keyboard_navigation() {
        use std::{ffi::OsString, path::PathBuf};
        let mut shell = LiveShell::new().unwrap();
        let artwork = Arc::new(image::RgbaImage::from_pixel(
            3,
            3,
            image::Rgba([24, 96, 220, 255]),
        ));
        let application = shell.desktop_host.application_mut();
        application.set_outputs(vec![nickel_file::desktop::DesktopOutput {
            id: "primary".into(),
            work_area: nickel_file::desktop::Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 600.0,
            },
            scale: 1.0,
        }]);
        application.set_active_output(
            "primary".into(),
            nickel_file::desktop::Point::default(),
            1.0,
        );
        application.layout.reconcile(
            ["first.txt", "second.txt"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| {
                    (
                        nickel_file::FileIdentity(41, index as u64 + 1),
                        nickel_file::FileEntry {
                            display_name_override: None,
                            name: OsString::from(name),
                            path: PathBuf::from("/desktop").join(name),
                            is_directory: false,
                            size: Some(1),
                            modified: None,
                        },
                    )
                })
                .collect(),
        );
        application
            .icon_cache
            .insert(PathBuf::from("/desktop/first.txt"), Arc::clone(&artwork));
        let before_commands = shell.scene(SurfaceRole::Desktop, 400, 600);
        assert!(nickel_ui::backend::contains_image_pixels(
            &before_commands,
            &artwork
        ));
        let before_token = shell.desktop_change_token;

        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(1),
                button: nickel_input::PointerButton::Primary,
                edge: nickel_input::KeyEdge::Pressed,
                position: Some(nickel_input::Point { x: 4.0, y: 4.0 }),
            },
        )));
        assert_eq!(
            shell.desktop_host.application().layout.active(),
            Some(nickel_file::desktop::DesktopEntryId(
                nickel_file::FileIdentity(41, 1)
            ))
        );
        assert_ne!(shell.desktop_change_token, before_token);
        assert_ne!(shell.desktop_host.commands(), before_commands);
        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(2),
                button: nickel_input::PointerButton::Primary,
                edge: nickel_input::KeyEdge::Released,
                position: Some(nickel_input::Point { x: 4.0, y: 4.0 }),
            },
        )));
        let selected_commands = shell.scene(SurfaceRole::Desktop, 400, 600);
        assert!(shell.desktop_host.application().layout.icons_visible());
        assert!(nickel_ui::backend::contains_image_pixels(
            &selected_commands,
            &artwork
        ));

        let pointer_token = shell.desktop_change_token;
        assert!(
            shell.desktop_input(nickel_input::InputEvent::Key(nickel_input::KeyEvent {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(3),
                physical: nickel_input::PhysicalKey::Code(KeyCode::ArrowDown),
                logical: nickel_input::LogicalKey::Named(nickel_input::NamedKey::ArrowDown),
                location: nickel_input::KeyLocation::Standard,
                edge: nickel_input::KeyEdge::Pressed,
                repeat: false,
                modifiers: nickel_input::ModifierState::default(),
            },))
        );
        assert_eq!(
            shell.desktop_host.application().layout.active(),
            Some(nickel_file::desktop::DesktopEntryId(
                nickel_file::FileIdentity(41, 2)
            ))
        );
        assert_ne!(shell.desktop_change_token, pointer_token);
    }

    #[test]
    fn desktop_selection_marquee_preserves_icons_through_delayed_refresh_poll_and_release() {
        use std::{ffi::OsString, path::PathBuf};

        let mut shell = LiveShell::new().unwrap();
        let path = PathBuf::from("/desktop/visible-through-selection.txt");
        let entry = nickel_file::FileEntry {
            display_name_override: None,
            name: OsString::from("visible-through-selection.txt"),
            path: path.clone(),
            is_directory: false,
            size: Some(17),
            modified: None,
        };
        let identity = nickel_file::FileIdentity(51, 1);
        let artwork = Arc::new(image::RgbaImage::from_pixel(
            3,
            3,
            image::Rgba([24, 96, 220, 255]),
        ));
        let application = shell.desktop_host.application_mut();
        application.browser = None;
        application.watch = None;
        application.set_outputs(vec![nickel_file::desktop::DesktopOutput {
            id: "primary".into(),
            work_area: nickel_file::desktop::Rect {
                x: 0.0,
                y: 0.0,
                width: 400.0,
                height: 600.0,
            },
            scale: 1.0,
        }]);
        application.set_active_output(
            "primary".into(),
            nickel_file::desktop::Point::default(),
            1.0,
        );
        application
            .layout
            .reconcile(vec![(identity, entry.clone())]);
        application
            .icon_cache
            .insert(path.clone(), Arc::clone(&artwork));
        let initial = shell.scene(SurfaceRole::Desktop, 400, 600);
        assert!(nickel_ui::backend::contains_image_pixels(
            &initial, &artwork
        ));

        let press = nickel_input::Point { x: 300.0, y: 300.0 };
        let drag = nickel_input::Point { x: 0.0, y: 0.0 };
        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(1),
                button: nickel_input::PointerButton::Primary,
                edge: nickel_input::KeyEdge::Pressed,
                position: Some(press),
            },
        )));
        let host_token_before_motion = shell.desktop_change_token;
        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Motion {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(2),
                position: drag,
                delta: Some(nickel_input::Vector {
                    x: -300.0,
                    y: -300.0,
                }),
            },
        )));
        assert_eq!(
            shell.desktop_change_token, host_token_before_motion,
            "pointer motion is accumulated without rebuilding the desktop host"
        );
        assert!(shell.desktop_application_dirty);
        let dragging = shell.scene(SurfaceRole::Desktop, 400, 600);
        assert!(!shell.desktop_application_dirty);
        assert!(nickel_ui::backend::contains_image_pixels(
            &dragging, &artwork
        ));
        let dragging_overlays = shell
            .desktop_host
            .application()
            .frame_overlays(ViewContext::new(
                Rect::new(0.0, 0.0, 400.0, 600.0),
                InputModality::Pointer,
            ));
        assert!(dragging_overlays.iter().any(|overlay| matches!(
            overlay,
            FrameOverlay::SelectionMarquee { fill: Some(color), .. }
                if (*color >> 24) > 0 && (*color >> 24) < 0xff
        )));

        // Model the successful watcher reconciliation which follows icon
        // metadata access, then run the actual scheduled host poll path.
        let application = shell.desktop_host.application_mut();
        application.layout.reconcile(vec![(identity, entry)]);
        application.directory_generation = application.directory_generation.wrapping_add(1);
        shell.desktop_deadline = Some(Instant::now());
        let due = Instant::now() + Duration::from_millis(300);
        let _ = shell.poll_host_deadlines(due);
        let after_poll = shell.scene(SurfaceRole::Desktop, 400, 600);
        assert!(nickel_ui::backend::contains_image_pixels(
            &after_poll,
            &artwork
        ));
        let after_poll_overlays =
            shell
                .desktop_host
                .application()
                .frame_overlays(ViewContext::new(
                    Rect::new(0.0, 0.0, 400.0, 600.0),
                    InputModality::Pointer,
                ));
        assert!(after_poll_overlays.iter().any(|overlay| matches!(
            overlay,
            FrameOverlay::SelectionMarquee { fill: Some(color), .. }
                if (*color >> 24) > 0 && (*color >> 24) < 0xff
        )));

        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(3),
                button: nickel_input::PointerButton::Primary,
                edge: nickel_input::KeyEdge::Released,
                position: Some(drag),
            },
        )));
        let released = shell.scene(SurfaceRole::Desktop, 400, 600);
        assert!(nickel_ui::backend::contains_image_pixels(
            &released, &artwork
        ));
        assert!(
            shell
                .desktop_host
                .application()
                .frame_overlays(ViewContext::new(
                    Rect::new(0.0, 0.0, 400.0, 600.0),
                    InputModality::Pointer,
                ))
                .iter()
                .all(|overlay| !matches!(overlay, FrameOverlay::SelectionMarquee { .. }))
        );
    }

    #[test]
    fn desktop_secondary_press_opens_overlay_without_hiding_items_on_release_or_motion() {
        let mut shell = LiveShell::new().unwrap();
        let point = nickel_input::Point { x: 300.0, y: 300.0 };
        let button = |edge, order| {
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(order),
                button: nickel_input::PointerButton::Secondary,
                edge,
                position: Some(point),
            })
        };

        assert!(shell.desktop_host.application().layout.icons_visible());
        assert!(shell.desktop_input(button(nickel_input::KeyEdge::Pressed, 1)));
        assert!(shell.desktop_host.application().context_menu.is_some());
        assert!(shell.desktop_host.inspect().open_overlay.is_some());

        let _ = shell.desktop_input(button(nickel_input::KeyEdge::Released, 2));
        let _ = shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Motion {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(3),
                position: nickel_input::Point { x: 300.0, y: 260.0 },
                delta: Some(nickel_input::Vector { x: 0.0, y: -40.0 }),
            },
        ));

        assert!(shell.desktop_host.application().layout.icons_visible());
        assert!(shell.desktop_host.application().context_menu.is_some());
        assert!(shell.desktop_host.inspect().open_overlay.is_some());

        let second_point = nickel_input::Point { x: 360.0, y: 340.0 };
        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(4),
                button: nickel_input::PointerButton::Secondary,
                edge: nickel_input::KeyEdge::Pressed,
                position: Some(second_point),
            },
        )));
        assert!(shell.desktop_host.inspect().open_overlay.is_some());
        assert_eq!(
            shell
                .desktop_host
                .application()
                .context_menu
                .as_ref()
                .and_then(|menu| menu.anchor),
            Some(nickel_file::desktop::Point {
                x: second_point.x as f32,
                y: second_point.y as f32,
            })
        );

        let _ = shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(5),
                button: nickel_input::PointerButton::Primary,
                edge: nickel_input::KeyEdge::Pressed,
                position: Some(nickel_input::Point { x: 50.0, y: 50.0 }),
            },
        ));
        assert!(shell.desktop_host.inspect().open_overlay.is_none());
        assert!(shell.desktop_input(button(nickel_input::KeyEdge::Pressed, 6)));
        assert!(shell.desktop_host.inspect().open_overlay.is_some());

        assert!(shell.desktop_input(nickel_input::InputEvent::FocusLost {
            order: nickel_input::EventOrder(7),
        }));
        assert!(shell.desktop_host.application().context_menu.is_none());
        assert!(shell.desktop_host.inspect().open_overlay.is_none());
    }

    #[test]
    fn desktop_menu_owner_survives_two_output_render_and_foreign_pointer_motion() {
        let mut shell = LiveShell::new().unwrap();
        shell.set_desktop_outputs(vec![
            nickel_file::desktop::DesktopOutput {
                id: "left".into(),
                work_area: nickel_file::desktop::Rect {
                    x: -800.0,
                    y: 0.0,
                    width: 800.0,
                    height: 600.0,
                },
                scale: 1.25,
            },
            nickel_file::desktop::DesktopOutput {
                id: "right".into(),
                work_area: nickel_file::desktop::Rect {
                    x: 0.0,
                    y: 0.0,
                    width: 800.0,
                    height: 600.0,
                },
                scale: 1.0,
            },
        ]);
        shell.set_desktop_output("left".into(), -800.0, 0.0, 1.25);
        let _ = shell.scene(SurfaceRole::Desktop, 800, 600);
        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(1),
                button: nickel_input::PointerButton::Secondary,
                edge: nickel_input::KeyEdge::Pressed,
                position: Some(nickel_input::Point { x: 200.0, y: 200.0 }),
            },
        )));
        assert_eq!(
            shell
                .desktop_host
                .application()
                .context_menu
                .as_ref()
                .map(|menu| menu.output.as_str()),
            Some("left")
        );

        // Simulate the shell's all-surface redraw ending on the other output.
        shell.set_desktop_output("right".into(), 0.0, 0.0, 1.0);
        let _ = shell.scene(SurfaceRole::Desktop, 800, 600);
        assert!(
            shell
                .desktop_host
                .application()
                .frame_overlays(ViewContext::new(
                    Rect::new(0.0, 0.0, 800.0, 600.0),
                    InputModality::Pointer,
                ))
                .iter()
                .all(|overlay| !matches!(overlay, FrameOverlay::Menu(_)))
        );
        assert!(!shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Motion {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(2),
                position: nickel_input::Point { x: 250.0, y: 220.0 },
                delta: Some(nickel_input::Vector { x: 50.0, y: 20.0 }),
            },
        )));
        assert!(shell.desktop_host.application().context_menu.is_some());
        assert!(shell.desktop_host.inspect().open_overlay.is_some());

        shell.set_desktop_output("left".into(), -800.0, 0.0, 1.25);
        let _ = shell.scene(SurfaceRole::Desktop, 800, 600);
        assert!(
            shell
                .desktop_host
                .application()
                .frame_overlays(ViewContext::new(
                    Rect::new(0.0, 0.0, 800.0, 600.0),
                    InputModality::Pointer,
                ))
                .iter()
                .any(|overlay| matches!(overlay, FrameOverlay::Menu(_)))
        );
        let _ = shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Motion {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(3),
                position: nickel_input::Point { x: 210.0, y: 210.0 },
                delta: Some(nickel_input::Vector { x: 10.0, y: 10.0 }),
            },
        ));
        assert!(shell.desktop_host.application().context_menu.is_some());

        // A press on the foreign surface is an explicit outside press: it
        // dismisses once and is consumed instead of reaching that desktop.
        shell.set_desktop_output("right".into(), 0.0, 0.0, 1.0);
        assert!(shell.desktop_input(nickel_input::InputEvent::Pointer(
            nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(4),
                button: nickel_input::PointerButton::Primary,
                edge: nickel_input::KeyEdge::Pressed,
                position: Some(nickel_input::Point { x: 50.0, y: 50.0 }),
            },
        )));
        assert!(shell.desktop_host.application().context_menu.is_none());
        assert!(shell.desktop_host.inspect().open_overlay.is_none());
        let dismissal = shell
            .desktop_host
            .application()
            .last_menu_dismissal
            .as_ref()
            .expect("outside press records a bounded dismissal");
        assert_eq!(dismissal.output, "left");
        assert_eq!(
            dismissal.reason,
            super::desktop::DesktopMenuDismissReason::OutsidePress
        );
    }

    #[test]
    fn desktop_menu_survives_owner_topology_change_and_closes_on_owner_removal() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        let output = |width| nickel_file::desktop::DesktopOutput {
            id: "primary".into(),
            work_area: nickel_file::desktop::Rect {
                x: 0.0,
                y: 0.0,
                width,
                height: 600.0,
            },
            scale: 1.0,
        };
        desktop.set_outputs(vec![output(800.0)]);
        desktop.open_background_context(Some(nickel_file::desktop::Point {
            x: 760.0,
            y: 200.0,
        }));
        let original_generation = desktop
            .context_menu
            .as_ref()
            .expect("menu is open")
            .topology_generation;

        desktop.set_outputs(vec![output(640.0)]);
        let menu = desktop.context_menu.as_ref().expect("owner survives resize");
        assert!(menu.topology_generation > original_generation);
        assert_eq!(menu.output, "primary");

        desktop.set_outputs(Vec::new());
        assert!(desktop.context_menu.is_none());
    }

    #[test]
    fn desktop_live_host_keeps_rename_click_transaction_out_of_the_file_plane() {
        use std::{ffi::OsString, path::PathBuf};

        let mut shell = LiveShell::new().unwrap();
        let application = shell.desktop_host.application_mut();
        application.set_outputs(vec![nickel_file::desktop::DesktopOutput {
            id: "primary".into(),
            work_area: nickel_file::desktop::Rect {
                x: 0.0,
                y: 0.0,
                width: 800.0,
                height: 600.0,
            },
            scale: 1.0,
        }]);
        application.set_active_output(
            "primary".into(),
            nickel_file::desktop::Point::default(),
            1.0,
        );
        application.layout.reconcile(
            (0..48)
                .map(|index| {
                    let name = format!("item-{index:02}.txt");
                    (
                        nickel_file::FileIdentity(83, index + 1),
                        nickel_file::FileEntry {
                            display_name_override: None,
                            name: OsString::from(&name),
                            path: PathBuf::from("/desktop").join(name),
                            is_directory: false,
                            size: Some(1),
                            modified: None,
                        },
                    )
                })
                .collect(),
        );
        let _ = shell.desktop_host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        let event = |button, edge, order, point| {
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                device: nickel_input::DeviceId(1),
                order: nickel_input::EventOrder(order),
                button,
                edge,
                position: Some(point),
            })
        };

        assert!(shell.desktop_input(event(
            nickel_input::PointerButton::Secondary,
            nickel_input::KeyEdge::Pressed,
            1,
            nickel_input::Point { x: 4.0, y: 4.0 },
        )));
        let _ = shell.desktop_input(event(
            nickel_input::PointerButton::Secondary,
            nickel_input::KeyEdge::Released,
            2,
            nickel_input::Point { x: 4.0, y: 4.0 },
        ));
        let selected = shell
            .desktop_host
            .application()
            .layout
            .active()
            .expect("secondary click selects its desktop item");

        let rename = shell
            .desktop_host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Rename"))
            .expect("the live desktop host exposes the rendered Rename row")
            .rect;
        let point = nickel_input::Point {
            x: (rename.origin.x + rename.size.width / 2.0) as f64,
            y: (rename.origin.y + rename.size.height / 2.0) as f64,
        };
        let underneath = shell
            .desktop_host
            .application()
            .hit(nickel_file::desktop::Point {
                x: point.x as f32,
                y: point.y as f32,
            });
        assert_ne!(
            underneath, None,
            "fixture must place a desktop item below Rename"
        );
        assert_ne!(underneath, Some(selected));

        assert!(shell.desktop_input(event(
            nickel_input::PointerButton::Primary,
            nickel_input::KeyEdge::Pressed,
            3,
            point,
        )));
        assert!(shell.desktop_host.inspect().pointer_capture.is_some());
        assert!(shell.desktop_host.application().pointer_down.is_none());
        assert_eq!(
            shell.desktop_host.application().layout.active(),
            Some(selected)
        );

        assert!(shell.desktop_input(event(
            nickel_input::PointerButton::Primary,
            nickel_input::KeyEdge::Released,
            4,
            point,
        )));
        assert!(shell.desktop_host.application().context_menu.is_none());
        assert!(shell.desktop_host.inspect().pointer_capture.is_none());
        assert!(shell.desktop_host.application().pointer_down.is_none());
        assert_eq!(
            shell.desktop_host.application().layout.active(),
            Some(selected)
        );
        assert_ne!(shell.desktop_host.application().layout.active(), underneath);
    }

    #[test]
    fn file_like_surfaces_share_the_file_plane_item_authority() {
        let file = include_str!("../../../../nickel-file/src/components.rs");
        let launcher = include_str!("../../launcher_view.rs");
        let desktop_production = include_str!("../../live_shell/desktop.rs");
        assert!(file.contains("FileGridItem::new_with_generation"));
        assert!(launcher.matches("FilePlaneItem::new").count() >= 2);
        assert!(desktop_production.contains("FilePlaneItem::new_with_generation"));
        let shared = include_str!("../../../../nickel-ui/src/ui/components.rs");
        assert!(shared.contains("pub struct FilePlaneItem"));
        assert!(shared.contains("fn from_image(message: Message"));
        assert!(shared.contains("Self::from_image(message, label"));
        assert!(shared.contains(".message(message)"));
        assert!(shared.contains("pub fn context_message"));
        assert!(shared.contains(".semantic_role(SemanticRole::Button)"));
    }

    #[test]
    fn desktop_icon_secondary_hit_takes_precedence_and_background_preserves_selection() {
        use std::{ffi::OsString, path::PathBuf};
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.layout.reconcile(vec![(
            nickel_file::FileIdentity(9, 2),
            nickel_file::FileEntry {
                display_name_override: None,
                name: OsString::from("entry.txt"),
                path: PathBuf::from("/desktop/entry.txt"),
                is_directory: false,
                size: Some(2),
                modified: None,
            },
        )]);
        let item = desktop.layout.items()[0].clone();
        let local = nickel_file::desktop::Point {
            x: item.position.x + 4.0,
            y: item.position.y + 4.0,
        };
        desktop.pointer_press(local, true, Default::default());
        assert_eq!(desktop.context_menu.as_ref().unwrap().entry, Some(item.id));
        assert!(desktop.layout.selected().contains(&item.id));

        desktop.pointer_press(
            nickel_file::desktop::Point { x: 700.0, y: 500.0 },
            true,
            Default::default(),
        );
        assert_eq!(desktop.context_menu.as_ref().unwrap().entry, None);
        assert!(desktop.layout.selected().contains(&item.id));
    }

    #[test]
    fn desktop_drag_accumulates_small_motion_until_crossing_the_snap_threshold() {
        use std::{ffi::OsString, path::PathBuf};
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.layout.reconcile(vec![(
            nickel_file::FileIdentity(12, 3),
            nickel_file::FileEntry {
                display_name_override: None,
                name: OsString::from("drag-me.txt"),
                path: PathBuf::from("/desktop/drag-me.txt"),
                is_directory: false,
                size: Some(3),
                modified: None,
            },
        )]);
        let item = desktop.layout.items()[0].clone();
        let start = nickel_file::desktop::Point {
            x: item.position.x + 4.0,
            y: item.position.y + 4.0,
        };
        assert!(desktop.pointer_press(start, false, Default::default()));

        for offset in [10.0, 20.0, 30.0, 40.0, 49.0] {
            let _ = desktop.pointer_motion(nickel_file::desktop::Point {
                x: start.x + offset,
                y: start.y,
            });
        }

        assert_eq!(
            desktop.layout.items()[0].position.x,
            item.position.x + desktop.layout.grid().0,
            "sub-threshold motion events must not be discarded individually"
        );
        let _ = desktop.pointer_motion(nickel_file::desktop::Point {
            x: start.x + 97.0,
            y: start.y,
        });
        assert_eq!(
            desktop.layout.items()[0].position.x,
            item.position.x + desktop.layout.grid().0,
            "crossing one cell must not double-count the next half-cell"
        );
        let _ = desktop.pointer_motion(nickel_file::desktop::Point {
            x: start.x + 145.0,
            y: start.y,
        });
        assert_eq!(
            desktop.layout.items()[0].position.x,
            item.position.x + desktop.layout.grid().0 * 2.0,
        );
        assert!(desktop.pointer_release(
            nickel_file::desktop::Point {
                x: start.x + 145.0,
                y: start.y,
            },
            Instant::now(),
        ));
    }

    #[test]
    fn desktop_background_menu_captures_pointer_and_uses_shared_accessible_overlay() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        let anchor = nickel_file::desktop::Point { x: 372.0, y: 214.0 };
        assert!(desktop.pointer_press(anchor, true, Default::default()));
        desktop.pointer_motion(nickel_file::desktop::Point { x: 40.0, y: 60.0 });

        let overlays = desktop.frame_overlays(ViewContext::new(
            Rect::new(0.0, 0.0, 800.0, 600.0),
            InputModality::Pointer,
        ));
        let menu = overlays
            .into_iter()
            .find_map(|overlay| match overlay {
                FrameOverlay::Menu(menu) => Some(menu),
                _ => None,
            })
            .expect("background context menu");
        assert!(
            matches!(menu.anchor, OverlayAnchor::Point { point, .. } if point == Point { x: 372.0, y: 214.0 })
        );
        assert!(menu.items.iter().any(|item| item.label == "Personalize"));
        assert!(
            menu.items
                .iter()
                .any(|item| item.label == "Display Settings")
        );
        assert!(menu.items.iter().any(|item| item.label == "Refresh"));
        let view = menu
            .items
            .iter()
            .find(|item| item.label == "View")
            .expect("presentation commands are grouped under View");
        assert!(
            view.children
                .iter()
                .any(|item| item.id.as_str() == "show-icons")
        );
        assert!(view.children.iter().any(|item| item.id.as_str() == "align"));
        let sort = menu
            .items
            .iter()
            .find(|item| item.label == "Sort By")
            .expect("sort commands are grouped under Sort By");
        assert!(
            sort.children
                .iter()
                .any(|item| item.id.as_str() == "sort-name")
        );
        assert!(
            sort.children
                .iter()
                .any(|item| item.id.as_str() == "manual")
        );
        assert!(
            menu.items.len() <= 9,
            "background commands must not flatten"
        );
        assert!(
            menu.items
                .iter()
                .all(|item| item.id.as_str() != "small-icons")
        );
        assert!(menu.row_height * menu.items.len() as f32 <= 600.0);
        assert_ne!(menu.background, 0x000000);
        assert_ne!(menu.border, menu.background);
        assert!(
            menu.items
                .iter()
                .all(|item| item.accessible_name.is_some() || !item.label.is_empty())
        );

        let host = UiHost::new(desktop, 800, 600);
        let root = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Desktop"))
            .unwrap();
        assert!(root.actions.contains(&ActionKind::ContextMenu));
    }

    #[test]
    fn desktop_background_menu_uses_keyboard_controller_and_accessibility_routes() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        assert!(desktop.key(&nickel_input::KeyEvent {
            device: nickel_input::DeviceId(1),
            order: nickel_input::EventOrder(1),
            physical: nickel_input::PhysicalKey::Code(KeyCode::ContextMenu),
            logical: nickel_input::LogicalKey::Named(nickel_input::NamedKey::ContextMenu),
            location: nickel_input::KeyLocation::Standard,
            edge: nickel_input::KeyEdge::Pressed,
            repeat: false,
            modifiers: nickel_input::ModifierState::default(),
        }));
        assert_eq!(desktop.context_menu.as_ref().unwrap().output, "primary");

        let desktop = super::DesktopApplication::fixture(None, palette);
        let mut host = UiHost::new(desktop, 800, 600);
        let root = host
            .accessibility_nodes()
            .iter()
            .find(|node| node.label.as_deref() == Some("Desktop"))
            .unwrap()
            .id
            .clone();
        let outcome = host
            .perform_accessibility_action(root, SemanticAction::Invoke(ActionKind::ContextMenu));
        assert!(outcome.failures.is_empty());
        assert!(host.application().context_menu.is_some());

        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.open_keyboard_context();
        assert!(
            desktop.context_menu.is_some(),
            "controller uses this command model"
        );
    }

    #[test]
    fn desktop_view_submenu_toggles_authoritative_icon_visibility_through_live_input() {
        fn send_button(
            shell: &mut LiveShell,
            order: &mut u64,
            button: nickel_input::PointerButton,
            edge: nickel_input::KeyEdge,
            point: nickel_input::Point,
        ) {
            *order += 1;
            let _ = shell.desktop_input(nickel_input::InputEvent::Pointer(
                nickel_input::PointerEvent::Button {
                    device: nickel_input::DeviceId(1),
                    order: nickel_input::EventOrder(*order),
                    button,
                    edge,
                    position: Some(point),
                },
            ));
        }

        fn invoke_visibility(shell: &mut LiveShell, order: &mut u64, expected_label: &str) {
            let anchor = nickel_input::Point { x: 300.0, y: 300.0 };
            send_button(
                shell,
                order,
                nickel_input::PointerButton::Secondary,
                nickel_input::KeyEdge::Pressed,
                anchor,
            );
            send_button(
                shell,
                order,
                nickel_input::PointerButton::Secondary,
                nickel_input::KeyEdge::Released,
                anchor,
            );
            let view = shell
                .desktop_host
                .accessibility_nodes()
                .iter()
                .find(|node| node.label.as_deref() == Some("View"))
                .expect("compact root menu exposes View")
                .rect;
            *order += 1;
            let _ = shell.desktop_input(nickel_input::InputEvent::Pointer(
                nickel_input::PointerEvent::Motion {
                    device: nickel_input::DeviceId(1),
                    order: nickel_input::EventOrder(*order),
                    position: nickel_input::Point {
                        x: f64::from(view.origin.x + view.size.width / 2.0),
                        y: f64::from(view.origin.y + view.size.height / 2.0),
                    },
                    delta: None,
                },
            ));
            let command = shell
                .desktop_host
                .accessibility_nodes()
                .iter()
                .find(|node| node.label.as_deref() == Some(expected_label))
                .expect("hovering View exposes its submenu")
                .rect;
            let point = nickel_input::Point {
                x: f64::from(command.origin.x + command.size.width / 2.0),
                y: f64::from(command.origin.y + command.size.height / 2.0),
            };
            send_button(
                shell,
                order,
                nickel_input::PointerButton::Primary,
                nickel_input::KeyEdge::Pressed,
                point,
            );
            send_button(
                shell,
                order,
                nickel_input::PointerButton::Primary,
                nickel_input::KeyEdge::Released,
                point,
            );
        }

        let mut shell = LiveShell::new().unwrap();
        let mut order = 0;
        assert!(shell.desktop_host.application().layout.icons_visible());
        invoke_visibility(&mut shell, &mut order, "Hide desktop icons");
        assert!(!shell.desktop_host.application().layout.icons_visible());
        invoke_visibility(&mut shell, &mut order, "Show desktop icons");
        assert!(shell.desktop_host.application().layout.icons_visible());
    }

    #[test]
    fn stale_desktop_menu_command_is_rejected_after_topology_change() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.open_background_context(None);
        desktop.set_outputs(vec![nickel_file::desktop::DesktopOutput {
            id: "new-output".into(),
            work_area: nickel_file::desktop::Rect {
                x: -500.0,
                y: 0.0,
                width: 500.0,
                height: 700.0,
            },
            scale: 1.5,
        }]);
        desktop.apply_desktop_command(super::DesktopCommand::IconsVisible(false));
        assert!(desktop.layout.icons_visible());
        assert!(desktop.context_menu.is_none());
    }

    #[test]
    fn desktop_presentation_commands_persist_authoritative_live_state() {
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::IconsVisible(false));
        assert!(!desktop.layout.icons_visible());

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::IconSize(128.0, 144.0));
        assert_eq!(desktop.layout.grid(), (128.0, 144.0));

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::Sort(
            nickel_file::desktop::SortKey::Size,
            nickel_file::desktop::SortDirection::Descending,
        ));
        assert_eq!(
            desktop.layout.arrangement(),
            nickel_file::desktop::Arrangement::Sorted {
                key: nickel_file::desktop::SortKey::Size,
                direction: nickel_file::desktop::SortDirection::Descending,
            }
        );

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::Manual);
        assert_eq!(
            desktop.layout.arrangement(),
            nickel_file::desktop::Arrangement::Manual
        );

        desktop.open_background_context(None);
        desktop.apply_desktop_command(super::DesktopCommand::FolderGrouping(
            nickel_file::desktop::FolderGrouping::Mixed,
        ));
        assert_eq!(
            desktop.layout.folder_grouping(),
            nickel_file::desktop::FolderGrouping::Mixed
        );
    }

    #[test]
    fn desktop_menu_rejects_selection_and_workspace_staleness_but_survives_projection() {
        use std::{ffi::OsString, path::PathBuf};
        let palette = nickel_core::theme::ThemePalette::from_appearance(Default::default());
        let mut desktop = super::DesktopApplication::fixture(None, palette);
        desktop.layout.reconcile(vec![(
            nickel_file::FileIdentity(44, 1),
            nickel_file::FileEntry {
                display_name_override: None,
                name: OsString::from("selected.txt"),
                path: PathBuf::from("/desktop/selected.txt"),
                is_directory: false,
                size: Some(1),
                modified: None,
            },
        )]);
        desktop.open_background_context(None);
        desktop.layout.select(
            nickel_file::desktop::DesktopEntryId(nickel_file::FileIdentity(44, 1)),
            Default::default(),
        );
        desktop.apply_desktop_command(super::DesktopCommand::IconsVisible(false));
        assert!(desktop.layout.icons_visible());
        assert!(desktop.context_menu.is_none());

        desktop.open_background_context(None);
        desktop.set_workspace(Some(2));
        assert!(desktop.context_menu.is_none());

        desktop.open_background_context(None);
        desktop.set_active_output("other".into(), nickel_file::desktop::Point::default(), 1.0);
        assert_eq!(
            desktop.context_menu.as_ref().map(|menu| menu.output.as_str()),
            Some("primary")
        );
    }

    #[test]
    fn desktop_settings_destinations_are_typed_and_keep_the_invoking_output() {
        assert_eq!(
            super::SettingsDestination::Appearance.arguments(),
            ["--screen", "appearance"]
        );
        assert_eq!(
            super::SettingsDestination::Display {
                output: "DP-2".into()
            }
            .arguments(),
            ["--screen", "display", "--output", "DP-2"]
        );
    }
