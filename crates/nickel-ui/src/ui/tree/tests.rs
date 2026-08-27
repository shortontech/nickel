use super::*;

#[derive(Clone, Debug, PartialEq, Eq)]
enum TestMessage {
    Named(&'static str),
    Option(usize),
    Volume(u8),
    Query(String),
}

#[test]
fn nested_button_is_laid_out_and_hit_tested() {
    let tree = UiTree::layout(
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
fn grid_places_all_children() {
    let tree = UiTree::layout(
        Grid::fixed(2).children([
            Button::new(TestMessage::Named("one"), "One"),
            Button::new(TestMessage::Named("two"), "Two"),
            Button::new(TestMessage::Named("three"), "Three"),
        ]),
        Rect::new(0.0, 0.0, 200.0, 100.0),
    );
    assert_eq!(tree.hits.len(), 3);
}

#[test]
fn horizontal_flex_remeasures_wrapped_child_at_its_resolved_width() {
    let prose = "Lorem ipsum dolor sit amet consectetur adipiscing elit deserunt fugiat. \
            Et omnis cillum fugiat sint illum esse fugiat. Minus fuga aut dolor quos cupidatat atque.";
    let tree = UiTree::<()>::layout(
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
    let tree = UiTree::layout(
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
fn slider_reports_horizontal_fraction() {
    let tree = UiTree::layout(
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
    let tree = UiTree::layout(
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
fn vertical_scroll_clips_painting_and_hit_regions() {
    let tree = UiTree::layout(
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
        Some(PaintCommand::PushClip(rect)) if *rect == Rect::new(0.0, 0.0, 200.0, 100.0)
    ));
    assert!(
        tree.commands
            .iter()
            .any(|command| matches!(command, PaintCommand::PopClip))
    );
    assert!(tree.commands.iter().any(|command| matches!(
        command,
        PaintCommand::RoundedFill { rect, .. }
            if rect.origin.x >= 189.0 && rect.size.width == SCROLLBAR_THICKNESS
    )));
    assert_eq!(
        tree.scroll_extent(&TestMessage::Named("scroll")),
        Some(ScrollExtent {
            viewport: Size::new(200.0, 100.0),
            content: Size::new(200.0, 200.0),
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
    let tree = UiTree::layout(
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
fn visible_scrollbar_track_drags_and_releases_shared_scroll_state() {
    let mut state = UiStateStore::default();
    let build = |state: &mut UiStateStore| {
        UiTree::layout_with_state(
            VerticalScroll::new(TestMessage::Named("scroll"), 0.0)
                .on_scroll(|_| TestMessage::Named("dragged"))
                .child(Spacer::vertical(240.0)),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            state,
        )
    };
    let tree = build(&mut state);
    let pressed_at = Point { x: 194.0, y: 50.0 };

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
    assert_eq!(pressed.messages, vec![TestMessage::Named("dragged")]);
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
fn controlled_vertical_scroll_keeps_the_supplied_offset() {
    let mut state = UiStateStore::default();
    state.touch(UiId::from("root/reader")).scroll_offset = 17.0;
    let tree = UiTree::layout_with_state(
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
    let tree = UiTree::layout(
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
fn sidebar_folder_separates_toggle_and_open_actions() {
    let tree = UiTree::layout(
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
    let tree = UiTree::<()>::layout(
        Container::new()
            .width(120.0)
            .height(32.0)
            .background(0xffffff)
            .top_corner_radius(7.0),
        Rect::new(0.0, 0.0, 120.0, 32.0),
    );
    assert!(tree.commands().iter().any(|command| matches!(
        command,
        PaintCommand::TopRoundedFill { radius, .. } if *radius == 7.0
    )));
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
    let tree = UiTree::layout(
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
    TEXT_MEASURER.with(|measurer| measurer.borrow_mut().styled.clear());
    let spans = vec![StyledTextSpan {
        range: 0..6,
        bold: true,
        italic: false,
        monospace: false,
        strikethrough: false,
        color: None,
        background: None,
    }];

    let first = measure_styled_text("Cached text", &spans, 1.0, true, None, 240.0);
    let second = measure_styled_text("Cached text", &spans, 1.0, true, None, 240.0);

    assert_eq!(first, second);
    TEXT_MEASURER.with(|measurer| assert_eq!(measurer.borrow().styled.len(), 1));
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
    let tree = UiTree::layout_with_diagnostics(element, Rect::new(0.0, 0.0, 32.0, 32.0));
    assert!(tree.diagnostics().iter().any(|diagnostic| {
        diagnostic.kind == DiagnosticKind::MissingAsset
            && diagnostic.id == UiId::from("root/missing")
    }));
}

#[test]
fn ellipsis_changes_presentation_without_entering_message_payloads() {
    let tree = UiTree::<TestMessage>::layout(
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
    let tree = UiTree::layout(
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
        .message_layout_rect(&TestMessage::Named("left"))
        .unwrap();
    let right = tree
        .message_layout_rect(&TestMessage::Named("right"))
        .unwrap();
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
            let tree = UiTree::layout(
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
                let rect = tree.message_layout_rect(&message).unwrap();
                assert!(rect.origin.x.is_finite() && rect.origin.y.is_finite());
                assert!(rect.size.width >= 0.0 && rect.size.height >= 0.0);
            }
        }
    }
}

#[test]
fn grid_resolves_fixed_auto_fractional_repeated_and_auto_fit_tracks() {
    let fixed = UiTree::layout(
        Grid::tracks([Track::px(40.0), Track::Auto, Track::fr(1.0)]).children([
            Button::new(TestMessage::Option(0), "A"),
            Button::new(TestMessage::Option(1), "A much wider label"),
            Button::new(TestMessage::Option(2), "C"),
        ]),
        Rect::new(0.0, 0.0, 300.0, 60.0),
    );
    assert_eq!(fixed.resolved_grid_columns(), Some(3));
    let repeated = UiTree::layout(
        Grid::tracks([Track::repeat(2, Track::fr(1.0))]).children([
            Button::new(TestMessage::Option(0), "A"),
            Button::new(TestMessage::Option(1), "B"),
        ]),
        Rect::new(0.0, 0.0, 200.0, 60.0),
    );
    assert_eq!(repeated.resolved_grid_columns(), Some(2));
    let auto_fit = UiTree::layout(
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
fn generated_valid_layouts_have_finite_nonnegative_geometry() {
    for width in [0.0, 1.0, 37.0, 400.0] {
        for height in [0.0, 1.0, 91.0, 300.0] {
            let tree = UiTree::layout(
                Row::new()
                    .gap(3.0)
                    .child(Button::new(TestMessage::Option(0), "A").min_width(0.0))
                    .child(Button::new(TestMessage::Option(1), "B").min_width(0.0)),
                Rect::new(0.0, 0.0, width, height),
            );
            for message in [TestMessage::Option(0), TestMessage::Option(1)] {
                let rect = tree.message_layout_rect(&message).unwrap();
                assert!(rect.origin.x.is_finite() && rect.origin.y.is_finite());
                assert!(rect.size.width.is_finite() && rect.size.height.is_finite());
                assert!(rect.size.width >= 0.0 && rect.size.height >= 0.0);
            }
        }
    }
}

#[test]
fn explicit_identity_survives_sibling_insertion_and_list_reordering() {
    let first = UiTree::layout(
        Column::new().id("list").children([
            Button::new(TestMessage::Option(1), "One").id("one"),
            Button::new(TestMessage::Option(2), "Two").id("two"),
        ]),
        Rect::new(0.0, 0.0, 200.0, 100.0),
    );
    let reordered = UiTree::layout(
        Column::new().id("list").children([
            Button::new(TestMessage::Option(3), "New").id("new"),
            Button::new(TestMessage::Option(2), "Two").id("two"),
            Button::new(TestMessage::Option(1), "One").id("one"),
        ]),
        Rect::new(0.0, 0.0, 200.0, 120.0),
    );
    assert_eq!(
        first.id_for_message(&TestMessage::Option(1)),
        reordered.id_for_message(&TestMessage::Option(1))
    );
    assert_eq!(
        first.id_for_message(&TestMessage::Option(2)),
        reordered.id_for_message(&TestMessage::Option(2))
    );
}

#[test]
fn pointer_keyboard_controller_and_accessibility_share_typed_activation() {
    let tree = UiTree::layout(
        Button::new(TestMessage::Option(7), "Seven").id("seven"),
        Rect::new(0.0, 0.0, 100.0, 42.0),
    );
    let id = tree
        .id_for_message(&TestMessage::Option(7))
        .unwrap()
        .clone();
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

    let controller_tree = || {
        UiTree::layout(
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
    let selected = state.controller_selected().cloned();
    let rebuilt = controller_tree();
    assert_eq!(state.controller_selected(), selected.as_ref());
    assert_eq!(
        rebuilt
            .handle_event(&mut state, UiEvent::ControllerActivate)
            .messages,
        vec![TestMessage::Option(2)]
    );
}

#[test]
fn pointer_drag_selects_visible_text_and_caret_blink_only_changes_paint() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let mut state = UiStateStore::default();
    let first = UiTree::layout_with_state(
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

    let selected_tree = UiTree::layout_with_state(
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
fn document_selection_crosses_text_runs_and_skips_buttons() {
    let build = |state: &mut UiStateStore| {
        UiTree::layout_with_state(
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
        .message_rect(&TestMessage::Named("button"))
        .expect("button bounds");
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
    let build = |state: &mut UiStateStore| {
        UiTree::layout_with_state(
            StyledText::new("open docs", Vec::new())
                .inline_message(5..9, TestMessage::Named("docs")),
            Rect::new(0.0, 0.0, 200.0, 40.0),
            state,
        )
    };
    let mut state = UiStateStore::default();
    let tree = build(&mut state);
    let glyph = tree
        .message_rect(&TestMessage::Named("docs"))
        .expect("inline message glyph");
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
    let tree = UiTree::<TestMessage>::layout(
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
    let tree = UiTree::layout_with_state(
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
    let tree = UiTree::layout_with_diagnostics(
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
fn invalid_indefinite_and_unbalanced_layout_conditions_are_reported() {
    let tree = UiTree::layout_with_diagnostics(
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

    let mut malformed = UiTree::<TestMessage> {
        diagnostics_enabled: true,
        ..UiTree::default()
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
        UiTree::layout_with_diagnostics(
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
        UiTree::layout(
            Text::<TestMessage>::new("disabled"),
            Rect::new(0.0, 0.0, 10.0, 10.0)
        )
        .diagnostics()
        .is_empty()
    );
    let disabled = UiTree::layout(
        Column::<TestMessage>::new().children((0..128).map(|index| Text::new(index.to_string()))),
        Rect::new(0.0, 0.0, 200.0, 400.0),
    );
    assert!(disabled.seen_ids.is_empty());
    assert!(disabled.diagnostic_keys.is_empty());
}

#[test]
fn diagnostic_overlay_rasterizes_at_low_and_high_dpi_without_geometry_changes() {
    let build = || {
        UiTree::layout_with_diagnostics(
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
        let mut renderer = crate::gpu::SdlComponentRenderer::new(
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
        UiTree::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 180.0, 120.0), &mut state);
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
        UiTree::layout_with_state(dropdown(), Rect::new(0.0, 0.0, 180.0, 120.0), &mut state);
    assert!(
        rebuilt
            .message_for_id(&UiId::from("root/choice/option-0"))
            .is_some()
    );

    let scroll =
        |state: &mut UiStateStore| {
            UiTree::layout_with_state(
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
        UiTree::layout_with_state(
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
fn text_field_focus_edit_selection_and_ime_survive_reconstruction_without_click_messages() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let build = |state: &mut UiStateStore, value: &str| {
        UiTree::layout_with_state(
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
    let tree = UiTree::layout_with_state(
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
    let tree = UiTree::layout_with_state(
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
fn multiline_text_field_hit_testing_and_caret_follow_explicit_lines() {
    fn query(value: String) -> TestMessage {
        TestMessage::Query(value)
    }

    let mut state = UiStateStore::default();
    let tree = UiTree::layout_with_state(
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

    let rebuilt = UiTree::layout_with_state(
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
fn ordinary_auto_overflow_owns_scroll_state_and_clips_descendants() {
    let build = |state: &mut UiStateStore| {
        UiTree::layout_with_state(
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
        UiTree::layout_with_state(
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
fn vertical_scroll_emits_the_resulting_offset() {
    fn scrolled(offset: f32) -> TestMessage {
        TestMessage::Volume(offset.round() as u8)
    }

    let mut state = UiStateStore::default();
    let tree = UiTree::layout_with_state(
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
fn horizontal_overflow_policy_scrolls_and_clips_on_its_own_axis() {
    let build = |state: &mut UiStateStore| {
        UiTree::layout_with_state(
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
    assert!(scrolled.accessibility_nodes().iter().all(|node| {
        node.rect.origin.x >= 0.0 && node.rect.origin.x + node.rect.size.width <= 80.0
    }));
}

#[test]
fn horizontal_scrollbar_has_a_gutter_below_content() {
    let tree = UiTree::<()>::layout(
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
    let tree = UiTree::layout(
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
        UiTree::layout_with_state(
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
