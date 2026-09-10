//! Per-job D-Bus framing limits, enforced before the library allocates a body.
use async_io::Async;
use std::{
    io,
    os::fd::{BorrowedFd, OwnedFd},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};
use zbus::{
    Message,
    connection::socket::{BoxedSplit, ReadHalf, Split, WriteHalf},
};

#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub frame_bytes: usize,
    pub auth_bytes: usize,
    pub wire_bytes: usize,
    pub frames: usize,
}
impl Limits {
    pub const CONTROL: Self = Self {
        frame_bytes: 8192,
        auth_bytes: 4096,
        wire_bytes: 65536,
        frames: 64,
    };
    pub const ACCESSIBILITY: Self = Self {
        frame_bytes: 65536,
        auth_bytes: 4096,
        wire_bytes: 1048576,
        frames: 1024,
    };
}

/// Whether the requested frame could have reached the native service.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum GuardedSendError {
    /// None of this frame's bytes were accepted; the dedicated transport is retired.
    NotAccepted,
    /// A prefix was accepted. Retiring prevents its remainder from being sent.
    Uncertain,
}
impl std::fmt::Display for GuardedSendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotAccepted => "D-Bus frame not accepted",
            Self::Uncertain => "D-Bus frame acceptance uncertain",
        })
    }
}
impl std::error::Error for GuardedSendError {}

#[derive(Debug, Default)]
struct TransportState {
    writer: async_lock::Mutex<()>,
    authenticated: AtomicBool,
    retired: AtomicBool,
}
impl TransportState {
    fn retire(&self, inner: &Async<std::os::unix::net::UnixStream>) {
        if !self.retired.swap(true, Ordering::AcqRel) {
            let _ = inner.get_ref().shutdown(std::net::Shutdown::Both);
        }
    }
}

/// Native write capability for one authenticated dedicated connection. No callbacks
/// are stored: callers borrow their original commit boundary for this dispatch.
#[derive(Clone, Debug)]
pub struct GuardedSender {
    inner: Arc<Async<std::os::unix::net::UnixStream>>,
    state: Arc<TransportState>,
}
impl GuardedSender {
    /// Permanently close this dedicated transport without flushing pending bytes.
    pub fn retire(&self) {
        self.state.retire(&self.inner);
    }
    pub fn is_retired(&self) -> bool {
        self.state.retired.load(Ordering::Acquire)
    }

    /// Send an already serialized, descriptor-free frame of at most 8 KiB.
    /// Each of at most 16 nonblocking native writes rechecks original authority.
    /// This never awaits writer ownership. Any rejected/incomplete attempt retires
    /// the socket, including zero-byte attempts, so no future can append/retry.
    pub fn send_guarded(
        &self,
        message: &Message,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<(), GuardedSendError> {
        use std::io::Write;
        let bytes = message.data().bytes();
        if bytes.len() > Limits::CONTROL.frame_bytes
            || !message.data().fds().is_empty()
            || message.header().unix_fds().is_some_and(|count| count != 0)
            || frame_length(bytes, Limits::CONTROL.frame_bytes).ok() != Some(bytes.len())
            || !self.state.authenticated.load(Ordering::Acquire)
            || self.state.retired.load(Ordering::Acquire)
        {
            self.retire();
            return Err(GuardedSendError::NotAccepted);
        }
        let Some(_writer) = self.state.writer.try_lock() else {
            self.retire();
            return Err(GuardedSendError::NotAccepted);
        };
        // Declared AFTER the writer guard: retirement happens before exclusion
        // is released on error/unwind, preventing an appended ordinary frame.
        let mut attempt = WriteAttempt {
            sender: self,
            complete: false,
        };
        let mut sent = 0;
        let classification = |sent| {
            if sent == 0 {
                GuardedSendError::NotAccepted
            } else {
                GuardedSendError::Uncertain
            }
        };
        for _ in 0..16 {
            if check().is_err() || self.state.retired.load(Ordering::Acquire) {
                return Err(classification(sent));
            }
            // Async::new/connect set O_NONBLOCK. std UnixStream Write performs
            // one native write; it does not hide an async partial-write loop.
            let mut stream = self.inner.get_ref();
            match stream.write(&bytes[sent..]) {
                Ok(0) => return Err(classification(sent)),
                Ok(count) => sent += count,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(classification(sent)),
            }
            if sent == bytes.len() {
                attempt.complete = true;
                return Ok(());
            }
        }
        Err(classification(sent))
    }
}
struct WriteAttempt<'a> {
    sender: &'a GuardedSender,
    complete: bool,
}
impl Drop for WriteAttempt<'_> {
    fn drop(&mut self) {
        if !self.complete {
            self.sender.retire();
        }
    }
}

