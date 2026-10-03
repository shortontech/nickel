//! Release-profile admission for production transient-overlay transitions.

#[path = "../src/release_admission.rs"]
mod release_admission;

use std::time::{Duration, Instant};

use nickel_ui::{
    Application, Button, FrameOverlay, HostBatch, HostEventOutcome, InputModality, Invalidation,
    OverlayAnchor, OverlayId, OverlayMenu, OverlayMenuItem, OverlayStyle, Point, SemanticRole,
    SemanticSelector, SoftwareRenderer, UiEvent, UiHost, UiId, View, ViewContext,
};
use release_admission::AdmissionReport;

const WARMUP_ITERATIONS: usize = 5;
const MEASURED_ITERATIONS: usize = 31;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Stage {
    Open,
    Reposition,
    Submenu,
    Close,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct PhaseWork {
    view_calls: usize,
    nodes_measured: usize,
    nodes_placed: usize,
    paint_commands_emitted: usize,
    paint_fragments_rebuilt: usize,
    paint_fragments_reused: usize,
    damage_rects: usize,
    semantic_nodes_rebuilt: usize,
    semantic_nodes_reused: usize,
    retained_paint_refreshes: usize,
    layout_invalidations: usize,
}

impl PhaseWork {
    fn from_outcome(outcome: &HostEventOutcome) -> Self {
        Self {
            view_calls: outcome.telemetry.view_calls,
            nodes_measured: outcome.telemetry.nodes_measured,
            nodes_placed: outcome.telemetry.nodes_placed,
            paint_commands_emitted: outcome.telemetry.paint_commands_emitted,
            paint_fragments_rebuilt: outcome.telemetry.paint_fragments_rebuilt,
            paint_fragments_reused: outcome.telemetry.paint_fragments_reused,
            damage_rects: outcome.telemetry.paint_damage_rects,
            semantic_nodes_rebuilt: outcome.telemetry.semantic_nodes_rebuilt,
            semantic_nodes_reused: outcome.telemetry.semantic_nodes_reused,
            retained_paint_refreshes: outcome.telemetry.retained_paint_refreshes,
            layout_invalidations: usize::from(outcome.invalidation == Invalidation::Layout),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct OverlayWork {
    open: PhaseWork,
    reposition: PhaseWork,
    submenu: PhaseWork,
    close: PhaseWork,
}

#[derive(Default)]
struct OverlaySamples {
    open: Vec<Duration>,
    reposition: Vec<Duration>,
    submenu: Vec<Duration>,
    close: Vec<Duration>,
    total: Vec<Duration>,
    exact: Option<OverlayWork>,
}

impl OverlaySamples {
    fn record(&mut self, timings: [Duration; 4], work: OverlayWork) {
        if let Some(exact) = self.exact {
            assert_eq!(work, exact, "deterministic overlay work changed");
        } else {
            self.exact = Some(work);
        }
        self.open.push(timings[0]);
        self.reposition.push(timings[1]);
        self.submenu.push(timings[2]);
        self.close.push(timings[3]);
        self.total.push(timings.into_iter().sum());
    }

    fn emit(&self) {
        let work = self.exact.expect("measured overlay work");
        let phases = [work.open, work.reposition, work.submenu, work.close];
        let sum = |field: fn(&PhaseWork) -> usize| phases.iter().map(field).sum();
        AdmissionReport::new("retained_overlay", "open_reposition_submenu_close")
            .metadata("iterations", MEASURED_ITERATIONS)
            .metadata("warmup_iterations", WARMUP_ITERATIONS)
            .metadata("transitions_per_iteration", phases.len())
            .work("view_calls", sum(|work| work.view_calls))
            .work("nodes_measured", sum(|work| work.nodes_measured))
            .work("nodes_placed", sum(|work| work.nodes_placed))
            .work(
                "paint_commands_emitted",
                sum(|work| work.paint_commands_emitted),
            )
            .work(
                "paint_fragments_rebuilt",
                sum(|work| work.paint_fragments_rebuilt),
            )
            .work(
                "paint_fragments_reused",
                sum(|work| work.paint_fragments_reused),
            )
            .work("damage_rects", sum(|work| work.damage_rects))
            .work(
                "semantic_nodes_rebuilt",
                sum(|work| work.semantic_nodes_rebuilt),
            )
            .work(
                "semantic_nodes_reused",
                sum(|work| work.semantic_nodes_reused),
            )
            .work(
                "retained_paint_refreshes",
                sum(|work| work.retained_paint_refreshes),
            )
            .work(
                "layout_invalidations",
                sum(|work| work.layout_invalidations),
            )
            .work("open_nodes_measured", work.open.nodes_measured)
            .work("open_nodes_placed", work.open.nodes_placed)
            .work(
                "open_paint_fragments_rebuilt",
                work.open.paint_fragments_rebuilt,
            )
            .work("open_damage_rects", work.open.damage_rects)
            .work("reposition_nodes_measured", work.reposition.nodes_measured)
            .work("reposition_nodes_placed", work.reposition.nodes_placed)
            .work(
                "reposition_paint_fragments_rebuilt",
                work.reposition.paint_fragments_rebuilt,
            )
            .work("reposition_damage_rects", work.reposition.damage_rects)
            .work("submenu_nodes_measured", work.submenu.nodes_measured)
            .work("submenu_nodes_placed", work.submenu.nodes_placed)
            .work(
                "submenu_paint_fragments_rebuilt",
                work.submenu.paint_fragments_rebuilt,
            )
            .work("submenu_damage_rects", work.submenu.damage_rects)
            .work("close_nodes_measured", work.close.nodes_measured)
            .work("close_nodes_placed", work.close.nodes_placed)
            .work(
                "close_paint_fragments_rebuilt",
                work.close.paint_fragments_rebuilt,
            )
            .work("close_damage_rects", work.close.damage_rects)
            .work(
                "open_semantic_nodes_rebuilt",
                work.open.semantic_nodes_rebuilt,
            )
            .work(
                "reposition_semantic_nodes_rebuilt",
                work.reposition.semantic_nodes_rebuilt,
            )
            .work(
                "submenu_semantic_nodes_rebuilt",
                work.submenu.semantic_nodes_rebuilt,
            )
            .work(
                "close_semantic_nodes_rebuilt",
                work.close.semantic_nodes_rebuilt,
            )
            .timings("open", &self.open)
            .timings("reposition", &self.reposition)
            .timings("submenu", &self.submenu)
            .timings("close", &self.close)
            .timings("total", &self.total)
            .emit();
    }
}

#[derive(Clone, Copy)]
struct OverlayApplication {
    point: Point,
}

impl Default for OverlayApplication {
    fn default() -> Self {
        Self {
            point: Point { x: 48.0, y: 36.0 },
        }
    }
}

impl Application for OverlayApplication {
    type Message = ();

    fn update(&mut self, (): Self::Message) {}

    fn view(&self, _context: ViewContext) -> impl View<Self::Message> {
        Button::new((), "Overlay anchor")
            .id("anchor")
            .width(120.0)
            .height(40.0)
    }

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        vec![FrameOverlay::Menu(
            OverlayMenu::new(
                "admission-menu",
                OverlayAnchor::Point {
                    invocation_target: UiId::from("root/anchor"),
                    point: self.point,
                },
            )
            .semantic_style(OverlayStyle {
                background: 0x202630,
                foreground: 0xe8edf4,
                border: 0x4a5260,
                selected: 0x365070,
                radius: 4,
            })
            .item(OverlayMenuItem::submenu(
                "view",
                "View",
                [
                    OverlayMenuItem::action("details", "Details", ()),
                    OverlayMenuItem::action("compact", "Compact", ()),
                ],
            ))
            .item(OverlayMenuItem::action("close", "Close", ())),
        )]
    }
}

fn anchor_id(host: &UiHost<OverlayApplication>) -> UiId {
    host.query(&SemanticSelector::Role(SemanticRole::Button))
        .into_iter()
        .find(|node| node.name.as_deref() == Some("Overlay anchor"))
        .expect("overlay anchor")
        .id
}

fn open(host: &mut UiHost<OverlayApplication>) -> HostEventOutcome {
    let anchor = anchor_id(host);
    host.open_transient(OverlayId::new("admission-menu"), anchor)
}

fn reposition(host: &mut UiHost<OverlayApplication>) -> HostEventOutcome {
    host.application_mut().point = Point { x: 220.0, y: 112.0 };
    host.step(HostBatch {
        application_changed: true,
        ..HostBatch::default()
    })
}

fn submenu(host: &mut UiHost<OverlayApplication>) -> HostEventOutcome {
    let bounds = host
        .semantic_nodes()
        .into_iter()
        .find(|node| node.name.as_deref() == Some("View"))
        .expect("submenu parent")
        .bounds;
    host.handle_event(UiEvent::PointerMoved(Point {
        x: bounds.origin.x + bounds.size.width / 2.0,
        y: bounds.origin.y + bounds.size.height / 2.0,
    }))
}

fn close(host: &mut UiHost<OverlayApplication>) -> HostEventOutcome {
    host.handle_event(UiEvent::Dismiss)
}

fn replay(stage: Stage) -> UiHost<OverlayApplication> {
    let mut host = UiHost::new(OverlayApplication::default(), 420, 240);
    host.adopt_input_modality(InputModality::Controller);
    host.handle_event(UiEvent::FocusGained);
    host.handle_event(UiEvent::ControllerDown);
    open(&mut host);
    if matches!(stage, Stage::Reposition | Stage::Submenu | Stage::Close) {
        reposition(&mut host);
    }
    if matches!(stage, Stage::Submenu | Stage::Close) {
        submenu(&mut host);
    }
    if stage == Stage::Close {
        close(&mut host);
    }
    host
}

fn assert_cold_equivalent(
    retained: &UiHost<OverlayApplication>,
    cold: &UiHost<OverlayApplication>,
) {
    assert_eq!(retained.layout_snapshot(), cold.layout_snapshot());
    assert_eq!(retained.commands(), cold.commands());
    assert_eq!(retained.semantic_nodes(), cold.semantic_nodes());
    let retained_inspection = retained.inspect();
    let cold_inspection = cold.inspect();
    assert_eq!(
        retained_inspection.open_overlay,
        cold_inspection.open_overlay
    );
    assert_eq!(
        retained_inspection.controller_target,
        cold_inspection.controller_target
    );
    assert_eq!(
        retained_inspection.keyboard_focus,
        cold_inspection.keyboard_focus
    );
    let mut retained_raster = SoftwareRenderer::new(420, 240, 1.0);
    let mut cold_raster = SoftwareRenderer::new(420, 240, 1.0);
    retained.render_software(&mut retained_raster);
    cold.render_software(&mut cold_raster);
    assert_eq!(retained_raster.pixels(), cold_raster.pixels());
}

fn exercise() -> ([Duration; 4], OverlayWork) {
    let mut host = UiHost::new(OverlayApplication::default(), 420, 240);
    host.adopt_input_modality(InputModality::Controller);
    host.handle_event(UiEvent::FocusGained);
    host.handle_event(UiEvent::ControllerDown);

    let started = Instant::now();
    let open_outcome = open(&mut host);
    let open_time = started.elapsed();
    assert_eq!(
        host.inspect().open_overlay,
        Some(OverlayId::new("admission-menu"))
    );
    assert_eq!(
        host.query(&SemanticSelector::Role(SemanticRole::MenuItem))
            .len(),
        4
    );
    assert_cold_equivalent(&host, &replay(Stage::Open));

    let menu_before = host
        .query(&SemanticSelector::Role(SemanticRole::Menu))
        .pop()
        .expect("open menu")
        .bounds;
    let started = Instant::now();
    let reposition_outcome = reposition(&mut host);
    let reposition_time = started.elapsed();
    let menu_after = host
        .query(&SemanticSelector::Role(SemanticRole::Menu))
        .pop()
        .expect("repositioned menu")
        .bounds;
    assert_ne!(menu_before.origin, menu_after.origin);
    assert_cold_equivalent(&host, &replay(Stage::Reposition));

    let started = Instant::now();
    let submenu_outcome = submenu(&mut host);
    let submenu_time = started.elapsed();
    assert_eq!(
        host.query(&SemanticSelector::Role(SemanticRole::Menu))
            .len(),
        2
    );
    assert_cold_equivalent(&host, &replay(Stage::Submenu));

    let started = Instant::now();
    let close_outcome = close(&mut host);
    let close_time = started.elapsed();
    assert!(host.inspect().open_overlay.is_none());
    assert!(
        host.query(&SemanticSelector::Role(SemanticRole::Menu))
            .is_empty()
    );
    assert_cold_equivalent(&host, &replay(Stage::Close));

    assert!(open_outcome.changed);
    // `open_transient` invokes the same canonical rebuild directly, so its
    // exact work counters are populated even though the step-level marker is
    // intentionally not set.
    assert!(!open_outcome.telemetry.rebuilt);
    for outcome in [
        &open_outcome,
        &reposition_outcome,
        &submenu_outcome,
        &close_outcome,
    ] {
        assert!(outcome.changed);
        assert_eq!(outcome.telemetry.view_calls, 1);
        assert!(outcome.telemetry.nodes_measured > 0);
        assert!(outcome.telemetry.nodes_placed > 0);
        assert!(outcome.telemetry.paint_commands_emitted > 0);
        assert!(outcome.telemetry.paint_fragments_rebuilt > 0);
        assert!(outcome.telemetry.semantic_nodes_rebuilt > 0);
        assert_eq!(outcome.telemetry.retained_paint_refreshes, 0);
    }
    assert_eq!(open_outcome.invalidation, Invalidation::Layout);
    assert_eq!(reposition_outcome.invalidation, Invalidation::Layout);
    assert_eq!(submenu_outcome.invalidation, Invalidation::Paint);
    assert_eq!(close_outcome.invalidation, Invalidation::Layout);
    for outcome in [&reposition_outcome, &submenu_outcome, &close_outcome] {
        assert!(outcome.telemetry.rebuilt);
    }

    (
        [open_time, reposition_time, submenu_time, close_time],
        OverlayWork {
            open: PhaseWork::from_outcome(&open_outcome),
            reposition: PhaseWork::from_outcome(&reposition_outcome),
            submenu: PhaseWork::from_outcome(&submenu_outcome),
            close: PhaseWork::from_outcome(&close_outcome),
        },
    )
}

#[test]
#[ignore = "release-profile retained overlay admission benchmark"]
fn overlay_transitions_emit_release_distribution_and_match_cold_oracle() {
    for _ in 0..WARMUP_ITERATIONS {
        exercise();
    }
    let mut samples = OverlaySamples::default();
    for _ in 0..MEASURED_ITERATIONS {
        let (timings, work) = exercise();
        samples.record(timings, work);
    }
    samples.emit();
}

#[test]
fn overlay_admission_fixture_covers_all_transitions() {
    let (_, work) = exercise();
    assert_eq!(work.open.layout_invalidations, 1);
    assert_eq!(work.reposition.layout_invalidations, 1);
    assert_eq!(work.submenu.layout_invalidations, 0);
    assert_eq!(work.close.layout_invalidations, 1);
}
