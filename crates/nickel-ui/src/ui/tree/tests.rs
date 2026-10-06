use super::*;

#[test]
fn virtual_row_accessibility_focus_is_revision_scoped_and_cancellable() {
    let build = |revision, range: std::ops::Range<usize>| {
        let heights = VirtualHeightIndex::new(&[24.0; 100], 0.0);
        VerticalScroll::new((), 0.0).id("scroll").child(
            VirtualColumn::new()
                .logical_navigation(100, revision)
                .window(heights.window_for_range(range.clone()).unwrap())
                .children(range.map(|ordinal| {
                    Button::new((), format!("Row {ordinal}"))
                        .id(format!("row-{ordinal}"))
                        .height(24.0)
                }))
                .into_element()
                .id("rows"),
        )
    };
    let bounds = Rect::new(0.0, 0.0, 320.0, 96.0);
    let mut state = UiStateStore::default();
    let first = UiFrame::resolve(build(1, 0..6), FrameRequest::new(bounds, &mut state));
    let collection = first
        .resolved_layout()
        .nodes()
        .iter()
        .find(|node| node.virtual_navigation.is_some())
        .unwrap()
        .id
        .clone();
    let request = |revision| UiEvent::AccessibilityRevealVirtualRow {
        collection: collection.clone(),
        revision,
        ordinal: 50,
        leading: 1200.0,
        height: 24.0,
    };
    first.handle_event(&mut state, request(2));
    assert!(state.virtual_focus.is_none());
    let outcome = first.handle_event(&mut state, request(1));
    assert_eq!(outcome.invalidation, Invalidation::Layout);
    assert!(outcome.messages.is_empty());
    assert!(state.virtual_focus.is_some());
    assert_eq!(
        first.reconcile_virtual_boundary(&mut state),
        Invalidation::None
    );
    assert!(state.focused().is_none());
    let pending = state.clone();
    let admitted = UiFrame::resolve_against(
        build(1, 48..54),
        FrameRequest::new(bounds, &mut state),
        &first,
    );
    assert_ne!(
        admitted.reconcile_virtual_boundary(&mut state),
        Invalidation::None
    );
    assert!(state.focused().unwrap().as_str().ends_with("/row-50"));
    assert!(state.virtual_focus.is_none());

    let mut disabled_state = pending.clone();
    let heights = VirtualHeightIndex::new(&[24.0; 100], 0.0);
    let disabled = UiFrame::resolve_against(
        VerticalScroll::new((), 0.0).id("scroll").child(
            VirtualColumn::new()
                .logical_navigation(100, 1)
                .window(heights.window_for_range(48..54).unwrap())
                .children((48..54).map(|ordinal| {
                    Button::new((), format!("Row {ordinal}"))
                        .enabled(false)
                        .id(format!("row-{ordinal}"))
                        .height(24.0)
                }))
                .into_element()
                .id("rows"),
        ),
        FrameRequest::new(bounds, &mut disabled_state),
        &first,
    );
    assert_eq!(
        disabled.reconcile_virtual_boundary(&mut disabled_state),
        Invalidation::None
    );
    assert!(disabled_state.focused().is_none());
    assert!(disabled_state.virtual_focus.is_none());

    let mut replaced = pending.clone();
    let replacement = UiFrame::resolve_against(
        build(2, 48..54),
        FrameRequest::new(bounds, &mut replaced),
        &first,
    );
    assert_eq!(
        replacement.reconcile_virtual_boundary(&mut replaced),
        Invalidation::None
    );
    assert!(replaced.virtual_focus.is_none());
    assert!(replaced.focused().is_none());
    for event in [
        UiEvent::FocusLost,
        UiEvent::KeyboardNavigateStart,
        UiEvent::PointerCancelled,
    ] {
        let mut cancelled = pending.clone();
        first.handle_event(&mut cancelled, event);
        assert!(cancelled.virtual_focus.is_none());
    }
    let mut destroyed = pending.clone();
    destroyed.destroy();
    assert!(destroyed.virtual_focus.is_none());
    let mut scoped = pending.clone();
    scoped
        .navigation_mut()
        .set_controller_scope(Some(UiId::from("other-scope")));
    assert_eq!(
        admitted.reconcile_virtual_boundary(&mut scoped),
        Invalidation::None
    );
    assert!(scoped.virtual_focus.is_none());
    let mut removed = pending;
    let empty = UiFrame::resolve(Column::<()>::new(), FrameRequest::new(bounds, &mut removed));
    assert_eq!(
        empty.reconcile_virtual_boundary(&mut removed),
        Invalidation::None
    );
    assert!(removed.virtual_focus.is_none());
}

#[test]
fn virtual_boundary_navigation_is_revision_scoped_and_cancellable() {
    let build = |revision, range: std::ops::Range<usize>| {
        let heights = VirtualHeightIndex::new(&[24.0; 100], 0.0);
        VerticalScroll::new((), 0.0).id("scroll").child(
            VirtualColumn::new()
                .logical_navigation(100, revision)
                .window(heights.window_for_range(range.clone()).unwrap())
                .children(range.map(|ordinal| {
                    Button::new((), format!("Row {ordinal}"))
                        .id(format!("row-{ordinal}"))
                        .height(24.0)
                }))
                .into_element()
                .id("rows"),
        )
    };
    let bounds = Rect::new(0.0, 0.0, 320.0, 96.0);
    let mut state = UiStateStore::default();
    let first = UiFrame::resolve(build(1, 0..6), FrameRequest::new(bounds, &mut state));
    let outcome = first.handle_event(&mut state, UiEvent::KeyboardNavigateEnd);
    assert_eq!(outcome.invalidation, Invalidation::Layout);
    assert!(outcome.messages.is_empty());
    assert!(state.virtual_boundary.is_some());
    assert!(
        state
            .state(&UiId::from("root/scroll"))
            .unwrap()
            .scroll_offset
            > 2_000.0
    );

    // A layout with the old window cannot select an overscan row as the end.
    assert_eq!(
        first.reconcile_virtual_boundary(&mut state),
        Invalidation::None
    );
    assert!(state.virtual_boundary.is_some());
    let pending = state.clone();
    let last = UiFrame::resolve_against(
        build(1, 94..100),
        FrameRequest::new(bounds, &mut state),
        &first,
    );
    assert_ne!(
        last.reconcile_virtual_boundary(&mut state),
        Invalidation::None
    );
    assert!(state.virtual_boundary.is_none());
    assert!(
        state
            .navigation()
            .controller_selected()
            .unwrap()
            .as_str()
            .ends_with("/row-99")
    );
    assert_eq!(
        last.reconcile_virtual_boundary(&mut state),
        Invalidation::None
    );

    let mut replaced = pending.clone();
    let replacement = UiFrame::resolve_against(
        build(2, 94..100),
        FrameRequest::new(bounds, &mut replaced),
        &first,
    );
    assert_eq!(
        replacement.reconcile_virtual_boundary(&mut replaced),
        Invalidation::None
    );
    assert!(replaced.virtual_boundary.is_none());
    assert!(replaced.navigation().controller_selected().is_none());

    for event in [
        UiEvent::FocusLost,
        UiEvent::KeyboardNavigateStart,
        UiEvent::PointerCancelled,
    ] {
        let mut cancelled = pending.clone();
        first.handle_event(&mut cancelled, event);
        assert!(cancelled.virtual_boundary.is_none());
    }
    let mut destroyed = pending.clone();
    destroyed.destroy();
    assert!(destroyed.virtual_boundary.is_none());
    let mut scoped = pending.clone();
    scoped
        .navigation_mut()
        .set_controller_scope(Some(UiId::from("other-scope")));
    assert_eq!(
        last.reconcile_virtual_boundary(&mut scoped),
        Invalidation::None
    );
    assert!(scoped.virtual_boundary.is_none());
    let mut removed = pending;
    let empty = UiFrame::resolve(Column::<()>::new(), FrameRequest::new(bounds, &mut removed));
    assert_eq!(
        empty.reconcile_virtual_boundary(&mut removed),
        Invalidation::None
    );
    assert!(removed.virtual_boundary.is_none());
}

#[test]
fn declaration_child_filter_matches_allocating_scope_oracle() {
    for explicit in [
        None,
        Some(""),
        Some("row-1"),
        Some("row-10"),
        Some("nested/leaf"),
        Some("名前"),
        Some("#01"),
    ] {
        let id = explicit.map(UiId::from);
        for index in [0, 1, 10, usize::MAX] {
            let scope = explicit
                .map(str::to_owned)
                .unwrap_or_else(|| format!("#{index}"));
            for target in [
                "",
                "#0",
                "#00",
                "#1",
                "#01",
                "#+1",
                "#1/leaf",
                "#184467440737095516160",
                "row-1",
                "row-10/leaf",
                "nested/leaf",
                "nested/leaf/child",
                "名前",
                "名前/child",
            ] {
                let expected = target == scope || target.starts_with(&format!("{scope}/"));
                assert_eq!(
                    declared_child_matches(target, id.as_ref(), index),
                    expected,
                    "target={target:?}, scope={scope:?}"
                );
            }
            assert!(declared_child_matches(&scope, id.as_ref(), index));
            assert!(declared_child_matches(
                &format!("{scope}/child"),
                id.as_ref(),
                index
            ));
        }
    }
    for count in [100, 1_000, 10_000] {
        let target = format!("#{}/leaf", count - 1);
        assert_eq!(
            (0..count)
                .filter(|index| declared_child_matches(&target, None, *index))
                .count(),
            1
        );
    }
}

#[test]
fn declared_hover_lookup_preserves_scoped_identity_and_ancestor_foreground() {
    let color: Color = 0xff0c2238;
    let tree = Column::<()>::new()
        .child(
            Column::new()
                .id("row-1")
                .child(Text::new("Wrong").id("leaf")),
        )
        .child(
            Column::new()
                .id("row-10")
                .child(Text::new("Right").id("nested/leaf")),
        )
        .child(Text::new("Anonymous"))
        .into_element()
        .foreground(color);
    let root = UiId::from("root");
    let mut state = UiStateStore::default();
    for (target, expected_id) in [
        ("root/row-1/leaf", Some("leaf")),
        ("root/row-10/nested/leaf", Some("nested/leaf")),
        ("root/#2", None),
    ] {
        let (found, foreground) =
            find_declared_element(&tree, &root, &UiId::from(target), None, &state).unwrap();
        assert_eq!(found.id.as_ref().map(UiId::as_str), expected_id);
        assert_eq!(foreground, Some(color));
    }
    assert!(
        find_declared_element(
            &tree,
            &root,
            &UiId::from("root-other/row-1/leaf"),
            None,
            &state,
        )
        .is_none()
    );
    state.set_hovered(Some(UiId::from("root/row-1")));
    assert!(
        find_declared_element(&tree, &root, &UiId::from("root/row-1/leaf"), None, &state,)
            .is_none(),
        "active ancestor must retain the conservative inheritance guard"
    );
    assert!(
        find_declared_element(
            &tree,
            &root,
            &UiId::from("root/row-10/nested/leaf"),
            None,
            &state,
        )
        .is_some(),
        "unrelated active sibling must not block the target"
    );
}

#[test]
fn static_text_revision_excludes_editors_and_action_authority() {
    let element = Column::new()
        .child(Text::new("Label"))
        .child(Text::new("Action").into_element().message(()))
        .child(TextField::on_change("Editable", |_| ()))
        .child(Text::new("Explicit").content_revision(77))
        .into_element()
        .static_text_content_revision(12);
    assert_eq!(element.content_revision, None);
    assert_eq!(element.children[0].content_revision, Some(12));
    assert_eq!(element.children[1].content_revision, None);
    assert_eq!(element.children[2].content_revision, None);
    assert_eq!(element.children[3].content_revision, Some(77));
    let mut selected = Column::<()>::new()
        .child(Text::new("Selectable"))
        .into_element();
    selected.style.selection_region = true;
    let selected = selected.static_text_content_revision(12);
    assert_eq!(selected.children[0].content_revision, None);

    let bounds = Rect::new(0.0, 0.0, 200.0, 80.0);
    let mut state = UiStateStore::default();
    let view = |label, revision| {
        Text::<()>::new(label)
            .into_element()
            .static_text_content_revision(revision)
    };
    let first = UiFrame::resolve(view("Before", 1), FrameRequest::new(bounds, &mut state));
    let unchanged = UiFrame::resolve_against(
        view("Before", 1),
        FrameRequest::new(bounds, &mut state),
        &first,
    );
    assert_eq!(unchanged.resource_diagnostics().nodes_measured, 0);
    let changed = UiFrame::resolve_against(
        view("Different content", 2),
        FrameRequest::new(bounds, &mut state),
        &unchanged,
    );
    assert_eq!(changed.resource_diagnostics().nodes_measured, 1);
    let cold = UiFrame::resolve(
        view("Different content", 2),
        FrameRequest::new(bounds, &mut state.clone()),
    );
    assert_eq!(changed.commands(), cold.commands());
    assert_eq!(
        changed.resolved.deterministic_snapshot(),
        cold.resolved.deterministic_snapshot()
    );
}

#[test]
fn vertical_scroll_skips_duplicate_content_measurement_only_for_equal_widths() {
    for count in [100, 1_000, 10_000] {
        let content = || {
            Column::<()>::new().children((0..count).map(|row| {
                Container::new()
                    .id(format!("row-{row}"))
                    .width(100.0)
                    .height(20.0)
            }))
        };
        for height in [100.0, count as f32 * 20.0 + 20.0] {
            let frame = UiFrame::layout_with_state(
                VerticalScroll::new((), 0.0).child(content()),
                Rect::new(0.0, 0.0, 200.0, height),
                &mut UiStateStore::default(),
            );
            let work = frame.resource_diagnostics();
            let scrolls = height < count as f32 * 20.0;
            assert_eq!(
                work.scroll_content_measurements,
                if scrolls { 2 } else { 1 }
            );
            assert_eq!(
                work.scroll_content_measurements_reused,
                if scrolls { 0 } else { 1 }
            );
            let extent = frame.scroll_extent(&()).unwrap();
            let width = 200.0
                - if scrolls {
                    scrollbar::SCROLLBAR_GUTTER
                } else {
                    0.0
                };
            let cold = measure_element(
                &content().into_element(),
                Constraints::loose(Size::new(width, f32::INFINITY)),
            );
            assert_eq!(
                extent.content,
                Size::new(cold.width.max(width), cold.height.max(height))
            );
        }
    }
}

#[test]
fn zero_width_scrollbar_gutter_reuses_content_measurement() {
    let mut root = VerticalScroll::new((), 0.0)
        .child(Spacer::vertical(240.0))
        .into_element();
    root.style.scrollbar_parts = Some(Box::new([DropdownPartStyle::default(); 2]));
    let frame = UiFrame::layout_with_state(
        root,
        Rect::new(0.0, 0.0, 200.0, 100.0),
        &mut UiStateStore::default(),
    );
    assert!(frame.scroll_extent(&()).unwrap().can_scroll());
    let work = frame.resource_diagnostics();
    assert_eq!(work.scroll_content_measurements, 1);
    assert_eq!(work.scroll_content_measurements_reused, 1);
    assert_eq!(frame.resolved.nodes[1].allocated.size.width, 200.0);
}

#[test]
fn scroll_intrinsic_measurement_reuses_widths_and_invalidates_changed_content() {
    let build = |height: f32| {
        VerticalScroll::new((), 0.0).id("scroll").child(
            Column::new()
                .id("content")
                .child(Container::new().height(height).width(50.0)),
        )
    };
    let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
    let mut state = UiStateStore::default();
    let first = UiFrame::resolve(build(600.0), FrameRequest::new(bounds, &mut state));
    assert_eq!(first.resource_diagnostics().scroll_content_measurements, 2);
    state.scroll_by(UiId::from("root/scroll"), 40.0, 500.0);
    let scrolled =
        UiFrame::resolve_against(build(600.0), FrameRequest::new(bounds, &mut state), &first);
    assert_eq!(scrolled.scroll_extent(&()).unwrap().offset, 40.0);
    assert_eq!(
        scrolled.resource_diagnostics().scroll_content_measurements,
        0
    );
    assert_eq!(
        scrolled
            .resource_diagnostics()
            .scroll_content_measurements_reused,
        2
    );
    let cold = UiFrame::resolve(build(600.0), FrameRequest::new(bounds, &mut state.clone()));
    assert_eq!(scrolled.scroll_extent(&()), cold.scroll_extent(&()));
    assert_eq!(scrolled.commands(), cold.commands());
    for (height, width) in [(900.0, 200.0), (600.0, 160.0), (50.0, 160.0)] {
        let bounds = Rect::new(0.0, 0.0, width, 100.0);
        let next = UiFrame::resolve_against(
            build(height),
            FrameRequest::new(bounds, &mut state),
            &scrolled,
        );
        assert!(next.resource_diagnostics().scroll_content_measurements > 0);
        let cold = UiFrame::resolve(build(height), FrameRequest::new(bounds, &mut state.clone()));
        assert_eq!(next.scroll_extent(&()), cold.scroll_extent(&()));
        assert_eq!(next.commands(), cold.commands());
        let repeat =
            UiFrame::resolve_against(build(height), FrameRequest::new(bounds, &mut state), &next);
        assert_eq!(
            repeat.resource_diagnostics().scroll_content_measurements,
            0,
            "both widths must remain cached after resizing"
        );
    }
}

#[test]
fn scrollbar_width_change_still_remeasures_wrapped_content() {
    let content = || {
        Text::<()>::new(
            "Wrapped content must use the width remaining beside the scrollbar. ".repeat(8),
        )
        .wrap(true)
    };
    for width in [40.0, 120.0, 200.0] {
        let height = 30.0;
        let frame = UiFrame::layout_with_state(
            VerticalScroll::new((), 0.0).child(content()),
            Rect::new(0.0, 0.0, width, height),
            &mut UiStateStore::default(),
        );
        let work = frame.resource_diagnostics();
        assert_eq!(work.scroll_content_measurements, 2);
        assert_eq!(work.scroll_content_measurements_reused, 0);
        let reduced = width - scrollbar::SCROLLBAR_GUTTER;
        let oracle = measure_element(
            &content().into_element(),
            Constraints::loose(Size::new(reduced, f32::INFINITY)),
        );
        assert_eq!(
            frame.scroll_extent(&()).unwrap().content.height,
            oracle.height.max(height)
        );
        assert_eq!(frame.resolved.nodes[1].allocated.size.width, reduced);
    }
}

#[test]
fn retained_paint_resource_delta_matches_full_accounting_when_vectors_grow() {
    let view = || {
        Container::new()
            .width(100.0)
            .height(40.0)
            .message(())
            .into_element()
            .interaction_backgrounds(0xff334455, 0xff556677)
    };
    let mut state = UiStateStore::default();
    let mut frame =
        UiFrame::layout_with_state(view(), Rect::new(0.0, 0.0, 100.0, 40.0), &mut state);
    let mut accounted = frame.resource_diagnostics().estimated_retained_bytes;
    let initial_vectors = frame.resource_vector_bytes();
    for point in [
        Point { x: 10.0, y: 10.0 },
        Point { x: -1.0, y: -1.0 },
        Point { x: 20.0, y: 10.0 },
    ] {
        frame.handle_event(&mut state, UiEvent::PointerMoved(point));
        let work = frame.refresh_retained_paint(&mut state).unwrap();
        accounted = accounted - work.vector_bytes_before + work.vector_bytes_after;
        assert_eq!(
            accounted,
            frame.resource_diagnostics().estimated_retained_bytes
        );
    }
    assert!(frame.resource_vector_bytes() > initial_vectors);
}

#[test]
fn local_paint_saves_only_affected_subtree_hit_records() {
    for count in [100, 1_000, 10_000] {
        let view = || {
            Column::new()
                .child(
                    Container::new()
                        .id("target")
                        .child(Button::new((), "Target").id("button")),
                )
                .children(
                    (1..count).map(|row| Container::new().id(format!("row-{row}")).height(20.0)),
                )
        };
        let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
        let mut state = UiStateStore::default();
        let mut frame = UiFrame::layout_with_state(view(), bounds, &mut state);
        let target = frame
            .resolved
            .find(&UiId::from("root/target/button"))
            .unwrap()
            .allocated;
        let point = Point {
            x: target.origin.x + target.size.width / 2.0,
            y: target.origin.y + target.size.height / 2.0,
        };
        let original_hits = frame
            .hits
            .iter()
            .map(|hit| (hit.id.clone(), hit.rect))
            .collect::<Vec<_>>();
        let original_stacks = frame
            .resolved
            .nodes
            .iter()
            .map(|node| node.hit_stack)
            .collect::<Vec<_>>();
        for point in [point, Point { x: -1.0, y: -1.0 }, point] {
            frame.handle_event(&mut state, UiEvent::PointerMoved(point));
            let work = frame.refresh_retained_paint(&mut state).unwrap();
            assert_eq!(work.interaction_records_saved, 2, "source count {count}");
            assert_eq!(
                frame
                    .hits
                    .iter()
                    .map(|hit| (hit.id.clone(), hit.rect))
                    .collect::<Vec<_>>(),
                original_hits
            );
            assert_eq!(
                frame
                    .resolved
                    .nodes
                    .iter()
                    .map(|node| node.hit_stack)
                    .collect::<Vec<_>>(),
                original_stacks
            );
            let cold = UiFrame::layout_with_state(view(), bounds, &mut state.clone());
            assert_eq!(frame.commands(), cold.commands());
            assert_eq!(
                frame.resolved.deterministic_snapshot(),
                cold.resolved.deterministic_snapshot()
            );
            assert_eq!(frame.semantic_nodes(), cold.semantic_nodes());
        }
    }
}

#[test]
fn pointer_motion_does_not_rescan_unchanged_declarations() {
    for count in [100, 1_000, 10_000] {
        let root = Column::<()>::new().children((0..count).map(|row| {
            Container::new()
                .id(format!("row-{row}"))
                .width(100.0)
                .height(20.0)
        }));
        let mut state = UiStateStore::default();
        POINTER_GEOMETRY_VISITS.with(|visits| visits.set(0));
        let frame = UiFrame::layout_with_state(root, Rect::new(0.0, 0.0, 200.0, 100.0), &mut state);
        assert_eq!(
            POINTER_GEOMETRY_VISITS.with(|visits| visits.get()),
            count + 1
        );
        assert!(!frame.pointer_position_affects_geometry);
        // First input can adopt pointer modality and request paint independently
        // of position-dependent geometry.
        frame.handle_event(&mut state, UiEvent::PointerMoved(Point { x: 0.0, y: 10.0 }));
        POINTER_GEOMETRY_VISITS.with(|visits| visits.set(0));
        for x in 0..256 {
            let outcome = frame.handle_event(
                &mut state,
                UiEvent::PointerMoved(Point {
                    x: x as f32,
                    y: 10.0,
                }),
            );
            assert_eq!(outcome.invalidation, Invalidation::None);
        }
        assert_eq!(POINTER_GEOMETRY_VISITS.with(|visits| visits.get()), 0);
    }
}

#[test]
fn pointer_geometry_summary_changes_with_declaration_replacement() {
    let view = |proximity: bool| {
        let child = Row::new()
            .child(Button::new((), "Item").width(40.0).height(40.0))
            .into_element();
        let child = if proximity {
            child.proximity_magnification(ProximityMagnification {
                maximum_scale: 1.5,
                radius: 1,
            })
        } else {
            child
        };
        Container::new().child(child)
    };
    let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
    let mut state = UiStateStore::default();
    let mut frame = UiFrame::resolve(view(false), FrameRequest::new(bounds, &mut state));
    for proximity in [true, false, true, false] {
        frame = UiFrame::resolve_against(
            view(proximity),
            FrameRequest::new(bounds, &mut state),
            &frame,
        );
        let oracle = pointer_position_affects_geometry(frame.declaration_root.as_ref().unwrap());
        assert_eq!(frame.pointer_position_affects_geometry, oracle);
        assert_eq!(oracle, proximity);
        let mut cold_state = state.clone();
        let cold = UiFrame::resolve(view(proximity), FrameRequest::new(bounds, &mut cold_state));
        for x in [10.0, 20.0, 30.0, 190.0] {
            let event = UiEvent::PointerMoved(Point { x, y: 10.0 });
            let retained = frame.handle_event(&mut state, event.clone());
            let fresh = cold.handle_event(&mut cold_state, event);
            assert_eq!(retained.invalidation, fresh.invalidation);
            assert_eq!(state.hovered(), cold_state.hovered());
        }
    }
}

#[test]
fn measured_virtual_heights_share_base_and_copy_only_changed_paths() {
    for count in [100usize, 1_000, 10_000] {
        for uniform in [false, true] {
            let original: Vec<f32> = (0..count)
                .map(|row| if uniform || row % 2 == 0 { 20.0 } else { 70.0 })
                .collect();
            let base = Arc::new(if uniform {
                VirtualHeightIndex::uniform(count, 20.0, 5.0)
            } else {
                VirtualHeightIndex::new(&original, 5.0)
            });
            let base_window = base.window(400.0, 100.0, 64.0);
            let mut measured = original.clone();
            let mut corrected = Arc::clone(&base);
            let mut previous = Vec::new();
            for (row, height) in [(0, 80.0), (count / 2, 12.5), (count - 1, 100.0), (0, 40.0)] {
                previous.push((Arc::clone(&corrected), corrected.window(400.0, 100.0, 64.0)));
                let (next, allocations) = corrected.with_measured_height(row, height).unwrap();
                assert!(
                    allocations <= count.ilog2() as usize + 2,
                    "{allocations} nodes allocated for {count} rows"
                );
                assert_eq!(next.height(row), Some(height));
                assert_eq!(next.len(), count);
                let (same, noop_work) = next.with_measured_height(row, height).unwrap();
                assert!(Arc::ptr_eq(&next, &same));
                assert_eq!(noop_work, 0);
                measured[row] = height;
                let oracle = VirtualHeightIndex::new(&measured, 5.0);
                for offset in [0.0, 400.0, count as f32 * 20.0, 1_000_000.0] {
                    let (window, work) = next.window_with_work(offset, 100.0, 64.0);
                    assert_eq!(window, oracle.window(offset, 100.0, 64.0));
                    assert_eq!(next.window_for_range(window.range.clone()), Some(window));
                    let depth = count.ilog2() as usize + 2;
                    assert!(
                        work <= 4 * depth * depth,
                        "{work} query operations for {count} rows"
                    );
                }
                corrected = next;
            }
            assert_eq!(base.window(400.0, 100.0, 64.0), base_window);
            for (snapshot, window) in previous {
                assert_eq!(snapshot.window(400.0, 100.0, 64.0), window);
            }
            for row in [0, count / 2, count - 1] {
                corrected = corrected
                    .with_measured_height(row, original[row])
                    .unwrap()
                    .0;
            }
            assert!(
                Arc::ptr_eq(&corrected, &base),
                "reverting corrections must release the sparse tree"
            );
            for invalid in [f32::NAN, f32::INFINITY, -1.0] {
                assert!(base.with_measured_height(0, invalid).is_none());
            }
            assert!(base.with_measured_height(count, 20.0).is_none());
        }
    }
    let base = Arc::new(VirtualHeightIndex::uniform(10_000, 20.2, 1.3));
    let (corrected, _) = base.with_measured_height(9_999, 30.7).unwrap();
    assert_eq!(corrected.height(9_999), Some(30.7));
    assert!(
        Arc::ptr_eq(
            &corrected,
            &corrected.with_measured_height(9_999, 30.7).unwrap().0
        ),
        "fractional geometry must not keep allocating unchanged corrections"
    );
    let invalid = Arc::new(VirtualHeightIndex::new(&[f32::INFINITY], 0.0));
    assert!(invalid.with_measured_height(0, 20.0).is_none());
}

