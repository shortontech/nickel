use super::{
    DisabledOutput, DrmRenderStrategy, IDENTIFY_BADGE_BYTES, IdentifyBadgeCache,
    RendererLifecycleLedger, RendererRetainedReason, TaskSwitcherBufferKey,
    consume_pending_dependent, copy_capture_damage, copy_mapped_damage_to_strided,
    copy_mapped_region_to_strided, damage_bounding_box, dependent_renderers_after_primary_removal,
    device_activation_priority, draw_memory_render_buffer, drm_render_strategy, mapped_damage_rows,
    mark_disabled_outputs_absent, normalize_capture_rows, parse_kde_cursor_settings,
    pending_recovery_devices, primary_dependency_to_activate, published_disabled_outputs,
    render_primary_available, renderer_retained_reason, switcher_visible_range, union_rectangles,
};
use smithay::utils::{Buffer, Physical, Rectangle, Size};
use std::collections::HashMap;

#[test]
fn render_strategy_keeps_evdi_copyout_and_explicit_fallback_distinct() {
    assert_eq!(drm_render_strategy(false, false), DrmRenderStrategy::Gbm);
    assert_eq!(drm_render_strategy(false, true), DrmRenderStrategy::Gbm);
    assert_eq!(
        drm_render_strategy(true, false),
        DrmRenderStrategy::EvdiCpuCopyout
    );
    assert_eq!(
        drm_render_strategy(true, true),
        DrmRenderStrategy::EvdiLlvmpipeFallback
    );
}

#[test]
fn overlay_pixels_are_drawn_into_the_persistent_render_buffer_allocation() {
    let mut drawn_at = std::ptr::null();
    let mut buffer = draw_memory_render_buffer(32, 16, |pixels| {
        drawn_at = pixels.as_ptr();
        pixels[0] = 73;
    });
    let mut observed_at = std::ptr::null();
    let mut observed_pixel = 0;
    buffer
        .render()
        .draw(|pixels| {
            observed_at = pixels.as_ptr();
            observed_pixel = pixels[0];
            Ok::<_, std::convert::Infallible>(Vec::new())
        })
        .unwrap();

    assert_eq!(observed_at, drawn_at);
    assert_eq!(observed_pixel, 73);
}

#[test]
fn task_switcher_overlay_key_reuses_only_unchanged_composition() {
    let key = TaskSwitcherBufferKey {
        candidates: vec![crate::window_registry::WindowId(1)],
        selected: 0,
        output_size: (1920, 1080),
        preview_generation: 3,
    };
    assert_eq!(key, key.clone());
    assert_ne!(
        key,
        TaskSwitcherBufferKey {
            preview_generation: 4,
            ..key.clone()
        }
    );
    assert_ne!(
        key,
        TaskSwitcherBufferKey {
            output_size: (1280, 720),
            ..key.clone()
        }
    );
}

#[test]
fn output_identification_badges_are_lazy_reused_and_retired() {
    let mut cache = IdentifyBadgeCache::default();
    assert_eq!(cache.diagnostics().live_bytes, 0);
    cache.get(0);
    cache.get(0);
    cache.get(1);
    let active = cache.diagnostics();
    assert_eq!(active.entries, 2);
    assert_eq!(active.live_bytes, IDENTIFY_BADGE_BYTES * 2);
    assert_eq!(active.rasterizations, 2);
    assert_eq!(active.avoided_rasterizations, 1);
    cache.retire();
    let retired = cache.diagnostics();
    assert_eq!(retired.entries, 0);
    assert_eq!(retired.live_bytes, 0);
    assert_eq!(retired.evictions, 2);
}

#[test]
fn output_identification_topology_shrink_retires_removed_labels() {
    let mut cache = IdentifyBadgeCache::default();
    for index in 0..4 {
        cache.get(index);
    }
    cache.retain_output_count(2);
    let diagnostics = cache.diagnostics();
    assert_eq!(diagnostics.entries, 2);
    assert_eq!(diagnostics.live_bytes, IDENTIFY_BADGE_BYTES * 2);
    assert_eq!(diagnostics.evictions, 2);
    assert!(cache.entries.keys().all(|index| *index < 2));
}

