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
        Foundation::{CloseHandle, HANDLE},
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
            } => Ok(ControllerPoll {
                lease_epoch,
                messages,
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
    fn connect(endpoint: &OsStr, timeout: Duration) -> io::Result<Self> {
        let endpoint = wide(endpoint);
        let timeout_ms = duration_ms(timeout);
        if !unsafe { WaitNamedPipeW(PCWSTR(endpoint.as_ptr()), timeout_ms) }.as_bool() {
            return Err(io::Error::last_os_error());
        }
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
