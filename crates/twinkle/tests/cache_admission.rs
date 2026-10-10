use std::time::Instant;

#[path = "../src/release_admission.rs"]
mod release_admission;

use release_admission::AdmissionReport;
use twinkle::{
    Button, Column, Component, Container, Invalidation, Length, Point, Rect, SelectionRegion,
    SemanticRole, SemanticSelector, SoftwareRenderer, Text, UiEvent, UiFrame, UiStateStore,
    text_layout_cache_diagnostics,
};

fn p95(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[(samples.len() * 95 / 100).min(samples.len() - 1)]
}

fn fixture(count: usize) -> UiFrame<usize> {
    UiFrame::layout(
        Column::new().children((0..count).map(|index| {
            Button::new(index, format!("Action {index}"))
                .id(format!("action-{index}"))
                .into_element()
        })),
        Rect::new(0.0, 0.0, 800.0, 1200.0),
    )
}

#[test]
fn repeated_high_cardinality_frame_creation_returns_to_a_stable_bound() {
    let baseline = fixture(500).resource_diagnostics();
    assert_eq!(baseline.retained_build_scratch_bytes, 0);
    for _ in 0..100 {
        let frame = fixture(500);
        let resources = frame.resource_diagnostics();
        assert_eq!(resources.retained_build_scratch_bytes, 0);
        assert_eq!(resources.node_count, baseline.node_count);
        assert_eq!(
            resources.message_binding_count,
            baseline.message_binding_count
        );
        assert_eq!(
            resources.estimated_retained_bytes,
            baseline.estimated_retained_bytes
        );
        drop(frame);
    }
}

#[test]
fn theme_scale_and_locale_churn_stays_within_frame_and_raster_bounds() {
    const LABELS: [&str; 3] = ["Settings", "الإعدادات", "設定"];
    const SCALES: [f32; 3] = [1.0, 1.5, 2.0];
    const THEMES: [u32; 2] = [0x10131a, 0xf4f7ff];
    let mut renderer = SoftwareRenderer::new_pixel_buffer(640, 360, 1.0);
    for generation in 0..180 {
        let label = LABELS[generation % LABELS.len()];
        let scale = SCALES[generation % SCALES.len()];
        let background = THEMES[generation % THEMES.len()];
        let frame = UiFrame::<usize>::layout(
            Container::new()
                .background(background)
                .child(Column::new().children((0..100).map(|index| {
                    Text::new(format!("{label} {index}"))
                        .scale(scale)
                        .into_element()
                }))),
            Rect::new(0.0, 0.0, 640.0, 360.0),
        );
        let resources = frame.resource_diagnostics();
        assert_eq!(resources.retained_build_scratch_bytes, 0);
        assert!(resources.estimated_retained_bytes <= 8 * 1024 * 1024);
        renderer.resize(640, 360, scale);
        renderer.invalidate();
        renderer.render(frame.commands());
        assert_eq!(renderer.pixels().len(), 640 * 360);
    }
}

#[test]
fn semantic_role_name_index_matches_linear_semantics_accessibility_and_raster() {
    let frame = fixture(500);
    let selector = SemanticSelector::RoleAndName {
        role: SemanticRole::Button,
        name: "Action 417".into(),
    };
    let indexed = frame.query(&selector);
    let linear = frame
        .semantic_nodes()
        .into_iter()
        .filter(|node| {
            node.role == Some(SemanticRole::Button) && node.name.as_deref() == Some("Action 417")
        })
        .collect::<Vec<_>>();
    assert_eq!(indexed, linear);
    let target = indexed.first().expect("indexed target");
    let accessibility = frame
        .accessibility_nodes()
        .iter()
        .find(|node| node.id == target.id)
        .expect("matching accessibility target");
    assert_eq!(accessibility.rect, target.bounds);
    assert_eq!(accessibility.label.as_deref(), target.name.as_deref());

    let mut renderer = SoftwareRenderer::new_pixel_buffer(800, 1200, 1.0);
    renderer.render(frame.commands());
    let before_query = renderer.pixels().to_vec();
    assert_eq!(frame.query(&selector), linear);
    renderer.invalidate();
    renderer.render(frame.commands());
    assert_eq!(renderer.pixels(), before_query);
}