#[test]
fn measured_height_storage_plateaus_across_full_traversals() {
    for count in [100usize, 1_000, 10_000] {
        let base = Arc::new(VirtualHeightIndex::uniform(count, 20.0, 5.0));
        let mut corrected = Arc::clone(&base);
        let mut measured = vec![20.0; count];
        let mut plateau = None;
        for round in 0..3 {
            for (row, measured_height) in measured.iter_mut().enumerate() {
                *measured_height = if row % 2 == 0 {
                    22.5 + round as f32
                } else {
                    40.75 + round as f32
                };
                let previous = Arc::downgrade(&corrected);
                let previous_was_base = Arc::ptr_eq(&corrected, &base);
                let (next, allocations) = corrected
                    .with_measured_height(row, *measured_height)
                    .unwrap();
                assert!(allocations <= count.ilog2() as usize + 2);
                corrected = next;
                if !previous_was_base {
                    assert!(
                        previous.upgrade().is_none(),
                        "correction retained a previous index wrapper"
                    );
                }
            }
            let bytes = corrected.retained_bytes();
            assert!(
                bytes <= count * 128,
                "correction payload grew beyond a bounded binary tree"
            );
            if let Some(previous) = plateau {
                assert_eq!(bytes, previous, "retained payload grew after warm-up");
            }
            plateau = Some(bytes);
            let oracle = VirtualHeightIndex::new(&measured, 5.0);
            for offset in [0.0, 400.0, count as f32 * 20.0, 1_000_000.0] {
                assert_eq!(
                    corrected.window(offset, 100.0, 64.0),
                    oracle.window(offset, 100.0, 64.0)
                );
            }
        }
        for (row, height) in measured.into_iter().enumerate() {
            let (next, allocations) = corrected.with_measured_height(row, height).unwrap();
            assert_eq!(allocations, 0);
            assert!(Arc::ptr_eq(&corrected, &next));
        }
    }
}

#[test]
fn retained_uniform_height_index_keeps_constant_payload_and_matches_expanded_geometry() {
    for count in [0usize, 1, 100, 1_000, 10_000] {
        for height in [1.0, 20.5, 126.0] {
            for gap in [0.0, 1.25, 5.0] {
                let compact = VirtualHeightIndex::uniform(count, height, gap);
                let expanded = VirtualHeightIndex::new(&vec![height; count], gap);
                assert_eq!(compact.len(), count);
                assert_eq!(compact.is_empty(), count == 0);
                assert_eq!(
                    compact.retained_bytes(),
                    0,
                    "uniform index allocated per-item extents"
                );
                for viewport in [100.0, 800.0] {
                    for offset in [0.0, 1.0, 36.0, count as f32 * 20.0, 1_000_000.0] {
                        for overscan in [0.0, 64.0] {
                            let (window, comparisons) =
                                compact.window_with_work(offset, viewport, overscan);
                            assert_eq!(window, expanded.window(offset, viewport, overscan));
                            assert_eq!(
                                compact.window_for_range(window.range.clone()),
                                Some(window)
                            );
                            let bound = if count == 0 {
                                0
                            } else {
                                2 * (count.ilog2() as usize + 2)
                            };
                            assert!(comparisons <= bound);
                        }
                    }
                }
                for range in [0..0, count..count, 0..count] {
                    assert_eq!(
                        compact.window_for_range(range.clone()),
                        expanded.window_for_range(range)
                    );
                }
                assert!(compact.window_for_range(count + 1..count + 1).is_none());
            }
        }
    }
}

#[test]
fn embedded_virtual_collection_uses_translated_ancestor_clip() {
    let index = VirtualHeightIndex::new(&[20.0; 100], 0.0);
    let clip = Rect::new(0.0, 0.0, 320.0, 100.0);
    let below = index.window_in_clip(Rect::new(0.0, 500.0, 320.0, 2000.0), clip, 40.0);
    assert!(below.range.is_empty());
    assert_eq!(below.trailing, 2000.0);
    let partial = index.window_in_clip(Rect::new(0.0, 60.0, 320.0, 2000.0), clip, 0.0);
    assert_eq!(partial, index.window(0.0, 40.0, 0.0));
    let scrolled = index.window_in_clip(Rect::new(0.0, -500.0, 320.0, 2000.0), clip, 40.0);
    assert_eq!(scrolled, index.window(460.0, 180.0, 0.0));
    assert!(
        index
            .window_in_clip(Rect::new(400.0, 0.0, 320.0, 2000.0), clip, 40.0)
            .range
            .is_empty()
    );
    assert!(
        index
            .window_in_clip(
                Rect::new(0.0, 0.0, 320.0, 2000.0),
                Rect::new(0.0, 0.0, 320.0, 0.0),
                40.0
            )
            .range
            .is_empty()
    );
}

#[test]
fn retained_variable_height_windows_match_linear_boundaries_with_logarithmic_work() {
    fn linear(
        heights: &[f32],
        gap: f32,
        offset: f32,
        viewport: f32,
        overscan: f32,
    ) -> VirtualWindow {
        let mut starts = Vec::new();
        let mut ends = Vec::new();
        let mut total = 0.0;
        for (index, height) in heights.iter().enumerate() {
            if index != 0 {
                total += gap;
            }
            starts.push(total);
            total += height.max(0.0);
            ends.push(total);
        }
        let offset = offset.clamp(0.0, (total - viewport).max(0.0));
        let first = ends
            .iter()
            .position(|end| *end >= (offset - overscan).max(0.0))
            .unwrap_or(heights.len());
        let end = starts
            .iter()
            .rposition(|start| *start <= (offset + viewport + overscan).min(total))
            .map_or(first, |index| index + 1)
            .max(first);
        let leading = starts.get(first).copied().unwrap_or(total);
        let visible_end = end
            .checked_sub(1)
            .and_then(|index| ends.get(index))
            .copied()
            .unwrap_or(leading);
        VirtualWindow {
            range: first..end,
            leading,
            trailing: (total - visible_end).max(0.0),
            total,
        }
    }
    for count in [0usize, 1, 100, 1_000, 10_000] {
        let heights = (0..count)
            .map(|index| match index % 4 {
                0 => 0.0,
                1 => 36.0,
                2 => 126.0,
                _ => 52.0,
            })
            .collect::<Vec<_>>();
        for gap in [0.0, 5.0] {
            let index = VirtualHeightIndex::new(&heights, gap);
            assert_eq!(index.len(), count);
            assert_eq!(index.is_empty(), count == 0);
            assert_eq!(
                index.retained_bytes(),
                count * 2 * std::mem::size_of::<f32>()
            );
            for viewport in [0.0, 100.0, 800.0] {
                for offset in [0.0, 1.0, 36.0, count as f32 * 20.0, 1_000_000.0] {
                    for overscan in [0.0, 64.0] {
                        let (window, comparisons) =
                            index.window_with_work(offset, viewport, overscan);
                        assert_eq!(window, linear(&heights, gap, offset, viewport, overscan));
                        let bound = if count == 0 {
                            0
                        } else {
                            2 * (count.ilog2() as usize + 2)
                        };
                        assert!(
                            comparisons <= bound,
                            "{comparisons} comparisons for {count} items"
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn unchanged_paint_refresh_does_not_clone_declaration_messages() {
    struct CountedMessage(std::sync::Arc<std::sync::atomic::AtomicUsize>);
    impl Clone for CountedMessage {
        fn clone(&self) -> Self {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            Self(self.0.clone())
        }
    }
    let clones = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let mut state = UiStateStore::default();
    let mut frame = UiFrame::resolve(
        Text::new("unchanged")
            .into_element()
            .message(CountedMessage(clones.clone())),
        FrameRequest::new(Rect::new(0.0, 0.0, 200.0, 100.0), &mut state),
    );
    clones.store(0, std::sync::atomic::Ordering::Relaxed);
    assert!(frame.refresh_retained_paint(&mut state).is_none());
    assert_eq!(clones.load(std::sync::atomic::Ordering::Relaxed), 0);
}

fn legacy_is_descendant_or_self<Message: Clone>(
    frame: &UiFrame<Message>,
    ancestor: &UiId,
    candidate: &UiId,
) -> bool {
    frame
        .node_index(ancestor)
        .is_some_and(|index| frame.subtree_contains_id(index, candidate))
        || frame.messages.iter().any(|region| {
            region.id == *candidate
                && region.navigation_owner.as_ref().is_some_and(|owner| {
                    owner == ancestor || legacy_is_descendant_or_self(frame, ancestor, owner)
                })
        })
}

#[test]
fn retained_ancestry_matches_tree_walk_for_normal_nodes() {
    let mut state = UiStateStore::default();
    let frame = UiFrame::resolve(
        Column::<()>::new().children((0..32).map(|index| {
            Container::<()>::new()
                .id(format!("item-{index}"))
                .height(1.0)
        })),
        FrameRequest::new(Rect::new(0.0, 0.0, 100.0, 32.0), &mut state),
    );
    let root = UiId::from("root");
    for index in 0..32 {
        let candidate = root.scoped(format!("item-{index}"));
        assert_eq!(
            frame.is_descendant_or_self(&root, &candidate),
            legacy_is_descendant_or_self(&frame, &root, &candidate)
        );
        assert_eq!(
            frame.is_descendant_or_self(&candidate, &root),
            legacy_is_descendant_or_self(&frame, &candidate, &root)
        );
    }
}

#[test]
fn retained_ancestry_scales_across_two_thousand_siblings() {
    const NODES: usize = 2_000;
    let mut state = UiStateStore::default();
    let frame = UiFrame::resolve(
        Column::<()>::new().children((0..NODES - 1).map(|index| {
            Container::<()>::new()
                .id(format!("item-{index}"))
                .height(1.0)
        })),
        FrameRequest::new(Rect::new(0.0, 0.0, 100.0, NODES as f32), &mut state),
    );
    let root = UiId::from("root");
    assert_eq!(frame.retained_nodes.nodes().count(), NODES);
    for index in 0..NODES - 1 {
        let candidate = root.scoped(format!("item-{index}"));
        assert!(frame.is_descendant_or_self(&root, &candidate));
        assert!(!frame.is_descendant_or_self(&candidate, &root));
    }
}

#[test]
fn selection_marquee_is_frame_owned_and_rejects_empty_geometry() {
    let mut frame = UiFrame::<()>::default();
    frame.selection_marquee(Rect::new(4.0, 8.0, 40.0, 24.0), 0xabcdef, 3.0);
    frame.selection_marquee(Rect::new(4.0, 8.0, 0.0, 24.0), 0xabcdef, 3.0);

    assert_eq!(frame.commands().len(), 1);
    assert!(matches!(
        &frame.commands()[0],
        PaintCommand::OverlayStroke { rect, color, width }
            if *rect == Rect::new(4.0, 8.0, 40.0, 24.0)
                && *color == 0xabcdef
                && *width == 3.0
    ));
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum TestMessage {
    Named(&'static str),
    Option(usize),
    Volume(u8),
    Query(String),
    Drag(DragPhase, i32, i32),
    Drop(String, String, i32, i32),
}

fn map_volume(value: f32) -> TestMessage {
    TestMessage::Volume((value * 100.0).round() as u8)
}

fn map_query(value: String) -> TestMessage {
    TestMessage::Query(value)
}

fn map_drag(_seed: TestMessage, gesture: DragGesture) -> TestMessage {
    TestMessage::Drag(
        gesture.phase,
        gesture.position.x.round() as i32,
        gesture.position.y.round() as i32,
    )
}

fn map_drop(_seed: TestMessage, gesture: DropGesture) -> TestMessage {
    TestMessage::Drop(
        gesture.source_id.as_str().to_owned(),
        gesture.target_id.as_str().to_owned(),
        gesture.source_bounds.size.width.round() as i32,
        gesture.target_bounds.size.width.round() as i32,
    )
}

#[test]
fn captured_drag_releases_on_another_drop_target() {
    let tree = UiFrame::layout(
        Row::new()
            .child(
                Container::new()
                    .id("source")
                    .width(80.0)
                    .height(40.0)
                    .on_drag((TestMessage::Named("seed"), map_drag)),
            )
            .child(
                Container::new()
                    .id("target")
                    .width(80.0)
                    .height(40.0)
                    .on_drop((TestMessage::Named("seed"), map_drop)),
            ),
        Rect::new(0.0, 0.0, 160.0, 40.0),
    );
    let mut state = UiStateStore::default();
    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 20.0, y: 20.0 }),
    );
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point { x: 120.0, y: 20.0 }),
        )
        .messages,
        vec![
            TestMessage::Drag(DragPhase::Ended, 120, 20),
            TestMessage::Drop("root/source".into(), "root/target".into(), 80, 80),
        ]
    );
    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 120.0, y: 20.0 }),
    );
    assert!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point { x: 120.0, y: 20.0 }),
        )
        .messages
        .is_empty()
    );
}

#[test]
fn proximity_magnification_eases_hover_and_neighbor_geometry() {
    let view = || {
        Row::new()
            .children([
                Button::new(TestMessage::Named("left"), "L")
                    .id("left")
                    .width(40.0)
                    .height(40.0),
                Button::new(TestMessage::Named("center"), "C")
                    .id("center")
                    .width(40.0)
                    .height(40.0),
                Button::new(TestMessage::Named("right"), "R")
                    .id("right")
                    .width(40.0)
                    .height(40.0),
            ])
            .into_element()
            .proximity_magnification(ProximityMagnification {
                maximum_scale: 1.5,
                radius: 1,
            })
            .transition_duration_ms(1_000.0)
    };
    let mut state = UiStateStore::default();
    state.set_hovered(Some(UiId::from("root/center")));
    let resting = UiFrame::layout_with_state(view(), Rect::new(0.0, 0.0, 240.0, 80.0), &mut state);
    let width = |frame: &UiFrame<TestMessage>, id: &str| {
        frame
            .resolved_layout()
            .find(&UiId::from(id))
            .unwrap()
            .allocated
            .size
            .width
    };
    assert_eq!(width(&resting, "root/center"), 40.0);
    std::thread::sleep(std::time::Duration::from_millis(20));
    let first = UiFrame::layout_with_state(view(), Rect::new(0.0, 0.0, 240.0, 80.0), &mut state);
    let center_first = width(&first, "root/center");
    let left_first = width(&first, "root/left");
    assert!(center_first > left_first && left_first > 40.0);

    std::thread::sleep(std::time::Duration::from_millis(20));
    let second = UiFrame::layout_with_state(view(), Rect::new(0.0, 0.0, 240.0, 80.0), &mut state);
    assert!(
        width(&second, "root/center") > center_first,
        "second={}, first={center_first}, active={}",
        width(&second, "root/center"),
        state.geometry_animation_active()
    );
    assert!(state.geometry_animation_active());
}

#[test]
fn pointer_departure_clears_proximity_hover() {
    let view = || {
        Row::new()
            .child(
                Button::new(TestMessage::Named("item"), "Item")
                    .id("item")
                    .width(40.0)
                    .height(40.0),
            )
            .into_element()
            .proximity_magnification(ProximityMagnification {
                maximum_scale: 1.5,
                radius: 1,
            })
    };
    let mut state = UiStateStore::default();
    let frame = UiFrame::layout_with_state(view(), Rect::new(0.0, 0.0, 100.0, 60.0), &mut state);
    frame.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point { x: 20.0, y: 20.0 }),
    );
    assert!(state.hovered().is_some());

    frame.handle_event(&mut state, UiEvent::PointerCancelled);

    assert_eq!(state.hovered(), None);
}

#[test]
fn motion_within_one_hover_target_does_not_repaint_position_independent_tree() {
    let tree = UiFrame::layout(
        Button::new(TestMessage::Named("press"), "Button")
            .id("button")
            .width(100.0)
            .height(40.0),
        Rect::new(0.0, 0.0, 120.0, 60.0),
    );
    let mut state = UiStateStore::default();
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerMoved(Point { x: 10.0, y: 10.0 }),
        )
        .invalidation,
        Invalidation::Paint
    );
    for x in 11..90 {
        assert_eq!(
            tree.handle_event(
                &mut state,
                UiEvent::PointerMoved(Point {
                    x: x as f32,
                    y: 20.0,
                }),
            )
            .invalidation,
            Invalidation::None
        );
    }
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerMoved(Point {
                x: 1_000.0,
                y: 1_000.0,
            }),
        )
        .invalidation,
        Invalidation::Paint
    );
}

#[test]
fn proximity_magnification_keeps_coordinate_dependent_motion_live() {
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        Row::new()
            .child(
                Button::new(TestMessage::Named("item"), "Item")
                    .id("item")
                    .width(100.0)
                    .height(40.0),
            )
            .into_element()
            .proximity_magnification(ProximityMagnification {
                maximum_scale: 1.5,
                radius: 1,
            }),
        Rect::new(0.0, 0.0, 120.0, 60.0),
        &mut state,
    );
    tree.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point { x: 10.0, y: 10.0 }),
    );
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerMoved(Point { x: 80.0, y: 10.0 }),
        )
        .invalidation,
        Invalidation::Layout
    );
}

#[test]
fn declarative_drag_target_captures_motion_beyond_its_bounds() {
    let tree = UiFrame::layout(
        Container::new()
            .id("draggable")
            .width(80.0)
            .height(40.0)
            .accessibility_label("Draggable item")
            .on_drag((TestMessage::Named("drag seed"), map_drag)),
        Rect::new(10.0, 20.0, 80.0, 40.0),
    );
    let mut state = UiStateStore::default();

    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerPressed(Point { x: 20.0, y: 30.0 }),
        )
        .messages,
        vec![TestMessage::Drag(DragPhase::Started, 20, 30)]
    );
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerMoved(Point { x: 180.0, y: 130.0 }),
        )
        .messages,
        vec![TestMessage::Drag(DragPhase::Moved, 180, 130)]
    );
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point { x: 180.0, y: 130.0 }),
        )
        .messages,
        vec![TestMessage::Drag(DragPhase::Ended, 180, 130)]
    );
    assert!(state.captured().is_none());

    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 20.0, y: 30.0 }),
    );
    assert_eq!(
        tree.handle_event(&mut state, UiEvent::FocusLost).messages,
        vec![TestMessage::Drag(DragPhase::Cancelled, 10, 20)]
    );
    assert!(state.captured().is_none());
}

#[test]
fn semantic_message_queries_preserve_duplicates_and_require_uniqueness() {
    let duplicate = TestMessage::Named("open");
    let tree = UiFrame::layout(
        Row::new()
            .child(
                Container::new()
                    .id("first")
                    .width(80.0)
                    .height(40.0)
                    .semantic_role(SemanticRole::Button)
                    .accessibility_label("Open first")
                    .message(duplicate.clone()),
            )
            .child(
                Container::new()
                    .id("second")
                    .width(80.0)
                    .height(40.0)
                    .semantic_role(SemanticRole::Button)
                    .accessibility_label("Open second")
                    .message(duplicate.clone()),
            )
            .child(
                Container::new()
                    .id("save")
                    .width(80.0)
                    .height(40.0)
                    .semantic_role(SemanticRole::Button)
                    .accessibility_label("Save")
                    .message(TestMessage::Named("save")),
            ),
        Rect::new(0.0, 0.0, 240.0, 40.0),
    );

    let targets = tree.semantic_targets_for_message(&duplicate);
    assert_eq!(targets.len(), 2);
    assert_eq!(targets[0].name.as_deref(), Some("Open first"));
    assert_eq!(targets[1].name.as_deref(), Some("Open second"));
    assert!(targets.iter().all(|target| target.interactive));
    assert_eq!(
        tree.unique_semantic_target_for_message(&duplicate),
        Err(SemanticQueryError::Ambiguous { matches: 2 })
    );
    assert_eq!(
        tree.unique_semantic_target_for_message(&TestMessage::Named("missing")),
        Err(SemanticQueryError::Missing)
    );
    assert_eq!(
        tree.unique_semantic_target_for_message(&TestMessage::Named("save"))
            .expect("save is unique")
            .name
            .as_deref(),
        Some("Save")
    );
    assert_eq!(
        tree.query(&SemanticSelector::Role(SemanticRole::Button))
            .len(),
        3
    );
    assert_eq!(
        tree.query_unique(&SemanticSelector::RoleAndName {
            role: SemanticRole::Button,
            name: "Save".into(),
        })
        .expect("role and name identify Save")
        .id,
        UiId::from("root/save")
    );
}

#[test]
fn resolved_frame_releases_build_only_storage_and_reports_live_authority() {
    let mut state = UiStateStore::default();
    let frame = UiFrame::resolve(
        Button::new(TestMessage::Named("save"), "Save").id("save"),
        FrameRequest::new(Rect::new(0.0, 0.0, 160.0, 48.0), &mut state)
            .diagnostics(DiagnosticMode::Collect),
    );

    let resources = frame.resource_diagnostics();
    assert!(resources.node_count > 0);
    assert!(resources.paint_primitive_count > 0);
    assert!(resources.hit_target_count > 0);
    assert!(resources.message_binding_count > 0);
    assert!(resources.accessibility_node_count > 0);
    assert!(resources.estimated_retained_bytes > 0);
    assert_eq!(resources.retained_build_scratch_bytes, 0);
}

#[test]
fn button_semantics_accessibility_and_pointer_share_one_activation_authority() {
    let mut state = UiStateStore::default();
    let frame = UiFrame::resolve(
        Button::new(TestMessage::Named("save"), "Save")
            .id("save")
            .background(0x223344),
        FrameRequest::new(Rect::new(0.0, 0.0, 160.0, 48.0), &mut state),
    );
    let id = UiId::from("root/save");
    let semantic = frame
        .semantic_nodes()
        .into_iter()
        .find(|node| node.id == id)
        .expect("button semantic node");
    assert_eq!(semantic.role, Some(SemanticRole::Button));
    assert_eq!(semantic.name.as_deref(), Some("Save"));
    assert_eq!(semantic.actions, vec![ActionKind::Activate]);
    let accessibility = frame
        .accessibility_nodes()
        .iter()
        .find(|node| node.id == id)
        .expect("same accessibility node");
    assert_eq!(accessibility.role.as_deref(), Some("button"));
    assert_eq!(accessibility.semantic_role, semantic.role);
    assert_eq!(accessibility.actions, semantic.actions);
    assert_eq!(accessibility.rect, semantic.bounds);
    assert_eq!(
        frame
            .unique_semantic_target_for_message(&TestMessage::Named("save"))
            .unwrap()
            .bounds,
        semantic.bounds
    );
    assert!(frame.commands().iter().any(|command| match command {
        PaintCommand::Fill { rect, .. }
        | PaintCommand::RoundedFill { rect, .. }
        | PaintCommand::TopRoundedFill { rect, .. } => *rect == semantic.bounds,
        _ => false,
    }));

    frame.handle_event(&mut state, UiEvent::FocusNext);
    assert_eq!(state.focused(), Some(&id));
    frame.handle_event(&mut state, UiEvent::ControllerDown);
    assert_eq!(state.navigation().controller_selected(), Some(&id));

    let semantic_outcome = frame
        .perform_semantic_action(&id, SemanticAction::Invoke(ActionKind::Activate))
        .expect("advertised action is invokable");
    assert_eq!(semantic_outcome.messages, vec![TestMessage::Named("save")]);

    let point = Point {
        x: semantic.bounds.origin.x + semantic.bounds.size.width / 2.0,
        y: semantic.bounds.origin.y + semantic.bounds.size.height / 2.0,
    };
    assert!(
        frame
            .handle_event(&mut state, UiEvent::PointerPressed(point))
            .messages
            .is_empty()
    );
    assert_eq!(
        frame
            .handle_event(&mut state, UiEvent::PointerReleased(point))
            .messages,
        semantic_outcome.messages
    );
}

fn bounded_custom_commands(rect: Rect) -> Vec<PaintCommand> {
    vec![
        PaintCommand::Fill {
            rect,
            color: 0x112233,
        },
        PaintCommand::Fill {
            rect: Rect::new(
                rect.origin.x - 1.0,
                rect.origin.y,
                rect.size.width,
                rect.size.height,
            ),
            color: 0xff0000,
        },
        PaintCommand::OverlayFill {
            rect,
            color: 0xff00ff,
        },
    ]
}

#[test]
fn custom_paint_is_clipped_to_allocation_and_cannot_mutate_overlay_authority() {
    let frame = UiFrame::<()>::layout(
        CustomPaint::new(bounded_custom_commands)
            .id("art")
            .width(80.0)
            .height(40.0),
        Rect::new(0.0, 0.0, 80.0, 40.0),
    );
    assert_eq!(
        frame
            .commands()
            .iter()
            .filter(|command| matches!(command, PaintCommand::Fill { .. }))
            .count(),
        1
    );
    assert!(
        !frame
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::OverlayFill { .. }))
    );
    assert!(frame.semantic_nodes().is_empty());
}

#[test]
fn text_measure_cache_bypass_preserves_plain_styled_semantic_and_raster_authority() {
    let build = || {
        UiFrame::<()>::layout(
            Column::new()
                .child(Text::new("Plain text").accessibility_label("plain"))
                .child(StyledText::new(
                    "Styled text",
                    vec![StyledTextSpan {
                        range: 0..6,
                        bold: true,
                        italic: false,
                        monospace: false,
                        font_family: None,
                        strikethrough: false,
                        underline: TextUnderlineStyle::None,
                        color: None,
                        background: None,
                    }],
                )),
            Rect::new(0.0, 0.0, 240.0, 100.0),
        )
    };
    let cached = with_text_measure_cache_mode(TextMeasureCacheMode::Enabled, build);
    let bypass = with_text_measure_cache_mode(TextMeasureCacheMode::BypassDerived, build);
    assert_eq!(cached.resolved_layout(), bypass.resolved_layout());
    assert_eq!(cached.semantic_nodes(), bypass.semantic_nodes());
    assert_eq!(cached.accessibility_nodes(), bypass.accessibility_nodes());
    assert_eq!(cached.commands(), bypass.commands());
}

#[test]
fn slider_and_text_field_semantic_values_use_production_mappers() {
    let frame = UiFrame::layout(
        Column::new()
            .child(Slider::on_change(map_volume, 0.5).id("volume"))
            .child(TextField::on_change("nickel", map_query).id("query")),
        Rect::new(0.0, 0.0, 320.0, 80.0),
    );
    let volume = UiId::from("root/volume");
    let query = UiId::from("root/query");
    let nodes = frame.semantic_nodes();
    let slider = nodes
        .iter()
        .find(|node| node.id == volume)
        .expect("slider semantics");
    assert_eq!(slider.role, Some(SemanticRole::Slider));
    assert_eq!(
        slider.actions,
        vec![
            ActionKind::Increment,
            ActionKind::Decrement,
            ActionKind::SetValue
        ]
    );
    assert_eq!(
        slider.value,
        Some(SemanticValueSnapshot::Number {
            value: 0.5,
            minimum: 0.0,
            maximum: 1.0,
            step: 0.05000000074505806,
        })
    );
    assert_eq!(
        frame
            .perform_semantic_action(&volume, SemanticAction::Invoke(ActionKind::Increment),)
            .expect("increment is advertised")
            .messages,
        vec![TestMessage::Volume(55)]
    );
    assert_eq!(
        frame
            .perform_semantic_action(
                &volume,
                SemanticAction::SetValue(SemanticValueInput::Number(0.8)),
            )
            .expect("numeric value is advertised")
            .messages,
        vec![TestMessage::Volume(80)]
    );
    for (value, expected) in [
        (0.0, vec![ActionKind::Increment, ActionKind::SetValue]),
        (1.0, vec![ActionKind::Decrement, ActionKind::SetValue]),
    ] {
        let boundary = UiFrame::layout(
            Slider::on_change(map_volume, value).id("volume"),
            Rect::new(0.0, 0.0, 320.0, 40.0),
        );
        assert_eq!(boundary.semantic_nodes()[0].actions, expected);
    }

    let text = nodes
        .iter()
        .find(|node| node.id == query)
        .expect("text field semantics");
    assert_eq!(text.role, Some(SemanticRole::TextField));
    assert_eq!(text.actions, vec![ActionKind::SetValue]);
    assert_eq!(
        text.value,
        Some(SemanticValueSnapshot::Text("nickel".into()))
    );
    assert_eq!(
        frame
            .perform_semantic_action(
                &query,
                SemanticAction::SetValue(SemanticValueInput::Text("search".into())),
            )
            .expect("text value is advertised")
            .messages,
        vec![TestMessage::Query("search".into())]
    );
    assert_eq!(
        frame.perform_semantic_action(&query, SemanticAction::Invoke(ActionKind::Activate),),
        Err(SemanticActionError::ActionUnavailable)
    );
}