#[test]
fn output_identification_expiry_retires_badges_while_rendering_is_inactive() {
    let rendering_active = false;
    let mut cache = IdentifyBadgeCache::default();
    cache.get(0);
    cache.get(1);
    cache.retire();
    assert!(!rendering_active);
    assert_eq!(cache.diagnostics().entries, 0);
    assert_eq!(cache.diagnostics().live_bytes, 0);
    assert_eq!(cache.diagnostics().evictions, 2);
}

#[test]
fn lifecycle_generations_make_retirement_and_recreation_idempotent() {
    let mut lifecycle = RendererLifecycleLedger::<u8>::default();
    let original = lifecycle.activate(4);
    assert_eq!(lifecycle.activate(4), original);
    assert_eq!(lifecycle.activations, 1);

    assert!(lifecycle.retire(4));
    assert!(!lifecycle.retire(4));
    assert_eq!(lifecycle.generation(4), None);
    let recreated = lifecycle.activate(4);
    assert_ne!(recreated, original);
    assert_eq!(lifecycle.generation(4), Some(recreated));
    assert_ne!(lifecycle.generation(4), Some(original));
    assert_eq!(lifecycle.activations, 2);
    assert_eq!(lifecycle.retirements, 1);
}

#[test]
fn lifecycle_churn_removal_and_suspend_do_not_ratchet_live_resources() {
    let mut lifecycle = RendererLifecycleLedger::<u8>::default();
    for _ in 0..64 {
        let generation = lifecycle.activate(9);
        assert_eq!(lifecycle.generation(9), Some(generation));
        assert!(lifecycle.retire(9));
        assert_eq!(lifecycle.generation(9), None);
    }
    assert_eq!(lifecycle.activations, 64);
    assert_eq!(lifecycle.retirements, 64);
    assert!(lifecycle.live_generations.is_empty());
    assert!(!lifecycle.retire(9));
    assert_eq!(lifecycle.retirements, 64);
}

#[test]
fn primary_is_activated_first_and_only_when_cross_gpu_composition_requires_it() {
    let primary = 1_u8;
    let secondary = 2_u8;
    let mut discovery_order = [secondary, primary];
    discovery_order.sort_by_key(|node| device_activation_priority(*node, primary));
    assert_eq!(discovery_order, [primary, secondary]);
    assert_eq!(
        primary_dependency_to_activate(secondary, primary, true, false),
        Some(primary)
    );
    assert_eq!(
        primary_dependency_to_activate(secondary, primary, true, true),
        None
    );
    assert_eq!(
        primary_dependency_to_activate(primary, primary, true, false),
        None
    );
    assert_eq!(
        primary_dependency_to_activate(secondary, primary, false, false),
        None
    );
    assert!(!render_primary_available(secondary, primary, false, false));
    assert!(render_primary_available(secondary, primary, false, true));
    assert!(render_primary_available(secondary, primary, true, false));
}

#[test]
fn primary_removal_retires_every_dependent_device_and_notifier_generation() {
    let primary = 1_u8;
    let mut lifecycle = RendererLifecycleLedger::<u8>::default();
    lifecycle.activate(primary);
    lifecycle.activate(2);
    lifecycle.activate(3);
    let dependents = dependent_renderers_after_primary_removal(
        primary,
        primary,
        lifecycle.live_generations.keys().copied(),
    );
    assert_eq!(dependents.len(), 2);
    for dependent in dependents {
        assert!(lifecycle.retire(dependent));
    }
    assert!(lifecycle.retire(primary));
    assert!(lifecycle.live_generations.is_empty());
    assert_eq!(lifecycle.retirements, 3);

    assert!(
        dependent_renderers_after_primary_removal(
            2,
            primary,
            lifecycle.live_generations.keys().copied(),
        )
        .is_empty()
    );
    assert!(!render_primary_available(2, primary, false, false));
}

