use nickel_gaze::{
    cache::acquire_bundle,
    camera::CameraSource,
    contract::{BlinkDetector, BlinkPhase, TrackingState},
    grid::{COLUMNS, GridLayout, GridObservation, GridTracker, ROWS},
    model::GazeModel,
};
use nickel_ui::{
    Align, AnyView, Application, Button, Column, ComponentBuilderExt, Container, Grid, Image,
    Justify, Row, SemanticRole, Shortcut, Spacer, Text, TextAlign, ViewContext,
};
use std::{
    error::Error,
    sync::{
        Arc,
        mpsc::{self, Receiver},
    },
    thread,
    time::{Duration, Instant},
};

const BACKGROUND: u32 = 0xff121722;
const PANEL: u32 = 0xff1c2432;
const GRID: u32 = 0xff526176;
const TEXT: u32 = 0xffe6edf7;
const MUTED: u32 = 0xff9aa8ba;
const ACCENT: u32 = 0xff4ca6ff;
const HIGHLIGHT: u32 = 0xff254f75;
const WARNING: u32 = 0xffffbf69;

#[derive(Clone, Debug)]
struct LiveObservation {
    state: TrackingState,
    blink: BlinkPhase,
    horizontal: f32,
    vertical: f32,
    left_gaze: (f32, f32),
    right_gaze: (f32, f32),
    left_eye_openness: f32,
    right_eye_openness: f32,
    left_eye_patch: Arc<image::RgbaImage>,
    right_eye_patch: Arc<image::RgbaImage>,
    face_confidence: f32,
    gaze_confidence: f32,
    camera: String,
    model: String,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("nickel-gaze-grid: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let camera = argument("--camera");
    let receiver = start_tracking(camera)?;
    nickel_ui::run(GazeGridApplication::new(receiver))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Message {
    Recenter,
}

pub struct GazeGridApplication {
    receiver: Receiver<LiveObservation>,
    started: Instant,
    tracker: GridTracker,
    live: Option<LiveObservation>,
}

impl GazeGridApplication {
    fn new(receiver: Receiver<LiveObservation>) -> Self {
        let mut tracker = GridTracker::default();
        tracker.arm();
        Self {
            receiver,
            started: Instant::now(),
            tracker,
            live: None,
        }
    }

    fn recenter(&mut self) {
        self.tracker.arm();
    }
}

/// Deterministic, side-effect-free states used by the UI workbench.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GazeGridFixtureState {
    Disconnected,
    Connected,
    Empty,
    Populated,
}

impl GazeGridApplication {
    pub fn fixture(state: GazeGridFixtureState) -> Self {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let mut application = Self::new(receiver);
        application.started = Instant::now();
        application.live = match state {
            GazeGridFixtureState::Disconnected => None,
            GazeGridFixtureState::Connected => {
                Some(fixture_observation(TrackingState::LowConfidence, 0.0, 0.0))
            }
            GazeGridFixtureState::Empty => {
                Some(fixture_observation(TrackingState::FaceLost, 0.0, 0.0))
            }
            GazeGridFixtureState::Populated => {
                let observation = fixture_observation(TrackingState::Tracking, 0.21, -0.18);
                application.tracker.update(
                    GridObservation {
                        state: observation.state,
                        blink: observation.blink,
                        horizontal: observation.horizontal,
                        vertical: observation.vertical,
                        left_gaze: observation.left_gaze,
                        right_gaze: observation.right_gaze,
                        left_eye_openness: observation.left_eye_openness,
                        right_eye_openness: observation.right_eye_openness,
                        confidence: observation.gaze_confidence,
                    },
                    Duration::from_millis(250),
                );
                Some(observation)
            }
        };
        application
    }
}

fn fixture_observation(state: TrackingState, horizontal: f32, vertical: f32) -> LiveObservation {
    let tracking = state == TrackingState::Tracking;
    LiveObservation {
        state,
        blink: BlinkPhase::Open,
        horizontal,
        vertical,
        left_gaze: (horizontal - 0.02, vertical),
        right_gaze: (horizontal + 0.02, vertical),
        left_eye_openness: if tracking { 0.82 } else { 0.0 },
        right_eye_openness: if tracking { 0.79 } else { 0.0 },
        left_eye_patch: Arc::new(image::RgbaImage::new(32, 32)),
        right_eye_patch: Arc::new(image::RgbaImage::new(32, 32)),
        face_confidence: if tracking { 0.94 } else { 0.0 },
        gaze_confidence: if tracking { 0.88 } else { 0.0 },
        camera: "Simulated camera".to_owned(),
        model: "Fixture model".to_owned(),
    }
}

pub struct GazeGridFixtureProvider;

struct GazeGridFixture;

const GAZE_VARIANTS: &[nickel_ui_testkit::FixtureVariant] = &[
    gaze_variant("disconnected", "Disconnected"),
    gaze_variant("connected", "Connected"),
    gaze_variant("empty", "No face detected"),
    gaze_variant("populated", "Tracking gaze"),
];

const fn gaze_variant(id: &'static str, title: &'static str) -> nickel_ui_testkit::FixtureVariant {
    nickel_ui_testkit::FixtureVariant {
        id,
        title,
        viewport: nickel_ui_testkit::ViewportPreset {
            id: "gaze-grid",
            width: 1120,
            height: 720,
        },
        theme: nickel_ui_testkit::FixtureTheme::Dark,
        locale: nickel_ui_testkit::DEFAULT_LOCALE,
        scale: nickel_ui_testkit::DEFAULT_SCALE,
        controller_family: nickel_ui::ControllerFamily::Generic,
        accessibility: nickel_ui_testkit::DEFAULT_ACCESSIBILITY,
    }
}

static GAZE_METADATA: nickel_ui_testkit::FixtureMetadata = nickel_ui_testkit::FixtureMetadata {
    id: "gaze.grid",
    title: "Gaze calibration grid",
    description: "Production gaze-grid UI with deterministic, simulated tracking states.",
    tags: &["gaze", "grid", "accessibility", "controller"],
    source: nickel_ui_testkit::FixtureSource {
        crate_name: "nickel-gaze",
        file: file!(),
        line: line!(),
    },
    variants: GAZE_VARIANTS,
    assets: &[],
    simulated_effects: &[],
};

impl nickel_ui_testkit::Fixture for GazeGridFixture {
    type App = GazeGridApplication;