#[test]
fn masked_text_field_semantics_never_publish_the_input_value() {
    let frame = UiFrame::layout(
        TextField::on_change_masked_with_placeholder("secret", "Password", '•', map_query)
            .id("password")
            .accessibility_label("Password"),
        Rect::new(0.0, 0.0, 320.0, 40.0),
    );
    let password = &frame.semantic_nodes()[0];

    assert!(frame.has_protected_text());
    assert!(
        !UiFrame::<()>::layout(Text::new("Ordinary"), Rect::new(0.0, 0.0, 100.0, 30.0))
            .has_protected_text()
    );

    assert_eq!(password.role, Some(SemanticRole::TextField));
    assert_eq!(password.name.as_deref(), Some("Password"));
    assert_eq!(password.actions, vec![ActionKind::SetValue]);
    assert_eq!(
        password.value,
        Some(SemanticValueSnapshot::ProtectedText { character_count: 6 })
    );
}

#[test]
fn focused_masked_text_field_never_paints_the_input_value() {
    let build = |state: &mut UiStateStore, value: &str| {
        UiFrame::layout_with_state(
            TextField::on_change_masked(value, '•', map_query).id("password"),
            Rect::new(0.0, 0.0, 320.0, 40.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let first = build(&mut state, "secret");
    first.handle_event(&mut state, UiEvent::FocusNext);

    let focused = build(&mut state, "secret");
    assert!(focused.commands().iter().any(|command| {
        matches!(command, PaintCommand::Text { text, .. } if text == "••••••")
    }));
    assert!(focused.commands().iter().all(|command| {
        !matches!(command, PaintCommand::Text { text, .. } if text.contains("secret"))
    }));
}

#[test]
fn single_line_text_field_centers_placeholder_and_masked_text() {
    for scale in [12.0, 18.0, 27.0] {
        let line_height = scale * 1.3;
        for height in [line_height, line_height + 18.0, line_height + 44.0] {
            for value in [
                "".to_owned(),
                "nickel".to_owned(),
                "very-long-password".repeat(24),
            ] {
                let field = Rect::new(0.0, 0.0, 340.0, height);
                let frame = UiFrame::layout(
                    TextField::on_change_masked_with_placeholder(
                        &value,
                        "密码 Password العربية",
                        '•',
                        map_query,
                    )
                    .scale(scale)
                    .single_line_height(field.size.height),
                    field,
                );
                let bounds = frame
                    .commands()
                    .iter()
                    .find_map(|command| match command {
                        PaintCommand::Text { bounds, .. } => Some(*bounds),
                        _ => None,
                    })
                    .expect("single-line text paint");

                let field_center = field.origin.y + field.size.height / 2.0;
                let text_center = bounds.origin.y + bounds.size.height / 2.0;
                assert!((field_center - text_center).abs() <= 0.5);
                assert!(bounds.origin.y >= field.origin.y);
                assert!(bounds.origin.y + bounds.size.height <= field.origin.y + height);
            }
        }
    }
}

#[test]
fn empty_editable_field_paints_its_placeholder() {
    let frame = UiFrame::layout(
        TextField::on_change_with_placeholder("", "Search associations", map_query),
        Rect::new(0.0, 0.0, 260.0, 44.0),
    );
    assert!(frame.commands().iter().any(|command| {
        matches!(command, PaintCommand::Text { text, .. } if text == "Search associations")
    }));
}

#[test]
fn masked_field_geometry_uses_concealed_glyphs_for_caret_selection_and_ime() {
    fn geometry(
        value: &str,
        preedit: Option<&str>,
        select_all: bool,
    ) -> (Rect, Option<Rect>, String, Rect) {
        let bounds = Rect::new(20.0, 30.0, 340.0, 64.0);
        let build = |state: &mut UiStateStore| {
            UiFrame::layout_with_state(
                TextField::on_change_masked_with_placeholder(value, "Password", '•', map_query)
                    .id("password")
                    .scale(18.0)
                    .single_line_height(bounds.size.height),
                bounds,
                state,
            )
        };
        let mut state = UiStateStore::default();
        let initial = build(&mut state);
        initial.handle_event(&mut state, UiEvent::FocusNext);
        if select_all {
            initial.handle_event(&mut state, UiEvent::TextSelectAll);
        }
        if let Some(preedit) = preedit {
            initial.handle_event(&mut state, UiEvent::ImePreedit(preedit.into()));
        }
        let frame = build(&mut state);
        let caret = frame
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Fill { rect, .. } if rect.size.width == 1.5 => Some(*rect),
                _ => None,
            })
            .expect("focused caret");
        let selection = frame.commands().iter().find_map(|command| match command {
            PaintCommand::Fill { rect, color } if *color == 0x315a8f => Some(*rect),
            _ => None,
        });
        let (text, text_bounds) = frame
            .commands()
            .iter()
            .find_map(|command| match command {
                PaintCommand::Text { text, bounds, .. } => Some((text.clone(), *bounds)),
                _ => None,
            })
            .expect("masked text");
        (caret, selection, text, text_bounds)
    }

    let (ascii_caret, _, ascii_text, ascii_bounds) = geometry("iiii", None, false);
    let (fallback_caret, _, fallback_text, fallback_bounds) = geometry("ＷＷＷＷ", None, false);
    assert_eq!(ascii_text, "••••");
    assert_eq!(fallback_text, ascii_text);
    assert_eq!(fallback_caret.origin.x, ascii_caret.origin.x);
    assert_eq!(fallback_caret.origin.y, ascii_caret.origin.y);

    let (_, ascii_selection, _, _) = geometry("iiii", None, true);
    let (_, fallback_selection, _, _) = geometry("ＷＷＷＷ", None, true);
    assert_eq!(fallback_selection, ascii_selection);

    let (ascii_ime_caret, _, ascii_ime_text, _) = geometry("ii", Some("ab"), false);
    let (fallback_ime_caret, _, fallback_ime_text, _) = geometry("ii", Some("世界"), false);
    assert_eq!(ascii_ime_text, "••••");
    assert_eq!(fallback_ime_text, ascii_ime_text);
    assert_eq!(fallback_ime_caret.origin.x, ascii_ime_caret.origin.x);
    assert_eq!(fallback_ime_caret.origin.y, ascii_ime_caret.origin.y);

    assert_eq!(fallback_bounds, ascii_bounds);
    assert_eq!(ascii_caret.origin.y, ascii_bounds.origin.y);
    assert_eq!(ascii_selection.unwrap().origin.y, ascii_bounds.origin.y);
}

#[test]
fn image_presentations_resolve_deterministic_bounds_and_alignment() {
    let viewport = Rect::new(10.0, 20.0, 200.0, 100.0);
    let source = Size::new(100.0, 100.0);

    assert_eq!(
        ImagePresentation::new(ImageFit::Contain).bounds(viewport, source),
        Rect::new(60.0, 20.0, 100.0, 100.0)
    );
    assert_eq!(
        ImagePresentation::new(ImageFit::Cover).bounds(viewport, source),
        Rect::new(10.0, -30.0, 200.0, 200.0)
    );
    assert_eq!(
        ImagePresentation::new(ImageFit::Stretch).bounds(viewport, source),
        viewport
    );
    assert_eq!(
        ImagePresentation::new(ImageFit::Center)
            .aligned(ImageAlignment::End, ImageAlignment::Start)
            .bounds(viewport, Size::new(40.0, 20.0)),
        Rect::new(170.0, 20.0, 40.0, 20.0)
    );
    assert_eq!(
        ImagePresentation::new(ImageFit::Span).bounds(viewport, source),
        ImagePresentation::new(ImageFit::Cover).bounds(viewport, source)
    );
    assert_eq!(
        ImagePresentation::new(ImageFit::Tile).bounds(viewport, Size::new(40.0, 20.0)),
        Rect::new(90.0, 60.0, 40.0, 20.0)
    );

    let preview = Rect::new(0.0, 0.0, 260.0, 116.0);
    for (source, expected) in [
        (Size::new(3440.0, 1440.0), Size::new(260.0, 108.84)),
        (Size::new(1920.0, 1080.0), Size::new(206.22, 116.0)),
        (Size::new(1200.0, 1200.0), Size::new(116.0, 116.0)),
        (Size::new(900.0, 1600.0), Size::new(65.25, 116.0)),
    ] {
        let bounds = ImagePresentation::new(ImageFit::Contain).bounds(preview, source);
        assert!((bounds.size.width - expected.width).abs() < 0.02);
        assert!((bounds.size.height - expected.height).abs() < 0.02);
        assert!(bounds.size.width <= preview.size.width);
        assert!(bounds.size.height <= preview.size.height);
        assert!((bounds.origin.x + bounds.size.width / 2.0 - 130.0).abs() < 0.02);
        assert!((bounds.origin.y + bounds.size.height / 2.0 - 58.0).abs() < 0.02);
    }
}

#[test]
fn preview_containment_is_scale_invariant_and_rejects_invalid_source_metadata() {
    let presentation = ImagePresentation::new(ImageFit::Contain);
    let logical_viewport = Rect::new(7.0, 11.0, 203.0, 137.0);
    let source = Size::new(1919.0, 1079.0);
    let logical = presentation.bounds(logical_viewport, source);

    for scale in [1.0_f32, 1.25, 1.5, 2.0] {
        let physical = presentation.bounds(
            Rect::new(
                logical_viewport.origin.x * scale,
                logical_viewport.origin.y * scale,
                logical_viewport.size.width * scale,
                logical_viewport.size.height * scale,
            ),
            Size::new(source.width * scale, source.height * scale),
        );
        assert!((physical.origin.x / scale - logical.origin.x).abs() < 0.01);
        assert!((physical.origin.y / scale - logical.origin.y).abs() < 0.01);
        assert!((physical.size.width / scale - logical.size.width).abs() < 0.01);
        assert!((physical.size.height / scale - logical.size.height).abs() < 0.01);
    }

    for invalid in [
        Size::new(0.0, 10.0),
        Size::new(10.0, 0.0),
        Size::new(f32::NAN, 10.0),
        Size::new(10.0, f32::INFINITY),
    ] {
        assert_eq!(
            presentation.bounds(logical_viewport, invalid).size,
            Size::new(0.0, 0.0)
        );
    }
}

#[test]
fn tile_repeats_within_the_real_clip_and_high_density_is_renderer_owned() {
    let source = Arc::new(RgbaImage::new(20, 20));
    let two_x = Arc::new(RgbaImage::new(40, 40));
    let tree = UiFrame::<TestMessage>::layout(
        Image::new(9, source.clone())
            .high_density(two_x.clone())
            .fit(ImageFit::Tile)
            .width(50.0)
            .height(50.0),
        Rect::new(0.0, 0.0, 50.0, 50.0),
    );
    let tiles = tree
        .commands()
        .iter()
        .filter(|command| matches!(command, PaintCommand::Image { id: 9, .. }))
        .collect::<Vec<_>>();
    assert_eq!(tiles.len(), 9);
    assert!(tiles.iter().all(|command| matches!(
        command,
        PaintCommand::Image { image, high_density: Some(high_density), .. }
            if Arc::ptr_eq(image, &source) && Arc::ptr_eq(high_density, &two_x)
    )));
    assert!(
        matches!(tree.commands().first(), Some(PaintCommand::PushClip(rect)) if *rect == Rect::new(0.0, 0.0, 50.0, 50.0))
    );
    assert!(matches!(
        tree.commands().last(),
        Some(PaintCommand::PopClip)
    ));
}

#[test]
fn cover_image_is_cropped_by_its_allocated_viewport() {
    let image = Arc::new(RgbaImage::new(100, 100));
    let tree = UiFrame::<TestMessage>::layout(
        Image::new(7, image)
            .width(200.0)
            .height(100.0)
            .fit(ImageFit::Cover),
        Rect::new(10.0, 20.0, 200.0, 100.0),
    );

    assert!(matches!(
        tree.commands(),
        [
            PaintCommand::PushClip(clip),
            PaintCommand::Image { bounds, id: 7, .. },
            PaintCommand::PopClip
        ] if *clip == Rect::new(10.0, 20.0, 200.0, 100.0)
            && *bounds == Rect::new(10.0, -30.0, 200.0, 200.0)
    ));
}

#[test]
fn nested_button_is_laid_out_and_hit_tested() {
    let tree = UiFrame::layout(
        Column::new()
            .height(100.0)
            .child(Header::new("Steam"))
            .child(Button::new(TestMessage::Named("launch"), "Launch").width(100.0)),
        Rect::new(0.0, 0.0, 300.0, 100.0),
    );
    assert_eq!(
        tree.message_at(Point { x: 10.0, y: 40.0 }),
        Some(&TestMessage::Named("launch"))
    );
    assert!(
        tree.commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::Text { text, .. } if text == "Steam"))
    );
}

#[test]
fn fixed_grid_gives_each_child_a_distinct_hit_region() {
    let tree = UiFrame::layout(
        Grid::fixed(2).children([
            Button::new(TestMessage::Named("one"), "One"),
            Button::new(TestMessage::Named("two"), "Two"),
            Button::new(TestMessage::Named("three"), "Three"),
        ]),
        Rect::new(0.0, 0.0, 200.0, 100.0),
    );
    let messages = [
        TestMessage::Named("one"),
        TestMessage::Named("two"),
        TestMessage::Named("three"),
    ];
    let regions = messages
        .iter()
        .map(|message| {
            let rect = tree
                .semantic_targets_for_message(message)
                .into_iter()
                .next()
                .expect("every grid child should expose its action bounds")
                .bounds;
            let center = Point {
                x: rect.origin.x + rect.size.width * 0.5,
                y: rect.origin.y + rect.size.height * 0.5,
            };
            assert_eq!(tree.message_at(center), Some(message));
            (message, rect)
        })
        .collect::<Vec<_>>();

    for (index, (_, first)) in regions.iter().enumerate() {
        for (_, second) in regions.iter().skip(index + 1) {
            let separated = first.origin.x + first.size.width <= second.origin.x
                || second.origin.x + second.size.width <= first.origin.x
                || first.origin.y + first.size.height <= second.origin.y
                || second.origin.y + second.size.height <= first.origin.y;
            assert!(
                separated,
                "grid hit regions overlap: {first:?} and {second:?}"
            );
        }
    }
}

#[test]
fn horizontal_flex_remeasures_wrapped_child_at_its_resolved_width() {
    let prose = "Lorem ipsum dolor sit amet consectetur adipiscing elit deserunt fugiat. \
            Et omnis cillum fugiat sint illum esse fugiat. Minus fuga aut dolor quos cupidatat atque.";
    let tree = UiFrame::<()>::layout(
        Row::new()
            .fill_width()
            .justify_content(Justify::Center)
            .child(
                Container::new()
                    .fill_width()
                    .max_width(900.0)
                    .padding(Insets::all(28.0))
                    .child(StyledText::new(prose, Vec::new()).scale(2.0).wrap(true)),
            ),
        Rect::new(0.0, 0.0, 960.0, 720.0),
    );
    let bounds = tree
        .commands()
        .iter()
        .find_map(|command| match command {
            PaintCommand::StyledText { bounds, .. } => Some(*bounds),
            _ => None,
        })
        .expect("wrapped prose command");
    assert!(
        bounds.size.height >= 41.5,
        "wrapped prose was allocated only {:?}",
        bounds.size
    );
}

#[test]
fn file_grid_tiles_expose_actions_and_centered_labels() {
    let icon = Arc::new(RgbaImage::new(16, 16));
    let tree = UiFrame::layout(
        FileGrid::columns(2).gap(8.0).height(120.0).items([
            FileGridItem::new(TestMessage::Named("file:one"), "One", 1, icon.clone())
                .colors(0x101010, 0x202020, 0xffffff)
                .icon_size(48.0),
            FileGridItem::new(TestMessage::Named("file:two"), "Two", 2, icon)
                .colors(0x101010, 0x202020, 0xffffff),
        ]),
        Rect::new(0.0, 0.0, 240.0, 120.0),
    );
    assert_eq!(
        tree.message_at(Point { x: 20.0, y: 20.0 }),
        Some(&TestMessage::Named("file:one"))
    );
    assert_eq!(
        tree.message_at(Point { x: 180.0, y: 20.0 }),
        Some(&TestMessage::Named("file:two"))
    );
    assert!(tree.commands.iter().any(|command| {
        matches!(
            command,
            PaintCommand::Text {
                text,
                align: TextAlign::Center,
                ..
            } if text == "One"
        )
    }));
    assert!(tree.commands.iter().any(
            |command| matches!(command, PaintCommand::Image { bounds, id: 1, .. } if bounds.size.height == 48.0)
        ));
}

#[test]
fn file_plane_item_contains_long_multiline_labels() {
    let icon = Arc::new(RgbaImage::new(16, 16));
    let tree = UiFrame::layout(
        Container::<TestMessage>::new()
            .width(96.0)
            .height(104.0)
            .child(
                FilePlaneItem::new(
                    TestMessage::Named("long-file"),
                    "monitors_receipt.pdf\nRobert Half - Security Engineer.txt",
                    1,
                    icon,
                )
                .icon_size(48.0)
                .label_height(36.0),
            ),
        Rect::new(0.0, 0.0, 96.0, 104.0),
    );
    let text = tree
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::Text { bounds, .. } => Some(*bounds),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert!(!text.is_empty());
    assert!(text.iter().all(|bounds| {
        bounds.origin.x >= 0.0
            && bounds.origin.y >= 0.0
            && bounds.origin.x + bounds.size.width <= 96.0
            && bounds.origin.y + bounds.size.height <= 104.0
    }));
    assert!(
        tree.commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::PushClip(_)))
    );
    assert!(tree.accessibility_nodes().iter().any(|node| {
        node.label.as_deref() == Some("monitors_receipt.pdf\nRobert Half - Security Engineer.txt")
    }));
}

#[test]
fn file_plane_item_owns_hit_context_semantics_and_state_visuals() {
    let icon = Arc::new(RgbaImage::new(16, 16));
    let build = |selected| {
        UiFrame::layout(
            FilePlaneItem::new(TestMessage::Named("open"), "Document", 8, icon.clone())
                .id("shared-file-item")
                .width(96.0)
                .height(104.0)
                .context_message(TestMessage::Named("context"))
                .semantic_role(SemanticRole::GridCell)
                .selected_background(selected, 0x334455)
                .interaction_backgrounds(0x223344, 0x445566)
                .focus_background_tint(0x556677)
                .controller_focus_background_tint(0x667788),
            Rect::new(0.0, 0.0, 96.0, 104.0),
        )
    };
    let idle = build(false);
    assert_eq!(
        idle.message_at(Point { x: 48.0, y: 52.0 }),
        Some(&TestMessage::Named("open"))
    );
    let node = idle
        .accessibility_nodes()
        .iter()
        .find(|node| node.id.as_str().ends_with("shared-file-item"))
        .expect("shared item semantic node");
    assert_eq!(node.semantic_role, Some(SemanticRole::GridCell));
    assert_eq!(node.label.as_deref(), Some("Document"));
    assert!(node.actions.contains(&ActionKind::Activate));
    assert!(node.actions.contains(&ActionKind::ContextMenu));
    assert!(!idle.commands().iter().any(|command| matches!(
        command,
        PaintCommand::Fill {
            color: 0x334455,
            ..
        }
    )));

    let mut state = UiStateStore::default();
    assert_eq!(
        idle.handle_event(
            &mut state,
            UiEvent::PointerContext(Point { x: 48.0, y: 52.0 })
        )
        .messages,
        vec![TestMessage::Named("context")]
    );
    assert!(build(true).commands().iter().any(|command| matches!(
        command,
        PaintCommand::Fill {
            color: 0x334455,
            ..
        }
    )));
}

#[test]
fn slider_reports_horizontal_fraction() {
    let tree = UiFrame::layout(
        Slider::new(TestMessage::Named("volume"), 0.5).width(200.0),
        Rect::new(0.0, 0.0, 200.0, 24.0),
    );
    let (message, fraction) = tree
        .message_at_with_horizontal_fraction(Point { x: 150.0, y: 12.0 })
        .expect("slider hit");
    assert_eq!(message, &TestMessage::Named("volume"));
    assert!((fraction - 0.75).abs() < 0.001);
    assert_eq!(
        tree.horizontal_fraction_for_message(&TestMessage::Named("volume"), 250.0),
        Some(1.0)
    );
}

#[test]
fn value_control_emits_typed_payload_and_component_messages_map() {
    fn volume_message(fraction: f32) -> TestMessage {
        TestMessage::Volume((fraction * 100.0).round() as u8)
    }

    let mapped = Button::new(2_usize, "Second")
        .into_element()
        .map_message(TestMessage::Option);
    let tree = UiFrame::layout(
        Column::new()
            .child(mapped)
            .child(Slider::on_change(volume_message, 0.5).width(200.0)),
        Rect::new(0.0, 0.0, 200.0, 66.0),
    );

    assert_eq!(
        tree.message_at_owned(Point { x: 20.0, y: 10.0 }),
        Some(TestMessage::Option(2))
    );
    assert_eq!(
        tree.message_at_owned(Point { x: 150.0, y: 54.0 }),
        Some(TestMessage::Volume(75))
    );
}

#[test]
fn slider_drag_emits_continuous_mapped_values_until_release() {
    fn volume_message(fraction: f32) -> TestMessage {
        TestMessage::Volume((fraction * 100.0).round() as u8)
    }

    let tree = UiFrame::layout(
        Slider::on_change(volume_message, 0.25)
            .id("volume")
            .width(200.0),
        Rect::new(0.0, 0.0, 200.0, 24.0),
    );
    let mut state = UiStateStore::default();
    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 50.0, y: 12.0 }),
    );
    let dragged = tree.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point { x: 180.0, y: 12.0 }),
    );
    assert_eq!(dragged.messages, vec![TestMessage::Volume(90)]);
    assert!(state.captured().is_some());

    tree.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point { x: 180.0, y: 12.0 }),
    );
    assert!(state.captured().is_none());
}

#[test]
fn vertical_scroll_clips_painting_and_hit_regions() {
    let tree = UiFrame::layout(
        VerticalScroll::new(TestMessage::Named("scroll"), 50.0).child(
            Column::<TestMessage>::new()
                .gap(10.0)
                .child(
                    Container::new()
                        .height(60.0)
                        .message(TestMessage::Named("one")),
                )
                .child(
                    Container::new()
                        .height(60.0)
                        .message(TestMessage::Named("two")),
                )
                .child(
                    Container::new()
                        .height(60.0)
                        .message(TestMessage::Named("three")),
                ),
        ),
        Rect::new(0.0, 0.0, 200.0, 100.0),
    );

    assert!(matches!(
        tree.commands.first(),
        Some(PaintCommand::PushClip(rect))
            if *rect == Rect::new(0.0, 0.0, 200.0 - SCROLLBAR_GUTTER, 100.0)
    ));
    assert!(
        tree.commands
            .iter()
            .any(|command| matches!(command, PaintCommand::PopClip))
    );
    assert!(tree.commands.iter().any(|command| matches!(
        command,
        PaintCommand::RoundedFill { rect, .. }
            if rect.origin.x == 200.0 - SCROLLBAR_THICKNESS - SCROLLBAR_INSET
                && rect.size.width == SCROLLBAR_THICKNESS
    )));
    assert_eq!(
        tree.scroll_extent(&TestMessage::Named("scroll")),
        Some(ScrollExtent {
            viewport: Size::new(200.0, 100.0),
            content: Size::new(200.0 - SCROLLBAR_GUTTER, 200.0),
            offset_x: 0.0,
            offset: 50.0,
        })
    );
    assert_eq!(
        tree.message_at(Point { x: 20.0, y: 5.0 }),
        Some(&TestMessage::Named("one"))
    );
    assert_eq!(
        tree.message_at(Point { x: 20.0, y: 40.0 }),
        Some(&TestMessage::Named("two"))
    );
    assert_eq!(
        tree.message_at(Point { x: 20.0, y: 95.0 }),
        Some(&TestMessage::Named("three"))
    );
    assert_eq!(tree.message_at(Point { x: 20.0, y: 105.0 }), None);
    assert!(tree.accessibility_nodes().iter().all(|node| {
        node.rect.origin.y >= 0.0 && node.rect.origin.y + node.rect.size.height <= 100.0
    }));
}

#[test]
fn vertical_scroll_clamps_offset_to_content_end() {
    let tree = UiFrame::layout(
        VerticalScroll::new(TestMessage::Named("scroll"), 500.0).child(
            Column::new()
                .gap(10.0)
                .child(
                    Container::new()
                        .height(60.0)
                        .message(TestMessage::Named("one")),
                )
                .child(
                    Container::new()
                        .height(60.0)
                        .message(TestMessage::Named("two")),
                ),
        ),
        Rect::new(0.0, 0.0, 200.0, 100.0),
    );

    assert_eq!(
        tree.scroll_extent(&TestMessage::Named("scroll"))
            .expect("scroll extent")
            .offset,
        30.0
    );
}

#[test]
fn visible_scrollbar_thumb_drags_and_releases_shared_scroll_state() {
    let mut state = UiStateStore::default();
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
                .on_scroll(|_| TestMessage::Named("dragged"))
                .child(Spacer::vertical(240.0)),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            state,
        )
    };
    let tree = build(&mut state);
    let pressed_at = Point { x: 194.0, y: 20.0 };

    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 20.0, y: 20.0 }),
    );
    let ordinary_release = tree.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point { x: 20.0, y: 20.0 }),
    );
    assert!(ordinary_release.messages.is_empty());

    let pressed = tree.handle_event(&mut state, UiEvent::PointerPressed(pressed_at));
    assert!(pressed.messages.is_empty());
    let pressed_offset = state
        .state(&UiId::from("root"))
        .expect("scroll state after press")
        .scroll_offset;
    assert!(state.captured().is_some());

    let rebuilt = build(&mut state);
    assert!(state.captured().is_some());
    let dragged = rebuilt.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point { x: 194.0, y: 90.0 }),
    );
    assert_eq!(dragged.messages, vec![TestMessage::Named("dragged")]);
    assert!(
        state
            .state(&UiId::from("root"))
            .expect("scroll state after drag")
            .scroll_offset
            > pressed_offset
    );

    rebuilt.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point { x: 194.0, y: 20.0 }),
    );
    assert_eq!(
        state
            .state(&UiId::from("root"))
            .expect("same-frame reverse drag")
            .scroll_offset,
        0.0
    );
    rebuilt.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point { x: 194.0, y: 90.0 }),
    );
    assert!(
        state
            .state(&UiId::from("root"))
            .expect("same-frame forward drag")
            .scroll_offset
            > 100.0
    );

    rebuilt.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point { x: 194.0, y: 90.0 }),
    );
    assert!(state.captured().is_none());
}

