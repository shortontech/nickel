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
        assert!(production.contains("\"color\": foreground"));
    }

    #[test]
    fn configured_wallpaper_changes_replace_the_live_desktop_image() {
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
    }

    #[test]
    fn desktop_background_plugin_reports_memory_and_retires_wallpaper_when_disabled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("plugin-wallpaper.png");
        RgbaImage::from_pixel(8, 8, Rgba([44, 55, 66, 255]))
            .save(&path)
            .unwrap();
        let mut shell = LiveShell::new().unwrap();
        assert!(shell.refresh_configured_wallpaper(Some(path)));
        let plugin_scene = shell.scene(SurfaceRole::Desktop, 320, 200);
        let wallpaper_index = plugin_scene
            .iter()
            .position(|command| matches!(
                command,
                nickel_ui::backend::PaintCommand::Image { id: 0x6000, .. }
            ))
            .expect("JSX desktop wallpaper image");
        if let nickel_ui::backend::PaintCommand::Image { image, .. } = &plugin_scene[wallpaper_index]
        {
            assert!(Arc::ptr_eq(image, shell.wallpaper.as_ref().unwrap()));
        }
        assert!(!plugin_scene[wallpaper_index + 1..].iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Fill { rect, color }
                if rect.size.width >= 320.0 && rect.size.height >= 200.0 && *color >> 24 != 0
        )));
        assert!(shell.desktop_host.application().plugin_background);
        let plugin_host = shell.plugin_desktop_host.as_mut().unwrap();
        let image_bytes = plugin_host.application().retained_image_bytes();
        assert!(image_bytes > 0);
        let frame_bytes = plugin_host
            .step(HostBatch::default())
            .telemetry
            .retained_frame_bytes as u64;
        let id = &crate::plugin_panel::desktop_manifest().id;
        let status = shell.plugin_registry().get(id).unwrap();
        assert_eq!(
            status.memory.native_ui_bytes,
            Some(frame_bytes.saturating_add(image_bytes))
        );

        assert!(shell.set_plugin_enabled(id, false).unwrap());
        let retired_scene = shell.scene(SurfaceRole::Desktop, 320, 200);
        assert!(!retired_scene.iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Image { .. }
        )));
        assert!(!shell.desktop_host.application().plugin_background);
        assert_eq!(
            shell.plugin_registry().get(id).unwrap().memory,
            nickel_core::plugins::PluginMemory::default()
        );

        assert!(shell.set_plugin_enabled(id, true).unwrap());
        let resumed_scene = shell.scene(SurfaceRole::Desktop, 320, 200);
        assert!(resumed_scene.iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Image { id: 0x6000, .. }
        )));
        assert!(shell.desktop_host.application().plugin_background);
        assert!(shell.plugin_registry().get(id).unwrap().memory.native_ui_bytes.unwrap() > 0);
    }

    #[test]
    fn desktop_error_banner_retires_with_its_plugin() {
        let mut shell = LiveShell::new().unwrap();
        shell.desktop_host.application_mut().error = Some("Desktop files unavailable".into());
        let plugin_scene = shell.scene(SurfaceRole::Desktop, 320, 200);
        let banner_count = |commands: &[nickel_ui::backend::PaintCommand]| {
            commands.iter().filter(|command| matches!(
                command,
                nickel_ui::backend::PaintCommand::Text { text, .. }
                    if text == "Desktop files unavailable"
            )).count()
        };
        assert_eq!(banner_count(&plugin_scene), 1);
        assert!(shell.desktop_host.application().plugin_background);

        shell.set_plugin_enabled(&crate::plugin_panel::desktop_manifest().id, false).unwrap();
        let retired_scene = shell.scene(SurfaceRole::Desktop, 320, 200);
        assert_eq!(banner_count(&retired_scene), 0);
    }

    #[test]
    fn failed_wallpaper_decode_preserves_the_last_presentable_image() {
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
