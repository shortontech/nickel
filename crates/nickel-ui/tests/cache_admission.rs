use std::time::Instant;

#[path = "../src/release_admission.rs"]
mod release_admission;

use nickel_ui::{
    Button, Column, Component, Container, Rect, SemanticRole, SemanticSelector, SoftwareRenderer,
    Text, UiFrame, text_layout_cache_diagnostics,
};
use release_admission::AdmissionReport;

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