#[derive(Debug)]
struct CheckedWrite(GuardedSender);
#[async_trait::async_trait]
impl WriteHalf for CheckedWrite {
    async fn close(&mut self) -> io::Result<()> {
        self.0.retire();
        Ok(())
    }

    async fn sendmsg(&mut self, buffer: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<usize> {
        let _writer = self.0.state.writer.lock().await;
        let mut attempt = WriteAttempt {
            sender: &self.0,
            complete: false,
        };
        if self.0.state.retired.load(Ordering::Acquire) || !fds.is_empty() {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let mut inner = self.0.inner.clone();
        let result = inner.sendmsg(buffer, fds).await;
        attempt.complete = result.is_ok();
        result
    }
    async fn send_message(&mut self, message: &Message) -> zbus::Result<()> {
        // Exclusion spans the WHOLE ordinary frame, including every Pending and
        // partial native write. Guarded frames cannot enter between its chunks.
        let _writer = self.0.state.writer.lock().await;
        let mut attempt = WriteAttempt {
            sender: &self.0,
            complete: false,
        };
        if self.0.state.retired.load(Ordering::Acquire) || !message.data().fds().is_empty() {
            return Err(io::Error::from(io::ErrorKind::BrokenPipe).into());
        }
        let mut inner = self.0.inner.clone();
        let result = inner.send_message(message).await;
        attempt.complete = result.is_ok();
        result
    }
    async fn peer_credentials(&mut self) -> io::Result<zbus::fdo::ConnectionCredentials> {
        ReadHalf::peer_credentials(&mut self.0.inner).await
    }
}

#[derive(Debug)]
struct LimitedRead {
    state: Arc<TransportState>,
    inner: Arc<Async<std::os::unix::net::UnixStream>>,
    auth_bytes: usize,
    frame_bytes: usize,
    frames: usize,
    limits: Limits,
}

fn frame_length(header: &[u8], maximum: usize) -> io::Result<usize> {
    let invalid = || {
        io::Error::new(
            io::ErrorKind::InvalidData,
            "bounded D-Bus frame exceeds bounds",
        )
    };
    if header.len() < 16 || header[3] != 1 {
        return Err(invalid());
    }
    let integer = |range: std::ops::Range<usize>| -> io::Result<usize> {
        let bytes: [u8; 4] = header[range].try_into().map_err(|_| invalid())?;
        match header[0] {
            b'l' => Ok(u32::from_le_bytes(bytes) as usize),
            b'B' => Ok(u32::from_be_bytes(bytes) as usize),
            _ => Err(invalid()),
        }
    };
    let fields = integer(12..16)?;
    let body = integer(4..8)?;
    let head = 16usize
        .checked_add(fields)
        .and_then(|n| n.checked_add(7))
        .map(|n| n & !7)
        .ok_or_else(invalid)?;
    let total = head.checked_add(body).ok_or_else(invalid)?;
    if total > maximum {
        return Err(invalid());
    }
    Ok(total)
}

impl LimitedRead {
    async fn recvmsg(&mut self, buffer: &mut [u8]) -> io::Result<(usize, Vec<OwnedFd>)> {
        let room = self.limits.auth_bytes.saturating_sub(self.auth_bytes);
        if room == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bounded D-Bus authentication exceeds bounds",
            ));
        }
        let limit = buffer.len().min(room);
        let (count, fds) = self.inner.recvmsg(&mut buffer[..limit]).await?;
        self.auth_bytes += count;
        if !fds.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "bounded D-Bus descriptors are not allowed",
            ));
        }
        Ok((count, fds))
    }

    async fn receive_message(
        &mut self,
        seq: u64,
        buffered: &mut Vec<u8>,
        fds: &mut Vec<OwnedFd>,
    ) -> zbus::Result<Message> {
        if !fds.is_empty()
            || buffered.len() > self.limits.auth_bytes
            || self.frames >= self.limits.frames
        {
            return Err(zbus::Error::ExcessData);
        }
        while buffered.len() < 16 {
            let mut header = [0u8; 16];
            let needed = 16 - buffered.len();
            let (count, received_fds) = self.inner.recvmsg(&mut header[..needed]).await?;
            if count == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "bounded D-Bus disconnected",
                )
                .into());
            }
            if !received_fds.is_empty() {
                return Err(zbus::Error::ExcessData);
            }
            buffered.extend_from_slice(&header[..count]);
        }
        let size = frame_length(buffered, self.limits.frame_bytes)?;
        self.frame_bytes = self
            .frame_bytes
            .checked_add(size)
            .ok_or(zbus::Error::ExcessData)?;
        if self.frame_bytes > self.limits.wire_bytes {
            return Err(zbus::Error::ExcessData);
        }
        self.frames += 1;
        // Delegate unchanged, already-validated header bytes to the library parser.
        // Its next allocation is now bounded; no unsafe message parsing is needed.
        let message = self.inner.receive_message(seq, buffered, fds).await?;
        if !message.data().fds().is_empty() {
            return Err(zbus::Error::ExcessData);
        }
        Ok(message)
    }

    async fn peer_credentials(&mut self) -> io::Result<zbus::fdo::ConnectionCredentials> {
        ReadHalf::peer_credentials(&mut self.inner).await
    }
}