#[test]
fn scrollbar_hit_target_is_wider_than_its_visible_thumb() {
    let tree = UiFrame::<()>::layout(
        VerticalScroll::new((), 0.0).child(Spacer::vertical(240.0)),
        Rect::new(0.0, 0.0, 200.0, 100.0),
    );
    let scroll = tree.scrolls.first().expect("vertical scroll region");
    let (track, _) =
        scrollbar_geometry(scroll, ScrollbarAxis::Vertical).expect("visible scrollbar geometry");
    let hit = scrollbar_hit_rect(scroll, ScrollbarAxis::Vertical)
        .expect("scrollbar interaction geometry");

    assert_eq!(track.size.width, SCROLLBAR_THICKNESS);
    assert_eq!(hit.size.width, SCROLLBAR_HIT_THICKNESS);
    assert!(hit.origin.x < track.origin.x);
    assert!(
        tree.scrollbar_at(Point {
            x: hit.origin.x + 1.0,
            y: 50.0,
        })
        .is_some()
    );
}

#[test]
fn scrollbar_chrome_uses_the_injected_live_semantic_theme() {
    let dark = crate::SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
        0x101114, 0x15171b, 0x1b1e23, 0x32363e, 0x3c414b, 0xf2f3f5, 0xa8abb2, 0x9b62e8, 0x45305f,
        0x55b982, 0x55b982,
    ));
    let light = crate::SemanticTheme::from_tokens(crate::SemanticTokenSet::standard(
        0xf4f5f7, 0xe7e9ed, 0xffffff, 0xdfe3e8, 0xd4d9e0, 0x17191d, 0x555b66, 0x7440bd, 0xe5d8f7,
        0x207a4b, 0x207a4b,
    ));
    let build = |theme| {
        UiFrame::<()>::layout(
            VerticalScroll::new((), 0.0)
                .theme(theme)
                .child(Spacer::vertical(240.0)),
            Rect::new(0.0, 0.0, 200.0, 100.0),
        )
    };

    let dark_frame = build(dark);
    let light_frame = build(light);
    for (frame, expected) in [
        (&dark_frame, dark.scrollbar_palette().idle),
        (&light_frame, light.scrollbar_palette().idle),
    ] {
        assert!(frame.commands().iter().any(|command| matches!(
            command,
            PaintCommand::RoundedFill { color, .. } if *color == expected.track
        )));
        assert!(frame.commands().iter().any(|command| matches!(
            command,
            PaintCommand::RoundedFill { color, .. } if *color == expected.thumb
        )));
    }
    assert_ne!(
        dark.scrollbar_palette(),
        light.scrollbar_palette(),
        "theme changes must invalidate every scrollbar state palette"
    );
}

#[test]
fn scrollbar_thumb_uses_the_full_acquisition_width() {
    let mut state = UiStateStore::default();
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
                .on_scroll(|_| TestMessage::Named("dragged"))
                .child(Spacer::vertical(400.0)),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            state,
        )
    };
    let tree = build(&mut state);
    let scroll = tree.scrolls.first().expect("vertical scroll region");
    let (_, thumb) = scrollbar_geometry(scroll, ScrollbarAxis::Vertical).unwrap();
    let point = Point {
        x: scroll.rect.origin.x + scroll.rect.size.width - SCROLLBAR_HIT_THICKNESS + 1.0,
        y: thumb.origin.y + 4.0,
    };

    let pressed = tree.handle_event(&mut state, UiEvent::PointerPressed(point));
    assert!(
        pressed.messages.is_empty(),
        "thumb acquisition must not page"
    );
    assert!(state.captured().is_some(), "wide thumb target must drag");
}

#[test]
fn scrollbar_chrome_reflects_hover_press_and_keyboard_focus() {
    let root = || VerticalScroll::new((), 0.0).child(Spacer::vertical(400.0));
    let bounds = Rect::new(0.0, 0.0, 200.0, 100.0);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(root(), bounds, &mut state);
    let scroll = tree.scrolls.first().unwrap();
    let id = scrollbar_id(&scroll.id, ScrollbarAxis::Vertical);
    state.set_hovered(Some(id.clone()));
    let hovered = UiFrame::layout_with_state(root(), bounds, &mut state);
    assert!(hovered.commands().iter().any(|command| matches!(
        command,
        PaintCommand::RoundedFill { color, .. }
            if *color == crate::theme::FALLBACK_SCROLLBAR_PALETTE.hovered.thumb
    )));

    state.set_pressed(Some(id));
    let pressed = UiFrame::layout_with_state(root(), bounds, &mut state);
    assert!(pressed.commands().iter().any(|command| matches!(
        command,
        PaintCommand::RoundedFill { color, .. }
            if *color == crate::theme::FALLBACK_SCROLLBAR_PALETTE.pressed.thumb
    )));

    state.set_pressed(None);
    state.set_hovered(None);
    state.set_focus(Some(UiId::from("root")));
    let focused = UiFrame::layout_with_state(root(), bounds, &mut state);
    assert!(focused.commands().iter().any(|command| matches!(
        command,
        PaintCommand::RoundedFill { color, .. }
            if *color == crate::theme::FALLBACK_SCROLLBAR_PALETTE.focused.thumb
    )));
}

#[test]
fn scroll_viewport_exposes_and_performs_accessible_scroll_actions() {
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
            .on_scroll(|_| TestMessage::Named("changed"))
            .child(Spacer::vertical(400.0)),
        Rect::new(0.0, 0.0, 200.0, 100.0),
        &mut state,
    );
    let node = tree
        .semantic_nodes()
        .into_iter()
        .find(|node| node.id == UiId::from("root"))
        .expect("scrollbar semantic node");
    assert_eq!(node.role, Some(SemanticRole::ScrollBar));
    assert!(node.actions.contains(&ActionKind::Increment));
    assert!(!node.actions.contains(&ActionKind::Decrement));
    assert!(node.actions.contains(&ActionKind::Scroll));

    let outcome = tree
        .transition(
            &mut state,
            InputSource::Accessibility,
            InteractionIntent::Invoke {
                target: UiId::from("root"),
                action: SemanticAction::Invoke(ActionKind::Increment),
            },
        )
        .unwrap();
    assert_eq!(outcome.messages, vec![TestMessage::Named("changed")]);
    assert_eq!(
        state.state(&UiId::from("root")).unwrap().scroll_offset,
        80.0
    );
    let rebuilt = UiFrame::layout_with_state(
        VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
            .on_scroll(|_| TestMessage::Named("changed"))
            .child(Spacer::vertical(400.0)),
        Rect::new(0.0, 0.0, 200.0, 100.0),
        &mut state,
    );
    assert!(
        rebuilt
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id == UiId::from("root"))
            .unwrap()
            .actions
            .contains(&ActionKind::Decrement)
    );
}

#[test]
fn controller_can_select_and_adjust_a_scroll_viewport() {
    let mut state = UiStateStore::default();
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
                .on_scroll(|_| TestMessage::Named("changed"))
                .child(Spacer::vertical(400.0)),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            state,
        )
    };
    let tree = build(&mut state);

    tree.handle_event(&mut state, UiEvent::ControllerNext);
    assert_eq!(
        state.navigation().controller_selected(),
        Some(&UiId::from("root"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    assert!(state.navigation().controller_editing());

    let outcome = tree.handle_event(&mut state, UiEvent::ControllerAdjust(1.0));
    assert_eq!(outcome.messages, vec![TestMessage::Named("changed")]);
    assert_eq!(
        state.state(&UiId::from("root")).unwrap().scroll_offset,
        80.0
    );
}

#[test]
fn controller_adjusts_selected_slider_instead_of_ancestor_scroll() {
    let mut state = UiStateStore::default();
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
                .on_scroll(|_| TestMessage::Named("scrolled"))
                .child(
                    Column::new()
                        .child(
                            Slider::on_change(map_volume, 0.5)
                                .id("slider")
                                .accessibility_label("Value"),
                        )
                        .child(Spacer::vertical(400.0)),
                ),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            state,
        )
    };
    let tree = build(&mut state);

    tree.handle_event(&mut state, UiEvent::ControllerNext);
    tree.handle_event(&mut state, UiEvent::ControllerNext);
    assert_eq!(
        state.navigation().controller_selected(),
        Some(&UiId::from("root/#0/slider"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerActivate);

    let outcome = tree.handle_event(&mut state, UiEvent::ControllerAdjust(-1.0));
    assert_eq!(outcome.messages, vec![TestMessage::Volume(45)]);
    assert_eq!(state.state(&UiId::from("root")).unwrap().scroll_offset, 0.0);
}

#[test]
fn page_keys_scroll_the_focused_viewport_through_shared_state() {
    let mut state = UiStateStore::default();
    state.set_focus(Some(UiId::from("root")));
    let tree = UiFrame::layout_with_state(
        VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
            .on_scroll(|_| TestMessage::Named("changed"))
            .child(Spacer::vertical(400.0)),
        Rect::new(0.0, 0.0, 200.0, 100.0),
        &mut state,
    );

    let outcome = tree.handle_event(&mut state, UiEvent::KeyboardNavigatePageDown);
    assert_eq!(outcome.messages, vec![TestMessage::Named("changed")]);
    assert_eq!(
        state.state(&UiId::from("root")).unwrap().scroll_offset,
        80.0
    );
}

#[test]
fn scrollbar_track_click_pages_without_starting_a_drag() {
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
            .on_scroll(|_| TestMessage::Named("paged"))
            .child(Spacer::vertical(400.0)),
        Rect::new(0.0, 0.0, 200.0, 100.0),
        &mut state,
    );

    let outcome = tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 181.0, y: 75.0 }),
    );

    assert_eq!(outcome.messages, vec![TestMessage::Named("paged")]);
    assert_eq!(
        state
            .state(&UiId::from("root"))
            .expect("scroll state after paging")
            .scroll_offset,
        100.0
    );
    assert!(state.captured().is_none());
}

#[test]
fn scrollbar_drag_preserves_the_pointer_offset_inside_the_thumb() {
    let mut state = UiStateStore::default();
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            VerticalScroll::new((), 0.0).child(Spacer::vertical(400.0)),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            state,
        )
    };
    let tree = build(&mut state);
    let scroll = tree.scrolls.first().expect("vertical scroll region");
    let (_, thumb) =
        scrollbar_geometry(scroll, ScrollbarAxis::Vertical).expect("visible scrollbar geometry");
    let near_thumb_start = Point {
        x: 194.0,
        y: thumb.origin.y + 2.0,
    };

    tree.handle_event(&mut state, UiEvent::PointerPressed(near_thumb_start));
    let rebuilt = build(&mut state);
    rebuilt.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point {
            x: 194.0,
            y: near_thumb_start.y + 20.0,
        }),
    );

    let offset = state
        .state(&UiId::from("root"))
        .expect("scroll state after drag")
        .scroll_offset;
    let track_travel = scrollbar_geometry(scroll, ScrollbarAxis::Vertical)
        .map(|(track, thumb)| track.size.height - thumb.size.height)
        .expect("scrollbar travel");
    let expected = 20.0 / track_travel * 300.0;
    assert!((offset - expected).abs() < 0.01, "{offset} != {expected}");
}

#[test]
fn controlled_vertical_scroll_keeps_the_supplied_offset() {
    let mut state = UiStateStore::default();
    state.touch(UiId::from("root/reader")).scroll_offset = 17.0;
    let tree = UiFrame::layout_with_state(
        VerticalScroll::new(TestMessage::Named("scroll"), 80.0)
            .id("reader")
            .controlled(true)
            .height(50.0)
            .child(Container::new().height(200.0)),
        Rect::new(0.0, 0.0, 100.0, 50.0),
        &mut state,
    );
    assert_eq!(
        tree.scroll_extent(&TestMessage::Named("scroll"))
            .expect("scroll extent")
            .offset,
        80.0
    );
}

#[test]
fn expanded_dropdown_exposes_option_actions() {
    let tree = UiFrame::layout(
        Dropdown::new(
            TestMessage::Named("audio"),
            "Speakers",
            [
                ("Speakers", TestMessage::Option(0)),
                ("Headphones", TestMessage::Option(1)),
            ],
        )
        .expanded(true),
        Rect::new(0.0, 0.0, 240.0, 114.0),
    );
    assert_eq!(
        tree.message_at(Point { x: 20.0, y: 96.0 }),
        Some(&TestMessage::Option(1))
    );
}

#[test]
fn semantic_dropdown_activation_opens_and_selects_an_option() {
    let mut state = UiStateStore::default();
    let choice = UiId::from("root/choice");
    let view = |expanded| {
        Dropdown::new(
            TestMessage::Named("toggle"),
            "First",
            [
                ("First", TestMessage::Option(0)),
                ("Second", TestMessage::Option(1)),
            ],
        )
        .id("choice")
        .overlay(true)
        .expanded(expanded)
    };
    let bounds = Rect::new(0.0, 0.0, 240.0, 180.0);
    let tree = UiFrame::layout_with_state(view(false), bounds, &mut state);
    let opened = tree
        .transition(
            &mut state,
            InputSource::Accessibility,
            InteractionIntent::Invoke {
                target: choice.clone(),
                action: SemanticAction::Invoke(ActionKind::Activate),
            },
        )
        .unwrap();
    assert_eq!(opened.messages, vec![TestMessage::Named("toggle")]);
    assert!(
        state
            .state(&choice)
            .is_some_and(|entry| entry.dropdown_open)
    );

    let tree = UiFrame::layout_with_state(view(true), bounds, &mut state);
    let selected = tree
        .transition(
            &mut state,
            InputSource::Accessibility,
            InteractionIntent::Invoke {
                target: choice.scoped("option-1"),
                action: SemanticAction::Invoke(ActionKind::Activate),
            },
        )
        .unwrap();
    assert_eq!(selected.messages, vec![TestMessage::Option(1)]);
    assert!(
        !state
            .state(&choice)
            .is_some_and(|entry| entry.dropdown_open)
    );
}

#[test]
fn overlay_dropdown_flips_its_complete_option_list_inside_the_viewport() {
    let mut state = UiStateStore::default();
    state.set_dropdown_open(UiId::from("root/policy"), true);
    let tree = UiFrame::layout_with_state(
        Column::new().child(Container::new().height(150.0)).child(
            Dropdown::new(
                TestMessage::Named("policy"),
                "Current",
                (0..4).map(|index| (format!("Policy {index}"), TestMessage::Option(index))),
            )
            .id("policy")
            .overlay(true),
        ),
        Rect::new(0.0, 0.0, 240.0, 180.0),
        &mut state,
    );

    let bounds = (0..4)
        .map(|index| tree.semantic_targets_for_message(&TestMessage::Option(index))[0].bounds)
        .collect::<Vec<_>>();
    assert!(
        bounds
            .windows(2)
            .all(|pair| pair[0].origin.y < pair[1].origin.y)
    );
    assert!(bounds.iter().all(|bounds| bounds.origin.y >= 0.0));
    assert!(
        bounds
            .iter()
            .all(|bounds| bounds.origin.y + bounds.size.height <= 180.0)
    );
    assert!(bounds.last().unwrap().origin.y <= 150.0);
}

#[test]
fn window_focus_loss_closes_an_open_dropdown() {
    let mut state = UiStateStore::default();
    let dropdown = || {
        Dropdown::new(
            TestMessage::Named("toggle"),
            "Speakers",
            [
                ("Speakers", TestMessage::Option(0)),
                ("Headphones", TestMessage::Option(1)),
            ],
        )
        .id("audio")
    };
    let mut tree =
        UiFrame::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 240.0, 114.0), &mut state);

    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 20.0, y: 20.0 }),
    );
    tree.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point { x: 20.0, y: 20.0 }),
    );
    tree = UiFrame::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 240.0, 114.0), &mut state);
    assert!(tree.message_at(Point { x: 20.0, y: 96.0 }).is_some());

    tree.handle_event(&mut state, UiEvent::FocusLost);
    assert!(
        !state
            .state(&UiId::from("root/audio"))
            .is_some_and(|entry| entry.dropdown_open)
    );
}

#[test]
fn choice_menu_internal_focus_is_not_blur_but_external_focus_dismisses() {
    let mut state = UiStateStore::default();
    let view = || {
        Column::new()
            .child(
                Dropdown::new(
                    TestMessage::Named("toggle"),
                    "Model",
                    [("First", TestMessage::Named("first"))],
                )
                .id("choice")
                .accessibility_label("Model"),
            )
            .child(
                Container::new()
                    .id("outside")
                    .message(TestMessage::Named("outside"))
                    .accessibility_label("Outside"),
            )
    };
    let bounds = Rect::new(0.0, 0.0, 240.0, 140.0);
    let _tree = UiFrame::layout_with_state(view(), bounds, &mut state);
    let choice = UiId::from("root/choice");
    state.set_dropdown_open(choice.clone(), true);
    let tree = UiFrame::layout_with_state(view(), bounds, &mut state);
    let option = choice.scoped("option-0");

    tree.transition(
        &mut state,
        InputSource::Accessibility,
        InteractionIntent::Event(UiEvent::AccessibilityFocus(option)),
    )
    .unwrap();
    assert!(state.state(&choice).unwrap().dropdown_open);

    tree.transition(
        &mut state,
        InputSource::Accessibility,
        InteractionIntent::Event(UiEvent::AccessibilityFocus(UiId::from("root/outside"))),
    )
    .unwrap();
    assert!(!state.state(&choice).unwrap().dropdown_open);
}

#[test]
fn controller_activation_opens_dropdown_and_enters_its_options() {
    let mut state = UiStateStore::default();
    let dropdown = || {
        Dropdown::new(
            TestMessage::Named("toggle"),
            "Speakers",
            [
                ("Speakers", TestMessage::Option(0)),
                ("Headphones", TestMessage::Option(1)),
            ],
        )
        .id("audio")
    };
    let tree =
        UiFrame::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 240.0, 114.0), &mut state);

    tree.handle_event(&mut state, UiEvent::ControllerDown);
    let activated = tree.handle_event(&mut state, UiEvent::ControllerActivate);

    assert_eq!(activated.messages, [TestMessage::Named("toggle")]);
    assert_eq!(
        state.navigation().controller_scope(),
        Some(&UiId::from("root/audio"))
    );
    assert!(
        state
            .state(&UiId::from("root/audio"))
            .is_some_and(|entry| entry.dropdown_open)
    );
    assert_eq!(
        state.navigation().controller_selected(),
        Some(&UiId::from("root/audio/option-0"))
    );

    let rebuilt =
        UiFrame::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 240.0, 114.0), &mut state);
    assert!(
        rebuilt
            .message_for_id(&UiId::from("root/audio/option-1"))
            .is_some()
    );
}

#[test]
fn sidebar_folder_separates_toggle_and_open_actions() {
    let tree = UiFrame::layout(
        Sidebar::new(220.0)
            .child(HorizontalRule::new(0x808080))
            .child(SidebarFolder::new(
                TestMessage::Named("toggle"),
                TestMessage::Named("open"),
                "Desktop",
                false,
                0xffffff,
            )),
        Rect::new(0.0, 0.0, 220.0, 100.0),
    );
    assert_eq!(
        tree.message_at(Point { x: 10.0, y: 32.0 }),
        Some(&TestMessage::Named("toggle"))
    );
    assert_eq!(
        tree.message_at(Point { x: 80.0, y: 32.0 }),
        Some(&TestMessage::Named("open"))
    );
    assert!(tree.commands().iter().any(
        |command| matches!(command, PaintCommand::Fill { rect, .. } if rect.size.height == 1.0)
    ));
}

#[test]
fn top_corner_radius_emits_rounded_fill() {
    let tree = UiFrame::<()>::layout(
        Container::new()
            .width(120.0)
            .height(32.0)
            .background(0xf7f7f5)
            .top_corner_radius(7.0),
        Rect::new(0.0, 0.0, 120.0, 32.0),
    );
    assert!(tree.commands().iter().any(|command| matches!(
        command,
        PaintCommand::TopRoundedFill { radius, .. } if *radius == 7.0
    )));
}

#[test]
fn rounded_solid_surface_keeps_its_border_on_the_same_curve() {
    let tree = UiFrame::<()>::layout(
        Container::new()
            .width(120.0)
            .height(48.0)
            .background(0x101114)
            .border(0x8b5cf6, 2.0)
            .radius(8.0),
        Rect::new(0.0, 0.0, 120.0, 48.0),
    );
    let rounded = tree
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::RoundedFill {
                rect,
                color,
                radius,
            } => Some((*rect, *color, *radius)),
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(rounded.len(), 2);
    assert_eq!(rounded[0].1, 0x8b5cf6);
    assert_eq!(rounded[0].2, 8.0);
    assert_eq!(rounded[1].0, Rect::new(2.0, 2.0, 116.0, 44.0));
    assert_eq!(rounded[1].1, 0x101114);
    assert_eq!(rounded[1].2, 6.0);
    assert!(
        tree.commands()
            .iter()
            .all(|command| !matches!(command, PaintCommand::Stroke { .. }))
    );
}

#[test]
fn slider_thumb_uses_concentric_round_fills_without_a_square_stroke() {
    let tree = UiFrame::<()>::layout(
        Slider::new((), 0.5)
            .colors(0x202630, 0xe93662, 0x202630)
            .thumb_border(0x88a0a8b2)
            .width(200.0),
        Rect::new(0.0, 0.0, 200.0, 24.0),
    );
    let circles = tree
        .commands()
        .iter()
        .filter_map(|command| match command {
            PaintCommand::RoundedFill { rect, radius, .. }
                if (*radius - rect.size.width / 2.0).abs() < f32::EPSILON
                    && (rect.size.width - rect.size.height).abs() < f32::EPSILON =>
            {
                Some((*rect, *radius))
            }
            _ => None,
        })
        .collect::<Vec<_>>();
    assert_eq!(circles.len(), 2);
    assert_eq!(circles[0], (Rect::new(90.0, 2.0, 20.0, 20.0), 10.0));
    assert_eq!(circles[1], (Rect::new(91.0, 3.0, 18.0, 18.0), 9.0));
    assert!(
        tree.commands()
            .iter()
            .all(|command| !matches!(command, PaintCommand::Stroke { .. }))
    );
}

#[test]
fn intrinsic_measurement_covers_empty_and_nested_flex_content() {
    let empty = Container::<()>::new()
        .padding(Insets::symmetric(8.0, 5.0))
        .into_element();
    assert_eq!(
        empty.measure(Constraints::unbounded()),
        Size::new(16.0, 10.0)
    );

    let nested = Column::<()>::new()
        .gap(4.0)
        .child(Container::new().width(30.0).height(10.0))
        .child(Container::new().width(50.0).height(20.0))
        .into_element();
    assert_eq!(
        nested.measure(Constraints::unbounded()),
        Size::new(50.0, 34.0)
    );

    let row = Row::<()>::new()
        .gap(5.0)
        .padding(Insets::all(2.0))
        .child(Container::new().width(20.0).height(8.0))
        .child(Container::new().width(30.0).height(12.0))
        .into_element();
    assert_eq!(row.measure(Constraints::unbounded()), Size::new(59.0, 16.0));

    let empty_grid = Grid::<()>::new().padding(Insets::all(3.0)).into_element();
    assert_eq!(
        empty_grid.measure(Constraints::unbounded()),
        Size::new(6.0, 6.0)
    );
}

#[test]
fn max_content_measurement_reserves_the_proximity_animation_envelope() {
    let row = Row::<()>::new()
        .children([
            Container::new().width(40.0).height(40.0),
            Container::new().width(40.0).height(40.0),
            Container::new().width(40.0).height(40.0),
        ])
        .into_element()
        .proximity_magnification(ProximityMagnification {
            maximum_scale: 1.5,
            radius: 1,
        })
        .width_length(Length::MaxContent);

    assert_eq!(
        UiFrame::preferred_size(row, Size::new(500.0, 500.0)),
        Size::new(160.0, 60.0)
    );
}

#[test]
fn multiline_button_grows_from_one_line_and_stops_at_its_line_limit() {
    let constraints = Constraints::loose(Size::new(180.0, f32::INFINITY));
    let short = Button::new((), "Short title")
        .max_lines(2)
        .into_element()
        .measure(constraints);
    let wrapped = Button::new((), "A task title long enough to wrap onto another line")
        .max_lines(2)
        .into_element()
        .measure(constraints);
    let much_longer = Button::new(
        (),
        "A task title long enough to wrap onto many more lines than the component permits",
    )
    .max_lines(2)
    .into_element()
    .measure(constraints);

    assert_eq!(short.height, 42.0);
    assert!(wrapped.height > short.height);
    assert_eq!(much_longer.height, wrapped.height);

    let label = "A task title that wraps onto its visible second line";
    let tree = UiFrame::layout(
        Button::new((), label)
            .max_lines(2)
            .label_align(TextAlign::Start)
            .background(0x202630)
            .radius(10.0),
        Rect::new(0.0, 0.0, 180.0, 80.0),
    );
    assert!(tree.commands().iter().any(|command| matches!(
        command,
        PaintCommand::Text { text, align: TextAlign::Start, .. } if text == label
    )));
    assert!(tree.commands().iter().any(|command| matches!(
        command,
        PaintCommand::RoundedFill { radius, .. } if *radius == 10.0
    )));
}

#[test]
fn shaped_text_wraps_and_respects_line_height_and_max_lines() {
    let single = Text::<()>::new("The quick brown fox jumps over the lazy dog")
        .scale(2.0)
        .into_element()
        .measure(Constraints::loose(Size::new(500.0, f32::INFINITY)));
    let wrapped = Text::<()>::new("The quick brown fox jumps over the lazy dog")
        .scale(2.0)
        .wrap(true)
        .line_height(24.0)
        .max_lines(2)
        .into_element()
        .measure(Constraints::loose(Size::new(110.0, f32::INFINITY)));
    assert!(wrapped.width <= 110.0);
    assert!(wrapped.height > single.height);
    assert_eq!(wrapped.height, 48.0);

    let min_content = Text::<()>::new("short extraordinarily-long-word")
        .width_length(Length::MinContent)
        .into_element()
        .measure(Constraints::unbounded());
    let max_content = Text::<()>::new("short extraordinarily-long-word")
        .width_length(Length::MaxContent)
        .into_element()
        .measure(Constraints::unbounded());
    assert!(min_content.width < max_content.width);
}

#[test]
fn repeated_styled_text_measurement_reuses_a_bounded_cache_entry() {
    TEXT_MEASURER.with(|measurer| {
        let mut measurer = measurer.borrow_mut();
        measurer.styled.clear();
        measurer.styled_bytes = 0;
    });
    let spans = vec![StyledTextSpan {
        range: 0..6,
        bold: true,
        italic: false,
        monospace: false,
        font_family: None,
        strikethrough: false,
        underline: TextUnderlineStyle::None,
        color: None,
        background: None,
    }];

    let first = measure_styled_text("Cached text", &spans, 1.0, true, None, 240.0);
    let second = measure_styled_text("Cached text", &spans, 1.0, true, None, 240.0);

    assert_eq!(first, second);
    TEXT_MEASURER.with(|measurer| {
        let measurer = measurer.borrow();
        assert_eq!(measurer.styled.len(), 1);
        assert!(measurer.styled_bytes <= TEXT_MEASURE_CACHE_BYTE_BUDGET);
    });
}

