//! Explicitly gated AF_UNIX control used only by native Windows/Proton acceptance.

use std::{
    io::{self, Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use nickel_session_protocol::{
    ClientEnvelope, ErrorCode, FRAME_HEADER_BYTES, MAX_FRAME_BYTES, ServerEnvelope, ServerMessage,
    decode, encode,
};
use uds_windows::{UnixListener, UnixStream};

use crate::winit_shell::{ShellEventSender, ShellUserEvent};

pub(crate) const ENABLE_ENV: &str = "NICKEL_WINDOWS_TEST_CONTROL";
pub(crate) const ENDPOINT_ENV: &str = "NICKEL_SHELL_TEST_CONTROL";
pub(crate) const TOKEN_ENV: &str = "NICKEL_SESSION_TOKEN";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) struct WindowsTestControl {
    stop: Arc<AtomicBool>,
    endpoint: PathBuf,
    worker: Option<thread::JoinHandle<()>>,
}

impl WindowsTestControl {
    pub(crate) fn start(
        enabled_by_argument: bool,
        sender: ShellEventSender,
    ) -> Result<Option<Self>, String> {
        if !enabled_by_argument
            || std::env::var_os(ENABLE_ENV).as_deref() != Some(std::ffi::OsStr::new("1"))
        {
            return Ok(None);
        }
        let endpoint = std::env::var_os(ENDPOINT_ENV)
            .map(PathBuf::from)
            .ok_or_else(|| format!("{ENDPOINT_ENV} is required with Windows test control"))?;
        let token = std::env::var(TOKEN_ENV)
            .ok()
            .filter(|token| token.len() >= 32)
            .ok_or_else(|| format!("{TOKEN_ENV} must contain at least 32 bytes"))?;
        remove_endpoint(&endpoint)?;
        let listener = UnixListener::bind(&endpoint).map_err(|error| {
            format!("could not bind Windows AF_UNIX test endpoint {endpoint:?}: {error}")
        })?;
        listener
            .set_nonblocking(true)
            .map_err(|error| error.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_endpoint = endpoint.clone();
        let worker = thread::Builder::new()
            .name("nickel-windows-test-control".into())
            .spawn(move || {
                while !worker_stop.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((stream, _)) => handle_connection(stream, &token, &sender),
                        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(10));
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Windows test-control accept failed");
                            thread::sleep(Duration::from_millis(25));
                        }
                    }
                }
                let _ = std::fs::remove_file(worker_endpoint);
            })
            .map_err(|error| error.to_string())?;
        Ok(Some(Self {
            stop,
            endpoint,
            worker: Some(worker),
        }))
    }
}

impl Drop for WindowsTestControl {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        let _ = std::fs::remove_file(&self.endpoint);
    }
}

fn remove_endpoint(endpoint: &Path) -> Result<(), String> {
    match std::fs::remove_file(endpoint) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!(
            "could not remove stale test endpoint {endpoint:?}: {error}"
        )),
    }
}

fn handle_connection(mut stream: UnixStream, token: &str, sender: &ShellEventSender) {
    let _ = stream.set_read_timeout(Some(REQUEST_TIMEOUT));
    let _ = stream.set_write_timeout(Some(REQUEST_TIMEOUT));
    let request = read_frame::<ClientEnvelope>(&mut stream);
    let envelope = match request {
        Ok(envelope) if constant_time_eq(envelope.token.as_bytes(), token.as_bytes()) => envelope,
        Ok(envelope) => {
            let _ = write_response(
                &mut stream,
                envelope.request_id,
                ServerMessage::Error {
                    code: ErrorCode::Unauthorized,
                    message: "invalid session capability token".into(),
                },
            );
            return;
        }
        Err(error) => {
            let _ = write_response(
                &mut stream,
                0,
                ServerMessage::Error {
                    code: ErrorCode::InvalidRequest,
                    message: error.to_string(),
                },
            );
            return;
        }
    };
    let (response, receive) = mpsc::sync_channel(1);
    let request_id = envelope.request_id;
    if sender
        .send_event(ShellUserEvent::TestControl(
            super::ShellTestRequest::Protocol(super::WindowsShellTestRequest {
                envelope,
                response,
            }),
        ))
        .is_err()
    {
        let _ = write_response(
            &mut stream,
            request_id,
            ServerMessage::Error {
                code: ErrorCode::Internal,
                message: "shell event loop is unavailable".into(),
            },
        );
        return;
    }
    let message = receive
        .recv_timeout(REQUEST_TIMEOUT)
        .unwrap_or_else(|_| ServerEnvelope {
            request_id,
            message: ServerMessage::Error {
                code: ErrorCode::Internal,
                message: "shell test-control request timed out".into(),
            },
        });
    if let Ok(frame) = encode(&message) {
        let _ = stream.write_all(&frame);
    }
}

fn read_frame<T: serde::de::DeserializeOwned>(reader: &mut impl Read) -> io::Result<T> {
    let mut header = [0_u8; FRAME_HEADER_BYTES];
    reader.read_exact(&mut header)?;
    let payload =
        u32::from_le_bytes(header[6..10].try_into().expect("fixed frame header")) as usize;
    let total = FRAME_HEADER_BYTES.saturating_add(payload);
    if total > MAX_FRAME_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "test-control frame exceeds limit",
        ));
    }
    let mut frame = Vec::with_capacity(total);
    frame.extend_from_slice(&header);
    frame.resize(total, 0);
    reader.read_exact(&mut frame[FRAME_HEADER_BYTES..])?;
    decode(&frame).map_err(io::Error::other)
}

fn write_response(
    stream: &mut impl Write,
    request_id: u64,
    message: ServerMessage,
) -> io::Result<()> {
    let frame = encode(&ServerEnvelope {
        request_id,
        message,
    })
    .map_err(io::Error::other)?;
    stream.write_all(&frame)
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= usize::from(
            left.get(index).copied().unwrap_or_default()
                ^ right.get(index).copied().unwrap_or_default(),
        );
    }
    difference == 0
}

#[cfg(test)]
mod tests {
    use super::{constant_time_eq, read_frame};
    use nickel_session_protocol::{ClientEnvelope, Query, Request, encode};
    use std::io::{self, Cursor, Read};

    struct Fragmented<R> {
        inner: R,
        limit: usize,
    }
    impl<R: Read> Read for Fragmented<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.inner.read(&mut buffer[..buffer.len().min(self.limit)])
        }
    }

    #[test]
    fn fragmented_authenticated_protocol_frame_is_reassembled() {
        let envelope = ClientEnvelope {
            token: "x".repeat(32),
            request_id: 7,
            request: Request::Query(Query::ShellRuntimeDiagnostics),
        };
        let frame = encode(&envelope).unwrap();
        let mut reader = Fragmented {
            inner: Cursor::new(frame),
            limit: 3,
        };
        assert_eq!(read_frame::<ClientEnvelope>(&mut reader).unwrap(), envelope);
    }

    #[test]
    fn capability_comparison_rejects_prefixes_and_different_values() {
        assert!(constant_time_eq(b"same", b"same"));
        assert!(!constant_time_eq(b"same", b"same-more"));
        assert!(!constant_time_eq(b"same", b"sand"));
    }
}
