//! Bounded Windows named-pipe transport for local session clients.

use crate::{
    ClientEnvelope, ControllerHostRequest, ControllerHostResponse, FRAME_HEADER_BYTES,
    MAX_FRAME_BYTES, Request, ServerEnvelope, ServerMessage,
    controller_broker::{BrokerMessage, ConnectionGeneration, EventId, HostId, LeaseEpoch},
};
use std::{
    ffi::OsStr,
    io,
    os::windows::ffi::OsStrExt,
    sync::{
        Mutex,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows::{
    Win32::{
        Foundation::{CloseHandle, ERROR_NO_DATA, ERROR_PIPE_LISTENING, HANDLE},
        Storage::FileSystem::{
            CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_GENERIC_READ, FILE_GENERIC_WRITE,
            FILE_SHARE_NONE, OPEN_EXISTING, ReadFile, WriteFile,
        },
        System::Pipes::{PIPE_NOWAIT, PIPE_READMODE_BYTE, SetNamedPipeHandleState, WaitNamedPipeW},
    },
    core::PCWSTR,
};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
const RETRY_INTERVAL: Duration = Duration::from_millis(1);

/// Nonblocking authenticated controller channel for UI event loops.
pub struct AsyncControllerConnection {
    pipe: NamedPipe,
    token: String,
    pending: Option<PendingRequest>,
    timeout: Duration,
}

struct PendingRequest {
    id: u64,
    started: Instant,
    frame: Vec<u8>,
    written: usize,
    reply: Vec<u8>,
    reply_bytes: Option<usize>,
}

impl AsyncControllerConnection {
    /// Starts attachment without waiting for the pipe or its server response. `Ok(None)` means no
    /// session was advertised; every advertised-session setup failure is an error.
    pub fn begin_from_environment(timeout: Duration) -> io::Result<Option<Self>> {
        let Some(advertisement) = crate::local_transport::advertisement_from_environment()? else {
            return Ok(None);
        };
        let mut connection = Self {
            pipe: NamedPipe::connect_now(&advertisement.endpoint)?,
            token: advertisement.capability,
            pending: None,
            timeout,
        };
        connection.send(ControllerHostRequest::Attach)?;
        Ok(Some(connection))
    }

    pub fn send(&mut self, request: ControllerHostRequest) -> io::Result<()> {
        if self.pending.is_some() {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "controller request already outstanding",
            ));
        }
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let frame = crate::encode(&ClientEnvelope {
            token: self.token.clone(),
            request_id: id,
            request: Request::ControllerHost(request),
        })
        .map_err(io::Error::other)?;
        self.pending = Some(PendingRequest {
            id,
            started: Instant::now(),
            frame,
            written: 0,
            reply: Vec::with_capacity(FRAME_HEADER_BYTES),
            reply_bytes: None,
        });
        self.progress_write()
    }

    pub fn receive(&mut self) -> io::Result<Option<ControllerHostResponse>> {
        let Some(pending) = self.pending.as_ref() else {
            return Ok(None);
        };
        if pending.started.elapsed() >= self.timeout {
            self.pending = None;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "session controller request timed out",
            ));
        }
        self.progress_write()?;
        let Some(pending) = self.pending.as_mut() else {
            unreachable!("pending request exists while receiving")
        };
        if pending.written < pending.frame.len() {
            return Ok(None);
        }
        loop {
            let limit = pending.reply_bytes.unwrap_or(FRAME_HEADER_BYTES);
            if pending.reply.len() == limit {
                if pending.reply_bytes.is_none() {
                    let payload = u32::from_le_bytes(
                        pending.reply[6..10].try_into().expect("fixed frame header"),
                    ) as usize;
                    let total = FRAME_HEADER_BYTES.saturating_add(payload);
                    if total > MAX_FRAME_BYTES {
                        self.pending = None;
                        return Err(invalid_data("named-pipe frame exceeds size limit"));
                    }
                    pending.reply_bytes = Some(total);
                    if total != pending.reply.len() {
                        pending.reply.reserve(total - pending.reply.len());
                        continue;
                    }
                }
                let pending = self.pending.take().expect("pending response");
                let envelope =
                    crate::decode::<ServerEnvelope>(&pending.reply).map_err(io::Error::other)?;
                if envelope.request_id != pending.id {
                    return Err(invalid_data("controller response correlation mismatch"));
                }
                return match envelope.message {
                    ServerMessage::ControllerHost(response) => Ok(Some(response)),
                    ServerMessage::Error { message, .. } => Err(io::Error::other(message)),
                    _ => Err(invalid_data("invalid controller response")),
                };
            }
            let mut buffer = [0_u8; 4096];
            let wanted = (limit - pending.reply.len()).min(buffer.len());
            match self.pipe.try_read(&mut buffer[..wanted])? {
                Some(read) => pending.reply.extend_from_slice(&buffer[..read]),
                None => return Ok(None),
            }
        }
    }

    fn progress_write(&mut self) -> io::Result<()> {
        let pending = self.pending.as_mut().expect("request is pending");
        while pending.written < pending.frame.len() {
            match self.pipe.try_write(&pending.frame[pending.written..])? {
                Some(written) => pending.written += written,
                None => break,
            }
        }
        Ok(())
    }

    /// Sends an orderly shutdown boundary without waiting for the outstanding poll response.
    pub fn relinquish(&mut self, connection_generation: ConnectionGeneration) -> io::Result<()> {
        if self
            .pending
            .as_ref()
            .is_some_and(|pending| pending.written < pending.frame.len())
        {
            return Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "controller request write is incomplete",
            ));
        }
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let frame = crate::encode(&ClientEnvelope {
            token: self.token.clone(),
            request_id: id,
            request: Request::ControllerHost(ControllerHostRequest::Relinquish {
                connection_generation,
            }),
        })
        .map_err(io::Error::other)?;
        match self.pipe.try_write(&frame)? {
            Some(written) if written == frame.len() => Ok(()),
            _ => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "controller relinquish write would block",
            )),
        }
    }
}