#[test]
fn styled_shaping_reuses_span_aware_clusters_and_matches_a_cold_oracle() {
    let text = "office e\u{301} שלום 🦀";
    let spans = vec![
        StyledTextSpan {
            range: 0..6,
            bold: true,
            italic: false,
            monospace: false,
            font_family: None,
            strikethrough: false,
            underline: TextUnderlineStyle::None,
            color: None,
            background: None,
        },
        StyledTextSpan {
            range: 7..8,
            bold: false,
            italic: true,
            monospace: false,
            font_family: None,
            strikethrough: false,
            underline: TextUnderlineStyle::None,
            color: None,
            background: None,
        },
        StyledTextSpan {
            range: 11..19,
            bold: false,
            italic: true,
            monospace: true,
            font_family: None,
            strikethrough: false,
            underline: TextUnderlineStyle::Single,
            color: Some(0xff00ff),
            background: None,
        },
    ];
    let cached = with_text_measure_cache_mode(TextMeasureCacheMode::Enabled, || {
        let first = shape_styled_text(text, &spans, 1.0, true, Some(19.0), 96.0);
        let second = shape_styled_text(text, &spans, 1.0, true, Some(19.0), 96.0);
        assert!(Arc::ptr_eq(&first, &second));
        first
    });
    let cold = with_text_measure_cache_mode(TextMeasureCacheMode::BypassDerived, || {
        shape_styled_text(text, &spans, 1.0, true, Some(19.0), 96.0)
    });
    let geometry = |layout: &PlainTextLayout| {
        layout
            .clusters
            .iter()
            .map(|cluster| {
                (
                    cluster.line,
                    cluster.start,
                    cluster.end,
                    cluster.x.to_bits(),
                    cluster.y.to_bits(),
                    cluster.width.to_bits(),
                    cluster.height.to_bits(),
                    cluster.rtl,
                )
            })
            .collect::<Vec<_>>()
    };

    assert_eq!(cached.size, cold.size);
    assert_eq!(cached.line_widths, cold.line_widths);
    assert_eq!(geometry(&cached), geometry(&cold));
    for line in &cached.visual_lines {
        assert!(line.carets.windows(2).all(|pair| pair[0].x <= pair[1].x));
    }
    let mut grapheme_boundaries = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    grapheme_boundaries.push(text.len());
    assert!(cached.clusters.iter().all(|cluster| {
        grapheme_boundaries.contains(&cluster.start) && grapheme_boundaries.contains(&cluster.end)
    }));
    assert_eq!(
        cached.clusters.iter().map(|cluster| cluster.end).max(),
        Some(text.len())
    );
}

#[test]
fn styled_selection_geometry_comes_from_the_cached_span_aware_layout() {
    let text = "wide office שלום";
    let spans = vec![StyledTextSpan {
        range: 0..11,
        bold: true,
        italic: true,
        monospace: true,
        font_family: None,
        strikethrough: false,
        underline: TextUnderlineStyle::None,
        color: None,
        background: None,
    }];
    let frame = UiFrame::<TestMessage>::layout(
        SelectionRegion::automatic().child(
            StyledText::new(text, spans.clone())
                .scale(1.0)
                .selection_run_id("styled"),
        ),
        Rect::new(0.0, 0.0, 240.0, 80.0),
    );
    let actual = &frame.selection_regions[0].runs[0].glyphs;
    let shaped = shape_styled_text(text, &spans, 1.0, false, None, f32::INFINITY);

    assert_eq!(actual.len(), shaped.clusters.len());
    for (glyph, cluster) in actual.iter().zip(&shaped.clusters) {
        assert_eq!(
            (glyph.start, glyph.end, glyph.rtl),
            (cluster.start, cluster.end, cluster.rtl)
        );
        assert_eq!(glyph.rect.origin.x.to_bits(), cluster.x.to_bits());
        assert_eq!(glyph.rect.origin.y.to_bits(), cluster.y.to_bits());
        assert_eq!(glyph.rect.size.width.to_bits(), cluster.width.to_bits());
        assert_eq!(glyph.rect.size.height.to_bits(), cluster.height.to_bits());
    }
}

#[test]
fn unstyled_rich_text_reuses_the_plain_shaped_layout() {
    with_text_measure_cache_mode(TextMeasureCacheMode::Enabled, || {
        let rich = measure_styled_text("shared clusters", &[], 1.0, false, None, f32::INFINITY);
        let plain = measure_text(
            "shared clusters",
            1.0,
            false,
            false,
            None,
            None,
            f32::INFINITY,
        );
        assert_eq!(rich, plain);
        TEXT_MEASURER.with(|measurer| {
            let measurer = measurer.borrow();
            assert_eq!(measurer.plain.len(), 1);
            assert!(measurer.styled.is_empty());
        });
    });
}

#[test]
fn shaped_plain_text_reuses_clusters_and_matches_a_cold_unicode_layout() {
    let text = "office e\u{301} שלום 🦀";
    let cached = with_text_measure_cache_mode(TextMeasureCacheMode::Enabled, || {
        let first = shape_plain_text(
            text,
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Public,
        );
        let second = shape_plain_text(
            text,
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Public,
        );
        assert!(Arc::ptr_eq(&first, &second));
        first
    });
    let cold = with_text_measure_cache_mode(TextMeasureCacheMode::BypassDerived, || {
        shape_plain_text(
            text,
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Public,
        )
    });

    assert_eq!(cached.size, cold.size);
    let positions = |layout: &PlainTextLayout| {
        layout
            .clusters
            .iter()
            .map(|cluster| {
                (
                    cluster.line,
                    cluster.start,
                    cluster.end,
                    cluster.x.to_bits(),
                    cluster.y.to_bits(),
                    cluster.width.to_bits(),
                    cluster.height.to_bits(),
                    cluster.rtl,
                )
            })
            .collect::<Vec<_>>()
    };
    assert_eq!(positions(&cached), positions(&cold));
    let mut grapheme_boundaries = text
        .grapheme_indices(true)
        .map(|(index, _)| index)
        .collect::<Vec<_>>();
    grapheme_boundaries.push(text.len());
    assert!(cached.clusters.iter().all(|cluster| {
        grapheme_boundaries.contains(&cluster.start) && grapheme_boundaries.contains(&cluster.end)
    }));
}

#[test]
fn cached_selection_clusters_match_cold_wrapped_bidi_geometry() {
    let build = || {
        UiFrame::<TestMessage>::layout(
            SelectionRegion::automatic().child(
                Text::new("Latin e\u{301} — שלום — 🦀 wrapped text")
                    .selection_run_id("mixed")
                    .wrap(true)
                    .width(110.0),
            ),
            Rect::new(0.0, 0.0, 160.0, 120.0),
        )
    };
    let geometry = |frame: &UiFrame<TestMessage>| {
        frame.selection_regions[0].runs[0]
            .glyphs
            .iter()
            .map(|glyph| {
                (
                    glyph.start,
                    glyph.end,
                    glyph.rect.origin.x.to_bits(),
                    glyph.rect.origin.y.to_bits(),
                    glyph.rect.size.width.to_bits(),
                    glyph.rect.size.height.to_bits(),
                    glyph.rtl,
                )
            })
            .collect::<Vec<_>>()
    };
    let cached = with_text_measure_cache_mode(TextMeasureCacheMode::Enabled, build);
    let cold = with_text_measure_cache_mode(TextMeasureCacheMode::BypassDerived, build);

    assert_eq!(geometry(&cached), geometry(&cold));
    assert_eq!(cached.resolved_layout(), cold.resolved_layout());
    assert_eq!(cached.commands(), cold.commands());
}

#[test]
fn shaped_plain_text_cache_is_bounded_and_can_exclude_protected_text() {
    with_text_measure_cache_mode(TextMeasureCacheMode::Enabled, || {
        TEXT_MEASURER.with(|measurer| {
            let mut measurer = measurer.borrow_mut();
            measurer.plain.clear();
            measurer.plain_bytes = 0;
            measurer.diagnostics = TextLayoutCacheDiagnostics::default();
        });
        let protected = "correct horse battery staple";
        let _ = shape_plain_text(
            protected,
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Protected,
        );
        TEXT_MEASURER.with(|measurer| assert!(measurer.borrow().plain.is_empty()));

        for index in 0..=TEXT_MEASURE_CACHE_CAPACITY {
            let text = format!("entry-{index}");
            let _ = shape_plain_text(
                &text,
                1.0,
                false,
                false,
                None,
                Some(1),
                f32::INFINITY,
                TextRetention::Public,
            );
        }
        let _ = shape_plain_text(
            &format!("entry-{}", TEXT_MEASURE_CACHE_CAPACITY),
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Public,
        );
        TEXT_MEASURER.with(|measurer| {
            let measurer = measurer.borrow();
            assert!(measurer.plain.len() <= TEXT_MEASURE_CACHE_CAPACITY);
            assert!(measurer.plain_bytes <= TEXT_MEASURE_CACHE_BYTE_BUDGET);
            assert!(measurer.plain.keys().all(|key| key.text != protected));
        });
        let diagnostics = text_layout_cache_diagnostics();
        assert_eq!(diagnostics.hits, 1);
        assert!(diagnostics.misses > TEXT_MEASURE_CACHE_CAPACITY as u64);
        assert!(diagnostics.evictions >= 1);
        assert!(diagnostics.rejections >= 1);
    });
}

#[test]
fn visual_caret_index_matches_a_linear_unicode_oracle() {
    for text in [
        "office affinity",
        "e\u{301}cole",
        "👩🏽‍💻🦀 family 👨‍👩‍👧‍👦",
        "abc שלום 123 العربية",
    ] {
        let layout = with_text_measure_cache_mode(TextMeasureCacheMode::BypassDerived, || {
            shape_plain_text(
                text,
                1.0,
                false,
                false,
                None,
                Some(1),
                f32::INFINITY,
                TextRetention::Public,
            )
        });
        let carets = &layout.visual_lines[0].carets;
        assert!(carets.windows(2).all(|pair| pair[0].x <= pair[1].x));
        let expected_boundaries = text
            .grapheme_indices(true)
            .map(|(offset, _)| offset)
            .chain(std::iter::once(text.len()))
            .collect::<Vec<_>>();
        assert!(
            expected_boundaries
                .iter()
                .all(|expected| { carets.iter().any(|caret| caret.offset == *expected) })
        );

        let cold = |x: f32| {
            let after = carets.iter().position(|caret| caret.x >= x);
            match after {
                Some(0) => carets.first().map(|caret| caret.offset),
                Some(after) => {
                    let previous = carets[after - 1];
                    let next = carets[after];
                    Some(if x - previous.x <= next.x - x {
                        previous.offset
                    } else {
                        next.offset
                    })
                }
                None => carets.last().map(|caret| caret.offset),
            }
        };
        let width = layout.line_widths[0].ceil() as usize;
        for sample in 0..=width.saturating_mul(4) {
            let x = sample as f32 / 4.0;
            assert_eq!(layout.offset_at_x(0, x), cold(x), "{text:?} at {x}");
        }
    }
}

#[test]
fn layout_keys_isolate_masking_scale_and_font_generation() {
    with_text_measure_cache_mode(TextMeasureCacheMode::Enabled, || {
        TEXT_MEASURER.with(|measurer| {
            let mut measurer = measurer.borrow_mut();
            measurer.diagnostics = TextLayoutCacheDiagnostics::default();
        });
        let public = shape_plain_text(
            "••••",
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Public,
        );
        let masked = shape_plain_text(
            "••••",
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Masked('•'),
        );
        let scaled = shape_plain_text(
            "••••",
            2.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Masked('•'),
        );
        assert!(!Arc::ptr_eq(&public, &masked));
        assert!(!Arc::ptr_eq(&masked, &scaled));

        let before = text_layout_cache_diagnostics();
        ProcessFontSystem::new().invalidate_resolution();
        let regenerated = shape_plain_text(
            "••••",
            2.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Masked('•'),
        );
        let after = text_layout_cache_diagnostics();
        assert!(!Arc::ptr_eq(&scaled, &regenerated));
        assert_eq!(
            after.generation_invalidations,
            before.generation_invalidations + 1
        );
        assert_eq!(after.entries, 1);
    });
}

#[test]
fn protected_layouts_are_rejected_without_retaining_plaintext() {
    with_text_measure_cache_mode(TextMeasureCacheMode::Enabled, || {
        TEXT_MEASURER.with(|measurer| {
            let mut measurer = measurer.borrow_mut();
            measurer.diagnostics = TextLayoutCacheDiagnostics::default();
        });
        let secret = "hunter2 👩🏽‍💻";
        let first = shape_plain_text(
            secret,
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Protected,
        );
        let second = shape_plain_text(
            secret,
            1.0,
            false,
            false,
            None,
            Some(1),
            f32::INFINITY,
            TextRetention::Protected,
        );
        assert!(!Arc::ptr_eq(&first, &second));
        let diagnostics = text_layout_cache_diagnostics();
        assert_eq!(diagnostics.entries, 0);
        assert_eq!(diagnostics.retained_bytes, 0);
        assert_eq!(diagnostics.rejections, 2);
        TEXT_MEASURER.with(|measurer| {
            assert!(measurer.borrow().plain.keys().all(|key| key.text != secret));
        });
    });
}

#[test]
#[ignore = "release-mode visual caret lookup workload"]
fn visual_caret_index_release_workload_is_materially_faster_than_linear_lookup() {
    use std::time::Instant;

    let text = "office שלום 👩🏽‍💻 ".repeat(512);
    let layout = shape_plain_text(
        &text,
        1.0,
        false,
        false,
        None,
        Some(1),
        f32::INFINITY,
        TextRetention::Public,
    );
    let carets = &layout.visual_lines[0].carets;
    let width = layout.line_widths[0].max(1.0);
    let samples = (0..20_000)
        .map(|sample| (sample as f32 * 17.0) % width)
        .collect::<Vec<_>>();

    let indexed_started = Instant::now();
    let mut indexed_checksum = 0_usize;
    for x in &samples {
        indexed_checksum ^= std::hint::black_box(layout.offset_at_x(0, *x).unwrap_or(0));
    }
    let indexed = indexed_started.elapsed();

    let linear_started = Instant::now();
    let mut linear_checksum = 0_usize;
    for x in &samples {
        let after = carets.iter().position(|caret| caret.x >= *x);
        let offset = match after {
            Some(0) => carets[0].offset,
            Some(after) => {
                let previous = carets[after - 1];
                let next = carets[after];
                if *x - previous.x <= next.x - *x {
                    previous.offset
                } else {
                    next.offset
                }
            }
            None => carets.last().map_or(0, |caret| caret.offset),
        };
        linear_checksum ^= std::hint::black_box(offset);
    }
    let linear = linear_started.elapsed();

    assert_eq!(indexed_checksum, linear_checksum);
    eprintln!(
        "visual caret lookup: carets={} samples={} indexed={indexed:?} linear={linear:?}",
        carets.len(),
        samples.len()
    );
    assert!(
        indexed.as_nanos().saturating_mul(4) < linear.as_nanos(),
        "indexed lookup must be at least 4x faster: indexed={indexed:?}, linear={linear:?}"
    );
}

#[test]
#[ignore = "release-mode cache admission benchmark"]
fn text_measure_caches_have_measured_equivalent_benefit() {
    use std::time::Instant;

    fn p95(mut samples: Vec<f64>) -> f64 {
        samples.sort_by(f64::total_cmp);
        samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
    }

    let text = "Nickel declarative text measurement wraps consistently across repeated frames";
    let spans = vec![StyledTextSpan {
        range: 0..6,
        bold: true,
        italic: false,
        monospace: false,
        font_family: None,
        strikethrough: false,
        underline: TextUnderlineStyle::None,
        color: None,
        background: None,
    }];
    let mut cached_plain = Vec::new();
    let mut bypass_plain = Vec::new();
    let mut cached_styled = Vec::new();
    let mut bypass_styled = Vec::new();
    for _ in 0..100 {
        TEXT_MEASURER.with(|measurer| measurer.borrow_mut().cache_enabled = true);
        let expected_plain = measure_text(text, 1.0, false, true, None, None, 320.0);
        let started = Instant::now();
        assert_eq!(
            measure_text(text, 1.0, false, true, None, None, 320.0),
            expected_plain
        );
        cached_plain.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        let expected_styled = measure_styled_text(text, &spans, 1.0, true, None, 320.0);
        let started = Instant::now();
        assert_eq!(
            measure_styled_text(text, &spans, 1.0, true, None, 320.0),
            expected_styled
        );
        cached_styled.push(started.elapsed().as_secs_f64() * 1_000_000.0);

        TEXT_MEASURER.with(|measurer| measurer.borrow_mut().cache_enabled = false);
        let started = Instant::now();
        assert_eq!(
            measure_text(text, 1.0, false, true, None, None, 320.0),
            expected_plain
        );
        bypass_plain.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        let started = Instant::now();
        assert_eq!(
            measure_styled_text(text, &spans, 1.0, true, None, 320.0),
            expected_styled
        );
        bypass_styled.push(started.elapsed().as_secs_f64() * 1_000_000.0);
    }
    TEXT_MEASURER.with(|measurer| measurer.borrow_mut().cache_enabled = true);
    let cached_plain = p95(cached_plain);
    let bypass_plain = p95(bypass_plain);
    let cached_styled = p95(cached_styled);
    let bypass_styled = p95(bypass_styled);
    println!(
        "plain cached_p95_us={cached_plain:.3} bypass_p95_us={bypass_plain:.3}; styled cached_p95_us={cached_styled:.3} bypass_p95_us={bypass_styled:.3}"
    );
    assert!(bypass_plain - cached_plain >= 5.0);
    assert!(bypass_styled - cached_styled >= 5.0);
}

#[test]
fn constrained_image_preserves_intrinsic_aspect_ratio() {
    let image = Arc::new(RgbaImage::new(200, 100));
    let measured = Image::<()>::new(1, image)
        .width(80.0)
        .into_element()
        .measure(Constraints::unbounded());
    assert_eq!(measured, Size::new(80.0, 40.0));
}

#[test]
fn unavailable_image_uses_bounded_fallback_and_reports_its_source() {
    let image = Arc::new(RgbaImage::new(0, 0));
    let element = Image::<()>::new(7, image).id("missing");
    assert_eq!(
        element
            .clone()
            .into_element()
            .measure(Constraints::unbounded()),
        Size::new(16.0, 16.0)
    );
    let tree = UiFrame::layout_with_diagnostics(element, Rect::new(0.0, 0.0, 32.0, 32.0));
    assert!(tree.diagnostics().iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::MissingAsset
            && diagnostic.id == UiId::from("root/missing")
    }));
}

#[test]
fn ellipsis_changes_presentation_without_entering_message_payloads() {
    let tree = UiFrame::<TestMessage>::layout(
        Text::new("A deliberately long label")
            .width(45.0)
            .ellipsis(true),
        Rect::new(0.0, 0.0, 45.0, 24.0),
    );
    assert!(
        tree.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { text, .. } if text.ends_with('…'))
        )
    );
}

#[test]
fn flex_alignment_justification_and_limits_resolve_deterministically() {
    let tree = UiFrame::layout(
        Row::new()
            .align_items(Align::Center)
            .justify_content(Justify::SpaceBetween)
            .child(
                Container::new()
                    .width(80.0)
                    .height(20.0)
                    .min_width(70.0)
                    .shrink(1.0)
                    .message(TestMessage::Named("left")),
            )
            .child(
                Container::new()
                    .width(80.0)
                    .height(10.0)
                    .min_width(20.0)
                    .shrink(1.0)
                    .message(TestMessage::Named("right")),
            ),
        Rect::new(0.0, 0.0, 120.0, 40.0),
    );
    let left = tree
        .unique_semantic_target_for_message(&TestMessage::Named("left"))
        .unwrap()
        .bounds;
    let right = tree
        .unique_semantic_target_for_message(&TestMessage::Named("right"))
        .unwrap()
        .bounds;
    assert_eq!(left.size.width, 70.0);
    assert_eq!(right.size.width, 50.0);
    assert_eq!(left.origin.y, 10.0);
    assert_eq!(right.origin.y, 15.0);
}

#[test]
fn every_justification_and_cross_axis_alignment_mode_is_finite() {
    for justify in [
        Justify::Start,
        Justify::Center,
        Justify::End,
        Justify::SpaceBetween,
        Justify::SpaceAround,
        Justify::SpaceEvenly,
    ] {
        for align in [
            Align::Start,
            Align::Center,
            Align::End,
            Align::Stretch,
            Align::Baseline,
        ] {
            let tree = UiFrame::layout(
                Row::new()
                    .justify_content(justify)
                    .align_items(align)
                    .child(
                        Button::new(TestMessage::Option(0), "A")
                            .width(30.0)
                            .height(20.0),
                    )
                    .child(
                        Button::new(TestMessage::Option(1), "B")
                            .width(30.0)
                            .height(10.0),
                    ),
                Rect::new(0.0, 0.0, 120.0, 40.0),
            );
            for message in [TestMessage::Option(0), TestMessage::Option(1)] {
                let rect = tree
                    .unique_semantic_target_for_message(&message)
                    .unwrap()
                    .bounds;
                assert!(rect.origin.x.is_finite() && rect.origin.y.is_finite());
                assert!(rect.size.width >= 0.0 && rect.size.height >= 0.0);
            }
        }
    }
}

#[test]
fn grid_resolves_fixed_auto_fractional_repeated_and_auto_fit_tracks() {
    let fixed = UiFrame::layout(
        Grid::tracks([Track::px(40.0), Track::Auto, Track::fr(1.0)]).children([
            Button::new(TestMessage::Option(0), "A"),
            Button::new(TestMessage::Option(1), "A much wider label"),
            Button::new(TestMessage::Option(2), "C"),
        ]),
        Rect::new(0.0, 0.0, 300.0, 60.0),
    );
    assert_eq!(fixed.resolved_grid_columns(), Some(3));
    let repeated = UiFrame::layout(
        Grid::tracks([Track::repeat(2, Track::fr(1.0))]).children([
            Button::new(TestMessage::Option(0), "A"),
            Button::new(TestMessage::Option(1), "B"),
        ]),
        Rect::new(0.0, 0.0, 200.0, 60.0),
    );
    assert_eq!(repeated.resolved_grid_columns(), Some(2));
    let auto_fit = UiFrame::layout(
        Grid::new()
            .columns(Track::repeat_auto_fit(Track::minmax(80.0, Track::fr(1.0))))
            .children([
                Button::new(TestMessage::Option(0), "A"),
                Button::new(TestMessage::Option(1), "B"),
                Button::new(TestMessage::Option(2), "C"),
            ]),
        Rect::new(0.0, 0.0, 250.0, 120.0),
    );
    assert_eq!(auto_fit.resolved_grid_columns(), Some(3));
}

#[test]
fn grid_uses_justification_and_item_alignment() {
    let grid = UiFrame::layout(
        Grid::tracks([Track::px(40.0), Track::px(60.0)])
            .justify_content(Justify::Center)
            .align_items(Align::Center)
            .children([
                Button::new(TestMessage::Option(0), "Short").height(20.0),
                Button::new(TestMessage::Option(1), "Tall").height(40.0),
            ]),
        Rect::new(0.0, 0.0, 200.0, 80.0),
    );
    let short = grid
        .unique_semantic_target_for_message(&TestMessage::Option(0))
        .unwrap()
        .bounds;
    let tall = grid
        .unique_semantic_target_for_message(&TestMessage::Option(1))
        .unwrap()
        .bounds;
    assert_eq!(short.origin.x, 50.0);
    assert_eq!(tall.origin.x, 90.0);
    assert_eq!(short.origin.y, 10.0);
    assert_eq!(tall.origin.y, 0.0);
}

#[test]
fn generated_valid_layouts_have_finite_nonnegative_geometry() {
    for width in [0.0, 1.0, 37.0, 400.0] {
        for height in [0.0, 1.0, 91.0, 300.0] {
            let tree = UiFrame::layout(
                Row::new()
                    .gap(3.0)
                    .child(Button::new(TestMessage::Option(0), "A").min_width(0.0))
                    .child(Button::new(TestMessage::Option(1), "B").min_width(0.0)),
                Rect::new(0.0, 0.0, width, height),
            );
            for message in [TestMessage::Option(0), TestMessage::Option(1)] {
                let rect = tree
                    .unique_semantic_target_for_message(&message)
                    .unwrap()
                    .bounds;
                assert!(rect.origin.x.is_finite() && rect.origin.y.is_finite());
                assert!(rect.size.width.is_finite() && rect.size.height.is_finite());
                assert!(rect.size.width >= 0.0 && rect.size.height >= 0.0);
            }
        }
    }
}

#[test]
fn explicit_identity_survives_sibling_insertion_and_list_reordering() {
    let first = UiFrame::layout(
        Column::new().id("list").children([
            Button::new(TestMessage::Option(1), "One").id("one"),
            Button::new(TestMessage::Option(2), "Two").id("two"),
        ]),
        Rect::new(0.0, 0.0, 200.0, 100.0),
    );
    let reordered = UiFrame::layout(
        Column::new().id("list").children([
            Button::new(TestMessage::Option(3), "New").id("new"),
            Button::new(TestMessage::Option(2), "Two").id("two"),
            Button::new(TestMessage::Option(1), "One").id("one"),
        ]),
        Rect::new(0.0, 0.0, 200.0, 120.0),
    );
    assert_eq!(
        first
            .semantic_targets_for_message(&TestMessage::Option(1))
            .into_iter()
            .map(|target| target.id)
            .collect::<Vec<_>>(),
        reordered
            .semantic_targets_for_message(&TestMessage::Option(1))
            .into_iter()
            .map(|target| target.id)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        first
            .semantic_targets_for_message(&TestMessage::Option(2))
            .into_iter()
            .map(|target| target.id)
            .collect::<Vec<_>>(),
        reordered
            .semantic_targets_for_message(&TestMessage::Option(2))
            .into_iter()
            .map(|target| target.id)
            .collect::<Vec<_>>()
    );
}

#[test]
fn pointer_keyboard_controller_and_accessibility_share_typed_activation() {
    let tree = UiFrame::layout(
        Button::new(TestMessage::Option(7), "Seven").id("seven"),
        Rect::new(0.0, 0.0, 100.0, 42.0),
    );
    let id = tree
        .semantic_targets_for_message(&TestMessage::Option(7))
        .into_iter()
        .next()
        .unwrap()
        .id;
    let mut state = UiStateStore::default();
    tree.reconcile_state(&mut state);

    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
    );
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 })
        )
        .messages,
        vec![TestMessage::Option(7)]
    );
    tree.handle_event(&mut state, UiEvent::FocusNext);
    for event in [UiEvent::KeyboardActivate, UiEvent::ControllerActivate] {
        assert_eq!(
            tree.handle_event(&mut state, event).messages,
            vec![TestMessage::Option(7)]
        );
    }
    assert_eq!(
        tree.handle_event(&mut state, UiEvent::AccessibilityActivate(id))
            .messages,
        vec![TestMessage::Option(7)]
    );

    let accessibility_id = tree
        .semantic_targets_for_message(&TestMessage::Option(7))
        .into_iter()
        .next()
        .unwrap()
        .id;
    state.set_focus(None);
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::AccessibilityFocus(accessibility_id.clone())
        )
        .invalidation,
        Invalidation::Paint
    );
    assert_eq!(state.focused(), Some(&accessibility_id));
    let mut focused_tree = tree.clone();
    focused_tree.apply_interaction_state(&state);
    assert!(
        focused_tree
            .resolved_layout()
            .nodes
            .iter()
            .any(|node| node.id == accessibility_id && node.interaction.focused)
    );

    let controller_tree = || {
        UiFrame::layout(
            Row::new().children([
                Button::new(TestMessage::Option(1), "One").id("one"),
                Button::new(TestMessage::Option(2), "Two").id("two"),
            ]),
            Rect::new(0.0, 0.0, 200.0, 42.0),
        )
    };
    let first = controller_tree();
    first.handle_event(&mut state, UiEvent::ControllerNext);
    first.handle_event(&mut state, UiEvent::ControllerNext);
    let selected = state.navigation().controller_selected().cloned();
    let rebuilt = controller_tree();
    assert_eq!(state.navigation().controller_selected(), selected.as_ref());
    assert_eq!(
        rebuilt
            .handle_event(&mut state, UiEvent::ControllerActivate)
            .messages,
        vec![TestMessage::Option(2)]
    );
}