#[test]
fn primary_return_consumes_preserved_secondary_and_evdi_recovery_without_hotplug() {
    let primary = 1_u8;
    let mut lifecycle = RendererLifecycleLedger::<u8>::default();
    let mut pending = std::collections::HashSet::from([2_u8, 3_u8]);
    let discovered = HashMap::from([(primary, "primary"), (2, "secondary"), (3, "evdi")]);

    lifecycle.activate(primary);
    assert_eq!(
        renderer_retained_reason(true, 0, false, !pending.is_empty()),
        Some(RendererRetainedReason::PendingDependentRecovery)
    );
    let recovery = pending_recovery_devices(&pending, &discovered);
    assert_eq!(recovery.len(), 2);
    for node in recovery {
        lifecycle.activate(node);
        assert!(consume_pending_dependent(&mut pending, node, primary));
    }

    assert!(pending.is_empty());
    assert_eq!(lifecycle.live_generations.len(), 3);
    assert!(lifecycle.generation(2).is_some());
    assert!(lifecycle.generation(3).is_some());
}

#[test]
fn direct_changed_and_secondary_removal_consume_pending_without_forgetting_primary_state() {
    let primary = 1_u8;
    let mut pending = std::collections::HashSet::from([2_u8, 3_u8]);

    // A secondary recovered by its own Changed event uses the same production transaction as
    // primary-return recovery and releases that portion of the primary pin.
    assert!(consume_pending_dependent(&mut pending, 2, primary));
    assert_eq!(pending, std::collections::HashSet::from([3]));
    assert_eq!(
        renderer_retained_reason(true, 0, false, !pending.is_empty()),
        Some(RendererRetainedReason::PendingDependentRecovery)
    );

    // Removing the primary does not consume records needed when it returns.
    assert!(!consume_pending_dependent(&mut pending, primary, primary));
    assert_eq!(pending, std::collections::HashSet::from([3]));

    // Physically forgetting the final pending secondary consumes its record and releases an
    // otherwise zero-surface primary.
    assert!(consume_pending_dependent(&mut pending, 3, primary));
    assert!(pending.is_empty());
    assert_eq!(renderer_retained_reason(true, 0, false, false), None);
}

#[test]
fn unplugged_ordinary_and_deferred_evdi_outputs_are_absent_from_snapshots() {
    let mut outputs = HashMap::from([
        (
            "ordinary",
            DisabledOutput {
                node: 1_u8,
                output: "DP-1",
                present: true,
            },
        ),
        (
            "evdi",
            DisabledOutput {
                node: 2_u8,
                output: "DVI-I-1",
                present: true,
            },
        ),
    ]);
    mark_disabled_outputs_absent(&mut outputs, &1);
    mark_disabled_outputs_absent(&mut outputs, &2);
    assert!(published_disabled_outputs(&outputs).next().is_none());

    outputs.get_mut("ordinary").unwrap().present = true;
    assert_eq!(
        published_disabled_outputs(&outputs)
            .copied()
            .collect::<Vec<_>>(),
        ["DP-1"]
    );
}

#[test]
fn disable_disconnect_reenable_and_failed_reenable_preserve_lifecycle_contract() {
    let mut lifecycle = RendererLifecycleLedger::<&str>::default();
    let active = lifecycle.activate("evdi");
    assert_eq!(
        renderer_retained_reason(false, 1, false, false),
        Some(RendererRetainedReason::ActiveSurfaces)
    );

    // Disable/disconnect retires the only resource and its notifier generation.
    assert_eq!(renderer_retained_reason(false, 0, false, false), None);
    assert!(lifecycle.retire("evdi"));
    assert_eq!(lifecycle.generation("evdi"), None);

    // A failed physical reconciliation rolls the provisional activation back. The saved
    // administrative configuration is owned separately by DisabledOutput in production.
    let failed = lifecycle.activate("evdi");
    assert_ne!(failed, active);
    assert!(lifecycle.retire("evdi"));
    assert_eq!(lifecycle.generation("evdi"), None);

    // A later physical reconnect gets one fresh generation; repeated notifications are
    // idempotent and cannot duplicate a notifier or live renderer.
    let reenabled = lifecycle.activate("evdi");
    assert_ne!(reenabled, failed);
    assert_eq!(lifecycle.activate("evdi"), reenabled);
    assert_eq!(lifecycle.live_generations.len(), 1);
}