#[async_trait::async_trait]
impl ReadHalf for LimitedRead {
    async fn recvmsg(&mut self, buffer: &mut [u8]) -> io::Result<(usize, Vec<OwnedFd>)> {
        let result = LimitedRead::recvmsg(self, buffer).await;
        if result.is_err() {
            self.state.retire(&self.inner);
        }
        result
    }
    async fn receive_message(
        &mut self,
        seq: u64,
        buffered: &mut Vec<u8>,
        fds: &mut Vec<OwnedFd>,
    ) -> zbus::Result<Message> {
        let result = LimitedRead::receive_message(self, seq, buffered, fds).await;
        if result.is_err() {
            self.state.retire(&self.inner);
        }
        result
    }
    async fn peer_credentials(&mut self) -> io::Result<zbus::fdo::ConnectionCredentials> {
        LimitedRead::peer_credentials(self).await
    }
}

fn socket_with_sender(
    stream: Async<std::os::unix::net::UnixStream>,
    limits: Limits,
) -> (BoxedSplit, GuardedSender) {
    let inner = Arc::new(stream);
    let state = Arc::new(TransportState::default());
    let sender = GuardedSender {
        inner: inner.clone(),
        state: state.clone(),
    };
    let socket = Split::new(
        Box::new(LimitedRead {
            state,
            inner,
            auth_bytes: 0,
            frame_bytes: 0,
            frames: 0,
            limits,
        }) as Box<dyn ReadHalf>,
        Box::new(CheckedWrite(sender.clone())) as Box<dyn WriteHalf>,
    );
    (socket, sender)
}

pub fn bounded_socket(stream: Async<std::os::unix::net::UnixStream>, limits: Limits) -> BoxedSplit {
    socket_with_sender(stream, limits).0
}