/// Persistent authenticated controller channel for a session-managed host.
///
/// Absence of `NICKEL_SESSION_CONTROL` permits local input. Once advertised, a missing capability,
/// unavailable pipe, invalid response, or timeout is an error and must not enable local polling.
pub struct ControllerConnection {
    pipe: Mutex<NamedPipe>,
    token: String,
    host: HostId,
    generation: ConnectionGeneration,
}

pub struct ControllerPoll {
    pub lease_epoch: Option<LeaseEpoch>,
    pub messages: Vec<BrokerMessage<crate::ControllerEnvelopePayload>>,
    pub execution_oracle: Option<crate::ControllerExecutionOracle>,
}

impl ControllerConnection {
    pub fn connect_from_environment(timeout: Duration) -> io::Result<Option<Self>> {
        let Some(advertisement) = crate::local_transport::advertisement_from_environment()? else {
            return Ok(None);
        };
        Self::connect_at(&advertisement.endpoint, advertisement.capability, timeout).map(Some)
    }

    fn connect_at(endpoint: &OsStr, token: String, timeout: Duration) -> io::Result<Self> {
        let mut connection = Self {
            pipe: Mutex::new(NamedPipe::connect(endpoint, timeout)?),
            token,
            host: HostId(0),
            generation: ConnectionGeneration(0),
        };
        match connection.exchange(ControllerHostRequest::Attach)? {
            ControllerHostResponse::Attached {
                host,
                connection_generation,
            } => {
                connection.host = host;
                connection.generation = connection_generation;
                Ok(connection)
            }
            _ => Err(invalid_data("session rejected controller attachment")),
        }
    }

    pub fn host(&self) -> HostId {
        self.host
    }

    pub fn connection_generation(&self) -> ConnectionGeneration {
        self.generation
    }

    pub fn request_lease(&self) -> io::Result<ControllerHostResponse> {
        self.exchange(ControllerHostRequest::RequestLease {
            connection_generation: self.generation,
        })
    }

