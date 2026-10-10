    #[test]
    fn application_catalog_shared_projection_reuses_values_and_checks_live_authority() {
        with_package_runtime_stack(|| {
            use nickel_core::plugins::PluginCapability;
            let mut shell = LiveShell::new().unwrap();
            shell.launcher = crate::launcher::Launcher::new((0..256).map(|index| crate::model::Application::new(format!("fixture-{index}"),format!("Fixture {index}"),None,None,None)).collect());
            let first = shell.external_plugin_applications_shared("nickel-default").unwrap();
            let builds = shell.application_catalog_builds;
            let before = crate::allocation_counter::thread_allocation_operations();
            for _ in 0..128 {
                let next = shell.external_plugin_applications_shared("nickel-default").unwrap();
                assert!(Arc::ptr_eq(&first, &next));
            }
            assert_eq!(crate::allocation_counter::thread_allocation_operations(), before, "unchanged catalog reads must not deep-copy the inventory");
            assert_eq!(shell.application_catalog_builds, builds);
            shell.launcher = crate::launcher::Launcher::new(vec![crate::model::Application::new("replacement".into(), "Replacement".into(), None, None, None)]);
            let replacement = shell.external_plugin_applications_shared("nickel-default").unwrap();
            assert!(!Arc::ptr_eq(&first, &replacement));
            assert_eq!(replacement[0]["name"], "Replacement");
            assert_eq!(first.as_array().unwrap().len(), 256, "held snapshots remain immutable");
            shell.windows = vec![crate::model::OpenWindow {
                id: crate::model::WindowId(71),
                application_id: Some(crate::model::ApplicationId::new("running-only")),
                active: true,
                title: "Running title".into(),
                state: crate::model::WindowState::default(),
            }];
            let running = shell.external_plugin_applications_shared("nickel-default").unwrap();
            assert!(running.as_array().unwrap().iter().any(|item| item["name"] == "Running title"));
            shell.windows[0].title = "Changed title".into();
            let renamed = shell.external_plugin_applications_shared("nickel-default").unwrap();
            assert!(!Arc::ptr_eq(&running, &renamed));
            assert!(renamed.as_array().unwrap().iter().any(|item| item["name"] == "Changed title"));
            shell.external_plugin_packages.get_mut("nickel-default").unwrap().manifest.capabilities.retain(|capability| *capability != PluginCapability::WindowsRead);
            let restricted = shell.external_plugin_applications_shared("nickel-default").unwrap();
            assert_eq!(restricted.as_array().unwrap().len(), 1);
            shell.external_plugin_packages.get_mut("nickel-default").unwrap().manifest.capabilities.retain(|capability| *capability != PluginCapability::ApplicationsRead);
            assert!(shell.external_plugin_applications_shared("nickel-default").is_none());
        });
    }

    #[test]
    fn application_icon_projection_admits_virtual_rows_and_schedules_geometry_followup() {
        with_package_runtime_stack(|| {
            let mut package = nickel_core::plugins::PluginPackage::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins/example-surface-dialog")).unwrap();
            package.manifest.capabilities.push(nickel_core::plugins::PluginCapability::ApplicationsRead);
            package.source = "function App() { const apps=nickel.applications.list(); return h(Window,{width:300,height:200,title:'Virtual icons'},h(ScrollView,{id:'scroll',height:160},h(VirtualColumn,{id:'apps',items:apps,itemKey:app=>app.id,itemHeight:20,overscan:40,renderItem:app=>h(Button,{id:app.id,icon:app.icon,iconSize:64,showLabel:false,onClick:()=>{}},app.name)}))); }".into();
            let surface = package.manifest.surfaces.iter().find(|surface| surface.id == "home").unwrap().clone();
            let key = nickel_core::plugins::PluginSurfaceKey {plugin_id:package.manifest.id.clone(),surface_id:surface.id.clone()};
            let application = crate::plugin_panel::PluginPanelApplication::from_package_surface(&package, &Default::default(), &surface).unwrap();
            let mut shell = LiveShell::new().unwrap();
            shell.launcher = crate::launcher::Launcher::new((0..256).map(|index| crate::model::Application::new(format!("fixture-{index}"),format!("Fixture {index}"),None,None,None)).collect());
            shell.launcher_icons = crate::launcher_icon_cache::LauncherIconCache::new();
            shell.plugin_registry.register(package.manifest.clone()).unwrap();
            shell.plugin_registry.set_enabled(&key.plugin_id, true).unwrap();
            shell.plugin_registry.mark_running(&key.plugin_id).unwrap();
            shell.external_plugin_packages.insert(key.plugin_id.clone(), nickel_core::plugins::PluginPackageSource::embedded(package));
            shell.plugin_surface_hosts.insert(key.clone(), (surface, twinkle::UiHost::new(application, 300, 200)));
            shell.plugin_panel_scene(&key, 300, 200).unwrap();
            let entries = shell.launcher_icons.diagnostics().entries;
            assert!(entries > 0 && entries < 24, "resolved {entries} icons before native row admission");
            assert!(shell.plugin_image_deadlines.contains_key(&key), "geometry-changing artwork must schedule remaining demand");
            let mut followups = 0;
            while let Some(deadline) = shell.plugin_image_deadlines.get(&key).copied() {
                assert!(followups < 8, "artwork/virtual geometry failed to converge");
                assert!(shell.poll_deadlines(deadline).redraw.contains(&SurfaceRole::Panel));
                shell.plugin_panel_scene(&key, 300, 200).unwrap();
                followups += 1;
            }
            let host = &shell.plugin_surface_hosts[&key].1;
            assert!(host.application().application_image_demand().len() <= 6);
            assert!(host.resolved_layout().nodes().len() < 100);
            // A completed artwork worker must repaint ordinary package windows,
            // even when no input arrives to replace their initial placeholders.
            shell.launcher_icon_revision = shell.launcher_icons.revision().wrapping_sub(1);
            assert!(shell.refresh_fast_changes().contains(&SurfaceRole::Panel));
            let cached = shell.launcher_icons.diagnostics().entries;
            for _ in 0..16 { shell.plugin_panel_scene(&key, 300, 200).unwrap(); }
            assert_eq!(shell.launcher_icons.diagnostics().entries, cached);
            assert!(!shell.plugin_image_deadlines.contains_key(&key));
            assert!(shell.close_plugin_window(&key).unwrap());
            assert!(!shell.plugin_surface_hosts.contains_key(&key));
            assert!(!shell.plugin_image_deadlines.contains_key(&key));
        });
    }

    #[test]
    fn application_icon_projection_does_not_resolve_unused_inventory() {
        with_package_runtime_stack(|| {
            let mut package = nickel_core::plugins::PluginPackage::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins/example-surface-dialog")).unwrap();
            package.manifest.capabilities.push(nickel_core::plugins::PluginCapability::ApplicationsRead);
            package.source = "function App() { const [show,setShow]=useState(false); const app=show ? nickel.applications.list()[0] : null; return h(Window,{width:300,height:200,title:'Icon demand fixture'},h(Column,null,h(Button,{id:'toggle',onClick:()=>setShow(!show)},'Toggle icon'),app ? h(Image,{asset:app.icon,width:32,height:32}) : h(Text,null,'Keyboard shortcuts'))); }".into();
            let surface = package.manifest.surfaces.iter().find(|surface| surface.id == "home").unwrap().clone();
            let key = nickel_core::plugins::PluginSurfaceKey {plugin_id:package.manifest.id.clone(),surface_id:surface.id.clone()};
            let application = crate::plugin_panel::PluginPanelApplication::from_package_surface(&package, &Default::default(), &surface).unwrap();
            let mut shell = LiveShell::new().unwrap();
            shell.launcher = crate::launcher::Launcher::new((0..256).map(|index| crate::model::Application::new(format!("fixture-{index}"),format!("Fixture {index}"),None,None,None)).collect());
            shell.launcher_icons = crate::launcher_icon_cache::LauncherIconCache::new();
            shell.plugin_registry.register(package.manifest.clone()).unwrap();
            shell.plugin_registry.set_enabled(&key.plugin_id, true).unwrap();
            shell.plugin_registry.mark_running(&key.plugin_id).unwrap();
            shell.external_plugin_packages.insert(key.plugin_id.clone(), nickel_core::plugins::PluginPackageSource::embedded(package));
            shell.plugin_surface_hosts.insert(key.clone(), (surface, twinkle::UiHost::new(application, 300, 200)));
            let started = Instant::now();
            let before = crate::allocation_counter::thread_allocation_operations();
            for _ in 0..16 { shell.plugin_panel_scene(&key, 300, 200).unwrap(); }
            eprintln!("unused_application_icons: scenes=16 inventory=256 cache_entries={} elapsed_ns={} allocations={}", shell.launcher_icons.diagnostics().entries, started.elapsed().as_nanos(), crate::allocation_counter::thread_allocation_operations()-before);
            assert_eq!(shell.launcher_icons.diagnostics().entries, 0, "a page without application images must not enqueue or retain inventory icons");
            for expected in [1, 1] {
                let host = &mut shell.plugin_surface_hosts.get_mut(&key).unwrap().1;
                let target = host.query_unique(&twinkle::SemanticSelector::RoleAndName {role:twinkle::SemanticRole::Button,name:"Toggle icon".into()}).unwrap();
                host.perform_semantic_action(target.id, twinkle::SemanticAction::Invoke(twinkle::ActionKind::Activate));
                shell.plugin_panel_scene(&key, 300, 200).unwrap();
                assert_eq!(shell.launcher_icons.diagnostics().entries, expected, "only the admitted icon may enter the cache");
            }
            assert!(!shell.plugin_surface_hosts[&key].1.application().has_application_images());
        });
    }

    #[test]
    fn wallpaper_preview_completion_redraws_live_consumer_and_revocation_removes_pixels() {
        with_package_runtime_stack(|| {
            use twinkle::backend::PaintCommand;
            let directory = tempfile::tempdir().unwrap();
            RgbaImage::from_pixel(160, 90, Rgba([31, 63, 95, 255]))
                .save(directory.path().join("fixture.png")).unwrap();
            let catalog = crate::wallpaper_selection::Catalog::discover_fixture(directory.path());
            let asset = format!("wallpaper:{}", catalog.choices(None)[0].id);
            let mut package = nickel_core::plugins::PluginPackage::load(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins/example-surface-dialog")).unwrap();
            package.manifest.capabilities.push(nickel_core::plugins::PluginCapability::WallpaperRead);
            package.source = format!("function App() {{ return h(Window, {{width:300,height:200,title:'Preview fixture'}}, h(Image, {{asset:{},width:160,height:90}})); }}", serde_json::to_string(&asset).unwrap());
            let surface = package.manifest.surfaces.iter().find(|surface| surface.id == "home").unwrap().clone();
            let key = nickel_core::plugins::PluginSurfaceKey {plugin_id:package.manifest.id.clone(),surface_id:surface.id.clone()};
            let application = crate::plugin_panel::PluginPanelApplication::from_package_surface(&package, &Default::default(), &surface).unwrap();
            let mut shell = LiveShell::new().unwrap();
            shell.plugin_registry.register(package.manifest.clone()).unwrap();
            shell.plugin_registry.set_enabled(&key.plugin_id, true).unwrap();
            shell.plugin_registry.mark_running(&key.plugin_id).unwrap();
            shell.external_plugin_packages.insert(key.plugin_id.clone(), nickel_core::plugins::PluginPackageSource::embedded(package));
            // Seed the ordinary read-only observation first, then replace only
            // its native approved catalog with an isolated fixture catalog.
            shell.appearance_capabilities.snapshot("wallpaper");
            shell.appearance_capabilities.previews.set_catalog(catalog);
            shell.plugin_surface_hosts.insert(key.clone(), (surface, twinkle::UiHost::new(application, 300, 200)));
            let initial = shell.plugin_panel_scene(&key, 300, 200).unwrap();
            assert!(!initial.iter().any(|command| matches!(command, PaintCommand::Image {id,..} if *id >= 64000 && *id < 64128)));
            assert!(shell.host_deadline_sources().iter().any(|(name,_)| *name == "wallpaper-previews"));
            let timeout = Instant::now() + Duration::from_secs(5);
            let mut redrawn = false;
            while let Some(deadline) = shell.appearance_capabilities.previews.next_deadline() {
                assert!(Instant::now() < timeout);
                redrawn |= shell.poll_deadlines(deadline).redraw.contains(&SurfaceRole::Panel);
                std::thread::yield_now();
            }
            assert!(redrawn, "preview completion must wake its native consumer");
            let loaded = shell.plugin_panel_scene(&key, 300, 200).unwrap();
            assert!(loaded.iter().any(|command| matches!(command, PaintCommand::Image {id,..} if *id >= 64000 && *id < 64128)));
            assert!(shell.plugin_surface_hosts[&key].1.application().has_wallpaper_images());
            // The package manifest is the same authority consulted by resource
            // projection; a stale registry copy must not keep decoding enabled.
            shell.external_plugin_packages.get_mut(&key.plugin_id).unwrap().manifest.capabilities.retain(|capability| *capability != nickel_core::plugins::PluginCapability::WallpaperRead);
            let revoked = shell.plugin_panel_scene(&key, 300, 200).unwrap();
            assert!(!revoked.iter().any(|command| matches!(command, PaintCommand::Image {id,..} if *id >= 64000 && *id < 64128)));
            assert!(!shell.plugin_surface_hosts[&key].1.application().has_wallpaper_images());
            assert!(shell.appearance_capabilities.previews.images().is_empty());
            assert!(shell.appearance_capabilities.previews.next_deadline().is_none());
        });
    }

    #[test]
    fn full_preview_refresh_moves_provider_pixels_and_preserves_unchanged_identity() {
        with_package_runtime_stack(|| {
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
        });
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
    fn panel_reentry_cancels_a_stale_preview_leave_deadline() {
        with_package_runtime_stack(|| {
        let mut shell = LiveShell::new().unwrap();
        shell.preview_leave_deadline = Some(Instant::now());

        assert!(shell.panel_pointer_entered());
        assert!(shell.preview_leave_deadline.is_none());
        assert!(!shell.panel_pointer_entered());
        });
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
        with_package_runtime_stack(|| {
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
                twinkle::backend::PaintCommand::Image { id: 1, .. })),
            "the native desktop must paint the arriving wallpaper"
        );
        });
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
        with_package_runtime_stack(|| {
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
        });
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
    fn tray_source_keeps_only_sixty_four_items_without_discarding_source_quality() {
        let items = (0..67)
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
            (3..67).map(|index| index.to_string()).collect::<Vec<_>>()
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
            2_097_152
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