#[test]
fn controller_confirm_cancels_pending_pointer_activation_before_release() {
    let tree = UiFrame::layout(
        Button::new(TestMessage::Option(7), "Seven").id("seven"),
        Rect::new(0.0, 0.0, 100.0, 42.0),
    );
    let mut state = UiStateStore::default();
    tree.reconcile_state(&mut state);
    tree.handle_event(&mut state, UiEvent::FocusNext);
    let point = Point { x: 10.0, y: 10.0 };

    assert!(
        tree.handle_event(&mut state, UiEvent::PointerPressed(point))
            .messages
            .is_empty()
    );
    assert_eq!(
        tree.handle_event(&mut state, UiEvent::ControllerActivate)
            .messages,
        vec![TestMessage::Option(7)]
    );
    assert!(
        tree.handle_event(&mut state, UiEvent::PointerReleased(point))
            .messages
            .is_empty()
    );
}

#[test]
fn context_only_target_supports_every_invocation_route() {
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            Container::new()
                .id("application")
                .width(180.0)
                .height(48.0)
                .context_message(TestMessage::Named("context"))
                .child(Text::new("Application")),
            Rect::new(0.0, 0.0, 180.0, 48.0),
            state,
        )
    };
    let expected = vec![TestMessage::Named("context")];

    let mut pointer_state = UiStateStore::default();
    assert_eq!(
        build(&mut pointer_state)
            .handle_event(
                &mut pointer_state,
                UiEvent::PointerContext(Point { x: 20.0, y: 20.0 })
            )
            .messages,
        expected
    );

    let mut keyboard_state = UiStateStore::default();
    let keyboard = build(&mut keyboard_state);
    keyboard.handle_event(&mut keyboard_state, UiEvent::FocusNext);
    assert_eq!(
        keyboard
            .handle_event(&mut keyboard_state, UiEvent::KeyboardContextMenu)
            .messages,
        expected
    );

    let mut controller_state = UiStateStore::default();
    let controller = build(&mut controller_state);
    controller.handle_event(&mut controller_state, UiEvent::ControllerDown);
    assert_eq!(
        controller
            .handle_event(&mut controller_state, UiEvent::ControllerContextMenu)
            .messages,
        expected
    );

    let mut accessibility_state = UiStateStore::default();
    let accessibility = build(&mut accessibility_state);
    let id = accessibility
        .accessibility_nodes()
        .iter()
        .find(|node| node.interactive)
        .expect("application accessibility action")
        .id
        .clone();
    assert_eq!(
        accessibility
            .handle_event(
                &mut accessibility_state,
                UiEvent::AccessibilityContextMenu(id)
            )
            .messages,
        expected
    );
}

#[test]
fn controller_dpad_uses_spatial_geometry_stops_at_edges_and_reports_modality() {
    let view = Column::new().children([
        Row::new().children([
            Button::new(TestMessage::Option(1), "One")
                .id("one")
                .width(100.0),
            Button::new(TestMessage::Option(2), "Two")
                .id("two")
                .width(100.0),
        ]),
        Row::new().children([
            Button::new(TestMessage::Option(3), "Three")
                .id("three")
                .width(100.0),
            Button::new(TestMessage::Option(4), "Four")
                .id("four")
                .width(100.0),
        ]),
    ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 240.0, 100.0), &mut state);

    tree.handle_event(&mut state, UiEvent::ControllerDown);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/one"))
    );
    assert_eq!(state.input_modality(), InputModality::Controller);

    tree.handle_event(&mut state, UiEvent::ControllerRight);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/two")),
        "right should select Two, got {:?}",
        state.navigation().controller_selected()
    );
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/four")),
        "down should select Four, got {:?}",
        tree.resolved_layout()
            .nodes
            .iter()
            .map(|node| (&node.id, node.allocated))
            .collect::<Vec<_>>()
    );
    tree.handle_event(&mut state, UiEvent::ControllerLeft);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/three"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerUp);
    let top_left = state.navigation().controller_selected().cloned();
    tree.handle_event(&mut state, UiEvent::ControllerUp);
    assert_eq!(state.navigation().controller_selected(), top_left.as_ref());

    tree.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point { x: 10.0, y: 10.0 }),
    );
    assert_eq!(state.input_modality(), InputModality::Pointer);
    assert_eq!(state.navigation().controller_selected(), top_left.as_ref());
}

#[test]
fn controller_panes_restore_their_own_last_selected_descendant() {
    let view = Row::new().children([
        Container::new()
            .id("sidebar")
            .navigation_scope(crate::NavigationScope::pane(false))
            .children([
                Button::new(TestMessage::Option(1), "Home").id("home"),
                Button::new(TestMessage::Option(2), "Applications").id("applications"),
            ]),
        Container::new()
            .id("content")
            .navigation_scope(crate::NavigationScope::pane(true))
            .children([
                Button::new(TestMessage::Option(3), "Files").id("files"),
                Button::new(TestMessage::Option(4), "Terminal").id("terminal"),
            ]),
    ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 400.0, 120.0), &mut state);

    tree.handle_event(&mut state, UiEvent::ControllerDown);
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/terminal"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerPreviousPane);
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/applications"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerNextPane);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/terminal"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerPreviousPane);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/applications"))
    );
}

#[test]
fn declared_directional_neighbor_precedes_spatial_candidate() {
    let view = Container::new()
        .id("scope")
        .navigation_scope(crate::NavigationScope::group().neighbor(
            crate::NavigationDirection::Down,
            UiId::from("root/scope/third"),
        ))
        .children([
            Button::new(TestMessage::Option(1), "First").id("first"),
            Button::new(TestMessage::Option(2), "Second").id("second"),
            Button::new(TestMessage::Option(3), "Third").id("third"),
        ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 200.0, 160.0), &mut state);

    tree.handle_event(&mut state, UiEvent::ControllerDown);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/first"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/third"))
    );
}

#[test]
fn missing_declared_neighbor_uses_deterministic_spatial_fallback() {
    let view = Container::new()
        .id("scope")
        .navigation_scope(crate::NavigationScope::group().neighbor(
            crate::NavigationDirection::Down,
            UiId::from("root/scope/missing"),
        ))
        .children([
            Button::new(TestMessage::Option(1), "First").id("first"),
            Button::new(TestMessage::Option(2), "Second").id("second"),
        ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 200.0, 120.0), &mut state);

    tree.handle_event(&mut state, UiEvent::ControllerDown);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/second"))
    );
}

fn selected_suffix(state: &UiStateStore, suffix: &str) -> bool {
    state
        .navigation()
        .controller_selected()
        .is_some_and(|id| id.as_str().ends_with(suffix))
}

#[test]
fn navigation_scope_entry_first_last_and_target_use_production_targets() {
    for (entry, expected) in [
        (crate::NavigationEntry::First, "/first"),
        (crate::NavigationEntry::Last, "/third"),
        (
            crate::NavigationEntry::Target(UiId::from("root/scope/second")),
            "/second",
        ),
    ] {
        let view = Container::new()
            .id("scope")
            .navigation_scope(crate::NavigationScope::group().entry(entry))
            .children([
                Button::new(TestMessage::Option(1), "First").id("first"),
                Button::new(TestMessage::Option(2), "Second").id("second"),
                Button::new(TestMessage::Option(3), "Third").id("third"),
            ]);
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 240.0, 160.0), &mut state);
        tree.handle_event(&mut state, UiEvent::ControllerDown);
        tree.handle_event(&mut state, UiEvent::ControllerActivate);
        assert!(selected_suffix(&state, expected), "entry {expected:?}");
    }
}

#[test]
fn navigation_scope_exit_parent_contain_and_dismiss_are_distinct() {
    for (exit, expected_scope, expected_selected) in [
        (crate::NavigationExit::Parent, None, Some("/scope")),
        (
            crate::NavigationExit::Contain,
            Some("root/scope"),
            Some("/item"),
        ),
        (crate::NavigationExit::Dismiss, None, None),
    ] {
        let view = Container::new()
            .id("scope")
            .navigation_scope(crate::NavigationScope::group().exit(exit))
            .child(Button::new(TestMessage::Option(1), "Item").id("item"));
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 160.0, 80.0), &mut state);
        tree.handle_event(&mut state, UiEvent::ControllerDown);
        tree.handle_event(&mut state, UiEvent::ControllerActivate);
        tree.handle_event(&mut state, UiEvent::ControllerBack);
        assert_eq!(
            state.navigation().controller_scope().map(UiId::as_str),
            expected_scope
        );
        assert_eq!(
            state
                .navigation()
                .controller_selected()
                .map(|id| expected_selected.is_some_and(|suffix| id.as_str().ends_with(suffix))),
            expected_selected.map(|_| true)
        );
    }
}

#[test]
fn exiting_layout_only_scope_does_not_focus_its_container() {
    let view = Container::new()
        .id("pane")
        .navigation_scope(crate::NavigationScope::pane(true))
        .child(
            Container::new()
                .id("launcher")
                .navigation_scope(crate::NavigationScope::group())
                .child(Button::new(TestMessage::Option(1), "Chrome").id("chrome")),
        );
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 240.0, 120.0), &mut state);

    tree.handle_event(&mut state, UiEvent::ControllerDown);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    assert!(selected_suffix(&state, "/chrome"));

    tree.handle_event(&mut state, UiEvent::ControllerBack);
    tree.handle_event(&mut state, UiEvent::ControllerBack);
    assert_ne!(
        state.navigation().controller_selected().map(UiId::as_str),
        Some("root/pane")
    );
}

#[test]
fn left_bubbles_from_nested_waypoint_through_pane_to_previous_pane() {
    let view = Row::new().children([
        Container::new()
            .id("sidebar")
            .navigation_scope(crate::NavigationScope::pane(true))
            .child(Button::new(TestMessage::Option(1), "Home").id("home")),
        Container::new()
            .id("detail")
            .navigation_scope(crate::NavigationScope::pane(false))
            .child(
                Container::new()
                    .id("launcher")
                    .navigation_scope(crate::NavigationScope::group())
                    .semantic_role(SemanticRole::Button)
                    .accessibility_label("Application launcher")
                    .message(TestMessage::Option(2))
                    .child(Button::new(TestMessage::Option(3), "Chrome").id("chrome")),
            ),
    ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 480.0, 120.0), &mut state);

    tree.handle_event(&mut state, UiEvent::ControllerNextPane);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    assert!(selected_suffix(&state, "/chrome"));

    tree.handle_event(&mut state, UiEvent::ControllerLeft);
    assert!(selected_suffix(&state, "/launcher"));
    tree.handle_event(&mut state, UiEvent::ControllerLeft);
    assert!(selected_suffix(&state, "/home"));
    assert!(
        state
            .navigation()
            .controller_pane()
            .is_some_and(|id| id.as_str().ends_with("/sidebar"))
    );
}

#[test]
fn linear_navigation_honors_rtl_and_containment_at_edges() {
    let view = Container::new()
        .id("scope")
        .navigation_scope(
            crate::NavigationScope::group()
                .traversal(crate::NavigationTraversal::Linear)
                .direction(crate::ReadingDirection::RightToLeft)
                .exit(crate::NavigationExit::Contain),
        )
        .children([
            Button::new(TestMessage::Option(1), "First").id("first"),
            Button::new(TestMessage::Option(2), "Second").id("second"),
        ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 240.0, 80.0), &mut state);
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    tree.handle_event(&mut state, UiEvent::ControllerLeft);
    assert!(selected_suffix(&state, "/second"));
    tree.handle_event(&mut state, UiEvent::ControllerLeft);
    assert!(selected_suffix(&state, "/second"));
    tree.handle_event(&mut state, UiEvent::ControllerRight);
    assert!(selected_suffix(&state, "/first"));
}

#[test]
fn structural_next_continues_after_a_nested_scope_without_changing_dpad_containment() {
    let view = Container::new()
        .id("outer")
        .navigation_scope(crate::NavigationScope::group())
        .children([
            Container::new()
                .id("nested")
                .navigation_scope(crate::NavigationScope::group())
                .child(Button::new(TestMessage::Option(1), "Nested").id("nested-item")),
            Container::new()
                .id("after")
                .semantic_role(SemanticRole::Button)
                .accessibility_label("After")
                .message(TestMessage::Option(2)),
        ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 240.0, 120.0), &mut state);
    tree.handle_event(&mut state, UiEvent::ControllerNext);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    assert!(selected_suffix(&state, "/nested-item"));

    tree.handle_event(&mut state, UiEvent::ControllerNext);
    assert!(selected_suffix(&state, "/after"));
    assert_eq!(
        state.navigation().controller_scope().map(UiId::as_str),
        Some("root/outer")
    );
}

#[test]
fn pane_peers_are_scoped_to_their_navigation_parent() {
    let pane = |id, item, default| {
        Container::new()
            .id(id)
            .navigation_scope(crate::NavigationScope::pane(default))
            .child(Button::new(TestMessage::Named(item), item).id(item))
    };
    let view = Row::new().children([
        Container::new()
            .id("outer")
            .navigation_scope(crate::NavigationScope::group())
            .children([
                pane("left", "left-item", true),
                pane("right", "right-item", false),
            ]),
        Container::new()
            .id("other")
            .navigation_scope(crate::NavigationScope::group())
            .children([
                pane("alpha", "alpha-item", false),
                pane("beta", "beta-item", false),
            ]),
    ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 600.0, 100.0), &mut state);
    assert!(
        state
            .navigation()
            .controller_pane()
            .is_some_and(|id| id.as_str().ends_with("/outer/left"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerNextPane);
    assert!(
        state
            .navigation()
            .controller_pane()
            .is_some_and(|id| id.as_str().ends_with("/outer/right"))
    );
    tree.handle_event(&mut state, UiEvent::ControllerNextPane);
    assert!(
        state
            .navigation()
            .controller_pane()
            .is_some_and(|id| id.as_str().ends_with("/outer/right"))
    );
}

#[test]
fn retain_focus_false_and_removed_retained_target_use_declared_entry() {
    fn scope(retain: bool, include_second: bool) -> Container<TestMessage> {
        let mut scope = Container::new().id("scope").navigation_scope(
            crate::NavigationScope::group()
                .entry(crate::NavigationEntry::First)
                .retain_focus(retain),
        );
        scope = scope.child(Button::new(TestMessage::Option(1), "First").id("first"));
        if include_second {
            scope = scope.child(Button::new(TestMessage::Option(2), "Second").id("second"));
        }
        scope
    }
    for retain in [false, true] {
        let mut state = UiStateStore::default();
        let tree = UiFrame::layout_with_state(
            scope(retain, true),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            &mut state,
        );
        tree.handle_event(&mut state, UiEvent::ControllerDown);
        tree.handle_event(&mut state, UiEvent::ControllerActivate);
        tree.handle_event(&mut state, UiEvent::ControllerDown);
        assert!(selected_suffix(&state, "/second"));
        tree.handle_event(&mut state, UiEvent::ControllerBack);
        let rebuilt = UiFrame::layout_with_state(
            scope(retain, false),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            &mut state,
        );
        rebuilt.handle_event(&mut state, UiEvent::ControllerActivate);
        assert!(selected_suffix(&state, "/first"));
    }
}

#[test]
fn disabled_retained_target_falls_back_to_declared_entry() {
    let build = |second_enabled| {
        let mut second = Container::new()
            .id("second")
            .semantic_role(SemanticRole::Button)
            .accessibility_label("Second");
        if second_enabled {
            second = second.message(TestMessage::Option(2));
        }
        Container::new()
            .id("scope")
            .navigation_scope(crate::NavigationScope::group())
            .children([
                Container::new()
                    .id("first")
                    .semantic_role(SemanticRole::Button)
                    .accessibility_label("First")
                    .message(TestMessage::Option(1)),
                second,
            ])
    };
    let mut state = UiStateStore::default();
    let tree =
        UiFrame::layout_with_state(build(true), Rect::new(0.0, 0.0, 200.0, 100.0), &mut state);
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    tree.handle_event(&mut state, UiEvent::ControllerBack);
    let rebuilt =
        UiFrame::layout_with_state(build(false), Rect::new(0.0, 0.0, 200.0, 100.0), &mut state);
    rebuilt.handle_event(&mut state, UiEvent::ControllerActivate);
    assert!(selected_suffix(&state, "/first"));
}

#[test]
fn declared_scroll_owner_is_the_only_surface_revealed_for_scope_focus() {
    let view = VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
        .id("owner")
        .height(60.0)
        .child(
            Container::new()
                .id("scope")
                .navigation_scope(crate::NavigationScope::group().scroll_owner(UiId::from("owner")))
                .child(
                    Column::new().children([
                        Button::new(TestMessage::Option(1), "First")
                            .id("first")
                            .height(50.0),
                        Button::new(TestMessage::Option(2), "Second")
                            .id("second")
                            .height(50.0),
                    ]),
                ),
        );
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 200.0, 60.0), &mut state);
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    tree.handle_event(&mut state, UiEvent::ControllerActivate);
    tree.handle_event(&mut state, UiEvent::ControllerDown);
    assert!(selected_suffix(&state, "/second"));
    assert!(
        state
            .state(&UiId::from("root/owner"))
            .is_some_and(|entry| entry.scroll_offset > 0.0)
    );
}

#[test]
fn accessibility_focus_reveals_a_declared_offscreen_control_without_activation() {
    let view = || {
        VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
            .id("owner")
            .height(60.0)
            .child(Column::new().children((0..10).map(|index| {
                Button::new(TestMessage::Option(index), format!("Item {index}"))
                    .id(format!("item-{index}"))
                    .height(30.0)
            })))
    };
    let mut state = UiStateStore::default();
    let bounds = Rect::new(0.0, 0.0, 200.0, 60.0);
    let tree = UiFrame::layout_with_state(view(), bounds, &mut state);
    let target = tree.semantic_targets_for_message(&TestMessage::Option(9))[0]
        .id
        .clone();
    let outcome = tree.handle_event(&mut state, UiEvent::AccessibilityFocus(target.clone()));
    assert!(outcome.messages.is_empty());
    assert_eq!(state.focused(), Some(&target));
    assert_eq!(outcome.invalidation, Invalidation::Layout);
    let tree = UiFrame::layout_with_state(view(), bounds, &mut state);
    let rect = tree.resolved_layout().find(&target).unwrap().allocated;
    assert!(rect.origin.y >= 0.0 && rect.origin.y + rect.size.height <= 60.0);
    let offset = state
        .state(&UiId::from("root/owner"))
        .unwrap()
        .scroll_offset;
    let outcome = tree.handle_event(&mut state, UiEvent::AccessibilityFocus(target));
    assert!(outcome.messages.is_empty());
    assert_eq!(
        state
            .state(&UiId::from("root/owner"))
            .unwrap()
            .scroll_offset,
        offset
    );
}

#[test]
fn focus_reveal_does_not_scroll_a_non_ancestor_viewport() {
    let view = Column::new().children([
        VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
            .id("owner")
            .height(60.0)
            .child(Column::new().children(
                (0..10).map(|index| Button::new(TestMessage::Option(index), "Row").height(30.0)),
            ))
            .into_element(),
        Button::new(TestMessage::Named("footer"), "Footer")
            .id("footer")
            .into_element(),
    ]);
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(view, Rect::new(0.0, 0.0, 200.0, 100.0), &mut state);
    assert_eq!(tree.scrolls.len(), 1);
    let owner = tree.scrolls[0].id.clone();
    let target = tree.semantic_targets_for_message(&TestMessage::Named("footer"))[0]
        .id
        .clone();
    tree.handle_event(&mut state, UiEvent::AccessibilityFocus(target.clone()));
    assert_eq!(state.focused(), Some(&target));
    assert_eq!(
        state.state(&owner).map_or(0.0, |entry| entry.scroll_offset),
        0.0
    );
}

#[test]
fn pointer_drag_selects_visible_text_and_caret_blink_only_changes_paint() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let mut state = UiStateStore::default();
    let first = UiFrame::layout_with_state(
        TextField::on_change("select this", query).id("query"),
        Rect::new(0.0, 0.0, 200.0, 32.0),
        &mut state,
    );
    first.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 2.0, y: 10.0 }),
    );
    first.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point { x: 70.0, y: 10.0 }),
    );
    let selected = state
        .state(&UiId::from("root/query"))
        .and_then(|entry| entry.editor.as_ref())
        .and_then(TextEditor::selection);
    assert!(selected.is_some_and(|range| !range.is_empty()));

    let selected_tree = UiFrame::layout_with_state(
        TextField::on_change("select this", query).id("query"),
        Rect::new(0.0, 0.0, 200.0, 32.0),
        &mut state,
    );
    assert!(selected_tree.commands().iter().any(|command| matches!(
        command,
        PaintCommand::Fill {
            color: 0x315a8f,
            ..
        }
    )));
    assert_eq!(
        selected_tree
            .handle_event(&mut state, UiEvent::CaretBlink)
            .invalidation,
        Invalidation::Paint
    );
}

#[test]
fn transparent_text_field_keeps_keyboard_focus_without_filling_its_text_area() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let field = || TextField::on_change("typed text", query).id("query");
    let mut state = UiStateStore::default();
    let initial = UiFrame::layout_with_state(field(), Rect::new(0.0, 0.0, 240.0, 40.0), &mut state);
    initial.handle_event(&mut state, UiEvent::FocusNext);
    let focused = UiFrame::layout_with_state(field(), Rect::new(0.0, 0.0, 240.0, 40.0), &mut state);

    let fallback = crate::focused_surface(
        crate::theme::FALLBACK_FOCUS_SURFACE,
        crate::theme::FALLBACK_KEYBOARD_FOCUS_CUE,
    );
    assert!(
        !focused.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == fallback)
        )
    );
    assert_eq!(state.focused(), Some(&UiId::from("root/query")));
}

#[test]
fn focused_text_field_transforms_its_explicit_surface() {
    const SURFACE: Color = 0xe7e9ed;
    const FOREGROUND: Color = 0x17191d;
    const FOCUS: Color = 0x7040bb;

    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let field = || {
        TextField::on_change("theme input", query)
            .id("query")
            .color(FOREGROUND)
            .background(SURFACE)
            .focus_background_tint(FOCUS)
    };
    let mut state = UiStateStore::default();
    let initial = UiFrame::layout_with_state(field(), Rect::new(0.0, 0.0, 200.0, 32.0), &mut state);
    initial.handle_event(&mut state, UiEvent::FocusNext);
    let focused = UiFrame::layout_with_state(field(), Rect::new(0.0, 0.0, 200.0, 32.0), &mut state);
    let expected = crate::focused_surface_with_foreground(SURFACE, FOCUS, FOREGROUND);
    assert!(
        focused.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == expected)
        )
    );
    let fallback = crate::focused_surface_with_foreground(
        crate::theme::FALLBACK_FOCUS_SURFACE,
        FOCUS,
        FOREGROUND,
    );
    assert!(
        !focused.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == fallback)
        )
    );
}

#[test]
fn focused_text_field_uses_exact_declared_background() {
    const NORMAL: Color = 0xff253044;
    const FOCUSED: Color = 0xff356a92;
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }
    let field = || {
        TextField::on_change("query", query)
            .id("query")
            .background(NORMAL)
            .focus_background(FOCUSED)
    };
    let mut state = UiStateStore::default();
    let initial = UiFrame::layout_with_state(field(), Rect::new(0.0, 0.0, 200.0, 32.0), &mut state);
    assert!(
        initial
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::Fill { color, .. } if *color == NORMAL))
    );
    initial.handle_event(&mut state, UiEvent::FocusNext);
    let focused = UiFrame::layout_with_state(field(), Rect::new(0.0, 0.0, 200.0, 32.0), &mut state);
    assert!(
        focused.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == FOCUSED)
        )
    );
}

#[test]
fn focus_callbacks_follow_component_focus_and_window_blur() {
    let fields = || {
        Column::new()
            .child(
                TextField::on_change("first", |value| value)
                    .id("first")
                    .focus_message("focus-first".to_owned())
                    .blur_message("blur-first".to_owned()),
            )
            .child(
                TextField::on_change("second", |value| value)
                    .id("second")
                    .focus_message("focus-second".to_owned())
                    .blur_message("blur-second".to_owned()),
            )
    };
    let mut state = UiStateStore::default();
    let frame = UiFrame::layout_with_state(fields(), Rect::new(0.0, 0.0, 240.0, 80.0), &mut state);
    assert_eq!(
        frame.handle_event(&mut state, UiEvent::FocusNext).messages,
        ["focus-first"]
    );
    assert_eq!(
        frame.handle_event(&mut state, UiEvent::FocusNext).messages,
        ["blur-first", "focus-second"]
    );
    assert_eq!(
        frame.handle_event(&mut state, UiEvent::FocusLost).messages,
        ["blur-second"]
    );
    assert_eq!(
        frame
            .handle_event(&mut state, UiEvent::FocusGained)
            .messages,
        ["focus-second"]
    );
}

#[test]
fn document_selection_crosses_text_runs_and_skips_buttons() {
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            SelectionRegion::automatic().id("document").child(
                Column::new()
                    .gap(8.0)
                    .child(
                        Text::new("First")
                            .id("first")
                            .selection_boundary(TextBoundary::Block),
                    )
                    .child(Button::new(TestMessage::Named("button"), "Excluded").id("button"))
                    .child(
                        Text::new("Second")
                            .id("second")
                            .selection_boundary(TextBoundary::Block),
                    ),
            ),
            Rect::new(0.0, 0.0, 240.0, 140.0),
            state,
        )
    };

    let mut state = UiStateStore::default();
    let tree = build(&mut state);
    let first = tree
        .resolved_layout()
        .find(&UiId::from("root/document/#0/first"))
        .expect("first text")
        .allocated;
    let second = tree
        .resolved_layout()
        .find(&UiId::from("root/document/#0/second"))
        .expect("second text")
        .allocated;
    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point {
            x: first.origin.x + 1.0,
            y: first.origin.y + first.size.height * 0.5,
        }),
    );
    tree.handle_event(
        &mut state,
        UiEvent::PointerMoved(Point {
            x: second.origin.x + second.size.width - 1.0,
            y: second.origin.y + second.size.height * 0.5,
        }),
    );
    let release = tree.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point {
            x: second.origin.x + second.size.width - 1.0,
            y: second.origin.y + second.size.height * 0.5,
        }),
    );
    assert!(release.messages.is_empty());
    assert_eq!(tree.selected_text(&state).as_deref(), Some("First\nSecond"));
    tree.handle_event(&mut state, UiEvent::TextSelectAll);
    let copied = tree.handle_event(&mut state, UiEvent::TextCopy);
    assert_eq!(copied.clipboard_text.as_deref(), Some("First\nSecond"));
    tree.handle_event(
        &mut state,
        UiEvent::TextMoveLeft {
            extend_selection: true,
        },
    );
    assert_eq!(tree.selected_text(&state).as_deref(), Some("First\nSecon"));
    tree.handle_event(&mut state, UiEvent::SelectionClear);
    assert!(
        tree.handle_event(&mut state, UiEvent::TextCopy)
            .clipboard_text
            .is_none()
    );
    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point {
            x: first.origin.x + 1.0,
            y: first.origin.y + first.size.height * 0.5,
        }),
    );
    tree.handle_event(&mut state, UiEvent::TextSelectAll);
    let cut = tree.handle_event(&mut state, UiEvent::TextCut);
    assert!(cut.clipboard_text.is_none());
    assert!(cut.messages.is_empty());

    let selected = build(&mut state);
    assert!(
        selected
            .commands()
            .iter()
            .filter(|command| matches!(
                command,
                PaintCommand::Fill {
                    color: 0x315a8f,
                    ..
                }
            ))
            .count()
            >= 2
    );

    let button = selected
        .semantic_targets_for_message(&TestMessage::Named("button"))
        .into_iter()
        .next()
        .expect("button bounds")
        .bounds;
    selected.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point {
            x: button.origin.x + 2.0,
            y: button.origin.y + 2.0,
        }),
    );
    let activated = selected.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point {
            x: button.origin.x + 2.0,
            y: button.origin.y + 2.0,
        }),
    );
    assert_eq!(activated.messages, vec![TestMessage::Named("button")]);
    assert!(state.selection_owner().is_none());
}

