pub mod diagnostics;

use std::{
    env,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use tracing_subscriber::{EnvFilter, fmt, prelude::*};

const MAX_LOG_BYTES: u64 = 5 * 1024 * 1024;

fn diagnostic_metadata_allowed(metadata: &tracing::Metadata<'_>) -> bool {
    // These dependencies log entire RPC messages, HTTP/2 headers/frames, or
    // rendered text, including input text, image data and credentials. Reject them before
    // fields are collected, independently of user-selected verbosity. Nickel
    // owns the bounded counters and coarse diagnostics for these operations.
    let target = metadata.target();
    // Smithay instruments native input method arguments, including keycodes,
    // edges, pointer coordinates and touch data. A verbose dependency filter
    // must not turn those arguments into retained input history.
    if [
        "smithay::input",
        "smithay::wayland::text_input",
        "smithay::wayland::input_method",
    ]
    .iter()
    .any(|prefix| {
        target == *prefix
            || target
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with("::"))
    }) {
        return false;
    }
    !matches!(
        metadata.target().split("::").next(),
        Some("rmcp" | "h2" | "cosmic_text")
    )
}

pub fn init(application: &str) -> io::Result<PathBuf> {
    let directory = log_directory()?;
    fs::create_dir_all(&directory)?;
    let path = directory.join(format!("{application}.log"));
    let file = BoundedLog::open(path.clone(), MAX_LOG_BYTES)?;
    let filter = EnvFilter::try_from_env("NICKEL_LOG")
        .unwrap_or_else(|_| EnvFilter::new("nickel=debug,warn"));
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::filter::filter_fn(
            diagnostic_metadata_allowed,
        ))
        .with(diagnostics::layer().with_filter(tracing_subscriber::filter::LevelFilter::WARN))
        .with(
            fmt::layer()
                .with_ansi(false)
                .with_target(true)
                .with_writer(Mutex::new(file))
                .with_filter(filter),
        );
    if subscriber.try_init().is_ok() {
        diagnostics::mark_initialized();
    }
    install_panic_logging();
    tracing::info!(application, path = %path.display(), "logging initialized");
    Ok(path)
}

fn log_directory() -> io::Result<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        if let Some(local_app_data) = env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local_app_data).join("Nickel").join("logs"));
        }
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "LOCALAPPDATA is not set",
        ))
    }
    #[cfg(not(target_os = "windows"))]
    {
        let state_home = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .or_else(|| {
                env::var_os("HOME").map(|home| PathBuf::from(home).join(".local").join("state"))
            })
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "XDG_STATE_HOME and HOME are not set",
                )
            })?;
        Ok(state_home.join("nickel").join("logs"))
    }
}

fn rotate_if_needed(path: &Path) -> io::Result<()> {
    if path
        .metadata()
        .is_ok_and(|metadata| metadata.len() >= MAX_LOG_BYTES)
    {
        let previous = path.with_extension("log.previous");
        if previous.exists() {
            fs::remove_file(&previous)?;
        }
        fs::rename(path, previous)?;
    }
    Ok(())
}

// Bound retention throughout a running session, including a single oversized
// diagnostic. Closing before renaming also permits rotation on Windows.
struct BoundedLog {
    path: PathBuf,
    file: Option<fs::File>,
    bytes: u64,
    limit: u64,
}

impl BoundedLog {
    fn open(path: PathBuf, limit: u64) -> io::Result<Self> {
        if limit == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "zero log limit",
            ));
        }
        rotate_if_needed(&path)?;
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let bytes = file.metadata()?.len();
        Ok(Self {
            path,
            file: Some(file),
            bytes,
            limit,
        })
    }

    fn rotate(&mut self) -> io::Result<()> {
        if let Some(mut file) = self.file.take() {
            file.flush()?;
            drop(file);
        }
        let previous = self.path.with_extension("log.previous");
        match fs::remove_file(&previous) {
            Ok(()) => (),
            Err(error) if error.kind() == io::ErrorKind::NotFound => (),
            Err(error) => return Err(error),
        }
        fs::rename(&self.path, previous)?;
        self.bytes = 0;
        Ok(())
    }
}

