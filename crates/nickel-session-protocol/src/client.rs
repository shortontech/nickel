//! Bounded local request transport for ordinary session clients.

use crate::{
    ClientEnvelope, ControllerHostRequest, ControllerHostResponse, MAX_FRAME_BYTES, Request,
    ServerEnvelope, ServerMessage,
    controller_broker::{BrokerMessage, ConnectionGeneration, EventId, HostId, LeaseEpoch},
};
use std::{
    io,
    os::unix::net::UnixDatagram,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
struct ReplyPath(PathBuf);
impl Drop for ReplyPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

pub struct AsyncControllerConnection {
    socket: UnixDatagram,
    _path: ReplyPath,
    token: String,
    pending: Option<(u64, std::time::Instant)>,
    timeout: Duration,
}

impl AsyncControllerConnection {
    /// Starts attachment without waiting for a server response. `Ok(None)` means no session was
    /// advertised; every advertised-session setup failure is an error.
    pub fn begin_from_environment(timeout: Duration) -> io::Result<Option<Self>> {
        let Some(advertisement) = crate::local_transport::advertisement_from_environment()? else {
            return Ok(None);
        };
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "session runtime unavailable")
        })?;
        Self::begin_to(
            Path::new(&advertisement.endpoint),
            Path::new(&runtime),
            advertisement.capability,
            timeout,
        )
        .map(Some)
    }

    pub fn begin_to(
        server: &Path,
        runtime: &Path,
        token: String,
        timeout: Duration,
    ) -> io::Result<Self> {
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let path = Path::new(&runtime).join(format!(
            "ni-controller-async-{:x}-{id:x}.sock",
            std::process::id()
        ));
        let socket = UnixDatagram::bind(&path)?;
        socket.connect(server)?;
        socket.set_nonblocking(true)?;
        let mut connection = Self {
            socket,
            _path: ReplyPath(path),
            token,
            pending: None,
            timeout,
        };
        connection.send(ControllerHostRequest::Attach)?;
        Ok(connection)
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
        self.socket.send(&frame)?;
        self.pending = Some((id, std::time::Instant::now()));
        Ok(())
    }

    pub fn receive(&mut self) -> io::Result<Option<ControllerHostResponse>> {
        let Some((id, sent_at)) = self.pending else {
            return Ok(None);
        };
        if sent_at.elapsed() >= self.timeout {
            self.pending = None;
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                "session controller request timed out",
            ));
        }
        let mut buffer = vec![0; MAX_FRAME_BYTES];
        let count = match self.socket.recv(&mut buffer) {
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => return Ok(None),
            Err(error) => return Err(error),
        };
        let envelope =
            crate::decode::<ServerEnvelope>(&buffer[..count]).map_err(io::Error::other)?;
        if envelope.request_id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "controller response correlation mismatch",
            ));
        }
        self.pending = None;
        match envelope.message {
            ServerMessage::ControllerHost(response) => Ok(Some(response)),
            ServerMessage::Error { message, .. } => Err(io::Error::other(message)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid controller response",
            )),
        }
    }

    /// Sends an orderly shutdown boundary without waiting for the outstanding poll response.
    /// Datagram framing keeps this request distinct and ordered on the connected local socket.
    pub fn relinquish(&mut self, connection_generation: ConnectionGeneration) -> io::Result<()> {
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let frame = crate::encode(&ClientEnvelope {
            token: self.token.clone(),
            request_id: id,
            request: Request::ControllerHost(ControllerHostRequest::Relinquish {
                connection_generation,
            }),
        })
        .map_err(io::Error::other)?;
        self.socket.send(&frame)?;
        Ok(())
    }
}

/// Persistent authenticated controller channel for a session-managed host.
///
/// `connect_from_environment` returns `Ok(None)` only when no Nickel session is advertised. Any
/// discovery, authentication, or transport failure is an error and must not enable local polling.
pub struct ControllerConnection {
    socket: UnixDatagram,
    _path: ReplyPath,
    token: String,
    host: HostId,
    generation: ConnectionGeneration,
    timeout: Duration,
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
        let runtime = std::env::var_os("XDG_RUNTIME_DIR").ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "session runtime unavailable")
        })?;
        Self::connect_to(
            Path::new(&advertisement.endpoint),
            Path::new(&runtime),
            advertisement.capability,
            timeout,
        )
        .map(Some)
    }

    pub fn connect_to(
        server: &Path,
        runtime: &Path,
        token: String,
        timeout: Duration,
    ) -> io::Result<Self> {
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let path = runtime.join(format!(
            "ni-controller-{:x}-{id:x}.sock",
            std::process::id()
        ));
        let socket = UnixDatagram::bind(&path)?;
        socket.connect(server)?;
        socket.set_read_timeout(Some(timeout))?;
        socket.set_write_timeout(Some(timeout))?;
        let mut connection = Self {
            socket,
            _path: ReplyPath(path),
            token,
            host: HostId(0),
            generation: ConnectionGeneration(0),
            timeout,
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
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "session rejected controller attachment",
            )),
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
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid controller poll response",
            )),
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
        let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
        let frame = crate::encode(&ClientEnvelope {
            token: self.token.clone(),
            request_id: id,
            request: Request::ControllerHost(request),
        })
        .map_err(io::Error::other)?;
        self.socket.set_read_timeout(Some(self.timeout))?;
        self.socket.send(&frame)?;
        let mut buffer = vec![0; MAX_FRAME_BYTES];
        let count = self.socket.recv(&mut buffer)?;
        let envelope =
            crate::decode::<ServerEnvelope>(&buffer[..count]).map_err(io::Error::other)?;
        if envelope.request_id != id {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "controller response correlation mismatch",
            ));
        }
        match envelope.message {
            ServerMessage::ControllerHost(response) => Ok(response),
            ServerMessage::Error { message, .. } => Err(io::Error::other(message)),
            _ => Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "invalid controller response",
            )),
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