#[test]
fn inline_message_ranges_are_clickable_and_use_the_hand_cursor() {
    let spans = vec![StyledTextSpan {
        range: 5..9,
        bold: true,
        italic: true,
        monospace: true,
        font_family: None,
        strikethrough: false,
        underline: TextUnderlineStyle::Single,
        color: Some(0x00aaff),
        background: None,
    }];
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            StyledText::new("open docs", spans.clone())
                .inline_message(5..9, TestMessage::Named("docs")),
            Rect::new(0.0, 0.0, 200.0, 40.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let tree = build(&mut state);
    let glyph = tree
        .semantic_targets_for_message(&TestMessage::Named("docs"))
        .into_iter()
        .next()
        .expect("inline message glyph")
        .bounds;
    let point = Point {
        x: glyph.origin.x + glyph.size.width / 2.0,
        y: glyph.origin.y + glyph.size.height / 2.0,
    };

    assert_eq!(tree.pointer_icon_at(point), PointerIcon::Hand);
    tree.handle_event(&mut state, UiEvent::PointerPressed(point));
    let rebuilt = build(&mut state);
    assert_eq!(
        rebuilt
            .handle_event(&mut state, UiEvent::PointerReleased(point))
            .messages,
        vec![TestMessage::Named("docs")]
    );
}

#[test]
fn selectable_line_trailing_space_resolves_to_the_line_end() {
    let tree = UiFrame::<TestMessage>::layout(
        SelectionRegion::automatic().child(
            StyledText::new("select me", Vec::new())
                .width_length(Length::Fill)
                .selection_run_id("line"),
        ),
        Rect::new(0.0, 0.0, 300.0, 40.0),
    );
    let (_, endpoint) = tree
        .selection_hit_at(Point { x: 280.0, y: 8.0 })
        .expect("trailing line whitespace remains selectable");

    assert_eq!(endpoint.run_id, "line");
    assert_eq!(endpoint.offset, "select me".len());
}

#[test]
fn document_drag_edge_autoscrolls_at_a_bounded_rate() {
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        Column::<TestMessage>::new()
            .id("scroll")
            .height(90.0)
            .overflow_y(Overflow::Auto)
            .child(
                SelectionRegion::automatic()
                    .id("document")
                    .child(Column::new().children((0..30).map(|index| {
                        Text::new(format!("Line {index}")).selection_boundary(TextBoundary::Block)
                    }))),
            ),
        Rect::new(0.0, 0.0, 240.0, 90.0),
        &mut state,
    );
    let first = tree
        .selection_regions
        .first()
        .and_then(|region| region.runs.first())
        .and_then(|run| run.glyphs.first())
        .expect("visible selectable glyph")
        .rect;
    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point {
            x: first.origin.x + 1.0,
            y: first.origin.y + 1.0,
        }),
    );
    for _ in 0..4 {
        let outcome = tree.handle_event(
            &mut state,
            UiEvent::PointerMoved(Point { x: 20.0, y: 89.0 }),
        );
        assert!(matches!(outcome.invalidation, Invalidation::Layout));
    }
    let offset = state
        .state(&UiId::from("root/scroll"))
        .expect("scroll state")
        .scroll_offset;
    assert!(offset > 0.0);
    assert!(offset <= 72.0);
}

#[test]
fn structured_diagnostics_are_bounded_deduplicated_and_attributed() {
    let tree = UiFrame::layout_with_diagnostics(
        Column::new()
            .id("fixture")
            .min_width(200.0)
            .max_width(20.0)
            .child(
                Row::new()
                    .id("overflow")
                    .overflow(Overflow::Clip, Overflow::Clip)
                    .child(
                        Button::new(TestMessage::Option(1), "wide")
                            .id("duplicate")
                            .width(180.0)
                            .min_width(180.0)
                            .shrink(0.0),
                    )
                    .child(
                        Button::new(TestMessage::Option(2), "also wide")
                            .id("duplicate")
                            .width(180.0)
                            .min_width(180.0)
                            .shrink(0.0),
                    ),
            )
            .child(
                VerticalScroll::new(TestMessage::Named("scroll"), 500.0)
                    .id("scroll")
                    .height(30.0)
                    .child(Text::new("short")),
            )
            .child(Text::new("cannot fit").id("text").width(1.0).height(1.0)),
        Rect::new(0.0, 0.0, 100.0, 80.0),
    );
    for expected in [
        DiagnosticKind::ContradictoryConstraints,
        DiagnosticKind::FlexOverflow,
        DiagnosticKind::DuplicateIdentity,
        DiagnosticKind::ClippedInteraction,
        DiagnosticKind::ScrollOffsetClamped,
        DiagnosticKind::UnsatisfiedContent,
    ] {
        assert!(
            tree.diagnostics().iter().any(|item| item.kind == expected),
            "missing {expected:?}: {:?}",
            tree.diagnostics()
        );
    }
    let unique = tree
        .diagnostics()
        .iter()
        .map(|item| (item.kind, item.id.clone()))
        .collect::<HashSet<_>>();
    assert_eq!(unique.len(), tree.diagnostics().len());
    assert!(tree.diagnostics().len() <= 128);
    assert!(
        tree.diagnostics()
            .iter()
            .all(|item| item.id.as_str().starts_with("root/fixture"))
    );
}

#[test]
fn diagnostic_only_text_measurement_is_skipped_without_changing_frame_output() {
    let view = || {
        Column::new().child(
            Text::<TestMessage>::new("cannot fit")
                .id("text")
                .wrap(false)
                .width(1.0)
                .height(1.0),
        )
    };
    let bounds = Rect::new(0.0, 0.0, 100.0, 40.0);
    let ordinary = UiFrame::layout(view(), bounds);
    let diagnostic = UiFrame::layout_with_diagnostics(view(), bounds);

    assert_eq!(
        ordinary.resource_diagnostics().diagnostic_text_measurements,
        0
    );
    assert_eq!(
        diagnostic
            .resource_diagnostics()
            .diagnostic_text_measurements,
        1
    );
    assert!(
        diagnostic
            .diagnostics()
            .iter()
            .any(|item| item.kind == DiagnosticKind::UnsatisfiedContent)
    );
    assert_eq!(ordinary.resolved_layout(), diagnostic.resolved_layout());
    assert_eq!(ordinary.commands(), diagnostic.commands());
    assert_eq!(ordinary.semantic_nodes(), diagnostic.semantic_nodes());
    assert_eq!(
        ordinary.accessibility_nodes(),
        diagnostic.accessibility_nodes()
    );
}

#[test]
fn invalid_indefinite_and_unbalanced_layout_conditions_are_reported() {
    let tree = UiFrame::layout_with_diagnostics(
        Text::<TestMessage>::new("percent").width_length(Length::percent(0.5)),
        Rect::new(0.0, 0.0, f32::INFINITY, 30.0),
    );
    assert!(
        tree.diagnostics()
            .iter()
            .any(|item| item.kind == DiagnosticKind::InvalidGeometry)
    );
    assert!(
        tree.diagnostics()
            .iter()
            .any(|item| item.kind == DiagnosticKind::IndefinitePercentage)
    );

    let mut malformed = UiFrame::<TestMessage> {
        diagnostics_enabled: true,
        ..UiFrame::default()
    };
    malformed.commands.push(PaintCommand::PopClip);
    malformed.validate_clip_commands();
    assert_eq!(
        malformed.diagnostics()[0].kind,
        DiagnosticKind::UnbalancedClip
    );
}

#[test]
fn resolved_layout_is_headless_deterministic_and_overlay_is_non_interfering() {
    let build = || {
        UiFrame::layout_with_diagnostics(
            Row::new()
                .id("toolbar")
                .padding(Insets::all(4.0))
                .child(Button::new(TestMessage::Named("save"), "Save").id("save")),
            Rect::new(0.0, 0.0, 240.0, 48.0),
        )
    };
    let mut tree = build();
    let before_snapshot = tree.resolved_layout().deterministic_snapshot();
    let before_hit = tree.message_at(Point { x: 20.0, y: 20.0 }).cloned();
    assert!(
        tree.resolved_layout()
            .find(&UiId::from("root/toolbar/save"))
            .is_some()
    );
    tree.enable_diagnostic_overlay(None);
    assert_eq!(
        before_snapshot,
        tree.resolved_layout().deterministic_snapshot()
    );
    assert_eq!(
        before_hit,
        tree.message_at(Point { x: 20.0, y: 20.0 }).cloned()
    );
    assert_eq!(
        before_snapshot,
        build().resolved_layout().deterministic_snapshot()
    );
    assert!(
        UiFrame::layout(
            Text::<TestMessage>::new("disabled"),
            Rect::new(0.0, 0.0, 10.0, 10.0)
        )
        .diagnostics()
        .is_empty()
    );
    let disabled = UiFrame::layout(
        Column::<TestMessage>::new().children((0..128).map(|index| Text::new(index.to_string()))),
        Rect::new(0.0, 0.0, 200.0, 400.0),
    );
    assert!(disabled.seen_ids.is_empty());
    assert!(disabled.diagnostic_keys.is_empty());
}

#[test]
fn diagnostic_overlay_rasterizes_at_low_and_high_dpi_without_geometry_changes() {
    let build = || {
        UiFrame::layout_with_diagnostics(
            Grid::new()
                .id("scale-fixture")
                .columns(Track::repeat_auto_fit(Track::minmax(60.0, Track::fr(1.0))))
                .children((0..6).map(|index| {
                    Button::new(TestMessage::Option(index), format!("item {index}")).id(index)
                })),
            Rect::new(0.0, 0.0, 240.0, 120.0),
        )
    };
    let baseline = build().resolved_layout().deterministic_snapshot();
    for scale in [1.0, 2.0] {
        let mut tree = build();
        tree.enable_diagnostic_overlay_with_damage(None, &[Rect::new(12.0, 8.0, 80.0, 32.0)]);
        let mut renderer = crate::gpu::SoftwareRenderer::new(
            (240.0 * scale) as u32,
            (120.0 * scale) as u32,
            scale,
        );
        assert!(!renderer.render(tree.commands()).is_empty());
        assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
        assert_eq!(baseline, tree.resolved_layout().deterministic_snapshot());
        assert_eq!(
            tree.message_at(Point { x: 30.0, y: 20.0 }),
            Some(&TestMessage::Option(0))
        );
    }
}

#[test]
fn semantic_roles_require_a_name_or_explicit_decorative_exemption() {
    let unnamed = UiFrame::layout_with_diagnostics(
        Container::new()
            .semantic_role(SemanticRole::GraphicalCustomControl)
            .message(TestMessage::Named("activate")),
        Rect::new(0.0, 0.0, 80.0, 40.0),
    );
    assert!(unnamed.semantic_nodes().is_empty());
    assert!(unnamed.accessibility_nodes().is_empty());
    assert!(
        unnamed
            .diagnostics()
            .iter()
            .any(|diagnostic| { diagnostic.kind == DiagnosticKind::MissingAccessibleName })
    );

    let named = UiFrame::layout_with_diagnostics(
        Container::new()
            .semantic_role(SemanticRole::ApplicationPresentation)
            .accessibility_label("Terminal")
            .message(TestMessage::Named("activate")),
        Rect::new(0.0, 0.0, 80.0, 40.0),
    );
    assert_eq!(named.semantic_nodes()[0].name.as_deref(), Some("Terminal"));
    assert!(named.diagnostics().is_empty());

    let decorative = UiFrame::layout_with_diagnostics(
        Container::<TestMessage>::new()
            .semantic_role(SemanticRole::GraphicalCustomControl)
            .decorative(),
        Rect::new(0.0, 0.0, 80.0, 40.0),
    );
    assert!(decorative.semantic_nodes().is_empty());
    assert!(decorative.accessibility_nodes().is_empty());
    assert!(decorative.diagnostics().is_empty());
}

#[test]
fn literal_semantic_roles_have_stable_accessibility_mappings() {
    assert_eq!(SemanticRole::Pane.as_str(), "pane");
    assert_eq!(SemanticRole::Group.as_str(), "group");
    assert_eq!(
        SemanticRole::ApplicationPresentation.as_str(),
        "application"
    );
    assert_eq!(
        SemanticRole::GraphicalCustomControl.as_str(),
        "graphics-object"
    );
}

#[test]
fn controller_selected_value_uses_child_background_without_a_focus_stroke() {
    const CONTROLLER_FOCUS: Color = 0xff4fb3;

    let mut slider_state = UiStateStore::default();
    let slider = || {
        Slider::on_change(map_volume, 0.5)
            .id("volume")
            .controller_focus_background_tint(CONTROLLER_FOCUS)
    };
    let initial_slider = UiFrame::layout_with_state(
        slider(),
        Rect::new(0.0, 0.0, 200.0, 24.0),
        &mut slider_state,
    );
    initial_slider.handle_event(&mut slider_state, UiEvent::ControllerDown);
    let selected_slider = UiFrame::layout_with_state(
        slider(),
        Rect::new(0.0, 0.0, 200.0, 24.0),
        &mut slider_state,
    );
    assert_eq!(
        slider_state.navigation().controller_selected(),
        Some(&UiId::from("root/volume"))
    );
    let expected = crate::focused_surface(crate::theme::FALLBACK_FOCUS_SURFACE, CONTROLLER_FOCUS);
    assert!(
        selected_slider.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == expected)
        )
    );
    assert!(!selected_slider.commands().iter().any(
        |command| matches!(command, PaintCommand::Stroke { color, .. } if *color == CONTROLLER_FOCUS)
    ));

    let mut dropdown_state = UiStateStore::default();
    let dropdown = || {
        Dropdown::new(
            TestMessage::Named("toggle"),
            "Off",
            [("On", TestMessage::Option(1))],
        )
        .id("animations")
        .controller_focus_background_tint(CONTROLLER_FOCUS)
    };
    let initial_dropdown = UiFrame::layout_with_state(
        dropdown(),
        Rect::new(0.0, 0.0, 180.0, 42.0),
        &mut dropdown_state,
    );
    let initial_bounds = initial_dropdown
        .semantic_targets_for_message(&TestMessage::Named("toggle"))
        .into_iter()
        .next()
        .expect("dropdown target")
        .bounds;
    initial_dropdown.handle_event(&mut dropdown_state, UiEvent::ControllerDown);
    let selected_dropdown = UiFrame::layout_with_state(
        dropdown(),
        Rect::new(0.0, 0.0, 180.0, 42.0),
        &mut dropdown_state,
    );
    assert_eq!(
        dropdown_state.navigation().controller_selected(),
        Some(&UiId::from("root/animations"))
    );
    assert_eq!(
        selected_dropdown.semantic_targets_for_message(&TestMessage::Named("toggle"))[0].bounds,
        initial_bounds,
        "focus presentation must not change layout or hit geometry"
    );
    assert!(
        selected_dropdown.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == expected)
        )
    );
    assert!(!selected_dropdown.commands().iter().any(
        |command| matches!(command, PaintCommand::Stroke { color, .. } if *color == CONTROLLER_FOCUS)
    ));
}

#[test]
fn stateful_layout_rehydrates_dropdown_scroll_focus_and_pointer_capture() {
    let mut state = UiStateStore::default();
    let dropdown = || {
        Dropdown::new(
            TestMessage::Named("toggle"),
            "One",
            [("Two", TestMessage::Option(2))],
        )
        .id("choice")
    };
    let first =
        UiFrame::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 180.0, 120.0), &mut state);
    first.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
    );
    assert!(state.captured().is_some());
    let released = first.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 }),
    );
    assert_eq!(released.messages, vec![TestMessage::Named("toggle")]);
    assert_eq!(released.invalidation, Invalidation::Layout);
    assert!(state.captured().is_none());
    assert!(state.focused().is_some());
    let rebuilt =
        UiFrame::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 180.0, 120.0), &mut state);
    assert!(
        rebuilt
            .message_for_id(&UiId::from("root/choice/option-0"))
            .is_some()
    );

    let scroll =
        |state: &mut UiStateStore| {
            UiFrame::layout_with_state(
                VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
                    .id("list")
                    .child(Column::new().children((0..8).map(|index| {
                        Text::<TestMessage>::new(format!("row {index}")).height(24.0)
                    }))),
                Rect::new(0.0, 0.0, 120.0, 48.0),
                state,
            )
        };
    let initial = scroll(&mut state);
    assert_eq!(
        initial
            .handle_event(
                &mut state,
                UiEvent::Scroll {
                    point: Point { x: 10.0, y: 10.0 },
                    delta_y: 30.0,
                },
            )
            .invalidation,
        Invalidation::Layout
    );
    assert_eq!(
        scroll(&mut state)
            .scroll_extent(&TestMessage::Named("scroll"))
            .expect("scroll extent")
            .offset,
        30.0
    );
}

#[test]
fn application_menu_overlays_content_selects_items_and_dismisses() {
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            Column::new()
                .child(
                    MenuBar::new().id("bar").child(
                        Menu::new(
                            TestMessage::Named("toggle"),
                            "File",
                            [
                                MenuItem::new("New", TestMessage::Named("new")),
                                MenuItem::disabled("Unavailable"),
                            ],
                        )
                        .id("file"),
                    ),
                )
                .child(
                    Container::new()
                        .id("body")
                        .height(1000.0)
                        .message(TestMessage::Named("body")),
                ),
            Rect::new(0.0, 0.0, 240.0, 160.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let closed = build(&mut state);
    let bar = closed
        .resolved_layout()
        .find(&UiId::from("root/bar"))
        .expect("menu bar");
    assert_eq!(bar.allocated.size.height, 30.0);
    let file_label = closed
        .commands()
        .iter()
        .find_map(|command| match command {
            PaintCommand::Text { bounds, text, .. } if text == "File" => Some(*bounds),
            _ => None,
        })
        .expect("menu header label");
    assert!(file_label.size.height >= 20.0);
    assert!(file_label.origin.y + file_label.size.height <= 30.0);
    let body_y = closed
        .resolved_layout()
        .find(&UiId::from("root/body"))
        .unwrap()
        .allocated
        .origin
        .y;
    closed.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
    );
    closed.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 }),
    );

    let open = build(&mut state);
    assert_eq!(
        open.resolved_layout()
            .find(&UiId::from("root/body"))
            .unwrap()
            .allocated
            .origin
            .y,
        body_y
    );
    assert_eq!(
        open.message_at(Point { x: 10.0, y: 47.0 }),
        Some(&TestMessage::Named("new"))
    );
    assert_eq!(open.message_at(Point { x: 10.0, y: 81.0 }), None);
    open.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 10.0, y: 47.0 }),
    );
    let selected = open.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point { x: 10.0, y: 47.0 }),
    );
    assert_eq!(selected.messages, vec![TestMessage::Named("new")]);
    assert!(
        !state
            .state(&UiId::from("root/bar/file"))
            .unwrap()
            .dropdown_open
    );

    let reopened = build(&mut state);
    reopened.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
    );
    reopened.handle_event(
        &mut state,
        UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 }),
    );
    build(&mut state).handle_event(&mut state, UiEvent::Dismiss);
    assert!(
        !state
            .state(&UiId::from("root/bar/file"))
            .unwrap()
            .dropdown_open
    );
}

#[test]
fn controller_enters_operates_and_exits_the_shared_menu_scope() {
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            Menu::new(
                TestMessage::Named("toggle"),
                "Actions",
                [
                    MenuItem::new("Launch", TestMessage::Named("launch")),
                    MenuItem::disabled("Unavailable"),
                    MenuItem::new("Pin", TestMessage::Named("pin")),
                ],
            )
            .id("actions"),
            Rect::new(0.0, 0.0, 220.0, 180.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let closed = build(&mut state);
    closed.handle_event(&mut state, UiEvent::ControllerDown);
    closed.handle_event(&mut state, UiEvent::ControllerActivate);
    assert!(
        state
            .state(&UiId::from("root/actions"))
            .is_some_and(|entry| entry.dropdown_open)
    );

    let open = build(&mut state);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/option-0"))
    );
    open.handle_event(&mut state, UiEvent::ControllerDown);
    assert!(
        state
            .navigation()
            .controller_selected()
            .is_some_and(|id| id.as_str().ends_with("/option-2"))
    );
    assert_eq!(
        open.handle_event(&mut state, UiEvent::ControllerActivate)
            .messages,
        [TestMessage::Named("pin")]
    );
    assert!(state.navigation().controller_scope().is_none());
    assert!(
        !state
            .state(&UiId::from("root/actions"))
            .is_some_and(|entry| entry.dropdown_open)
    );

    let reopened = build(&mut state);
    reopened.handle_event(&mut state, UiEvent::ControllerActivate);
    let reopened = build(&mut state);
    reopened.handle_event(&mut state, UiEvent::ControllerBack);
    assert!(state.navigation().controller_scope().is_none());
    assert!(
        !state
            .state(&UiId::from("root/actions"))
            .is_some_and(|entry| entry.dropdown_open)
    );
}

#[test]
fn text_field_focus_edit_selection_and_ime_survive_reconstruction_without_click_messages() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let build = |state: &mut UiStateStore, value: &str| {
        UiFrame::layout_with_state(
            TextField::on_change(value, query).id("query"),
            Rect::new(0.0, 0.0, 180.0, 32.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let first = build(&mut state, "nickel");
    first.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 10.0, y: 10.0 }),
    );
    assert_eq!(
        first
            .handle_event(
                &mut state,
                UiEvent::PointerReleased(Point { x: 10.0, y: 10.0 }),
            )
            .messages,
        Vec::<TestMessage>::new()
    );
    let id = UiId::from("root/query");
    assert_eq!(state.focused(), Some(&id));
    state.editor(id.clone(), "nickel").select_all();
    let changed = first
        .handle_event(&mut state, UiEvent::TextInput("silver".into()))
        .messages;
    assert_eq!(changed, vec![TestMessage::Query("silver".into())]);
    first.handle_event(&mut state, UiEvent::ImePreedit("世界".into()));
    let rebuilt = build(&mut state, "silver");
    assert!(rebuilt.commands().iter().any(|command| {
        matches!(command, PaintCommand::Text { text, .. } if text.contains("世界"))
    }));
    assert_eq!(
        state
            .state(&id)
            .and_then(|transient| transient.editor.as_ref())
            .expect("retained editor")
            .selection(),
        None
    );

    let externally_cleared = build(&mut state, "");
    assert!(externally_cleared.commands().iter().any(
        |command| matches!(command, PaintCommand::Fill { rect, .. } if rect.size.width == 1.5)
    ));
    assert_eq!(
        state
            .state(&id)
            .and_then(|transient| transient.editor.as_ref())
            .expect("controlled editor")
            .text(),
        ""
    );
}

#[test]
fn focused_text_field_supports_navigation_selection_and_deletion_messages() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        TextField::on_change("abc", query).id("query"),
        Rect::new(0.0, 0.0, 180.0, 32.0),
        &mut state,
    );
    tree.handle_event(&mut state, UiEvent::FocusNext);
    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::TextMoveLeft {
                extend_selection: true,
            },
        )
        .messages,
        vec![TestMessage::Query("abc".into())]
    );
    assert_eq!(
        tree.handle_event(&mut state, UiEvent::TextBackspace)
            .messages,
        vec![TestMessage::Query("ab".into())]
    );
    tree.handle_event(&mut state, UiEvent::TextSelectAll);
    assert_eq!(
        tree.handle_event(&mut state, UiEvent::TextInput("replacement".into()))
            .messages,
        vec![TestMessage::Query("replacement".into())]
    );
    tree.handle_event(
        &mut state,
        UiEvent::TextMoveHome {
            extend_selection: false,
        },
    );
    assert_eq!(
        tree.handle_event(&mut state, UiEvent::TextDelete).messages,
        vec![TestMessage::Query("eplacement".into())]
    );
}

#[test]
fn focused_text_field_copies_cuts_and_pastes_selection() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        TextField::on_change("copy me", query).id("query"),
        Rect::new(0.0, 0.0, 180.0, 32.0),
        &mut state,
    );
    tree.handle_event(&mut state, UiEvent::FocusNext);
    tree.handle_event(&mut state, UiEvent::TextSelectAll);

    let copied = tree.handle_event(&mut state, UiEvent::TextCopy);
    assert_eq!(copied.clipboard_text.as_deref(), Some("copy me"));
    assert!(copied.messages.is_empty());

    let cut = tree.handle_event(&mut state, UiEvent::TextCut);
    assert_eq!(cut.clipboard_text.as_deref(), Some("copy me"));
    assert_eq!(cut.messages, vec![TestMessage::Query(String::new())]);

    let pasted = tree.handle_event(&mut state, UiEvent::TextPaste("世界".into()));
    assert_eq!(pasted.messages, vec![TestMessage::Query("世界".into())]);
    let empty_cut = tree.handle_event(&mut state, UiEvent::TextCut);
    assert!(empty_cut.clipboard_text.is_none());
    assert!(empty_cut.messages.is_empty());
}

#[test]
fn every_text_field_owns_the_shared_stable_context_menu_and_secure_policy() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }
    let bounds = Rect::new(0.0, 0.0, 240.0, 120.0);
    let build = |state: &mut UiStateStore, value: &str, secure: bool| {
        let field = if secure {
            TextField::on_change_masked(value, '•', query).id("query")
        } else {
            TextField::on_change(value, query).id("query")
        };
        UiFrame::layout_with_state(field, bounds, state)
    };
    let editor_id = UiId::from("root/query");
    let menu_id = UiId::from("root/query/text-context-menu");
    let copy_id = menu_id.scoped("copy");
    let select_all_id = menu_id.scoped("selectall");

    let mut state = UiStateStore::default();
    state.set_clipboard_offer(Some("paste payload"));
    let initial = build(&mut state, "one\ne\u{301}🦀", false);
    initial.handle_event(&mut state, UiEvent::FocusNext);
    initial.handle_event(&mut state, UiEvent::ImePreedit("uncommitted".into()));
    let opened = initial.handle_event(&mut state, UiEvent::KeyboardContextMenu);
    assert_eq!(opened.invalidation, Invalidation::Layout);
    assert_eq!(state.focused(), Some(&editor_id));
    assert_eq!(
        state
            .state(&editor_id)
            .and_then(|entry| entry.editor.as_ref())
            .unwrap()
            .preedit(),
        "",
        "menu invocation cancels rather than commits IME preedit"
    );

    let menu = build(&mut state, "one\ne\u{301}🦀", false);
    let labels = menu
        .semantic_nodes()
        .iter()
        .filter(|node| node.parent.as_ref() == Some(&menu_id))
        .map(|node| (node.name.clone().unwrap(), node.enabled))
        .collect::<Vec<_>>();
    assert_eq!(
        labels,
        [
            ("Undo".into(), false),
            ("Redo".into(), false),
            ("Cut".into(), false),
            ("Copy".into(), false),
            ("Paste".into(), true),
            ("Delete".into(), false),
            ("Select All".into(), true),
        ]
    );
    state.set_clipboard_offer(None);
    let refreshed = build(&mut state, "one\ne\u{301}🦀", false);
    assert!(
        !refreshed
            .semantic_nodes()
            .into_iter()
            .find(|node| node.name.as_deref() == Some("Paste"))
            .unwrap()
            .enabled
    );
    menu.transition(
        &mut state,
        InputSource::Accessibility,
        InteractionIntent::Invoke {
            target: select_all_id,
            action: SemanticAction::Invoke(ActionKind::Activate),
        },
    )
    .unwrap();
    assert_eq!(state.focused(), Some(&editor_id));
    assert!(state.open_overlay_id().is_none());

    let reopened_base = build(&mut state, "one\ne\u{301}🦀", false);
    reopened_base.handle_event(&mut state, UiEvent::KeyboardContextMenu);
    let reopened = build(&mut state, "one\ne\u{301}🦀", false);
    let copied = reopened
        .transition(
            &mut state,
            InputSource::Accessibility,
            InteractionIntent::Invoke {
                target: copy_id.clone(),
                action: SemanticAction::Invoke(ActionKind::Activate),
            },
        )
        .unwrap();
    assert_eq!(copied.clipboard_text.as_deref(), Some("one\ne\u{301}🦀"));

    let secure_base = build(&mut state, "secret", true);
    secure_base.handle_event(&mut state, UiEvent::FocusNext);
    secure_base.handle_event(&mut state, UiEvent::TextSelectAll);
    secure_base.handle_event(&mut state, UiEvent::KeyboardContextMenu);
    let secure_menu = build(&mut state, "secret", true);
    let copy = secure_menu
        .semantic_nodes()
        .into_iter()
        .find(|node| node.id == copy_id)
        .unwrap();
    assert!(!copy.enabled);
    assert!(!format!("{copy:?}").contains("secret"));
    assert!(
        secure_menu
            .transition(
                &mut state,
                InputSource::Accessibility,
                InteractionIntent::Invoke {
                    target: copy_id,
                    action: SemanticAction::Invoke(ActionKind::Activate),
                },
            )
            .is_err()
    );
}