/// Bounded local Unix connection and EXTERNAL authentication. No TCP, exec,
/// autolaunch, or file-descriptor negotiation is admitted by this helper.
pub async fn connect_guarded(
    address: zbus::Address,
    limits: Limits,
    timeout: std::time::Duration,
) -> Result<(zbus::Connection, GuardedSender), String> {
    use zbus::address::{Transport, transport::UnixSocket};
    let Transport::Unix(unix) = address.transport() else {
        return Err("a local Unix bus is required".into());
    };
    let UnixSocket::File(path) = unix.path() else {
        return Err("a filesystem Unix bus address is required".into());
    };
    if limits.frame_bytes > 1048576
        || limits.auth_bytes > 4096
        || limits.wire_bytes > 1048576
        || limits.frames > 1024
    {
        return Err("D-Bus limits exceed policy".into());
    }
    futures_lite::future::race(
        async {
            let stream = Async::<std::os::unix::net::UnixStream>::connect(path)
                .await
                .map_err(|_| "bus connection unavailable")?;
            let (socket, sender) = socket_with_sender(stream, limits);
            let mut setup = WriteAttempt {
                sender: &sender,
                complete: false,
            };
            let connection = zbus::connection::Builder::socket(socket)
                .max_queued(8)
                .method_timeout(timeout)
                .build()
                .await
                .map_err(|_| "bus authentication unavailable".to_owned())?;
            sender.state.authenticated.store(true, Ordering::Release);
            setup.complete = true;
            drop(setup);
            Ok((connection, sender))
        },
        async {
            async_io::Timer::after(timeout).await;
            Err("bus setup timed out".into())
        },
    )
    .await
}

pub async fn connect(
    address: zbus::Address,
    limits: Limits,
    timeout: std::time::Duration,
) -> Result<zbus::Connection, String> {
    connect_guarded(address, limits, timeout)
        .await
        .map(|(connection, _)| connection)
}

pub fn connect_guarded_blocking(
    address: zbus::Address,
    limits: Limits,
    timeout: std::time::Duration,
) -> Result<(zbus::blocking::Connection, GuardedSender), String> {
    async_io::block_on(connect_guarded(address, limits, timeout))
        .map(|(connection, sender)| (connection.into(), sender))
}

