    #[test]
    fn desktop_labels_use_local_wallpaper_luminance_without_label_backplates() {
        let mut wallpaper = RgbaImage::from_pixel(200, 100, Rgba([245, 245, 245, 255]));
        for pixel in wallpaper
            .enumerate_pixels_mut()
            .filter_map(|(x, _, pixel)| (x >= 100).then_some(pixel))
        {
            *pixel = Rgba([8, 8, 8, 255]);
        }
        let viewport = nickel_ui::Size {
            width: 400.0,
            height: 200.0,
        };
        let left = Rect::new(20.0, 50.0, 80.0, 36.0);
        let right = Rect::new(300.0, 50.0, 80.0, 36.0);

        assert_eq!(
            desktop_label_foreground(Some(&wallpaper), viewport, left, 0x303030, None),
            0x111111,
            "light pixels beneath this label require dark ink"
        );
        assert_eq!(
            desktop_label_foreground(Some(&wallpaper), viewport, right, 0xf0f0f0, None),
            0xffffff,
            "dark pixels beneath this label require light ink"
        );

        let checker = RgbaImage::from_fn(9, 5, |x, y| {
            if (x + y) % 2 == 0 {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            }
        });
        assert_eq!(
            desktop_label_foreground(
                Some(&checker),
                nickel_ui::Size {
                    width: 9.0,
                    height: 5.0,
                },
                Rect::new(0.0, 0.0, 9.0, 5.0),
                0x303030,
                None,
            ),
            0x111111,
            "mixed regions use a deterministic low-percentile and mean tie break"
        );

        // Transparent/failed wallpaper data falls back to the themed desktop
        // base, and an interaction surface becomes the immediate backdrop.
        let transparent = RgbaImage::from_pixel(1, 1, Rgba([255, 255, 255, 0]));
        assert_eq!(
            desktop_label_foreground(Some(&transparent), viewport, left, 0x090909, None),
            0xffffff
        );
        assert_eq!(
            desktop_label_foreground(None, viewport, left, 0xf8f8f8, None),
            0x111111
        );
        assert_eq!(
            desktop_label_foreground(
                Some(&wallpaper),
                viewport,
                right,
                0x090909,
                Some(0xffeeeeee),
            ),
            0x111111,
            "selected/hover/focus surfaces take precedence over wallpaper"
        );

        let production = include_str!("../../live_shell/desktop.rs");
        assert!(production.contains(".foreground(label_foreground)"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn wallpaper_change_invalidates_desktop_siblings_after_settings_input() {
        with_package_runtime_stack(|| {
            use crate::internal_shell::{InternalOutput, InternalShellCoordinator};
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("wallpaper.png");
            RgbaImage::from_pixel(8, 8, Rgba([20, 80, 230, 255]))
                .save(&path)
                .unwrap();
            let mut coordinator = InternalShellCoordinator::new(
                Arc::new(crate::session_host::PlatformSessionHost),
                crate::PanelEdge::Bottom,
            )
            .unwrap();
            coordinator.shell_mut().show_plugin_window("nickel-default", "settings").unwrap();
            coordinator.set_outputs(&[0, 1].map(|index| InternalOutput {
                name: format!("wallpaper-{index}"),
                x: index * 800,
                y: 0,
                width: 800,
                height: 600,
                scale: 1.0,
            }));
            let desktops = coordinator.surfaces().iter()
                .filter(|surface| surface.role == SurfaceRole::Desktop)
                .map(|surface| surface.id).collect::<Vec<_>>();
            assert_eq!(desktops.len(), 2);
            let settings = coordinator.surfaces().iter()
                .find(|surface| surface.plugin.as_ref().is_some_and(|key| key.surface_id == "settings"))
                .unwrap().id;
            coordinator.scene(settings).unwrap();
            for desktop in &desktops {
                coordinator.scene(*desktop).unwrap();
            }
            assert!(coordinator.shell_mut().refresh_configured_wallpaper(Some(path)));
            let changed = coordinator.step_slot_changes(settings, HostBatch::default());
            for desktop in &desktops {
                assert!(changed.contains(desktop), "wallpaper commit must invalidate every desktop");
                let scene = coordinator.scene(*desktop).unwrap();
                assert!(nickel_ui::backend::contains_image_pixels(
                    &scene, coordinator.shell_mut().wallpaper.as_ref().unwrap()
                ));
            }
            let unchanged = coordinator.step_slot_changes(settings, HostBatch::default());
            assert!(desktops.iter().all(|desktop| !unchanged.contains(desktop)));
            assert!(coordinator.shell_mut().refresh_configured_wallpaper(None));
            let reset = coordinator.poll(Instant::now());
            for desktop in &desktops {
                assert!(reset.contains(desktop), "deadline reconciliation must invalidate wallpaper reset");
                coordinator.scene(*desktop).unwrap();
            }
            assert!(!coordinator.shell_mut().desktop_presentation_dirty());
        });
    }

    #[test]
    fn configured_wallpaper_changes_replace_the_live_desktop_image() {
        with_package_runtime_stack(|| {
        let directory = tempfile::tempdir().expect("wallpaper fixture directory");
        let first_path = directory.path().join("first.png");
        let second_path = directory.path().join("second.png");
        let first = RgbaImage::from_pixel(8, 8, Rgba([220, 30, 40, 255]));
        let second = RgbaImage::from_pixel(12, 9, Rgba([20, 80, 230, 255]));
        first.save(&first_path).expect("save first wallpaper");
        second.save(&second_path).expect("save second wallpaper");

        let mut shell = LiveShell::new().expect("live shell");
        assert!(shell.refresh_configured_wallpaper(Some(first_path)));
        let first_scene = shell.scene(SurfaceRole::Desktop, 320, 200);
        assert!(nickel_ui::backend::contains_image_pixels(
            &first_scene,
            shell.wallpaper.as_ref().expect("first wallpaper")
        ));

        assert!(shell.refresh_configured_wallpaper(Some(second_path)));
        let second_scene = shell.scene(SurfaceRole::Desktop, 320, 200);
        assert!(nickel_ui::backend::contains_image_pixels(
            &second_scene,
            shell.wallpaper.as_ref().expect("second wallpaper")
        ));
        assert_eq!(
            shell.wallpaper.as_ref().unwrap().get_pixel(0, 0).0,
            [20, 80, 230, 255]
        );
        assert!(!shell.refresh_configured_wallpaper(shell.wallpaper_path.clone()));
        });
    }

    #[test]
    fn desktop_error_banner_appears_once_in_native_view() {
        with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        shell.desktop_host.application_mut().error = Some("Desktop files unavailable".into());
        let scene = shell.scene(SurfaceRole::Desktop, 320, 200);
        let banner_count = scene.iter().filter(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Text { text, .. }
                if text == "Desktop files unavailable"
        )).count();
        assert_eq!(banner_count, 1);
        });
    }

    #[test]
    fn failed_wallpaper_decode_preserves_the_last_presentable_image() {
        with_package_runtime_stack(|| {
        let directory = tempfile::tempdir().expect("wallpaper fixture directory");
        let valid_path = directory.path().join("valid.png");
        let invalid_path = directory.path().join("invalid.png");
        RgbaImage::from_pixel(8, 8, Rgba([44, 55, 66, 255]))
            .save(&valid_path)
            .unwrap();
        std::fs::write(&invalid_path, b"not an image").unwrap();

        let mut shell = LiveShell::new().expect("live shell");
        assert!(shell.refresh_configured_wallpaper(Some(valid_path)));
        shell.scene(SurfaceRole::Desktop, 320, 200);
        let prior = shell.wallpaper.clone().expect("decoded wallpaper");

        assert!(shell.refresh_configured_wallpaper(Some(invalid_path)));
        shell.scene(SurfaceRole::Desktop, 320, 200);
        assert!(Arc::ptr_eq(
            shell.wallpaper.as_ref().expect("prior wallpaper retained"),
            &prior
        ));
        });
    }

    #[test]
    fn desktop_refresh_retains_only_icons_with_unchanged_meaningful_metadata() {
        let stable = std::path::PathBuf::from("/desktop/stable.desktop");
        let changed = std::path::PathBuf::from("/desktop/changed.desktop");
        let removed = std::path::PathBuf::from("/desktop/removed.desktop");
        let pixels = Arc::new(RgbaImage::from_pixel(1, 1, Rgba([1, 2, 3, 255])));
        let mut cache = HashMap::from([
            (stable.clone(), Arc::clone(&pixels)),
            (changed.clone(), Arc::clone(&pixels)),
            (removed.clone(), pixels),
        ]);
        let previous = HashMap::from([
            (stable.clone(), (false, Some(10), None)),
            (changed.clone(), (false, Some(10), None)),
            (removed, (false, Some(10), None)),
        ]);
        let current = HashMap::from([
            (stable.clone(), (false, Some(10), None)),
            (changed.clone(), (false, Some(11), None)),
        ]);

        retain_unchanged_desktop_icons(&mut cache, &previous, &current);

        assert_eq!(cache.keys().collect::<Vec<_>>(), [&stable]);
    }

    #[test]
    fn explicit_wallpaper_path_wins_without_loading_the_system_wallpaper() {
        let path = std::path::PathBuf::from("configured-wallpaper.png");
        let (resolved_path, wallpaper, size) = initial_wallpaper(Some(path.clone()), || {
            panic!("an explicit Nickel wallpaper must suppress the system fallback")
        });

        assert_eq!(resolved_path, Some(path));
        assert!(wallpaper.is_none());
        assert_eq!(size, (0, 0));
    }

    #[test]
    fn system_wallpaper_is_used_when_nickel_has_no_explicit_path() {
        let image = RgbaImage::from_pixel(3, 2, Rgba([10, 20, 30, 255]));
        let (resolved_path, wallpaper, size) = initial_wallpaper(None, || Some(image.clone()));

        assert!(resolved_path.is_none());
        assert_eq!(wallpaper.as_deref(), Some(&image));
        assert_eq!(size, (3, 2));
    }

    #[test]
    fn missing_system_wallpaper_degrades_to_the_themed_desktop_base() {
        let (resolved_path, wallpaper, size) = initial_wallpaper(None, || None);

        assert!(resolved_path.is_none());
        assert!(wallpaper.is_none());
        assert_eq!(size, (0, 0));
    }