    fn metadata() -> &'static nickel_ui_testkit::FixtureMetadata {
        &GAZE_METADATA
    }

    fn create() -> Self::App {
        Self::create_variant(&GAZE_VARIANTS[0])
    }

    fn create_variant(variant: &nickel_ui_testkit::FixtureVariant) -> Self::App {
        let state = match variant.id {
            "connected" => GazeGridFixtureState::Connected,
            "empty" => GazeGridFixtureState::Empty,
            "populated" => GazeGridFixtureState::Populated,
            _ => GazeGridFixtureState::Disconnected,
        };
        GazeGridApplication::fixture(state)
    }

    fn surface_size() -> (u32, u32) {
        (1120, 720)
    }

    fn default_activation() -> Option<nickel_ui_testkit::Selector> {
        Some(nickel_ui_testkit::Selector::RoleAndName {
            role: SemanticRole::Button,
            name: "Recenter".to_owned(),
        })
    }
}

impl nickel_ui_testkit::FixtureProvider for GazeGridFixtureProvider {
    fn register(
        &self,
        registry: &mut nickel_ui_testkit::FixtureRegistry,
    ) -> Result<(), nickel_ui_testkit::RegistryError> {
        registry.register::<GazeGridFixture>()
    }
}

impl Application for GazeGridApplication {
    type Message = Message;

    fn update(&mut self, message: Self::Message) {
        match message {
            Message::Recenter => self.recenter(),
        }
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        scene(
            context.viewport.size.width,
            context.viewport.size.height,
            &self.tracker,
            self.live.as_ref(),
            self.started.elapsed(),
        )
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Ok(observation) = self.receiver.try_recv() {
            self.tracker.update(
                GridObservation {
                    state: observation.state,
                    blink: observation.blink,
                    horizontal: observation.horizontal,
                    vertical: observation.vertical,
                    left_gaze: observation.left_gaze,
                    right_gaze: observation.right_gaze,
                    left_eye_openness: observation.left_eye_openness,
                    right_eye_openness: observation.right_eye_openness,
                    confidence: observation.gaze_confidence,
                },
                self.started.elapsed(),
            );
            self.live = Some(observation);
            changed = true;
        }
        changed
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        Some(std::time::Duration::from_millis(16))
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        match shortcut {
            Shortcut::Reload => {
                self.recenter();
                true
            }
            _ => false,
        }
    }

