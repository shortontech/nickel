use super::{
    DisabledOutput, IDENTIFY_BADGE_BYTES, IdentifyBadgeCache, RendererLifecycleLedger,
    RendererRetainedReason, TaskSwitcherBufferKey, consume_pending_dependent,
    dependent_renderers_after_primary_removal, device_activation_priority,
    mark_disabled_outputs_absent, normalize_capture_rows, parse_kde_cursor_settings,
    pending_recovery_devices, primary_dependency_to_activate, published_disabled_outputs,
    render_primary_available, renderer_retained_reason, switcher_visible_range,
};
use std::collections::HashMap;

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