#[test]
fn every_context_invocation_route_opens_the_same_text_menu() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }
    let bounds = Rect::new(0.0, 0.0, 200.0, 40.0);
    let editor = UiId::from("root/query");
    let invocations = [
        UiEvent::PointerContext(Point { x: 8.0, y: 8.0 }),
        UiEvent::TouchLongPress(Point { x: 8.0, y: 8.0 }),
        UiEvent::KeyboardContextMenu,
        UiEvent::ControllerContextMenu,
        UiEvent::AccessibilityContextMenu(editor.clone()),
    ];
    for invocation in invocations {
        let mut state = UiStateStore::default();
        let frame = UiFrame::layout_with_state(
            TextField::on_change("text", query).id("query"),
            bounds,
            &mut state,
        );
        frame.handle_event(&mut state, UiEvent::FocusNext);
        frame.handle_event(&mut state, invocation);
        let menu = UiFrame::layout_with_state(
            TextField::on_change("text", query).id("query"),
            bounds,
            &mut state,
        );
        assert!(menu.semantic_nodes().iter().any(|node| {
            node.role == Some(SemanticRole::Menu) && node.id == editor.scoped("text-context-menu")
        }));
    }
}

#[test]
fn text_context_menu_dismisses_when_its_document_or_field_becomes_stale() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }
    let bounds = Rect::new(0.0, 0.0, 200.0, 40.0);
    let mut state = UiStateStore::default();
    let initial = UiFrame::layout_with_state(
        TextField::on_change("first", query).id("first"),
        bounds,
        &mut state,
    );
    initial.handle_event(&mut state, UiEvent::FocusNext);
    initial.handle_event(&mut state, UiEvent::KeyboardContextMenu);
    state
        .editor(UiId::from("root/first"), "first")
        .insert(" changed");
    let stale = UiFrame::layout_with_state(
        TextField::on_change("first changed", query).id("first"),
        bounds,
        &mut state,
    );
    assert!(state.open_overlay_id().is_none());
    assert!(
        stale
            .semantic_nodes()
            .iter()
            .all(|node| node.role != Some(SemanticRole::Menu))
    );

    let reopened = UiFrame::layout_with_state(
        TextField::on_change("first", query).id("first"),
        bounds,
        &mut state,
    );
    reopened.handle_event(&mut state, UiEvent::FocusNext);
    reopened.handle_event(&mut state, UiEvent::KeyboardContextMenu);
    let replacement = UiFrame::layout_with_state(
        TextField::on_change("second", query).id("second"),
        bounds,
        &mut state,
    );
    assert!(state.open_overlay_id().is_none());
    assert!(
        replacement
            .semantic_nodes()
            .iter()
            .all(|node| node.role != Some(SemanticRole::Menu))
    );
}

#[test]
fn read_only_selection_uses_reduced_copy_and_select_all_menu() {
    let bounds = Rect::new(0.0, 0.0, 240.0, 80.0);
    let document = Arc::new(SelectionDocument::new([
        SelectionRun::inline("first", "read-only "),
        SelectionRun::inline("second", "🦀 text"),
    ]));
    let region_id = UiId::from("root/document");
    let build = |state: &mut UiStateStore, document: Arc<SelectionDocument>| {
        UiFrame::<TestMessage>::layout_with_state(
            SelectionRegion::new(document)
                .id("document")
                .child(Text::new("read-only 🦀 text").selectable(true)),
            bounds,
            state,
        )
    };

    let mut state = UiStateStore::default();
    let initial = build(&mut state, document.clone());
    *state.document_selection_mut(region_id.clone()) = document.select_all();
    state.set_selection_owner(Some(region_id.clone()));
    initial.handle_event(&mut state, UiEvent::KeyboardContextMenu);
    let menu = build(&mut state, document.clone());
    let menu_id = region_id.scoped("text-context-menu");
    let items = menu
        .semantic_nodes()
        .into_iter()
        .filter(|node| node.parent.as_ref() == Some(&menu_id))
        .map(|node| (node.name.unwrap(), node.enabled))
        .collect::<Vec<_>>();
    assert_eq!(items, [("Copy".into(), true), ("Select All".into(), true)]);

    let copied = menu.handle_event(
        &mut state,
        UiEvent::AccessibilityActivate(menu_id.scoped("copy")),
    );
    assert_eq!(copied.clipboard_text.as_deref(), Some("read-only 🦀 text"));
    assert!(state.open_overlay_id().is_none());

    let reopened = build(&mut state, document);
    reopened.handle_event(&mut state, UiEvent::KeyboardContextMenu);
    let changed = Arc::new(SelectionDocument::new([SelectionRun::inline(
        "replacement",
        "different",
    )]));
    let stale = build(&mut state, changed);
    assert!(state.open_overlay_id().is_none());
    assert!(
        stale
            .semantic_nodes()
            .iter()
            .all(|node| node.role != Some(SemanticRole::Menu))
    );
}

#[test]
fn effective_context_target_matches_editable_then_read_only_fallback_order() {
    fn changed(value: String) -> TestMessage {
        TestMessage::Query(value)
    }
    let document = Arc::new(SelectionDocument::new([SelectionRun::inline(
        "selection",
        "selected",
    )]));
    let region_id = UiId::from("root/document");
    let editor_id = UiId::from("root/editor");
    let ordinary_id = UiId::from("root/ordinary");
    let mut state = UiStateStore::default();
    let children = vec![
        AnyView::new(TextField::on_change("editable", changed).id("editor")),
        AnyView::new(Button::new(TestMessage::Named("ordinary"), "Ordinary").id("ordinary")),
        AnyView::new(
            SelectionRegion::new(document.clone())
                .id("document")
                .child(Text::new("selected").selectable(true)),
        ),
    ];
    let frame = UiFrame::<TestMessage>::layout_with_state(
        Container::new().children(children),
        Rect::new(0.0, 0.0, 240.0, 120.0),
        &mut state,
    );
    *state.document_selection_mut(region_id.clone()) = document.select_all();
    state.set_selection_owner(Some(region_id.clone()));

    state.set_focus(Some(ordinary_id));
    assert_eq!(
        frame.effective_context_target(&state, InputSource::Keyboard),
        Some(region_id.clone())
    );

    state.set_focus(Some(editor_id.clone()));
    assert_eq!(
        frame.effective_context_target(&state, InputSource::Keyboard),
        Some(editor_id.clone())
    );
    state
        .navigation_mut()
        .set_controller_selected(Some(editor_id.clone()));
    assert_eq!(
        frame.effective_context_target(&state, InputSource::Controller),
        Some(editor_id)
    );
}

#[test]
fn unselected_region_keeps_offscreen_document_lazy_until_copy() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let calls = Arc::new(AtomicUsize::new(0));
    let provider_calls = calls.clone();
    let document = Arc::new(SelectionDocument::lazy(123, move || {
        provider_calls.fetch_add(1, Ordering::SeqCst);
        vec![
            SelectionRun::block("visible", "Visible"),
            SelectionRun::block("offscreen", "Offscreen 🦀"),
        ]
    }));
    let mut state = UiStateStore::default();
    let build = |state: &mut UiStateStore| {
        UiFrame::<TestMessage>::layout_with_state(
            SelectionRegion::new(document.clone())
                .id("document")
                .child(Text::new("Visible").selectable(true)),
            Rect::new(0.0, 0.0, 240.0, 80.0),
            state,
        )
    };
    for _ in 0..10 {
        let frame = build(&mut state);
        let _ = frame.semantic_nodes();
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }
    let region_id = UiId::from("root/document");
    *state.document_selection_mut(region_id.clone()) = document.select_all();
    state.set_selection_owner(Some(region_id.clone()));
    let frame = build(&mut state);
    frame.handle_event(&mut state, UiEvent::KeyboardContextMenu);
    let menu = build(&mut state);
    let copied = menu.handle_event(
        &mut state,
        UiEvent::AccessibilityActivate(region_id.scoped("text-context-menu").scoped("copy")),
    );
    assert_eq!(
        copied.clipboard_text.as_deref(),
        Some("Visible\nOffscreen 🦀")
    );
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn multiline_text_field_hit_testing_and_caret_follow_explicit_lines() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        TextField::on_change("one\ntwo", query)
            .id("query")
            .wrap(true),
        Rect::new(0.0, 0.0, 200.0, 80.0),
        &mut state,
    );
    tree.handle_event(
        &mut state,
        UiEvent::PointerPressed(Point { x: 100.0, y: 28.0 }),
    );
    let inserted = tree.handle_event(&mut state, UiEvent::TextInput("!".into()));
    assert_eq!(
        inserted.messages,
        vec![TestMessage::Query("one\ntwo!".into())]
    );

    let rebuilt = UiFrame::layout_with_state(
        TextField::on_change("one\ntwo!", query)
            .id("query")
            .wrap(true),
        Rect::new(0.0, 0.0, 200.0, 80.0),
        &mut state,
    );
    assert!(rebuilt.commands().iter().any(|command| matches!(
        command,
        PaintCommand::Fill { rect, .. }
            if (rect.size.width - 1.5).abs() < f32::EPSILON && rect.origin.y > 10.0
    )));
}

#[test]
fn scrolling_clipped_content_back_into_view_matches_fresh_paint_and_hits() {
    let view = || {
        Column::new()
            .id("scroll")
            .height(80.0)
            .overflow_y(Overflow::Auto)
            .children((0..8).map(|index| {
                Row::new()
                    .id(format!("row-{index}"))
                    .height(40.0)
                    .shrink(0.0)
                    .child(Text::new(format!("Label {index}")).content_revision(index as u64))
                    .child(
                        Button::new(index, format!("Action {index}"))
                            .content_revision(index as u64),
                    )
            }))
    };
    let bounds = Rect::new(0.0, 0.0, 320.0, 80.0);
    let mut state = UiStateStore::default();
    let mut frame = UiFrame::resolve(view(), FrameRequest::new(bounds, &mut state));
    for offset in [40.0, 240.0, 20.0, 0.0, 240.0, 0.0] {
        state.touch(UiId::from("root/scroll")).scroll_offset = offset;
        frame = UiFrame::resolve_against(view(), FrameRequest::new(bounds, &mut state), &frame);
        let cold = UiFrame::resolve(view(), FrameRequest::new(bounds, &mut state.clone()));
        assert_eq!(frame.commands(), cold.commands(), "scroll offset {offset}");
        assert_eq!(frame.semantic_nodes(), cold.semantic_nodes());
        for y in [1.0, 20.0, 39.0, 60.0, 79.0] {
            for x in [20.0, 100.0, 200.0, 310.0] {
                assert_eq!(
                    frame.message_at(Point { x, y }),
                    cold.message_at(Point { x, y })
                );
            }
        }
    }
}

#[test]
fn ordinary_auto_overflow_owns_scroll_state_and_clips_descendants() {
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            Column::new()
                .id("automatic")
                .height(48.0)
                .overflow(Overflow::Clip, Overflow::Auto)
                .children((0..4).map(|index| {
                    Button::new(TestMessage::Option(index), format!("row {index}"))
                        .id(format!("row-{index}"))
                        .height(24.0)
                        .shrink(0.0)
                })),
            Rect::new(0.0, 0.0, 120.0, 48.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let initial = build(&mut state);
    let node = initial
        .resolved_layout()
        .find(&UiId::from("root/automatic"))
        .expect("resolved automatic overflow container");
    assert_eq!(node.scroll.expect("scroll metadata").content.height, 96.0);
    assert_eq!(
        initial.message_at(Point { x: 10.0, y: 60.0 }),
        None,
        "hit regions outside the viewport are clipped"
    );
    assert_eq!(
        initial
            .handle_event(
                &mut state,
                UiEvent::Scroll {
                    point: Point { x: 10.0, y: 10.0 },
                    delta_y: 30.0,
                },
            )
            .invalidation,
        Invalidation::Layout
    );
    let scrolled = build(&mut state);
    assert_eq!(
        scrolled
            .resolved_layout()
            .find(&UiId::from("root/automatic"))
            .and_then(|node| node.scroll)
            .expect("scroll metadata")
            .offset,
        30.0
    );
    assert_eq!(
        scrolled.message_at(Point { x: 10.0, y: 30.0 }),
        Some(&TestMessage::Option(2))
    );

    state.touch(UiId::from("root/automatic")).scroll_offset = 200.0;
    let clamped = build(&mut state);
    assert_eq!(
        clamped
            .resolved_layout()
            .find(&UiId::from("root/automatic"))
            .and_then(|node| node.scroll)
            .expect("scroll metadata")
            .offset,
        48.0
    );
    assert_eq!(
        state
            .state(&UiId::from("root/automatic"))
            .expect("retained state")
            .scroll_offset,
        48.0
    );
}

#[test]
fn follow_scroll_end_pins_growth_until_the_user_scrolls_up() {
    let build = |state: &mut UiStateStore, rows: usize| {
        UiFrame::layout_with_state(
            Column::<TestMessage>::new()
                .id("conversation")
                .height(60.0)
                .overflow_y(Overflow::Auto)
                .follow_scroll_end(true)
                .children((0..rows).map(|index| {
                    Container::new()
                        .id(index)
                        .height(30.0)
                        .child(Text::new(format!("row {index}")))
                })),
            Rect::new(0.0, 0.0, 200.0, 60.0),
            state,
        )
    };
    let id = UiId::from("root").scoped("conversation");
    let mut state = UiStateStore::default();
    let initial = build(&mut state, 4);
    let initial_extent = initial.resolved_layout().nodes()[0]
        .scroll
        .expect("scroll extent");
    assert_eq!(initial_extent.offset, 60.0);

    state.scroll_by(id.clone(), -30.0, 60.0);
    let anchored = build(&mut state, 5);
    let anchored_extent = anchored.resolved_layout().nodes()[0]
        .scroll
        .expect("scroll extent");
    assert_eq!(anchored_extent.offset, 30.0);

    state.scroll_by(id, 100.0, 90.0);
    let followed = build(&mut state, 6);
    let followed_extent = followed.resolved_layout().nodes()[0]
        .scroll
        .expect("scroll extent");
    assert_eq!(followed_extent.offset, 120.0);
}

#[test]
fn virtual_window_bounds_variable_height_work_at_start_middle_and_end() {
    let heights = [100.0, 200.0, 300.0];
    let start = VirtualWindow::from_heights(&heights, 10.0, 0.0, 100.0, 0.0);
    assert_eq!(start.range, 0..1);
    assert_eq!(
        (start.leading, start.trailing, start.total),
        (0.0, 520.0, 620.0)
    );

    let middle = VirtualWindow::from_heights(&heights, 10.0, 120.0, 100.0, 0.0);
    assert_eq!(middle.range, 1..2);
    assert_eq!((middle.leading, middle.trailing), (110.0, 310.0));

    let end = VirtualWindow::from_heights(&heights, 10.0, f32::MAX, 100.0, 0.0);
    assert_eq!(end.range, 2..3);
    assert_eq!((end.leading, end.trailing), (320.0, 0.0));

    let empty = VirtualWindow::from_heights(&[], 10.0, 0.0, 100.0, 100.0);
    assert_eq!(empty.range, 0..0);
    assert_eq!(empty.total, 0.0);
}

#[test]
fn uniform_virtual_window_matches_explicit_fixed_extents() {
    for count in [0usize, 1, 2, 31, 10_000] {
        for height in [1.0f32, 24.0, 31.5] {
            for gap in [0.0f32, 3.0] {
                let heights = vec![height; count];
                for offset in [0.0f32, 1.0, 48.0, 96_000.0, f32::MAX] {
                    let explicit = VirtualWindow::from_heights(&heights, gap, offset, 480.0, 48.0);
                    let uniform =
                        VirtualWindow::from_uniform(count, height, gap, offset, 480.0, 48.0);
                    assert_eq!(uniform, explicit);
                }
            }
        }
    }
}

#[test]
fn vertical_scroll_emits_the_resulting_offset() {
    fn scrolled(offset: f32) -> TestMessage {
        TestMessage::Volume(offset.round() as u8)
    }

    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        Column::new()
            .id("scroll")
            .height(60.0)
            .overflow_y(Overflow::Auto)
            .on_scroll(scrolled)
            .children((0..4).map(|_| Spacer::vertical(30.0))),
        Rect::new(0.0, 0.0, 200.0, 60.0),
        &mut state,
    );
    let outcome = tree.handle_event(
        &mut state,
        UiEvent::Scroll {
            point: Point { x: 10.0, y: 10.0 },
            delta_y: 30.0,
        },
    );
    assert_eq!(outcome.messages, vec![TestMessage::Volume(30)]);
}

#[test]
fn vertical_scroll_can_report_measured_extent_with_resulting_offset() {
    fn scrolled(extent: ScrollExtent) -> TestMessage {
        TestMessage::Query(format!(
            "{}:{}:{}",
            extent.viewport.height, extent.content.height, extent.offset
        ))
    }
    let mut state = UiStateStore::default();
    let tree = UiFrame::layout_with_state(
        Column::new()
            .id("scroll")
            .height(60.0)
            .overflow_y(Overflow::Auto)
            .on_scroll_extent(scrolled)
            .children((0..4).map(|_| Spacer::vertical(30.0))),
        Rect::new(0.0, 0.0, 200.0, 60.0),
        &mut state,
    );
    let outcome = tree.handle_event(
        &mut state,
        UiEvent::Scroll {
            point: Point { x: 10.0, y: 10.0 },
            delta_y: 30.0,
        },
    );
    assert_eq!(
        outcome.messages,
        vec![TestMessage::Query("60:120:30".into())]
    );
}

#[test]
fn horizontal_overflow_policy_scrolls_and_clips_on_its_own_axis() {
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            Row::new()
                .id("horizontal")
                .width(80.0)
                .overflow(Overflow::Auto, Overflow::Clip)
                .children((0..3).map(|index| {
                    Button::new(TestMessage::Option(index), format!("item {index}"))
                        .id(format!("item-{index}"))
                        .width(50.0)
                        .shrink(0.0)
                })),
            Rect::new(0.0, 0.0, 80.0, 32.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let initial = build(&mut state);
    let extent = initial
        .resolved_layout()
        .find(&UiId::from("root/horizontal"))
        .and_then(|node| node.scroll)
        .expect("horizontal scroll metadata");
    assert_eq!(extent.content.width, 150.0);
    assert_eq!(
        initial.handle_event(
            &mut state,
            UiEvent::ScrollHorizontal {
                point: Point { x: 10.0, y: 10.0 },
                delta_x: 60.0,
            },
        ),
        EventOutcome {
            messages: Vec::new(),
            clipboard_text: None,
            invalidation: Invalidation::Layout,
            disposition: crate::EventDisposition::Handled,
        }
    );
    let scrolled = build(&mut state);
    assert_eq!(
        scrolled
            .resolved_layout()
            .find(&UiId::from("root/horizontal"))
            .and_then(|node| node.scroll)
            .expect("horizontal scroll metadata")
            .offset_x,
        60.0
    );
    assert_eq!(
        scrolled.message_at(Point { x: 65.0, y: 10.0 }),
        Some(&TestMessage::Option(2))
    );
    for semantic in scrolled.semantic_nodes() {
        assert!(
            scrolled
                .accessibility_nodes()
                .iter()
                .any(|node| node.id == semantic.id),
            "offscreen semantic node must remain accessibility-addressable"
        );
    }
}

#[test]
fn horizontal_scrollbar_has_a_gutter_below_content() {
    let tree = UiFrame::<()>::layout(
        Row::new()
            .id("horizontal")
            .width(80.0)
            .overflow(Overflow::Auto, Overflow::Clip)
            .child(
                Container::new()
                    .id("wide-content")
                    .width(160.0)
                    .height(20.0),
            ),
        Rect::new(0.0, 0.0, 80.0, 34.0),
    );
    let content = tree
        .resolved_layout()
        .find(&UiId::from("root/horizontal/wide-content"))
        .expect("overflow content")
        .allocated;
    let scroll = tree
        .scrolls
        .iter()
        .find(|scroll| scroll.id == UiId::from("root/horizontal"))
        .expect("horizontal scroll region");
    let (track, _) = scrollbar_geometry(scroll, ScrollbarAxis::Horizontal)
        .expect("horizontal scrollbar geometry");
    assert!(
        content.origin.y + content.size.height <= track.origin.y,
        "content overlaps scrollbar: {content:?}, {track:?}"
    );
}

#[test]
fn auto_horizontal_overflow_has_no_gutter_when_content_fits() {
    let element = Row::<()>::new()
        .width(80.0)
        .overflow(Overflow::Auto, Overflow::Clip)
        .child(Container::new().width(40.0).height(20.0))
        .into_element();
    assert_eq!(
        element.measure(Constraints::loose(Size::new(80.0, 100.0))),
        Size::new(80.0, 20.0)
    );
}

#[test]
fn nested_scrollbar_chrome_is_clipped_by_the_outer_viewport() {
    let tree = UiFrame::layout(
        VerticalScroll::new(TestMessage::Named("document"), 0.0)
            .height(80.0)
            .child(
                Column::new().child(Spacer::vertical(80.0)).child(
                    Row::new()
                        .id("offscreen-horizontal")
                        .width(120.0)
                        .overflow_x(Overflow::Auto)
                        .child(Spacer::new().width(240.0).height(20.0)),
                ),
            ),
        Rect::new(0.0, 0.0, 120.0, 80.0),
    );
    assert!(tree.commands().iter().all(|command| {
        !matches!(command, PaintCommand::RoundedFill { rect, .. } if rect.origin.y >= 80.0)
    }));
    assert!(tree.scrollbar_at(Point { x: 20.0, y: 90.0 }).is_none());
}

#[test]
fn vertical_wheel_over_horizontal_overflow_chains_to_the_document() {
    let build = |state: &mut UiStateStore| {
        UiFrame::layout_with_state(
            Column::<TestMessage>::new()
                .id("document")
                .width(120.0)
                .height(80.0)
                .overflow_y(Overflow::Auto)
                .child(Spacer::vertical(10.0))
                .child(
                    Row::new()
                        .id("code")
                        .height(30.0)
                        .overflow(Overflow::Auto, Overflow::Clip)
                        .child(Spacer::new().width(240.0).height(30.0).shrink(0.0)),
                )
                .child(Spacer::vertical(200.0)),
            Rect::new(0.0, 0.0, 120.0, 80.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let tree = build(&mut state);
    let code = tree
        .resolved_layout()
        .find(&UiId::from("root/document/code"))
        .expect("nested horizontal code region");
    let point = Point {
        x: code.content.origin.x + 5.0,
        y: code.content.origin.y + 5.0,
    };

    assert_eq!(
        tree.handle_event(
            &mut state,
            UiEvent::Scroll {
                point,
                delta_y: 30.0,
            },
        )
        .invalidation,
        Invalidation::Layout
    );
    let rebuilt = build(&mut state);
    assert_eq!(
        rebuilt
            .resolved_layout()
            .find(&UiId::from("root/document"))
            .and_then(|node| node.scroll)
            .expect("outer document scroll")
            .offset,
        30.0
    );
}

#[test]
fn bounded_semantics_preserve_complete_projection_and_reject_small_budgets() {
    let frame = UiFrame::layout(
        Row::new().id("layout-parent").child(
            TextField::on_change("ordinary input", map_query)
                .id("query")
                .accessibility_label("Search"),
        ),
        Rect::new(0.0, 0.0, 320.0, 40.0),
    );
    assert_eq!(
        frame.bounded_semantic_nodes(64, 4096).unwrap(),
        frame.semantic_nodes()
    );
    assert_eq!(
        frame.bounded_semantic_nodes(1, 4096),
        Err(BoundedSemanticError::BudgetExceeded)
    );
    assert_eq!(
        frame.bounded_semantic_nodes(64, 0),
        Err(BoundedSemanticError::BudgetExceeded)
    );
    let huge = UiFrame::layout(
        TextField::on_change("display", map_query)
            .id("x".repeat(8192))
            .accessibility_label("name"),
        Rect::new(0.0, 0.0, 320.0, 40.0),
    );
    assert_eq!(
        huge.bounded_semantic_nodes(64, 4096),
        Err(BoundedSemanticError::BudgetExceeded)
    );
}

#[test]
fn bounded_semantics_refuse_protected_values_without_publishing_lengths() {
    let frame = UiFrame::layout(
        TextField::on_change_masked_with_placeholder("secret", "Password", '•', map_query),
        Rect::new(0.0, 0.0, 320.0, 40.0),
    );
    assert_eq!(
        frame.bounded_semantic_nodes(64, 4096),
        Err(BoundedSemanticError::ProtectedSurface)
    );
}

#[test]
fn definite_boxes_measure_without_shaping_their_descendants() {
    let box_element = Container::<()>::new()
        .width(240.0)
        .height(90.0)
        .padding(Insets::all(8.0))
        .child(Text::new("Wrapped text still needs layout and paint inside the box").wrap(true))
        .into_element();
    let before = text_layout_cache_diagnostics();
    assert_eq!(
        box_element.measure(Constraints::loose(Size::new(500.0, 400.0))),
        Size::new(240.0, 90.0)
    );
    let after = text_layout_cache_diagnostics();
    assert_eq!(after.hits, before.hits);
    assert_eq!(after.misses, before.misses);
    let frame = UiFrame::layout(box_element, Rect::new(0.0, 0.0, 240.0, 90.0));
    assert!(
        frame
            .commands()
            .iter()
            .any(|command| matches!(command, PaintCommand::Text { .. }))
    );
    assert!(frame.resolved_layout().nodes().len() > 1);
}

#[test]
fn definite_measurement_preserves_parent_constraints_and_style_limits() {
    let mut element = Container::<()>::new()
        .width(240.0)
        .height(90.0)
        .into_element();
    element.style.min_width = 300.0;
    element.style.max_height = 60.0;
    assert_eq!(
        element.measure(Constraints::loose(Size::new(280.0, 400.0))),
        Size::new(280.0, 60.0)
    );
    element.style.width = Length::Percent(0.5);
    element.style.height = Length::Fill;
    element.style.min_width = 0.0;
    element.style.max_height = f32::INFINITY;
    assert_eq!(
        element.measure(Constraints::loose(Size::new(280.0, 400.0))),
        Size::new(140.0, 400.0)
    );
    // Unbounded percentages and fill still depend on intrinsic content.
    element
        .children
        .push(Container::new().width(42.0).height(17.0).into_element());
    assert_eq!(
        element.measure(Constraints::unbounded()),
        Size::new(42.0, 17.0)
    );
}
