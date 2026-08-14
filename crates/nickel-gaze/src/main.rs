use nickel_gaze::{
    cache::acquire_bundle,
    camera::CameraSource,
    contract::{BlinkDetector, GazeSample, TrackingState},
    model::GazeModel,
};
use std::{
    env,
    error::Error,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
struct Arguments {
    list_cameras: bool,
    camera: Option<String>,
    model_directory: Option<PathBuf>,
    samples: Option<u64>,
    rate_hz: u32,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("nickel-gaze-probe: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let arguments = parse_arguments(env::args().skip(1))?;
    if arguments.list_cameras {
        for camera in CameraSource::enumerate()? {
            println!("{}\t{}\t{}", camera.index, camera.name, camera.description);
        }
        return Ok(());
    }

    let bundle = acquire_bundle(arguments.model_directory.as_deref())?;
    eprintln!(
        "model={} version={} revision={} license={} cache={} downloaded={}",
        bundle.manifest.name,
        bundle.manifest.version,
        bundle.manifest.source_revision,
        bundle.manifest.license,
        bundle.directory.display(),
        if bundle.downloaded.is_empty() {
            "none".to_owned()
        } else {
            bundle.downloaded.join(",")
        }
    );
    eprintln!("license_notice={}", bundle.manifest.license_file);
    let model_started = Instant::now();
    let mut model = GazeModel::load(&bundle)?;
    eprintln!(
        "model_ready_ms={:.2}",
        model_started.elapsed().as_secs_f64() * 1000.0
    );

    let mut camera = CameraSource::open(arguments.camera.as_deref())?;
    eprintln!("camera={}", camera.description());
    eprintln!("privacy=frames remain in memory and are not recorded or uploaded");

    let running = Arc::new(AtomicBool::new(true));
    let signal = Arc::clone(&running);
    ctrlc::set_handler(move || signal.store(false, Ordering::SeqCst))?;

    let start = Instant::now();
    let interval = Duration::from_secs_f64(1.0 / f64::from(arguments.rate_hz));
    let mut sequence = 0_u64;
    let mut blink = BlinkDetector::default();
    let mut prior_state = TrackingState::Acquiring;
    eprintln!("state=acquiring");

    while running.load(Ordering::SeqCst)
        && arguments.samples.is_none_or(|maximum| sequence < maximum)
    {
        let iteration = Instant::now();
        let captured = Instant::now();
        let frame = camera.frame()?;
        let inference_started = Instant::now();
        let observation = model.observe(&frame)?;
        let inference_ms = inference_started.elapsed().as_secs_f64() * 1000.0;
        let sample = match observation {
            Some(observation) => {
                let state =
                    if observation.face_confidence < 0.60 || observation.gaze_confidence < 0.05 {
                        TrackingState::LowConfidence
                    } else {
                        TrackingState::Tracking
                    };
                GazeSample {
                    sequence,
                    timestamp_ms: start.elapsed().as_millis(),
                    state,
                    face_confidence: observation.face_confidence,
                    left_eye_openness: observation.left_eye_openness,
                    right_eye_openness: observation.right_eye_openness,
                    blink: blink.update(
                        observation.left_eye_openness,
                        observation.right_eye_openness,
                    ),
                    horizontal: observation.horizontal,
                    vertical: observation.vertical,
                    gaze_confidence: observation.gaze_confidence,
                    frame_age_ms: captured.elapsed().as_millis(),
                    inference_ms,
                }
            }
            None => GazeSample {
                sequence,
                timestamp_ms: start.elapsed().as_millis(),
                state: TrackingState::FaceLost,
                face_confidence: 0.0,
                left_eye_openness: 0.0,
                right_eye_openness: 0.0,
                blink: blink.update(1.0, 1.0),
                horizontal: 0.0,
                vertical: 0.0,
                gaze_confidence: 0.0,
                frame_age_ms: captured.elapsed().as_millis(),
                inference_ms,
            },
        };
        if sample.state != prior_state {
            eprintln!("state={:?}", sample.state);
            prior_state = sample.state;
        }
        println!("{}", serde_json::to_string(&sample)?);
        sequence += 1;
        if let Some(remaining) = interval.checked_sub(iteration.elapsed()) {
            thread::sleep(remaining);
        }
    }
    eprintln!("stopped samples={sequence}");
    Ok(())
}

fn parse_arguments(arguments: impl IntoIterator<Item = String>) -> Result<Arguments, String> {
    let mut parsed = Arguments {
        rate_hz: 10,
        ..Arguments::default()
    };
    let mut arguments = arguments.into_iter();
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--list-cameras" => parsed.list_cameras = true,
            "--camera" => parsed.camera = Some(next_value(&mut arguments, "--camera")?),
            "--model-dir" => {
                parsed.model_directory =
                    Some(PathBuf::from(next_value(&mut arguments, "--model-dir")?));
            }
            "--samples" => {
                parsed.samples = Some(
                    next_value(&mut arguments, "--samples")?
                        .parse()
                        .map_err(|_| "--samples requires a positive integer".to_owned())?,
                );
            }
            "--rate-hz" => {
                parsed.rate_hz = next_value(&mut arguments, "--rate-hz")?
                    .parse()
                    .map_err(|_| "--rate-hz requires an integer from 1 through 60".to_owned())?;
                if !(1..=60).contains(&parsed.rate_hz) {
                    return Err("--rate-hz requires an integer from 1 through 60".to_owned());
                }
            }
            "--help" | "-h" => {
                println!(
                    "usage: nickel-gaze-probe [--list-cameras] [--camera INDEX|NAME] [--model-dir PATH] [--samples N] [--rate-hz N]"
                );
                std::process::exit(0);
            }
            _ => return Err(format!("unexpected argument {argument}")),
        }
    }
    Ok(parsed)
}

fn next_value(
    arguments: &mut impl Iterator<Item = String>,
    option: &str,
) -> Result<String, String> {
    arguments
        .next()
        .ok_or_else(|| format!("{option} requires a value"))
}

#[cfg(test)]
mod tests {
    use super::parse_arguments;

    #[test]
    fn parses_bounded_probe_options() {
        let arguments = parse_arguments([
            "--camera".to_owned(),
            "Facecam".to_owned(),
            "--samples".to_owned(),
            "3".to_owned(),
            "--rate-hz".to_owned(),
            "5".to_owned(),
        ])
        .expect("arguments should parse");
        assert_eq!(arguments.camera.as_deref(), Some("Facecam"));
        assert_eq!(arguments.samples, Some(3));
        assert_eq!(arguments.rate_hz, 5);
    }

    #[test]
    fn rejects_unbounded_output_rate() {
        assert!(parse_arguments(["--rate-hz".to_owned(), "100".to_owned()]).is_err());
    }
}