#[test]
fn renderer_retention_requires_active_output_or_cross_gpu_primary_dependency() {
    assert_eq!(
        renderer_retained_reason(false, 1, false, false),
        Some(RendererRetainedReason::ActiveSurfaces)
    );
    assert_eq!(
        renderer_retained_reason(true, 0, true, false),
        Some(RendererRetainedReason::PrimaryForCrossGpu)
    );
    assert_eq!(renderer_retained_reason(false, 0, true, false), None);
    assert_eq!(renderer_retained_reason(true, 0, false, false), None);
}

#[test]
fn task_switcher_keeps_the_selection_in_a_centered_bounded_window() {
    assert_eq!(switcher_visible_range(3, 1), 0..3);
    assert_eq!(switcher_visible_range(9, 0), 0..5);
    assert_eq!(switcher_visible_range(9, 4), 2..7);
    assert_eq!(switcher_visible_range(9, 8), 4..9);
}

#[test]
fn reads_cursor_preferences_from_kde_mouse_group() {
    let settings = parse_kde_cursor_settings(
        "[Keyboard]\nRepeatDelay=600\n[Mouse]\ncursorTheme=Oxygen_Black\ncursorSize=36\n",
    );
    assert_eq!(settings, (Some("Oxygen_Black".into()), Some(36)));
}

#[test]
fn ignores_cursor_preferences_outside_kde_mouse_group() {
    let settings = parse_kde_cursor_settings("[Other]\ncursorTheme=Oxygen_Black\ncursorSize=36\n");
    assert_eq!(settings, (None, None));
}

#[test]
fn preserves_rows_from_a_flipped_renderer_mapping() {
    let bottom = [9_u8; 8];
    let top = [3_u8; 8];
    let mapped = [bottom, top].concat();
    let normalized = normalize_capture_rows(&mapped, 2, 2, true).unwrap();
    assert_eq!(&normalized[..8], &bottom);
    assert_eq!(&normalized[8..], &top);
}

#[test]
fn reverses_rows_from_an_unflipped_renderer_mapping() {
    let bottom = [9_u8; 8];
    let top = [3_u8; 8];
    let mapped = [bottom, top].concat();
    let normalized = normalize_capture_rows(&mapped, 2, 2, false).unwrap();
    assert_eq!(&normalized[..8], &top);
    assert_eq!(&normalized[8..], &bottom);
}

#[test]
fn copyout_reports_only_changed_row_runs() {
    let mut destination = vec![0_u8; 4 * 4 * 4];
    let mut mapped = destination.clone();
    mapped[4 * 4..4 * 8].fill(7);
    mapped[4 * 12..4 * 16].fill(9);

    let damage = copy_capture_damage(&mut destination, &mapped, 4, 4, true).unwrap();

    assert_eq!(destination, mapped);
    assert_eq!(damage.len(), 2);
    assert_eq!(damage[0].loc.y, 1);
    assert_eq!(damage[0].size.h, 1);
    assert_eq!(damage[1].loc.y, 3);
    assert_eq!(damage[1].size.h, 1);
    assert!(
        copy_capture_damage(&mut destination, &mapped, 4, 4, true)
            .unwrap()
            .is_empty()
    );
}