    fn title(&self) -> &str {
        "Nickel Gaze Grid"
    }

    fn initial_size(&self) -> (u32, u32) {
        (1120, 720)
    }
}

fn start_tracking(camera: Option<String>) -> Result<Receiver<LiveObservation>, Box<dyn Error>> {
    let bundle = acquire_bundle(None)?;
    let model_name = format!("{} {}", bundle.manifest.name, bundle.manifest.version);
    let (sender, receiver) = mpsc::sync_channel(2);
    thread::Builder::new()
        .name("nickel-gaze-inference".to_owned())
        .spawn(move || {
            let result = (|| -> Result<(), Box<dyn Error>> {
                let mut model = GazeModel::load(&bundle)?;
                let mut source = CameraSource::open(camera.as_deref())?;
                let camera_name = source.description().to_owned();
                let mut blink = BlinkDetector::default();
                loop {
                    let frame = source.frame()?;
                    let sample =
                        if let Some((observation, patches)) = model.observe_debug(&frame)? {
                            let state = if observation.face_confidence >= 0.60
                                && observation.gaze_confidence >= 0.05
                            {
                                TrackingState::Tracking
                            } else {
                                TrackingState::LowConfidence
                            };
                            LiveObservation {
                                state,
                                blink: blink.update(
                                    observation.left_eye_openness,
                                    observation.right_eye_openness,
                                ),
                                horizontal: observation.horizontal,
                                vertical: observation.vertical,
                                left_gaze: observation.left_gaze,
                                right_gaze: observation.right_gaze,
                                left_eye_openness: observation.left_eye_openness,
                                right_eye_openness: observation.right_eye_openness,
                                left_eye_patch: Arc::new(
                                    image::DynamicImage::ImageRgb8(patches.left).into_rgba8(),
                                ),
                                right_eye_patch: Arc::new(
                                    image::DynamicImage::ImageRgb8(patches.right).into_rgba8(),
                                ),
                                face_confidence: observation.face_confidence,
                                gaze_confidence: observation.gaze_confidence,
                                camera: camera_name.clone(),
                                model: model_name.clone(),
                            }
                        } else {
                            LiveObservation {
                                state: TrackingState::FaceLost,
                                blink: BlinkPhase::Open,
                                horizontal: 0.0,
                                vertical: 0.0,
                                left_gaze: (0.0, 0.0),
                                right_gaze: (0.0, 0.0),
                                left_eye_openness: 0.0,
                                right_eye_openness: 0.0,
                                left_eye_patch: Arc::new(image::RgbaImage::new(32, 32)),
                                right_eye_patch: Arc::new(image::RgbaImage::new(32, 32)),
                                face_confidence: 0.0,
                                gaze_confidence: 0.0,
                                camera: camera_name.clone(),
                                model: model_name.clone(),
                            }
                        };
                    if sender.send(sample).is_err() {
                        break;
                    }
                }
                Ok(())
            })();
            if let Err(error) = result {
                eprintln!("nickel-gaze-grid inference stopped: {error}");
            }
        })?;
    Ok(receiver)
}