    pub fn poll(&self) -> io::Result<ControllerPoll> {
        match self.exchange(ControllerHostRequest::Poll {
            connection_generation: self.generation,
        })? {
            ControllerHostResponse::Messages {
                lease_epoch,
                messages,
                execution_oracle,
            } => Ok(ControllerPoll {
                lease_epoch,
                messages,
                execution_oracle,
            }),
            _ => Err(invalid_data("invalid controller poll response")),
        }
    }

    pub fn acknowledge_quiescence(
        &self,
        lease: LeaseEpoch,
        cutoff: EventId,
    ) -> io::Result<ControllerHostResponse> {
        self.exchange(ControllerHostRequest::AcknowledgeQuiescence {
            connection_generation: self.generation,
            lease_epoch: lease,
            cutoff,
        })
    }

    pub fn relinquish(&self) -> io::Result<ControllerHostResponse> {
        self.exchange(ControllerHostRequest::Relinquish {
            connection_generation: self.generation,
        })
    }

    fn exchange(&self, request: ControllerHostRequest) -> io::Result<ControllerHostResponse> {
        let mut pipe = self
            .pipe
            .lock()
            .map_err(|_| io::Error::other("controller pipe lock poisoned"))?;
        let message = exchange(&mut pipe, &self.token, Request::ControllerHost(request))?;
        match message {
            ServerMessage::ControllerHost(response) => Ok(response),
            ServerMessage::Error { message, .. } => Err(io::Error::other(message)),
            _ => Err(invalid_data("invalid controller response")),
        }
    }
}

impl Drop for ControllerConnection {
    fn drop(&mut self) {
        let _ = self.exchange(ControllerHostRequest::Detach {
            connection_generation: self.generation,
        });
    }
}

pub fn request_from_environment(
    request: Request,
    timeout: Duration,
) -> io::Result<Option<ServerMessage>> {
    let Some(advertisement) = crate::local_transport::advertisement_from_environment()? else {
        return Ok(None);
    };
    let mut pipe = NamedPipe::connect(&advertisement.endpoint, timeout)?;
    exchange(&mut pipe, &advertisement.capability, request).map(Some)
}

fn exchange(pipe: &mut NamedPipe, token: &str, request: Request) -> io::Result<ServerMessage> {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let frame = crate::encode(&ClientEnvelope {
        token: token.to_owned(),
        request_id: id,
        request,
    })
    .map_err(io::Error::other)?;
    pipe.write_all(&frame)?;
    let reply = pipe.read_frame()?;
    let envelope = crate::decode::<ServerEnvelope>(&reply).map_err(io::Error::other)?;
    if envelope.request_id != id {
        return Err(invalid_data("session response correlation mismatch"));
    }
    Ok(envelope.message)
}

struct NamedPipe {
    handle: HANDLE,
    timeout: Duration,
}

impl NamedPipe {
    fn connect_now(endpoint: &OsStr) -> io::Result<Self> {
        Self::open(endpoint, Duration::ZERO)
    }

    fn connect(endpoint: &OsStr, timeout: Duration) -> io::Result<Self> {
        let endpoint = wide(endpoint);
        let timeout_ms = duration_ms(timeout);
        if !unsafe { WaitNamedPipeW(PCWSTR(endpoint.as_ptr()), timeout_ms) }.as_bool() {
            return Err(io::Error::last_os_error());
        }
        Self::open_wide(&endpoint, timeout)
    }

    fn open(endpoint: &OsStr, timeout: Duration) -> io::Result<Self> {
        Self::open_wide(&wide(endpoint), timeout)
    }

    fn open_wide(endpoint: &[u16], timeout: Duration) -> io::Result<Self> {
        let handle = unsafe {
            CreateFileW(
                PCWSTR(endpoint.as_ptr()),
                (FILE_GENERIC_READ | FILE_GENERIC_WRITE).0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_ATTRIBUTE_NORMAL,
                None,
            )
        }
        .map_err(|error| io::Error::other(error.to_string()))?;
        let mode = PIPE_READMODE_BYTE | PIPE_NOWAIT;
        if let Err(error) = unsafe { SetNamedPipeHandleState(handle, Some(&mode), None, None) } {
            let _ = unsafe { CloseHandle(handle) };
            return Err(io::Error::other(error.to_string()));
        }
        Ok(Self { handle, timeout })
    }