/// Absence of a session is distinct from an unavailable configured authority.
/// Never fall back to an unrelated host session or expose its capability in diagnostics.
pub fn request_from_environment(
    request: Request,
    timeout: Duration,
) -> io::Result<Option<ServerMessage>> {
    let Some(advertisement) = crate::local_transport::advertisement_from_environment()? else {
        return Ok(None);
    };
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "session runtime unavailable"))?;
    request_at(
        Path::new(&advertisement.endpoint),
        Path::new(&runtime),
        advertisement.capability,
        request,
        timeout,
    )
    .map(Some)
}

fn request_at(
    server: &Path,
    runtime: &Path,
    token: String,
    request: Request,
    timeout: Duration,
) -> io::Result<ServerMessage> {
    let id = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let path = runtime.join(format!("ni-{:x}-{id:x}.sock", std::process::id()));
    // Binding is exclusive; do not unlink an existing path to acquire it.
    let socket = UnixDatagram::bind(&path)?;
    let _cleanup = ReplyPath(path);
    socket.connect(server)?;
    socket.set_read_timeout(Some(timeout))?;
    socket.set_write_timeout(Some(timeout))?;
    let frame = crate::encode(&ClientEnvelope {
        token,
        request_id: id,
        request,
    })
    .map_err(io::Error::other)?;
    socket.send(&frame)?;
    let mut buffer = vec![0; MAX_FRAME_BYTES];
    let count = socket.recv(&mut buffer)?;
    let envelope = crate::decode::<ServerEnvelope>(&buffer[..count]).map_err(io::Error::other)?;
    if envelope.request_id != id {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "session response correlation mismatch",
        ));
    }
    Ok(envelope.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn correlated_reply_and_failure_both_remove_the_reply_socket() {
        let root = std::env::temp_dir().join(format!(
            "nickel-protocol-{}-{}",
            std::process::id(),
            NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        for mismatch in [false, true] {
            let path = root.join("server");
            let socket = UnixDatagram::bind(&path).unwrap();
            let server = std::thread::spawn(move || {
                let mut buffer = vec![0; MAX_FRAME_BYTES];
                let (length, peer) = socket.recv_from(&mut buffer).unwrap();
                let request: ClientEnvelope = crate::decode(&buffer[..length]).unwrap();
                let reply = ServerEnvelope {
                    request_id: request.request_id + u64::from(mismatch),
                    message: ServerMessage::OnScreenKeyboard(Default::default()),
                };
                socket
                    .send_to(&crate::encode(&reply).unwrap(), peer.as_pathname().unwrap())
                    .unwrap();
            });
            let result = request_at(
                &path,
                &root,
                "test".into(),
                Request::Query(crate::Query::OnScreenKeyboard),
                Duration::from_secs(1),
            );
            assert_eq!(result.is_err(), mismatch);
            server.join().unwrap();
            std::fs::remove_file(&path).unwrap();
            assert_eq!(std::fs::read_dir(&root).unwrap().count(), 0);
        }
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn controller_connection_reuses_authenticated_socket_and_preserves_messages() {
        let root = std::env::temp_dir().join(format!(
            "nickel-controller-protocol-{}-{}",
            std::process::id(),
            NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let server_path = root.join("server");
        let server = UnixDatagram::bind(&server_path).unwrap();
        let worker = std::thread::spawn(move || {
            let mut buffer = vec![0; MAX_FRAME_BYTES];
            let mut first_peer = None;
            for step in 0..3 {
                let (length, peer) = server.recv_from(&mut buffer).unwrap();
                assert_eq!(
                    first_peer.get_or_insert_with(|| peer.as_pathname().unwrap().to_path_buf()),
                    peer.as_pathname().unwrap()
                );
                let request: ClientEnvelope = crate::decode(&buffer[..length]).unwrap();
                assert_eq!(request.token, "secret");
                let response = match (step, request.request) {
                    (0, Request::ControllerHost(ControllerHostRequest::Attach)) => {
                        ControllerHostResponse::Attached {
                            host: HostId(42),
                            connection_generation: ConnectionGeneration(3),
                        }
                    }
                    (
                        1,
                        Request::ControllerHost(ControllerHostRequest::Poll {
                            connection_generation: ConnectionGeneration(3),
                        }),
                    ) => ControllerHostResponse::Messages {
                        lease_epoch: Some(LeaseEpoch(7)),
                        execution_oracle: Some(crate::ControllerExecutionOracle {
                            routing_epoch: 11,
                            lease_epoch: LeaseEpoch(7),
                            connection_generation: ConnectionGeneration(3),
                            stream_generation: crate::controller_broker::StreamGeneration(8),
                            surface_generation: Some(12),
                        }),
                        messages: vec![BrokerMessage::Deliver(
                            crate::controller_broker::Delivery {
                                event_id: EventId(19),
                                connection_generation: ConnectionGeneration(3),
                                lease_epoch: LeaseEpoch(7),
                                stream_generation: crate::controller_broker::StreamGeneration(8),
                                payload: crate::ControllerEnvelopePayload {
                                    device_generation: 44,
                                    action: None,
                                    edge: crate::InputState::Released,
                                    repeat: false,
                                    family: crate::ControllerFamilyMessage::Xbox,
                                    routing_epoch: 11,
                                    evidence: None,
                                    surface_generation: Some(12),
                                },
                            },
                        )],
                    },
                    (
                        2,
                        Request::ControllerHost(ControllerHostRequest::Detach {
                            connection_generation: ConnectionGeneration(3),
                        }),
                    ) => ControllerHostResponse::Detached,
                    _ => panic!("unexpected controller request"),
                };
                server
                    .send_to(
                        &crate::encode(&ServerEnvelope {
                            request_id: request.request_id,
                            message: ServerMessage::ControllerHost(response),
                        })
                        .unwrap(),
                        peer.as_pathname().unwrap(),
                    )
                    .unwrap();
            }
        });
        let connection = ControllerConnection::connect_to(
            &server_path,
            &root,
            "secret".into(),
            Duration::from_secs(1),
        )
        .unwrap();
        assert_eq!(connection.host(), HostId(42));
        let poll = connection.poll().unwrap();
        assert_eq!(poll.lease_epoch, Some(LeaseEpoch(7)));
        assert!(matches!(
            poll.messages.as_slice(),
            [BrokerMessage::Deliver(crate::controller_broker::Delivery {
                event_id: EventId(19),
                payload: crate::ControllerEnvelopePayload {
                    device_generation: 44,
                    action: None,
                    edge: crate::InputState::Released,
                    repeat: false,
                    family: crate::ControllerFamilyMessage::Xbox,
                    routing_epoch: 11,
                    evidence: None,
                    surface_generation: Some(12),
                },
                ..
            })]
        ));
        drop(connection);
        worker.join().unwrap();
        std::fs::remove_file(server_path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }

    #[test]
    fn asynchronous_controller_receive_never_waits_for_server_reply() {
        let root = std::env::temp_dir().join(format!(
            "nickel-async-controller-{}-{}",
            std::process::id(),
            NEXT_REQUEST.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&root).unwrap();
        let server_path = root.join("server");
        let server = UnixDatagram::bind(&server_path).unwrap();
        let (received_tx, received_rx) = std::sync::mpsc::channel();
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let mut buffer = vec![0; MAX_FRAME_BYTES];
            let (length, peer) = server.recv_from(&mut buffer).unwrap();
            let request: ClientEnvelope = crate::decode(&buffer[..length]).unwrap();
            received_tx.send(()).unwrap();
            reply_rx.recv().unwrap();
            server
                .send_to(
                    &crate::encode(&ServerEnvelope {
                        request_id: request.request_id,
                        message: ServerMessage::ControllerHost(ControllerHostResponse::Attached {
                            host: HostId(5),
                            connection_generation: ConnectionGeneration(6),
                        }),
                    })
                    .unwrap(),
                    peer.as_pathname().unwrap(),
                )
                .unwrap();
        });
        let mut connection = AsyncControllerConnection::begin_to(
            &server_path,
            &root,
            "secret".into(),
            Duration::from_secs(1),
        )
        .unwrap();
        received_rx.recv().unwrap();
        assert!(connection.receive().unwrap().is_none());
        reply_tx.send(()).unwrap();
        let response = loop {
            if let Some(response) = connection.receive().unwrap() {
                break response;
            }
            std::thread::yield_now();
        };
        assert!(matches!(response, ControllerHostResponse::Attached { .. }));
        worker.join().unwrap();
        drop(connection);
        std::fs::remove_file(server_path).unwrap();
        std::fs::remove_dir(root).unwrap();
    }
}