fn scene(
    width: f32,
    height: f32,
    tracker: &GridTracker,
    live: Option<&LiveObservation>,
    now: Duration,
) -> impl nickel_ui::View<Message> {
    let header = 132.0;
    let layout = GridLayout::fit(width, height, header);
    let cursor = tracker.cursor(now);
    let selected = cursor.map(|point| layout.cell_at(point));
    let title = Text::new(if tracker.armed() {
        "Look at the center target and blink naturally"
    } else if tracker.neutral().is_some() {
        "Centered — use Recenter or Ctrl+R to recalibrate"
    } else {
        "Use Recenter or Ctrl+R to begin calibration"
    })
    .height(30.0)
    .scale(1.05)
    .color(TEXT)
    .bold(true);
    let status = live.map_or_else(
        || "Acquiring camera and model…".to_owned(),
        |sample| {
            let cell = selected
                .map(|(column, row)| format!("{}{}", (b'A' + column as u8) as char, row + 1))
                .unwrap_or_else(|| "--".to_owned());
            format!(
                "{:?}  face {:.2}  gaze {:.2}  h {:+.3}  v {:+.3}  cell {}  blink {:?}",
                sample.state,
                sample.face_confidence,
                sample.gaze_confidence,
                sample.horizontal,
                sample.vertical,
                cell,
                sample.blink
            )
        },
    );
    let status = Text::new(status).height(24.0).scale(0.78).color(
        if live.is_some_and(|sample| sample.state == TrackingState::Tracking) {
            MUTED
        } else {
            WARNING
        },
    );
    let mut header_details = Column::new()
        .child(title)
        .child(Spacer::vertical(6.0))
        .child(status);
    if let Some(sample) = live {
        header_details = header_details.child(Spacer::vertical(4.0)).child(
            Text::new(format!(
                "red left eye  |  green right eye  |  blue combined    {}  |  {}",
                sample.camera, sample.model
            ))
            .height(22.0)
            .scale(0.62)
            .color(MUTED),
        );
    }
    let reserved_controls = if live.is_some() { 250.0 } else { 110.0 };
    let mut header_row = Row::new()
        .gap(8.0)
        .child(header_details.width((width - reserved_controls).max(240.0)))
        .child(
            Button::new(Message::Recenter, "Recenter")
                .id("recenter")
                .background(HIGHLIGHT)
                .border(ACCENT, 1.0),
        );
    if let Some(sample) = live {
        let preview_size = 52.0;
        let preview = |label, id, patch| {
            Column::new()
                .child(Text::new(label).height(16.0).scale(0.48).color(TEXT))
                .child(
                    Image::new(id, patch)
                        .width(preview_size)
                        .height(preview_size),
                )
        };
        header_row = header_row.child(
            Row::new()
                .gap(8.0)
                .child(Column::new().child(Spacer::vertical(44.0)).child(preview(
                    "L crop",
                    901,
                    Arc::clone(&sample.left_eye_patch),
                )))
                .child(Column::new().child(Spacer::vertical(44.0)).child(preview(
                    "R crop",
                    902,
                    Arc::clone(&sample.right_eye_patch),
                ))),
        );
    }

    let mut cells = Vec::with_capacity(ROWS * COLUMNS);
    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let active = selected == Some((column, row));
            let label = format!("{}{}", (b'A' + column as u8) as char, row + 1);
            cells.push(
                Container::new()
                    .id(format!("cell-{label}"))
                    .semantic_role(SemanticRole::GridCell)
                    .accessibility_label(format!("Calibration cell {label}"))
                    .width(layout.cell)
                    .height(layout.cell)
                    .background(if active { HIGHLIGHT } else { BACKGROUND })
                    .border(GRID, 2.0)
                    .child(
                        Text::new(label)
                            .scale(0.72)
                            .align(TextAlign::Center)
                            .color(if active { TEXT } else { MUTED })
                            .bold(active)
                            .fill_width()
                            .height(layout.cell),
                    ),
            );
        }
    }
    let grid_size = layout.cell * COLUMNS as f32;
    let base = Grid::fixed(COLUMNS)
        .id("calibration-grid")
        .semantic_role(SemanticRole::Grid)
        .width(grid_size)
        .height(grid_size)
        .children(cells);
    let mut layers = Column::new().gap(-grid_size).child(base);
    layers = layers.child(marker_layer(
        layout.cell,
        (grid_size / 2.0, grid_size / 2.0),
        20.0,
        3.0,
        ACCENT,
        false,
    ));
    layers = layers.child(marker_layer(
        layout.cell,
        (grid_size / 2.0, grid_size / 2.0),
        3.0,
        20.0,
        ACCENT,
        false,
    ));
    if let Some(point) = cursor {
        let position = layout.cursor(point);
        let local = (position.0 - layout.bounds.x, position.1 - layout.bounds.y);
        let confidence = live
            .map_or(0.0, |sample| sample.gaze_confidence)
            .clamp(0.0, 1.0);
        let radius = 10.0 + (1.0 - confidence) * 24.0;
        layers = layers
            .child(marker_layer(
                layout.cell,
                local,
                radius * 2.0,
                radius * 2.0,
                0x554ca6ff,
                true,
            ))
            .child(marker_layer(layout.cell, local, 10.0, 10.0, ACCENT, true));
    }
    let (left, right) = tracker.eye_cursors(now);
    for (point, color) in [(left, 0xffe84d4d), (right, 0xff42d77d)] {
        if let Some(point) = point {
            let position = layout.cursor(point);
            layers = layers.child(marker_layer(
                layout.cell,
                (position.0 - layout.bounds.x, position.1 - layout.bounds.y),
                14.0,
                14.0,
                color,
                true,
            ));
        }
    }
    let content_height = (height - header).max(0.0);
    Column::new()
        .width(width)
        .height(height)
        .background(BACKGROUND)
        .child(
            Container::new()
                .height(header)
                .padding(nickel_ui::Insets {
                    top: 12.0,
                    right: 20.0,
                    bottom: 12.0,
                    left: 20.0,
                })
                .background(PANEL)
                .child(header_row),
        )
        .child(
            Row::new()
                .height(content_height)
                .align_items(Align::Center)
                .justify_content(Justify::Center)
                .child(layers.width(grid_size).height(grid_size)),
        )
}

