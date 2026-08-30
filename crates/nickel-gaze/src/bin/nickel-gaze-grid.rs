use nickel_gaze::{
    cache::acquire_bundle,
    camera::CameraSource,
    contract::{BlinkDetector, BlinkPhase, TrackingState},
    grid::{COLUMNS, GridLayout, GridObservation, GridTracker, ROWS},
    model::GazeModel,
};
use nickel_input::{InputEvent, KeyCode, KeyEdge, PhysicalKey, PointerEvent};
use nickel_ui::{PaintCommand, Rect, SdlCanvasPresenter, TextAlign};
use sdl3::event::Event;
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
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let window = video
        .window("Nickel Gaze Grid", 1120, 720)
        .resizable()
        .high_pixel_density()
        .position_centered()
        .build()?;
    let mut presenter = SdlCanvasPresenter::new(window)?;
    let mut events = sdl.event_pump()?;
    let mut input = nickel_input::sdl::Adapter::default();
    let started = Instant::now();
    let mut tracker = GridTracker::default();
    tracker.arm();
    let mut live: Option<LiveObservation> = None;
    let mut running = true;

    while running {
        for event in events.poll_iter() {
            match &event {
                Event::Quit { .. }
                | Event::Window {
                    win_event: sdl3::event::WindowEvent::CloseRequested,
                    ..
                } => running = false,
                _ => {}
            }
            if let Some(event) = input.normalize(&event) {
                match event {
                    InputEvent::Key(key) if key.edge == KeyEdge::Pressed && !key.repeat => {
                        match key.physical {
                            PhysicalKey::Code(KeyCode::Escape) => running = false,
                            PhysicalKey::Code(KeyCode::Space | KeyCode::KeyR) => tracker.arm(),
                            _ => {}
                        }
                    }
                    InputEvent::Pointer(PointerEvent::Button {
                        edge: KeyEdge::Pressed,
                        ..
                    }) => tracker.arm(),
                    _ => {}
                }
            }
        }
        while let Ok(observation) = receiver.try_recv() {
            tracker.update(
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
                started.elapsed(),
            );
            live = Some(observation);
        }
        let (width, height) = presenter.window().size();
        let commands = scene(
            width as f32,
            height as f32,
            &tracker,
            live.as_ref(),
            started.elapsed(),
        );
        presenter.present_accelerated(&commands, 1.0)?;
        thread::sleep(Duration::from_millis(16));
    }
    Ok(())
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
) -> Vec<PaintCommand> {
    let header = 132.0;
    let layout = GridLayout::fit(width, height, header);
    let cursor = tracker.cursor(now);
    let selected = cursor.map(|point| layout.cell_at(point));
    let mut commands = vec![fill(Rect::new(0.0, 0.0, width, height), BACKGROUND)];
    commands.push(fill(Rect::new(0.0, 0.0, width, header), PANEL));
    commands.push(text(
        Rect::new(20.0, 12.0, width - 40.0, 30.0),
        if tracker.armed() {
            "Look at the center target and blink naturally"
        } else if tracker.neutral().is_some() {
            "Centered — Space, R, or click to recenter"
        } else {
            "Press Space, R, or click to center"
        },
        1.05,
        TEXT,
        true,
    ));
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
    commands.push(text(
        Rect::new(20.0, 48.0, width - 40.0, 24.0),
        &status,
        0.78,
        if live.is_some_and(|sample| sample.state == TrackingState::Tracking) {
            MUTED
        } else {
            WARNING
        },
        false,
    ));
    if let Some(sample) = live {
        commands.push(text(
            Rect::new(20.0, 76.0, width - 40.0, 22.0),
            &format!(
                "red left eye  |  green right eye  |  blue combined    {}  |  {}",
                sample.camera, sample.model
            ),
            0.62,
            MUTED,
            false,
        ));
        let preview_size = 52.0;
        let preview_y = 72.0;
        let left_x = width - preview_size * 2.0 - 28.0;
        commands.push(PaintCommand::Image {
            bounds: Rect::new(left_x, preview_y, preview_size, preview_size),
            id: 901,
            image: Arc::clone(&sample.left_eye_patch),
            high_density: None,
        });
        commands.push(PaintCommand::Image {
            bounds: Rect::new(
                left_x + preview_size + 8.0,
                preview_y,
                preview_size,
                preview_size,
            ),
            id: 902,
            image: Arc::clone(&sample.right_eye_patch),
            high_density: None,
        });
        commands.push(text(
            Rect::new(left_x, 56.0, preview_size, 16.0),
            "L crop",
            0.48,
            TEXT,
            false,
        ));
        commands.push(text(
            Rect::new(left_x + preview_size + 8.0, 56.0, preview_size, 16.0),
            "R crop",
            0.48,
            TEXT,
            false,
        ));
    }

    for row in 0..ROWS {
        for column in 0..COLUMNS {
            let rect = Rect::new(
                layout.bounds.x + column as f32 * layout.cell,
                layout.bounds.y + row as f32 * layout.cell,
                layout.cell,
                layout.cell,
            );
            if selected == Some((column, row)) {
                commands.push(fill(rect, HIGHLIGHT));
            }
            commands.push(PaintCommand::Stroke {
                rect,
                color: GRID,
                width: 2.0,
            });
            commands.push(PaintCommand::Text {
                bounds: rect,
                text: format!("{}{}", (b'A' + column as u8) as char, row + 1),
                scale: 0.72,
                color: if selected == Some((column, row)) {
                    TEXT
                } else {
                    MUTED
                },
                align: TextAlign::Center,
                bold: selected == Some((column, row)),
                wrap: false,
            });
        }
    }
    let center = layout.center();
    commands.push(fill(
        Rect::new(center.0 - 10.0, center.1 - 1.5, 20.0, 3.0),
        ACCENT,
    ));
    commands.push(fill(
        Rect::new(center.0 - 1.5, center.1 - 10.0, 3.0, 20.0),
        ACCENT,
    ));
    if let Some(point) = cursor {
        let position = layout.cursor(point);
        let confidence = live
            .map_or(0.0, |sample| sample.gaze_confidence)
            .clamp(0.0, 1.0);
        let radius = 10.0 + (1.0 - confidence) * 24.0;
        commands.push(PaintCommand::RoundedFill {
            rect: Rect::new(
                position.0 - radius,
                position.1 - radius,
                radius * 2.0,
                radius * 2.0,
            ),
            color: 0x554ca6ff,
            radius,
        });
        commands.push(PaintCommand::RoundedFill {
            rect: Rect::new(position.0 - 5.0, position.1 - 5.0, 10.0, 10.0),
            color: ACCENT,
            radius: 5.0,
        });
    }
    let (left, right) = tracker.eye_cursors(now);
    for (point, color) in [(left, 0xffe84d4d), (right, 0xff42d77d)] {
        if let Some(point) = point {
            let position = layout.cursor(point);
            commands.push(PaintCommand::RoundedFill {
                rect: Rect::new(position.0 - 7.0, position.1 - 7.0, 14.0, 14.0),
                color,
                radius: 7.0,
            });
        }
    }
    commands
}

fn fill(rect: Rect, color: u32) -> PaintCommand {
    PaintCommand::Fill { rect, color }
}

fn text(rect: Rect, value: &str, scale: f32, color: u32, bold: bool) -> PaintCommand {
    PaintCommand::Text {
        bounds: rect,
        text: value.to_owned(),
        scale,
        color,
        align: TextAlign::Start,
        bold,
        wrap: false,
    }
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