#[test]
#[ignore = "release-mode admission measurement"]
fn bounded_semantic_role_name_index_meets_its_admission_budget() {
    let frame = fixture(2_000);
    let selector = SemanticSelector::RoleAndName {
        role: SemanticRole::Button,
        name: "Action 1999".into(),
    };
    let mut samples = Vec::new();
    for _ in 0..100 {
        let started = Instant::now();
        let target = frame.query_unique(&selector).expect("unique final action");
        samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
        assert_eq!(target.name.as_deref(), Some("Action 1999"));
    }
    let query_p95 = p95(samples);
    println!("semantic_role_name_index_2000_p95_us={query_p95:.3}");
    assert!(
        query_p95 <= 100.0,
        "secondary index admission threshold exceeded"
    );
}

#[test]
#[ignore = "release-mode admission measurement"]
fn complete_frame_reconstruction_stays_within_the_frame_work_budget() {
    const NODES: usize = 200;
    const SAMPLES: usize = 100;
    let mut declaration_samples = Vec::with_capacity(SAMPLES);
    let mut resolution_samples = Vec::with_capacity(SAMPLES);
    let mut complete_samples = Vec::with_capacity(SAMPLES);
    let diagnostics_before = text_layout_cache_diagnostics();
    let mut work = None;
    for _ in 0..SAMPLES {
        let complete_started = Instant::now();
        let declaration_started = Instant::now();
        let root = Column::new().children((0..NODES).map(|index| {
            Button::new(index, format!("Action {index}"))
                .id(format!("action-{index}"))
                .into_element()
        }));
        declaration_samples.push(declaration_started.elapsed());
        let resolution_started = Instant::now();
        let frame = UiFrame::layout(root, Rect::new(0.0, 0.0, 800.0, 1200.0));
        resolution_samples.push(resolution_started.elapsed());
        complete_samples.push(complete_started.elapsed());
        assert_eq!(
            frame
                .query(&SemanticSelector::Role(SemanticRole::Button))
                .len(),
            NODES
        );
        let resources = frame.resource_diagnostics();
        assert_eq!(resources.retained_build_scratch_bytes, 0);
        work.get_or_insert(resources);
        assert_eq!(work, Some(resources));
    }
    let diagnostics_after = text_layout_cache_diagnostics();
    let work = work.expect("frame work sample");
    AdmissionReport::new("cold_frame", "200_node_reconstruction")
        .metadata("nodes", NODES)
        .metadata("samples", SAMPLES)
        .work("nodes_measured", work.nodes_measured)
        .work("nodes_placed", work.nodes_placed)
        .work(
            "diagnostic_text_measurements",
            work.diagnostic_text_measurements,
        )
        .work("paint_nodes_executed", work.paint_nodes_executed)
        .work(
            "interaction_nodes_executed",
            work.interaction_nodes_executed,
        )
        .work("semantic_nodes_executed", work.semantic_nodes_executed)
        .work(
            "text_cache_hits",
            diagnostics_after
                .hits
                .saturating_sub(diagnostics_before.hits) as usize,
        )
        .work(
            "text_cache_misses",
            diagnostics_after
                .misses
                .saturating_sub(diagnostics_before.misses) as usize,
        )
        .timings("declaration", &declaration_samples)
        .timings("resolution", &resolution_samples)
        .timings("complete", &complete_samples)
        .emit();
    let reconstruction_p95 =
        release_admission::DurationDistribution::from_samples(&complete_samples)
            .p95
            .as_secs_f64()
            * 1_000.0;
    assert!(
        reconstruction_p95 <= 5.0,
        "focused frame reconstruction exceeded its predeclared 5 ms budget"
    );
}

fn unicode_selection_fixture(text: &str) -> UiFrame<()> {
    UiFrame::layout(
        SelectionRegion::automatic().id("unicode-selection").child(
            Text::new(text)
                .id("unicode-content")
                .width_length(Length::Fill)
                .wrap(true)
                .selection_run_id("unicode-content"),
        ),
        Rect::new(0.0, 0.0, 420.0, 1200.0),
    )
}