fn marker_layer(
    cell: f32,
    point: (f32, f32),
    width: f32,
    height: f32,
    color: u32,
    round: bool,
) -> Grid<Message> {
    let column = (point.0 / cell).floor().clamp(0.0, COLUMNS as f32 - 1.0) as usize;
    let row = (point.1 / cell).floor().clamp(0.0, ROWS as f32 - 1.0) as usize;
    let local_x = point.0 - column as f32 * cell;
    let local_y = point.1 - row as f32 * cell;
    let mut cells = Vec::with_capacity(ROWS * COLUMNS);
    for index in 0..ROWS * COLUMNS {
        let marker: AnyView<Message> = if index == row * COLUMNS + column {
            let shape = Container::new()
                .width(width)
                .height(height)
                .background(color)
                .radius(if round { width.min(height) / 2.0 } else { 0.0 })
                .accessibility_hidden(true);
            AnyView::new(
                Column::new()
                    .child(Spacer::vertical((local_y - height / 2.0).max(0.0)))
                    .child(
                        Row::new()
                            .child(Spacer::fixed((local_x - width / 2.0).max(0.0)))
                            .child(shape),
                    ),
            )
        } else {
            AnyView::new(Spacer::fixed(cell))
        };
        cells.push(marker);
    }
    Grid::fixed(COLUMNS)
        .width(cell * COLUMNS as f32)
        .height(cell * ROWS as f32)
        .children(cells)
}