pub fn connect_blocking(
    address: zbus::Address,
    limits: Limits,
    timeout: std::time::Duration,
) -> Result<zbus::blocking::Connection, String> {
    async_io::block_on(connect(address, limits, timeout)).map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;
    const MAX_FRAME: usize = Limits::ACCESSIBILITY.frame_bytes;
    #[test]
    fn frame_cap_checks_both_endians_and_padding_before_body_allocation() {
        for endian in [b'l', b'B'] {
            let mut header = [0u8; 16];
            header[0] = endian;
            header[3] = 1;
            let encode = |n: u32| {
                if endian == b'l' {
                    n.to_le_bytes()
                } else {
                    n.to_be_bytes()
                }
            };
            header[12..16].copy_from_slice(&encode(1));
            header[4..8].copy_from_slice(&encode((MAX_FRAME - 24) as u32));
            assert_eq!(frame_length(&header, MAX_FRAME).unwrap(), MAX_FRAME);
            header[4..8].copy_from_slice(&encode((MAX_FRAME - 23) as u32));
            assert!(frame_length(&header, MAX_FRAME).is_err());
            header[12..16].copy_from_slice(&encode(u32::MAX));
            assert!(frame_length(&header, MAX_FRAME).is_err());
        }
    }
    #[test]
    fn oversized_peer_is_rejected_from_header_without_waiting_for_a_body() {
        use std::io::Write;
        let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let mut header = [0u8; 16];
        header[0] = b'l';
        header[3] = 1;
        header[4..8].copy_from_slice(&(128u32 * 1024 * 1024).to_le_bytes());
        peer.write_all(&header).unwrap();
        let mut read = LimitedRead {
            state: Arc::new(TransportState::default()),
            inner: Arc::new(Async::new(stream).unwrap()),
            auth_bytes: 0,
            frame_bytes: 0,
            frames: 0,
            limits: Limits::CONTROL,
        };
        // Peer deliberately stays open without sending a body.
        async_io::block_on(async {
            let result = futures_lite::future::race(
                async {
                    read.receive_message(1, &mut Vec::new(), &mut Vec::new())
                        .await
                        .map(|_| ())
                },
                async {
                    async_io::Timer::after(std::time::Duration::from_millis(100)).await;
                    panic!("waited for oversized body")
                },
            )
            .await;
            assert!(result.is_err());
        });
    }
    fn socket_test<T>(future: impl std::future::Future<Output = T>) -> T {
        async_io::block_on(futures_lite::future::race(future, async {
            async_io::Timer::after(std::time::Duration::from_secs(3)).await;
            panic!("bounded socket test exceeded watchdog");
        }))
    }

    fn signal(sequence: u32) -> Message {
        Message::signal("/org/nickel/Test", "org.nickel.Test", "Changed")
            .unwrap()
            .build(&(sequence, "bounded fixture"))
            .unwrap()
    }

    #[test]
    fn authentication_bytes_are_cumulative_across_socket_reads() {
        use std::io::Write;
        let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let limit = 31;
        peer.write_all(&[b'A'; 64]).unwrap();
        let mut socket = bounded_socket(
            Async::new(stream).unwrap(),
            Limits {
                auth_bytes: limit,
                ..Limits::CONTROL
            },
        );
        socket_test(async {
            let mut total = 0;
            while total < limit {
                let mut bytes = [0; 7];
                let (count, descriptors) = socket.read_mut().recvmsg(&mut bytes).await.unwrap();
                assert!(count > 0);
                assert!(descriptors.is_empty());
                total += count;
            }
            assert_eq!(total, limit);
            let error = socket.read_mut().recvmsg(&mut [0; 7]).await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("authentication exceeds bounds"));
        });
    }

    #[test]
    fn real_handshake_rejects_auth_flood_before_setup_timeout() {
        handshake_peer(false);
    }

    #[test]
    fn real_handshake_times_out_while_accepted_peer_stays_silent() {
        handshake_peer(true);
    }

    fn handshake_peer(silent: bool) {
        use futures_lite::io::{AsyncReadExt, AsyncWriteExt};
        use std::time::{Duration, Instant};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bus");
        let listener = Async::new(std::os::unix::net::UnixListener::bind(&path).unwrap()).unwrap();
        let address: zbus::Address = format!("unix:path={}", path.display()).parse().unwrap();
        let started = Instant::now();
        socket_test(async {
            let (result, ()) = futures_lite::future::zip(
                connect(
                    address,
                    Limits {
                        auth_bytes: 64,
                        ..Limits::CONTROL
                    },
                    Duration::from_millis(100),
                ),
                async {
                    let (mut peer, _) = listener.accept().await.unwrap();
                    let mut auth = Vec::new();
                    while !auth.ends_with(b"\r\n") {
                        let mut byte = [0];
                        assert_eq!(peer.read(&mut byte).await.unwrap(), 1);
                        auth.push(byte[0]);
                        assert!(auth.len() < 128);
                    }
                    assert!(auth.windows(4).any(|bytes| bytes == b"AUTH"));
                    if !silent {
                        // Deliberately omit CRLF: the real auth parser must ask
                        // the bounded transport for more bytes, not reject a GUID.
                        peer.write_all(&[b'A'; 256]).await.unwrap();
                    }
                    // Keep the accepted stream alive past the helper's timeout,
                    // so EOF cannot accidentally satisfy either rejection test.
                    async_io::Timer::after(Duration::from_millis(200)).await;
                    drop(peer);
                },
            )
            .await;
            let error = result.unwrap_err();
            assert_eq!(
                error,
                if silent {
                    "bus setup timed out"
                } else {
                    "bus authentication unavailable"
                }
            );
        });
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn frame_count_limit_follows_successfully_decoded_socket_messages() {
        cumulative_limit(true);
    }

    #[test]
    fn cumulative_wire_limit_rejects_next_header_before_waiting_for_body() {
        cumulative_limit(false);
    }

    fn cumulative_limit(count_limit: bool) {
        use std::io::Write;
        let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let messages = [signal(1), signal(2), signal(3)];
        let first_two_bytes = messages[..2]
            .iter()
            .map(|message| message.data().len())
            .sum();
        let limits = if count_limit {
            Limits {
                frames: 2,
                ..Limits::CONTROL
            }
        } else {
            Limits {
                wire_bytes: first_two_bytes,
                ..Limits::CONTROL
            }
        };
        for message in &messages[..2] {
            peer.write_all(message.data().bytes()).unwrap();
        }
        // A limit violation must not wait for the third body. Keeping the peer
        // open also distinguishes bounded rejection from ordinary EOF failure.
        peer.write_all(&messages[2].data().bytes()[..16]).unwrap();
        let mut socket = bounded_socket(Async::new(stream).unwrap(), limits);
        socket_test(async {
            let mut buffered = Vec::new();
            let mut fds = Vec::new();
            for sequence in 1..=2 {
                let decoded = socket
                    .read_mut()
                    .receive_message(sequence, &mut buffered, &mut fds)
                    .await
                    .unwrap();
                let (value, text): (u32, String) = decoded.body().deserialize().unwrap();
                assert_eq!(value, sequence as u32);
                assert_eq!(text, "bounded fixture");
            }
            let result = futures_lite::future::race(
                socket
                    .read_mut()
                    .receive_message(3, &mut buffered, &mut fds),
                async {
                    async_io::Timer::after(std::time::Duration::from_millis(100)).await;
                    panic!("limit check waited for the withheld message body");
                },
            )
            .await;
            assert!(matches!(result, Err(zbus::Error::ExcessData)));
        });
    }

    #[test]
    fn authentication_socket_rejects_received_descriptors() {
        use std::os::fd::AsFd;
        use zbus::connection::socket::WriteHalf;
        let (stream, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let file = tempfile::tempfile().unwrap();
        let mut peer = Arc::new(Async::new(peer).unwrap());
        let mut socket = bounded_socket(Async::new(stream).unwrap(), Limits::CONTROL);
        socket_test(async {
            assert_eq!(peer.sendmsg(b"A", &[file.as_fd()]).await.unwrap(), 1);
            let error = socket.read_mut().recvmsg(&mut [0; 16]).await.unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::InvalidData);
            assert!(error.to_string().contains("descriptors are not allowed"));
        });
    }

    #[test]
    fn valid_descriptor_message_is_rejected_in_header_and_delegated_body_reads() {
        use std::io::Write;
        use std::os::fd::AsFd;
        use zbus::connection::socket::WriteHalf;
        let file = tempfile::tempfile().unwrap();
        let message = Message::signal("/org/nickel/Test", "org.nickel.Test", "Descriptor")
            .unwrap()
            .build(&(zbus::zvariant::Fd::from(file.as_fd()),))
            .unwrap();
        for bounded in [false, true] {
            for prefix in [0, 16] {
                let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
                peer.write_all(&message.data().bytes()[..prefix]).unwrap();
                let mut peer = Arc::new(Async::new(peer).unwrap());
                let mut reader: Box<dyn ReadHalf> = if bounded {
                    bounded_socket(Async::new(stream).unwrap(), Limits::CONTROL)
                        .take()
                        .0
                } else {
                    Box::new(Arc::new(Async::new(stream).unwrap()))
                };
                socket_test(async {
                    let tail = &message.data().bytes()[prefix..];
                    assert_eq!(
                        peer.sendmsg(tail, &[file.as_fd()]).await.unwrap(),
                        tail.len()
                    );
                    let result = reader
                        .receive_message(1, &mut Vec::new(), &mut Vec::new())
                        .await;
                    if bounded {
                        assert!(matches!(result, Err(zbus::Error::ExcessData)));
                    } else {
                        // Positive control: these are valid frames and ancillary
                        // data accepted by the exact zbus parser we delegate to.
                        let decoded = result.unwrap();
                        assert_eq!(decoded.data().fds().len(), 1);
                        let (_descriptor,): (zbus::zvariant::OwnedFd,) =
                            decoded.body().deserialize().unwrap();
                    }
                });
            }
        }
    }
    fn small_send_socket() -> (BoxedSplit, GuardedSender, std::os::unix::net::UnixStream) {
        use std::os::fd::AsRawFd;
        let (stream, peer) = std::os::unix::net::UnixStream::pair().unwrap();
        let bytes: libc::c_int = 1024;
        // SAFETY: this owned Unix socket and correctly sized integer live across
        // setsockopt; it affects only this test's private socket pair.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&bytes as *const libc::c_int).cast(),
                    std::mem::size_of_val(&bytes) as libc::socklen_t,
                )
            },
            0
        );
        peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let (socket, sender) = socket_with_sender(Async::new(stream).unwrap(), Limits::CONTROL);
        // Socket-level tests isolate post-auth writes; a separate real handshake
        // test verifies this transition is published only after Hello completes.
        sender.state.authenticated.store(true, Ordering::Release);
        (socket, sender, peer)
    }

    fn large_signal() -> Message {
        Message::signal("/org/nickel/Test", "org.nickel.Test", "Large")
            .unwrap()
            .build(&(vec![0x55_u8; 7000],))
            .unwrap()
    }

    #[test]
    fn guarded_send_rechecks_between_real_partial_native_writes() {
        use std::io::{Read, Write};
        for revoke in [false, true] {
            let (mut socket, sender, mut peer) = small_send_socket();
            let message = large_signal();
            assert!(message.data().len() < Limits::CONTROL.frame_bytes);
            let allowed = AtomicBool::new(true);
            let mut checks = 0;
            let mut received = Vec::new();
            let result = sender.send_guarded(&message, || {
                checks += 1;
                if checks == 2 {
                    let mut prefix = [0_u8; 8192];
                    let count = peer.read(&mut prefix).unwrap();
                    assert!(
                        count > 0 && count < message.data().len(),
                        "first native send must be partial"
                    );
                    received.extend_from_slice(&prefix[..count]);
                    // The peer has drained the prefix, so another native write
                    // would now succeed. Rejection must be the authority check,
                    // not a socket Pending/WouldBlock state.
                    if revoke {
                        allowed.store(false, Ordering::Release);
                    }
                }
                if allowed.load(Ordering::Acquire) {
                    Ok(())
                } else {
                    Err("revoked".into())
                }
            });
            assert_eq!(checks, 2);
            if revoke {
                assert_eq!(result, Err(GuardedSendError::Uncertain));
                assert!(sender.is_retired());
                let before = received.len();
                peer.read_to_end(&mut received).unwrap();
                assert_eq!(
                    received.len(),
                    before,
                    "bytes escaped after the boundary was revoked"
                );
                assert_eq!(received, &message.data().bytes()[..before]);
                assert!(socket_test(socket.write_mut().send_message(&signal(9))).is_err());
                assert!(peer.write_all(b"cannot reopen").is_err());
            } else {
                assert_eq!(result, Ok(()));
                assert!(!sender.is_retired());
                let mut tail = vec![0; message.data().len() - received.len()];
                peer.read_exact(&mut tail).unwrap();
                received.extend_from_slice(&tail);
                assert_eq!(received, message.data().bytes());
                sender.retire();
            }
        }
    }

    #[test]
    fn ordinary_partial_frame_excludes_guarded_frame_and_retirement_prevents_append() {
        use std::{
            future::Future,
            io::Read,
            task::{Context, Poll, Waker},
        };
        let (mut socket, sender, mut peer) = small_send_socket();
        let ordinary = large_signal();
        let mut sending = Box::pin(socket.write_mut().send_message(&ordinary));
        assert!(matches!(
            sending
                .as_mut()
                .poll(&mut Context::from_waker(Waker::noop())),
            Poll::Pending
        ));
        let mut bytes = [0; 8192];
        let count = peer.read(&mut bytes).unwrap();
        assert!(count > 0 && count < ordinary.data().len());
        let result =
            sender.send_guarded(&signal(7), || panic!("ordinary frame still owns exclusion"));
        assert_eq!(result, Err(GuardedSendError::NotAccepted));
        assert!(sender.is_retired());
        // Drop the parked ordinary future. Its remainder cannot be sent later.
        drop(sending);
        let mut tail = Vec::new();
        peer.read_to_end(&mut tail).unwrap();
        assert!(tail.is_empty());
        assert_eq!(&bytes[..count], &ordinary.data().bytes()[..count]);
        assert!(socket_test(socket.write_mut().send_message(&signal(8))).is_err());
    }

    #[test]
    fn guarded_send_before_auth_or_after_rejection_never_emits_bytes() {
        use std::io::Read;
        let (stream, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
        peer.set_read_timeout(Some(std::time::Duration::from_secs(1)))
            .unwrap();
        let (_socket, sender) = socket_with_sender(Async::new(stream).unwrap(), Limits::CONTROL);
        assert_eq!(
            sender.send_guarded(&signal(1), || Ok(())),
            Err(GuardedSendError::NotAccepted)
        );
        assert!(sender.is_retired());
        sender.state.authenticated.store(true, Ordering::Release);
        assert_eq!(
            sender.send_guarded(&signal(2), || Ok(())),
            Err(GuardedSendError::NotAccepted)
        );
        let mut bytes = Vec::new();
        peer.read_to_end(&mut bytes).unwrap();
        assert!(bytes.is_empty());
    }

    #[test]
    fn guarded_send_rejects_fds_and_oversized_serialized_frames_before_write() {
        use std::{io::Read, os::fd::AsFd};
        let file = tempfile::tempfile().unwrap();
        let descriptor = Message::signal("/org/nickel/Test", "org.nickel.Test", "Fd")
            .unwrap()
            .build(&(zbus::zvariant::Fd::from(file.as_fd()),))
            .unwrap();
        let oversized = Message::signal("/org/nickel/Test", "org.nickel.Test", "Huge")
            .unwrap()
            .build(&(vec![0_u8; 8192],))
            .unwrap();
        for message in [descriptor, oversized] {
            let (_socket, sender, mut peer) = small_send_socket();
            assert_eq!(
                sender.send_guarded(&message, || panic!("invalid serialized frame")),
                Err(GuardedSendError::NotAccepted)
            );
            assert!(sender.is_retired());
            let mut bytes = Vec::new();
            peer.read_to_end(&mut bytes).unwrap();
            assert!(bytes.is_empty());
        }
    }

    #[test]
    fn guarded_sender_is_published_after_real_authentication_and_hello() {
        use std::time::Duration;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("bus");
        let listener = Async::new(std::os::unix::net::UnixListener::bind(&path).unwrap()).unwrap();
        let address: zbus::Address = format!("unix:path={}", path.display()).parse().unwrap();
        socket_test(async {
            futures_lite::future::zip(
                async {
                    let (_connection, sender) =
                        connect_guarded(address, Limits::CONTROL, Duration::from_millis(500))
                            .await
                            .unwrap();
                    sender.send_guarded(&signal(42), || Ok(())).unwrap();
                },
                async {
                    let (peer, _) = listener.accept().await.unwrap();
                    let mut peer = Arc::new(peer);
                    async fn line(
                        peer: &mut Arc<Async<std::os::unix::net::UnixStream>>,
                    ) -> Vec<u8> {
                        let mut bytes = Vec::new();
                        while !bytes.ends_with(b"\r\n") {
                            let mut byte = [0];
                            let (count, fds) = ReadHalf::recvmsg(peer, &mut byte).await.unwrap();
                            assert_eq!(count, 1);
                            assert!(fds.is_empty());
                            bytes.push(byte[0]);
                            assert!(bytes.len() < 128);
                        }
                        bytes
                    }
                    assert!(
                        line(&mut peer)
                            .await
                            .windows(4)
                            .any(|value| value == b"AUTH")
                    );
                    let ok = b"OK 0123456789abcdef0123456789abcdef\r\n";
                    assert_eq!(peer.sendmsg(ok, &[]).await.unwrap(), ok.len());
                    assert_eq!(line(&mut peer).await, b"BEGIN\r\n");
                    let hello = peer
                        .receive_message(1, &mut Vec::new(), &mut Vec::new())
                        .await
                        .unwrap();
                    assert_eq!(hello.header().member().unwrap().as_str(), "Hello");
                    let reply = Message::method_return(&hello.header())
                        .unwrap()
                        .build(&":1.77")
                        .unwrap();
                    peer.send_message(&reply).await.unwrap();
                    let frame = peer
                        .receive_message(2, &mut Vec::new(), &mut Vec::new())
                        .await
                        .unwrap();
                    let (sequence, text): (u32, String) = frame.body().deserialize().unwrap();
                    assert_eq!((sequence, text.as_str()), (42, "bounded fixture"));
                },
            )
            .await;
        });
    }
}