fn checksum_bytes(checksum: &mut u64, bytes: &[u8]) {
    // FNV-1a is deliberately fixed rather than process-seeded: the emitted
    // checksum is comparable across admission hosts and test invocations.
    for byte in bytes {
        *checksum ^= u64::from(*byte);
        *checksum = checksum.wrapping_mul(0x100_0000_01b3);
    }
}

fn checksum_invalidation(checksum: &mut u64, invalidation: Invalidation) {
    let (tag, delay) = match invalidation {
        Invalidation::None => (0_u8, 0_u128),
        Invalidation::Paint => (1, 0),
        Invalidation::Layout => (2, 0),
        Invalidation::Scheduled(delay) => (3, delay.as_nanos()),
    };
    checksum_bytes(checksum, &[tag]);
    checksum_bytes(checksum, &delay.to_le_bytes());
}

#[test]
#[ignore = "release-mode long-Unicode selection admission workload"]
fn long_unicode_hit_testing_and_selection_emit_bounded_release_evidence() {
    const CONSTRUCTION_SAMPLES: usize = 31;
    const INTERACTION_SAMPLES: usize = 200;
    const REPETITIONS: usize = 128;
    const WIDTH: usize = 420;
    const HEIGHT: usize = 1200;

    // "office affine" exercises ligature-capable shaping, followed by bidi,
    // a decomposed combining sequence, and one extended ZWJ emoji cluster.
    // Repetition at a narrow width makes the same corpus cross many wraps.
    let unit = "office affine שלום عربي e\u{301} 👩🏽‍💻 ";
    let text = unit.repeat(REPETITIONS);
    let graphemes =
        unicode_segmentation::UnicodeSegmentation::graphemes(text.as_str(), true).count();

    // Warm shared shaping once. The measured construction phase still builds
    // the complete selection document and selectable geometry for every frame.
    let reference = unicode_selection_fixture(&text);
    let expected_work = reference.resource_diagnostics();
    let mut construction = Vec::with_capacity(CONSTRUCTION_SAMPLES);
    for _ in 0..CONSTRUCTION_SAMPLES {
        let started = Instant::now();
        let frame = unicode_selection_fixture(std::hint::black_box(&text));
        construction.push(started.elapsed());
        assert_eq!(frame.resource_diagnostics(), expected_work);
        assert_eq!(frame.selection_region_ids().count(), 1);
    }

    // A separately reconstructed frame is the cold oracle. Compare every
    // public native projection plus software pixels before exercising input.
    let cold = unicode_selection_fixture(&text);
    assert_eq!(reference.commands(), cold.commands());
    assert_eq!(reference.semantic_nodes(), cold.semantic_nodes());
    assert_eq!(reference.accessibility_nodes(), cold.accessibility_nodes());
    let mut retained_raster = SoftwareRenderer::new_pixel_buffer(WIDTH as u32, HEIGHT as u32, 1.0);
    let mut cold_raster = SoftwareRenderer::new_pixel_buffer(WIDTH as u32, HEIGHT as u32, 1.0);
    retained_raster.render(reference.commands());
    cold_raster.render(cold.commands());
    assert_eq!(retained_raster.pixels(), cold_raster.pixels());

    let text_bounds = reference
        .resolved_layout()
        .nodes()
        .iter()
        .find(|node| node.component == "Text")
        .expect("fixture text layout")
        .allocated;
    assert!(text_bounds.size.width > 0.0 && text_bounds.size.height > 0.0);
    let mut points = Vec::with_capacity(INTERACTION_SAMPLES);
    for sample in 0..INTERACTION_SAMPLES * 4 {
        let candidate = Point {
            x: text_bounds.origin.x
                + 1.0
                + ((sample * 97) as f32 % (text_bounds.size.width * 0.75).max(1.0)),
            y: text_bounds.origin.y + 8.0,
        };
        let mut setup_state = UiStateStore::default();
        if reference
            .handle_event(&mut setup_state, UiEvent::PointerPressed(candidate))
            .disposition
            == twinkle::EventDisposition::Handled
        {
            points.push(candidate);
            if points.len() == INTERACTION_SAMPLES {
                break;
            }
        }
    }
    assert_eq!(points.len(), INTERACTION_SAMPLES);
    let mut hit_test = Vec::with_capacity(INTERACTION_SAMPLES);
    let mut selection_update = Vec::with_capacity(INTERACTION_SAMPLES);
    let mut selection_text_construction = Vec::with_capacity(INTERACTION_SAMPLES);
    let mut checksum = 0xcbf2_9ce4_8422_2325_u64;
    let mut selected_bytes = 0_usize;
    let mut handled_hits = 0_usize;
    let mut nonempty_selections = 0_usize;

    for (sample, point) in points.iter().enumerate() {
        let mut state = UiStateStore::default();
        let hit_started = Instant::now();
        let pressed = reference.handle_event(&mut state, UiEvent::PointerPressed(*point));
        hit_test.push(hit_started.elapsed());

        let destination = Point {
            x: points[(sample * 37 + 71) % points.len()].x,
            y: text_bounds.origin.y + (text_bounds.size.height - 2.0).max(1.0),
        };
        let update_started = Instant::now();
        let moved = reference.handle_event(&mut state, UiEvent::PointerMoved(destination));
        selection_update.push(update_started.elapsed());

        handled_hits += usize::from(pressed.disposition == twinkle::EventDisposition::Handled);
        checksum_invalidation(&mut checksum, pressed.invalidation);
        checksum_invalidation(&mut checksum, moved.invalidation);
        let selection_started = Instant::now();
        let selected = reference.selected_text(&state);
        selection_text_construction.push(selection_started.elapsed());
        if let Some(selected) = selected {
            nonempty_selections += 1;
            selected_bytes = selected_bytes.saturating_add(selected.len());
            checksum_bytes(&mut checksum, selected.as_bytes());
        }
    }

    // Replay the exact probes against the independently reconstructed frame.
    // Equal selected text proves hit endpoints and selection updates agree,
    // while the projection/raster checks above cover construction equivalence.
    let mut cold_checksum = 0xcbf2_9ce4_8422_2325_u64;
    let mut cold_selected_bytes = 0_usize;
    for (sample, point) in points.iter().enumerate() {
        let mut state = UiStateStore::default();
        let pressed = cold.handle_event(&mut state, UiEvent::PointerPressed(*point));
        let destination = Point {
            x: points[(sample * 37 + 71) % points.len()].x,
            y: text_bounds.origin.y + (text_bounds.size.height - 2.0).max(1.0),
        };
        let moved = cold.handle_event(&mut state, UiEvent::PointerMoved(destination));
        checksum_invalidation(&mut cold_checksum, pressed.invalidation);
        checksum_invalidation(&mut cold_checksum, moved.invalidation);
        if let Some(selected) = cold.selected_text(&state) {
            cold_selected_bytes = cold_selected_bytes.saturating_add(selected.len());
            checksum_bytes(&mut cold_checksum, selected.as_bytes());
        }
    }
    assert_eq!(checksum, cold_checksum);
    assert_eq!(selected_bytes, cold_selected_bytes);
    assert_eq!(handled_hits, INTERACTION_SAMPLES);
    assert_eq!(nonempty_selections, INTERACTION_SAMPLES);
    assert!(selected_bytes > 0, "selection probes must construct text");

    AdmissionReport::new("text_selection", "long_unicode_wrapped")
        .metadata("construction_samples", CONSTRUCTION_SAMPLES)
        .metadata("interaction_samples", INTERACTION_SAMPLES)
        .metadata("input_bytes", text.len())
        .metadata("input_graphemes", graphemes)
        .metadata("repetitions", REPETITIONS)
        .work("checksum_low32", checksum as u32 as usize)
        .work("cold_equivalent", usize::from(checksum == cold_checksum))
        .work("handled_hits", handled_hits)
        .work("nodes_measured", expected_work.nodes_measured)
        .work("nodes_placed", expected_work.nodes_placed)
        .work("paint_nodes_executed", expected_work.paint_nodes_executed)
        .work(
            "semantic_nodes_executed",
            expected_work.semantic_nodes_executed,
        )
        .work("nonempty_selections", nonempty_selections)
        .work("selected_bytes", selected_bytes)
        .timings("selection_construction", &construction)
        .timings("hit_test", &hit_test)
        .timings("selection_text_construction", &selection_text_construction)
        .timings("selection_update", &selection_update)
        .emit();
}