#[test]
fn copyout_damage_respects_renderer_row_orientation() {
    let mut destination = vec![0_u8; 2 * 2 * 4];
    let mapped = [[8_u8; 8], [3_u8; 8]].concat();

    let damage = copy_capture_damage(&mut destination, &mapped, 2, 2, false).unwrap();

    assert_eq!(&destination[..8], &[3_u8; 8]);
    assert_eq!(&destination[8..], &[8_u8; 8]);
    assert_eq!(damage.len(), 1);
    assert_eq!(damage[0].size.h, 2);
}

#[test]
fn copyout_writes_damage_into_padded_dumb_buffer_rows() {
    let source = (0_u8..24).collect::<Vec<_>>();
    let mut destination = vec![99_u8; 32];
    let damage = [smithay::utils::Rectangle::new((1, 1).into(), (2, 1).into())];

    copy_mapped_damage_to_strided(&mut destination, 16, &source, 3, 2, true, &damage).unwrap();

    assert_eq!(&destination[20..28], &source[16..24]);
    assert!(destination[..20].iter().all(|pixel| *pixel == 99));
    assert!(destination[28..].iter().all(|pixel| *pixel == 99));
}

#[test]
fn mapped_damage_compares_display_order_across_padded_rows() {
    let mapped = [[3_u8; 8], [7_u8; 8]].concat();
    let mut displayed = vec![0_u8; 24];
    displayed[..8].fill(7);
    displayed[12..20].fill(3);

    assert!(
        mapped_damage_rows(&displayed, 12, &mapped, 2, 2, false)
            .unwrap()
            .is_empty()
    );
    displayed[12] = 9;
    let damage = mapped_damage_rows(&displayed, 12, &mapped, 2, 2, false).unwrap();
    assert_eq!(damage.len(), 1);
    assert_eq!(damage[0].loc.y, 1);
}

#[test]
fn alternating_copyout_bounds_current_and_previous_damage() {
    let damage = [
        Rectangle::<i32, Physical>::new((40, 30).into(), (20, 10).into()),
        Rectangle::<i32, Physical>::new((10, 50).into(), (15, 5).into()),
    ];

    let bounds = damage_bounding_box(&damage).unwrap();

    assert_eq!(bounds.loc, (10, 30).into());
    assert_eq!(bounds.size, (50, 25).into());

    let previous = Rectangle::<i32, Buffer>::new((70, 20).into(), (10, 15).into());
    let accumulated = union_rectangles(previous, bounds);
    assert_eq!(accumulated.loc, (10, 20).into());
    assert_eq!(accumulated.size, (70, 35).into());
}

#[test]
fn damaged_region_copy_preserves_pixels_outside_region_and_padding() {
    let region = Rectangle::<i32, Buffer>::new((1, 1).into(), (2, 2).into());
    let mapped = (0_u8..16).collect::<Vec<_>>();
    let mut destination = vec![99_u8; 4 * 20];

    let copied = copy_mapped_region_to_strided(
        &mut destination,
        20,
        &mapped,
        true,
        region,
        Size::from((4, 4)),
    )
    .unwrap();

    assert_eq!(copied, 16);
    assert_eq!(&destination[24..32], &mapped[..8]);
    assert_eq!(&destination[44..52], &mapped[8..]);
    assert!(destination[..24].iter().all(|byte| *byte == 99));
    assert!(destination[32..44].iter().all(|byte| *byte == 99));
    assert!(destination[52..].iter().all(|byte| *byte == 99));
}

#[test]
fn damaged_region_copy_respects_unflipped_readback() {
    let region = Rectangle::<i32, Buffer>::new((0, 1).into(), (2, 2).into());
    let mapped = [[3_u8; 8], [7_u8; 8]].concat();
    let mut destination = vec![0_u8; 3 * 8];

    copy_mapped_region_to_strided(
        &mut destination,
        8,
        &mapped,
        false,
        region,
        Size::from((2, 3)),
    )
    .unwrap();

    assert_eq!(&destination[8..16], &[7_u8; 8]);
    assert_eq!(&destination[16..24], &[3_u8; 8]);
}