impl Write for BoundedLog {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if self.bytes >= self.limit {
            self.rotate()?;
        }
        if self.file.is_none() {
            self.file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
        }
        let count = bytes.len().min((self.limit - self.bytes) as usize);
        let written = self.file.as_mut().unwrap().write(&bytes[..count])?;
        self.bytes += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.as_mut().map_or(Ok(()), Write::flush)
    }
}

fn install_panic_logging() {
    // Panic payloads and their Display implementation can contain arbitrary
    // input, credentials, or diagnostic response data. Neither inspect them nor
    // invoke a previous hook that could print them outside the collector.
    std::panic::set_hook(Box::new(|information| {
        let location = information.location();
        let file = location.map_or("unknown", |location| location.file());
        let line = location.map_or(0, |location| location.line());
        let column = location.map_or(0, |location| location.column());
        tracing::error!(file, line, column, "application panicked");
        // Preserve a coarse crash report even when file logging is filtered or
        // unavailable. A closed stderr must not trigger a second panic.
        let _ = writeln!(
            io::stderr().lock(),
            "application panicked at {file}:{line}:{column}"
        );
    }));
}

#[cfg(test)]
mod tests {
    use super::rotate_if_needed;
    use std::{fs, io::Write};

    #[test]
    fn panic_hook_retains_location_without_collecting_payload_or_calling_old_hook() {
        const CHILD_DIRECTORY: &str = "NICKEL_LOGGING_PANIC_TEST_DIRECTORY";
        const PAYLOAD: &str = "PRIVATE_PANIC_INPUT_7f92a5";
        if std::env::var_os(CHILD_DIRECTORY).is_some() {
            std::panic::set_hook(Box::new(|information| {
                eprintln!("OLD_HOOK {information}");
            }));
            super::init("panic-fixture").unwrap();
            panic!("{PAYLOAD}");
        }
        let directory = tempfile::tempdir().unwrap();
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "tests::panic_hook_retains_location_without_collecting_payload_or_calling_old_hook",
                "--nocapture",
            ])
            .env(CHILD_DIRECTORY, directory.path())
            .env("XDG_STATE_HOME", directory.path())
            .env("LOCALAPPDATA", directory.path())
            .env("NICKEL_LOG", "trace")
            .output()
            .unwrap();
        assert!(!output.status.success(), "child must actually panic");
        #[cfg(target_os = "windows")]
        let log = directory.path().join("Nickel/logs/panic-fixture.log");
        #[cfg(not(target_os = "windows"))]
        let log = directory.path().join("nickel/logs/panic-fixture.log");
        let log = fs::read_to_string(log).unwrap();
        let stderr = String::from_utf8(output.stderr).unwrap();
        let stdout = String::from_utf8(output.stdout).unwrap();
        for sink in [&log, &stderr, &stdout] {
            assert!(!sink.contains(PAYLOAD));
            assert!(!sink.contains("OLD_HOOK"));
        }
        assert!(log.contains("application panicked"));
        assert!(log.contains("lib.rs"));
        assert!(stderr.contains("application panicked at"));
        assert!(stderr.contains("lib.rs:"));
    }

    #[test]
    fn running_log_bounds_both_files_even_for_an_oversized_record() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nickel.log");
        let mut writer = super::BoundedLog::open(path.clone(), 8).unwrap();
        writer.write_all(b"12345678").unwrap();
        writer.write_all(b"abcdefghABCDEFGHend").unwrap();
        writer.flush().unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"end");
        assert_eq!(
            fs::read(path.with_extension("log.previous")).unwrap(),
            b"ABCDEFGH"
        );
        writer.write_all(b"").unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"end");
    }

    #[test]
    fn trace_overrides_cannot_collect_rpc_payloads_or_transport_headers() {
        use std::sync::{
            Arc, Mutex,
            atomic::{AtomicUsize, Ordering},
        };
        use tracing_subscriber::prelude::*;

        #[derive(Clone)]
        struct Writer(Arc<Mutex<Vec<u8>>>);
        impl Write for Writer {
            fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(bytes);
                Ok(bytes.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let bytes = Arc::new(Mutex::new(Vec::new()));
        let output = bytes.clone();
        let subscriber = tracing_subscriber::registry()
            .with(tracing_subscriber::filter::filter_fn(
                super::diagnostic_metadata_allowed,
            ))
            .with(tracing_subscriber::EnvFilter::new(
                "trace,rmcp::service=trace,h2::proto::connection=trace",
            ))
            .with(
                tracing_subscriber::fmt::layer()
                    .without_time()
                    .with_ansi(false)
                    .with_writer(move || Writer(output.clone())),
            );
        let evaluated = AtomicUsize::new(0);
        tracing::subscriber::with_default(subscriber, || {
            tracing::debug!(target: "rmcp::service", request = ?{
                evaluated.fetch_add(1, Ordering::SeqCst);
                "PRIVATE_TYPED_TEXT"
            }, "received request");
            tracing::trace!(target: "rmcp::transport::streamable_http_server", response = "PRIVATE_IMAGE_DATA");
            tracing::error!(target: "rmcp", error = "PRIVATE_CREDENTIAL");
            tracing::trace!(target: "cosmic_text::shape", text = ?{
                evaluated.fetch_add(1, Ordering::SeqCst);
                "PRIVATE_RENDERED_TEXT"
            }, "shaping text");
            tracing::trace!(target: "h2::proto::connection", headers = ?{
                evaluated.fetch_add(1, Ordering::SeqCst);
                "Bearer PRIVATE_TOKEN"
            }, "recv HEADERS");
            let _span = tracing::trace_span!(target: "smithay::input::keyboard", "input", keycode = ?{
                evaluated.fetch_add(1, Ordering::SeqCst);
                "PRIVATE_KEYCODE"
            });
            tracing::trace!(target: "smithay::input::pointer", event = ?{
                evaluated.fetch_add(1, Ordering::SeqCst);
                "PRIVATE_POINTER_PAYLOAD"
            });
            tracing::debug!(target: "smithay::wayland::text_input", surrounding_text = ?{
                evaluated.fetch_add(1, Ordering::SeqCst);
                "PRIVATE_SURROUNDING_TEXT"
            });
            tracing::debug!(target: "smithay::wayland::input_method", text = ?{
                evaluated.fetch_add(1, Ordering::SeqCst);
                "PRIVATE_INPUT_METHOD_TEXT"
            });
            tracing::info!(target: "nickel_remote_control", operation = "capture", elapsed_us = 40, "operation completed");
        });
        assert_eq!(
            evaluated.load(Ordering::SeqCst),
            0,
            "payload expressions must not be collected"
        );
        let log = String::from_utf8(bytes.lock().unwrap().clone()).unwrap();
        assert!(log.contains("operation completed"));
        assert!(log.contains("elapsed_us=40"));
        assert!(!log.contains("PRIVATE"));
        assert!(!log.contains("received request"));
    }

    #[test]
    fn log_rotation_keeps_small_files_and_moves_threshold_files() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nickel.log");
        let mut file = fs::File::create(&path).unwrap();
        file.write_all(b"hello").unwrap();
        drop(file);
        rotate_if_needed(&path).unwrap();
        assert_eq!(fs::read(&path).unwrap(), b"hello");
        assert!(!path.with_extension("log.previous").exists());

        fs::write(path.with_extension("log.previous"), b"stale").unwrap();
        fs::write(&path, vec![b'x'; 5_242_880]).unwrap();
        rotate_if_needed(&path).unwrap();
        assert!(!path.exists());
        assert_eq!(
            fs::read(path.with_extension("log.previous")).unwrap(),
            vec![b'x'; 5_242_880]
        );
    }
}