fn argument(name: &str) -> Option<String> {
    let mut arguments = std::env::args().skip(1);
    while let Some(argument) = arguments.next() {
        if argument == name {
            return arguments.next();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_ui::{ActionKind, SemanticAction, SemanticSelector, UiHost};
    use nickel_ui_testkit::{ActivationVia, FixtureProvider, FixtureRegistry};

    fn host() -> UiHost<GazeGridApplication> {
        let (_sender, receiver) = mpsc::sync_channel(1);
        UiHost::new(GazeGridApplication::new(receiver), 1120, 720)
    }

    fn observation() -> LiveObservation {
        LiveObservation {
            state: TrackingState::Tracking,
            blink: BlinkPhase::Open,
            horizontal: 0.1,
            vertical: -0.1,
            left_gaze: (0.1, -0.1),
            right_gaze: (0.1, -0.1),
            left_eye_openness: 0.8,
            right_eye_openness: 0.8,
            left_eye_patch: Arc::new(image::RgbaImage::new(2, 2)),
            right_eye_patch: Arc::new(image::RgbaImage::new(2, 2)),
            face_confidence: 0.9,
            gaze_confidence: 0.9,
            camera: "test camera".to_owned(),
            model: "test model".to_owned(),
        }
    }

    #[test]
    fn scene_exposes_the_calibration_grid_and_each_named_cell_semantically() {
        let host = host();
        let nodes = host.semantic_nodes();

        assert!(nodes.iter().any(|node| {
            node.role == Some(SemanticRole::Grid) && node.id.as_str().ends_with("/calibration-grid")
        }));
        let labels = nodes
            .iter()
            .filter(|node| node.role == Some(SemanticRole::GridCell))
            .filter_map(|node| node.name.as_deref())
            .collect::<Vec<_>>();
        assert_eq!(labels.len(), ROWS * COLUMNS);
        assert!(labels.contains(&"Calibration cell A1"));
        assert!(labels.contains(&"Calibration cell C3"));
    }

    #[test]
    fn scene_resolves_through_declarative_ui_authority() {
        let host = host();

        assert!(!host.commands().is_empty());
        assert!(host.inspect().diagnostics.is_empty());
    }

    #[test]
    fn recenter_is_a_semantic_host_action() {
        let mut host = host();
        let button = host
            .query_unique(&SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Recenter".to_owned(),
            })
            .expect("unique recenter button");

        let outcome =
            host.perform_semantic_action(button.id, SemanticAction::Invoke(ActionKind::Activate));

        assert!(outcome.changed);
        assert!(host.application_mut().tracker.armed());
    }

    #[test]
    fn host_resize_reflows_the_calibration_grid_from_view_context() {
        let mut host = host();
        let initial = host
            .query_unique(&SemanticSelector::Role(SemanticRole::Grid))
            .expect("calibration grid");

        host.resize(700, 500);
        let resized = host
            .query_unique(&SemanticSelector::Role(SemanticRole::Grid))
            .expect("resized calibration grid");

        assert!(resized.bounds.size.width < initial.bounds.size.width);
        assert!(resized.bounds.size.height < initial.bounds.size.height);
    }

    #[test]
    fn host_poll_is_the_only_bridge_for_gaze_observations() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut host = UiHost::new(GazeGridApplication::new(receiver), 1120, 720);
        let initial_generation = host.inspect().frame_generation;
        sender.send(observation()).expect("send observation");

        assert!(host.poll());
        assert!(host.inspect().frame_generation > initial_generation);
        assert_eq!(
            host.application_mut()
                .live
                .as_ref()
                .map(|live| live.camera.as_str()),
            Some("test camera")
        );
    }

    #[test]
    fn workbench_provider_uses_deterministic_production_views_for_every_state() {
        let mut registry = FixtureRegistry::new();
        GazeGridFixtureProvider.register(&mut registry).unwrap();
        let entries = registry.finish();
        let [entry] = entries.as_slice() else {
            panic!("one gaze fixture")
        };

        assert!(entry.metadata.assets.is_empty());
        assert!(entry.metadata.simulated_effects.is_empty());
        assert_eq!(entry.metadata.variants.len(), 4);
        for variant in entry.metadata.variants {
            let session = entry.open_variant(variant.id).unwrap();
            assert!(session.inspect().diagnostics.is_empty());
            assert!(session.inspect().overlay_failures.is_empty());
            assert!(
                session
                    .accessibility_nodes()
                    .iter()
                    .filter(|node| node.interactive)
                    .all(|node| node.label.as_deref().is_some_and(|label| !label.is_empty()))
            );
            assert_eq!(session.render(1.0), session.render(1.0));
        }
    }

    #[test]
    fn gaze_fixture_recenter_is_keyboard_controller_and_accessibility_reachable() {
        let mut registry = FixtureRegistry::new();
        GazeGridFixtureProvider.register(&mut registry).unwrap();
        let entry = registry.finish()[0];

        for via in [
            ActivationVia::Keyboard,
            ActivationVia::Controller,
            ActivationVia::Accessibility,
        ] {
            let mut session = entry.open_variant("populated").unwrap();
            session.activate(via).unwrap();
        }
    }
}