    fn try_write(&mut self, bytes: &[u8]) -> io::Result<Option<usize>> {
        let mut written = 0;
        match unsafe { WriteFile(self.handle, Some(bytes), Some(&mut written), None) } {
            Ok(()) if written > 0 => Ok(Some(written as usize)),
            Ok(()) => Ok(None),
            Err(error) if is_pipe_would_block(&error) => Ok(None),
            Err(error) => Err(io::Error::other(error.to_string())),
        }
    }

    fn try_read(&mut self, bytes: &mut [u8]) -> io::Result<Option<usize>> {
        let mut read = 0;
        match unsafe { ReadFile(self.handle, Some(bytes), Some(&mut read), None) } {
            Ok(()) if read > 0 => Ok(Some(read as usize)),
            Ok(()) => Ok(None),
            Err(error) if is_pipe_would_block(&error) => Ok(None),
            Err(error) => Err(io::Error::other(error.to_string())),
        }
    }

    fn write_all(&mut self, mut bytes: &[u8]) -> io::Result<()> {
        let deadline = Instant::now() + self.timeout;
        while !bytes.is_empty() {
            let mut written = 0;
            match unsafe { WriteFile(self.handle, Some(bytes), Some(&mut written), None) } {
                Ok(()) if written > 0 => bytes = &bytes[written as usize..],
                _ if Instant::now() < deadline => thread::sleep(RETRY_INTERVAL),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "named-pipe write timed out",
                    ));
                }
            }
        }
        Ok(())
    }

    fn read_frame(&mut self) -> io::Result<Vec<u8>> {
        let deadline = Instant::now() + self.timeout;
        let mut frame = vec![0; FRAME_HEADER_BYTES];
        self.read_exact_until(&mut frame, deadline)?;
        let payload =
            u32::from_le_bytes(frame[6..10].try_into().expect("fixed frame header")) as usize;
        let total = FRAME_HEADER_BYTES.saturating_add(payload);
        if total > MAX_FRAME_BYTES {
            return Err(invalid_data("named-pipe frame exceeds size limit"));
        }
        frame.resize(total, 0);
        self.read_exact_until(&mut frame[FRAME_HEADER_BYTES..], deadline)?;
        Ok(frame)
    }

    fn read_exact_until(&mut self, mut bytes: &mut [u8], deadline: Instant) -> io::Result<()> {
        while !bytes.is_empty() {
            let mut read = 0;
            match unsafe { ReadFile(self.handle, Some(bytes), Some(&mut read), None) } {
                Ok(()) if read > 0 => {
                    let (_, rest) = bytes.split_at_mut(read as usize);
                    bytes = rest;
                }
                _ if Instant::now() < deadline => thread::sleep(RETRY_INTERVAL),
                _ => {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "named-pipe read timed out",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn is_pipe_would_block(error: &windows::core::Error) -> bool {
    error.code() == windows::core::HRESULT::from_win32(ERROR_NO_DATA.0)
        || error.code() == windows::core::HRESULT::from_win32(ERROR_PIPE_LISTENING.0)
}

impl Drop for NamedPipe {
    fn drop(&mut self) {
        let _ = unsafe { CloseHandle(self.handle) };
    }
}

fn wide(value: &OsStr) -> Vec<u16> {
    value.encode_wide().chain(Some(0)).collect()
}

fn duration_ms(timeout: Duration) -> u32 {
    timeout.as_millis().clamp(1, u128::from(u32::MAX)) as u32
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::duration_ms;
    use std::time::Duration;

    #[test]
    fn named_pipe_wait_is_always_bounded() {
        assert_eq!(duration_ms(Duration::ZERO), 1);
        assert_eq!(duration_ms(Duration::from_millis(25)), 25);
        assert_eq!(duration_ms(Duration::MAX), u32::MAX);
    }
}
