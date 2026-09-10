// SPDX-License-Identifier: MIT
// SPDX-FileCopyrightText: Copyright (c) 2025 Asymptotic Inc.
// SPDX-FileCopyrightText: Copyright (c) 2025 Arun Raghavan

use std::{
    io::{Read, Write},
    os::{fd::RawFd, unix::net::UnixStream},
    sync::{Arc, Mutex, RwLock},
};

use pipewire_native_spa::{self as spa, pod::Pod};

use crate::{
    debug, default_topic, log, new_refcounted, protocol::ASYNC_SEQ_MASK, refcounted, trace, Id,
};

use super::marshal::{
    self,
    message::{ClientFooter, Header, Message},
    message::{ClientFooterPayload, ClientGeneration, CoreFooter, CoreFooterPayload},
    Marshallable,
};

default_topic!(log::topic::CONNECTION);

const MAX_MESSAGE_SIZE: usize = 16_777_216;

refcounted! {
    pub(crate) struct Connection {
        stream: RwLock<Option<UnixStream>>,
        hooks: Arc<Mutex<spa::hook::HookList<ConnectionEvents>>>,
        // Data received
        in_buf: RwLock<Vec<u8>>,
        in_size: RwLock<usize>,
        in_offset: RwLock<usize>,
        last_recv_generation: RwLock<i64>,
        // Data to send
        out_seq: RwLock<u32>,
        out_buf: RwLock<Vec<u8>>,
        out_size: RwLock<usize>,
        out_fds: RwLock<Vec<RawFd>>,
        last_sent_generation: RwLock<i64>,
        max_frame_bytes: RwLock<Option<usize>>,
    }
}

#[allow(unused)]
pub(crate) struct ConnectionEvents {
    pub(crate) destroy: Option<Box<dyn FnMut()>>,
    pub(crate) error: Option<Box<dyn FnMut(u32)>>,
    pub(crate) need_flush: Option<Box<dyn FnMut()>>,
    pub(crate) start: Option<Box<dyn FnMut(u32)>>,
}

impl Connection {
    pub(crate) fn new(stream: Option<UnixStream>) -> Self {
        debug!("Creating new connection to {stream:?}");
        Self {
            inner: new_refcounted(InnerConnection::new(stream)),
        }
    }

    pub(crate) fn next_seq(&self) -> u32 {
        *self.inner.out_seq.read().unwrap()
    }

    pub(crate) fn set_stream(&self, stream: UnixStream) {
        self.inner.stream.write().unwrap().replace(stream);
    }

    pub(crate) fn disconnect(&self) {
        self.inner.stream.write().unwrap().take();
        self.clear_buffers();
    }

    fn clear_buffers(&self) {
        self.inner.in_buf.write().unwrap().fill(0);
        *self.inner.in_size.write().unwrap() = 0;
        *self.inner.in_offset.write().unwrap() = 0;
        *self.inner.last_recv_generation.write().unwrap() = 0;
        *self.inner.out_seq.write().unwrap() = 0;
        self.inner.out_buf.write().unwrap().fill(0);
        *self.inner.out_size.write().unwrap() = 0;
        self.inner.out_fds.write().unwrap().clear();
        *self.inner.last_sent_generation.write().unwrap() = 0;
    }

    pub(crate) fn add_listener(&self, events: ConnectionEvents) -> spa::hook::HookId {
        self.inner.hooks.lock().unwrap().append(events)
    }

    pub(crate) fn remove_listener(&self, listener: spa::hook::HookId) {
        let _ = self.inner.hooks.lock().unwrap().remove(listener);
    }

    pub(crate) fn push<T: Marshallable + std::fmt::Debug>(
        &self,
        id: Id,
        object: T,
    ) -> std::io::Result<()> {
        let seq = *self.inner.out_seq.read().unwrap();

        let recv_generation = self.inner.last_recv_generation.read().unwrap();
        let mut sent_generation = self.inner.last_sent_generation.write().unwrap();

        // TODO: support CoreGeneration as well when we implement server
        let footer = if *recv_generation > *sent_generation {
            *sent_generation = *recv_generation;
            trace!("sending client generation {}", *recv_generation);
            let mut footer = ClientFooter::new();
            footer.push(ClientFooterPayload::Generation(ClientGeneration {
                client_generation: *recv_generation,
            }));
            Some(footer)
        } else {
            None
        };

        let message = Message {
            header: Header {
                id,
                opcode: object.opcode(),
                seq,
                size: 0,  // filled by encode
                n_fds: 0, // TOOO
            },
            object,
            footer,
        };

        let mut buf = self.inner.out_buf.write().unwrap();
        let mut size = self.inner.out_size.write().unwrap();

        loop {
            let rest = &mut buf.as_mut_slice()[*size..];
            match message.encode(rest) {
                Ok(written) => {
                    *size += written;
                    break;
                }
                Err(spa::pod::Error::NoSpace) => {
                    let capacity = buf.len();
                    if capacity > MAX_MESSAGE_SIZE {
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::InvalidInput,
                            format!("cannot send message > {MAX_MESSAGE_SIZE}"),
                        ));
                    }
                    buf.resize(capacity * 2, 0);
                    // And now we try again
                }
                _ => unreachable!(),
            }
        }

        trace!(
            "pushed message id:{id} opcode:{} seq:{seq} payload:{:?} (filled: {size})",
            message.header.opcode,
            message.object
        );

        *self.inner.out_seq.write().unwrap() = (seq + 1) & ASYNC_SEQ_MASK;
        spa::emit_hook!(self.inner.hooks, need_flush);

        Ok(())
    }

    /// Send at most one bounded chunk without dispatching callbacks or retrying.
    pub(crate) fn try_flush_once(&self, max_pending: usize) -> std::io::Result<usize> {
        let mut stream = self.inner.stream.write().unwrap();
        let stream = stream
            .as_mut()
            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::NotConnected))?;
        let mut buf = self.inner.out_buf.write().unwrap();
        let mut size = self.inner.out_size.write().unwrap();
        flush_once(stream, &mut buf, &mut size, max_pending)
    }

    pub(crate) fn flush(&self) -> std::io::Result<()> {
        let mut o_stream = self.inner.stream.write().unwrap();
        let stream = o_stream.as_mut().unwrap();
        let mut buf = self.inner.out_buf.write().unwrap();
        let mut size = self.inner.out_size.write().unwrap();
        let mut idx = 0;
        let mut res = Ok(());

        trace!("flushing {} bytes", *size);

        while idx < *size {
            let sent = match stream.write(&buf[idx..*size]) {
                Ok(size) => size,
                Err(err) => {
                    if err.kind() == std::io::ErrorKind::Interrupted {
                        continue;
                    } else {
                        res = Err(err);
                        break;
                    }
                }
            };

            idx += sent;
        }

        if idx == buf.len() {
            buf.clear();
            *size = 0;
        } else {
            buf.copy_within(idx.., 0);
            *size -= idx;
        }

        res
    }

    pub(crate) fn has_buffered_input(&self) -> bool {
        *self.inner.in_size.read().unwrap() > *self.inner.in_offset.read().unwrap()
    }

    pub(crate) fn set_frame_limit(&self, bytes: usize) {
        *self.inner.max_frame_bytes.write().unwrap() = Some(bytes);
    }

    pub(crate) fn next_message(&self) -> std::io::Result<Header> {
        let frame_limit = *self.inner.max_frame_bytes.read().unwrap();
        if frame_limit.is_some() {
            let mut buf = self.inner.in_buf.write().unwrap();
            let mut size = self.inner.in_size.write().unwrap();
            let mut offset = self.inner.in_offset.write().unwrap();
            if *offset > 0 {
                buf.copy_within(*offset..*size, 0);
                *size -= *offset;
                *offset = 0;
            }
        }
        loop {
            let (wanted_capacity, header) = self.parse_next()?;
            if frame_limit.is_some_and(|limit| wanted_capacity > limit) {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "incoming frame exceeds guarded bound",
                ));
            }
            trace!(
                "we need {wanted_capacity}, got header: {}",
                header.is_some()
            );

            let capacity = self.inner.in_buf.read().unwrap().len();
            if capacity < wanted_capacity {
                // Not enough space for header or message, make some space, try to fill some data,
                // and then retry
                trace!(
                    "expanding capacity to {}",
                    wanted_capacity.max(capacity * 2)
                );
                self.inner.in_buf.write().unwrap().resize(
                    wanted_capacity
                        .max(2 * capacity)
                        .min(frame_limit.unwrap_or(usize::MAX)),
                    0,
                );
                self.read()?;
            } else if let Some(header) = header {
                // We had enough space, and got the header.
                trace!(
                    "got message id:{} opcode:{} seq:{} size:{}",
                    header.id,
                    header.opcode,
                    header.seq,
                    header.size
                );

                // Let's make sure we also have the body
                let available =
                    *self.inner.in_size.read().unwrap() - *self.inner.in_offset.read().unwrap();
                if available >= header.size as usize {
                    return Ok(header);
                } else {
                    // We read the header but not the data, so continue reading.
                    self.read()?;
                }
            } else {
                // We had enough space, but don't have the data, let's try to read data into the
                // buffer
                self.read()?;
            }
        }
    }

    pub(crate) fn decode_message<T: Marshallable, F: Pod<DecodesTo = F>>(
        &self,
        header: &Header,
    ) -> std::io::Result<(T, Option<F>)> {
        let buf = self.inner.in_buf.read().unwrap();
        let mut size = self.inner.in_size.write().unwrap();
        let mut offset = self.inner.in_offset.write().unwrap();

        let start = *offset + marshal::HEADER_LEN;
        let end = start + header.size as usize;

        // Update the external offset and size before possibly bailing out with an error, otherwise
        // we could be reading the same chunk again and again when that happens.
        *offset += marshal::HEADER_LEN + header.size as usize;
        if *offset == *size {
            // We've consumed all the data
            *offset = 0;
            *size = 0;
        }

        let (body, body_size) = T::decode(header.opcode, &buf[start..end]).map_err(|e| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Could not decode message body: {e:?}"),
            )
        })?;

        let (footer, footer_size) = if body_size < header.size as usize {
            let (f, fs) = F::decode(&buf[start + body_size..]).map_err(|e| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Could not decode message footer: {e:?}"),
                )
            })?;
            (Some(f), fs)
        } else {
            (None, 0)
        };

        if body_size + footer_size != header.size as usize {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Mismatched message size({}) and body size({}) + footer_size({})",
                    header.size, body_size, footer_size
                ),
            ));
        }

        Ok((body, footer))
    }

    pub(crate) fn decode_core_message<T: Marshallable>(
        &self,
        header: &Header,
    ) -> std::io::Result<T> {
        let (object, footer) = self.decode_message(header)?;

        self.update_generation(footer.as_ref());

        Ok(object)
    }

    // TODO: support CoreGeneration as well when we implement server
    pub fn update_generation(&self, footer: Option<&CoreFooter>) {
        if let Some(footer) = footer {
            for p in &footer.payloads {
                match p {
                    CoreFooterPayload::Generation(g) => {
                        trace!("updating core generation to {}", g.registry_generation);
                        *self.inner.last_recv_generation.write().unwrap() = g.registry_generation;
                    }
                }
            }
        }
    }

    fn parse_next(&self) -> std::io::Result<(usize, Option<Header>)> {
        let size = *self.inner.in_size.read().unwrap();
        let offset = *self.inner.in_offset.read().unwrap();

        if size - offset < marshal::HEADER_LEN {
            return Ok((offset + marshal::HEADER_LEN, None));
        }

        trace!("looking for message header from [{offset}..{size}]");

        let buf = self.inner.in_buf.read().unwrap();
        let header = match Header::decode(&buf[offset..size]) {
            Ok((header, _)) => header,
            Err(e) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("Failed to parse message: {e:?}"),
                ))
            }
        };

        Ok((
            offset + marshal::HEADER_LEN + header.size as usize,
            Some(header),
        ))
    }

    fn read(&self) -> std::io::Result<()> {
        let mut stream_ref = self.inner.stream.write().unwrap();
        let stream = stream_ref.as_mut().unwrap();
        let mut buf = self.inner.in_buf.write().unwrap();
        let mut size = self.inner.in_size.write().unwrap();

        let read = stream.read(&mut buf[*size..])?;
        trace!("read {read} bytes at {size}");

        // TODO: control messages

        if read > 0 {
            *size += read;
            Ok(())
        } else {
            // Nothing to process, we're done
            Err(std::io::Error::from_raw_os_error(libc::EAGAIN))
        }
    }
}

impl InnerConnection {
    pub(crate) fn new(stream: Option<UnixStream>) -> Self {
        InnerConnection {
            stream: RwLock::new(stream),
            hooks: spa::hook::HookList::new(),
            in_buf: RwLock::new(vec![0; 16384]),
            in_size: RwLock::new(0),
            in_offset: RwLock::new(0),
            last_recv_generation: RwLock::new(0),
            out_seq: RwLock::new(0),
            out_buf: RwLock::new(vec![0; 16384]),
            out_size: RwLock::new(0),
            out_fds: RwLock::new(Vec::new()),
            last_sent_generation: RwLock::new(0),
            max_frame_bytes: RwLock::new(None),
        }
    }
}

// The stream is nonblocking; no retry on Interrupted or WouldBlock. Keeping this
// step independent of the event loop makes its native acceptance boundary usable
// by callers that must hold authorization across precisely one socket write.
fn flush_once(
    stream: &mut impl Write,
    buf: &mut Vec<u8>,
    size: &mut usize,
    max_pending: usize,
) -> std::io::Result<usize> {
    if *size > max_pending || max_pending > 65_536 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "pending write exceeds bound",
        ));
    }
    if *size == 0 {
        return Ok(0);
    }
    let sent = stream.write(&buf[..*size])?;
    if sent == 0 {
        return Err(std::io::ErrorKind::WriteZero.into());
    }
    let previous_size = *size;
    buf.copy_within(sent..previous_size, 0);
    *size -= sent;
    // The provider treats Vec length as writable encoding capacity. Preserve
    // that allocation: truncating to zero makes its next encode grow 0 * 2
    // forever. Only out_size denotes queued bytes.
    buf[*size..previous_size].fill(0);
    Ok(*size)
}

#[cfg(test)]
mod guarded_flush_tests {
    use super::*;
    struct ShortWriter {
        bytes: Vec<u8>,
        limit: usize,
        calls: usize,
    }
    impl Write for ShortWriter {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.calls += 1;
            if self.limit == 0 {
                return Err(std::io::ErrorKind::WouldBlock.into());
            }
            let count = bytes.len().min(self.limit);
            self.bytes.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            panic!("must not flush implicitly")
        }
    }
    #[test]
    fn partial_write_is_one_call_and_preserves_only_unsent_bytes() {
        let mut writer = ShortWriter {
            bytes: vec![],
            limit: 2,
            calls: 0,
        };
        let mut buf = vec![1, 2, 3, 4, 5];
        let mut size = 5;
        assert_eq!(flush_once(&mut writer, &mut buf, &mut size, 32).unwrap(), 3);
        assert_eq!(writer.calls, 1);
        assert_eq!(writer.bytes, [1, 2]);
        assert_eq!(&buf[..size], [3, 4, 5]);
        assert_eq!(flush_once(&mut writer, &mut buf, &mut size, 32).unwrap(), 1);
        assert_eq!(flush_once(&mut writer, &mut buf, &mut size, 32).unwrap(), 0);
        assert_eq!(writer.bytes, [1, 2, 3, 4, 5]);
        assert_eq!(size, 0);
        assert_eq!(buf.len(), 5);
    }
    #[test]
    fn would_block_and_oversized_queue_do_not_consume_or_retry() {
        let mut writer = ShortWriter {
            bytes: vec![],
            limit: 0,
            calls: 0,
        };
        let mut buf = vec![1, 2, 3];
        let mut size = 3;
        assert_eq!(
            flush_once(&mut writer, &mut buf, &mut size, 2)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::InvalidInput
        );
        assert_eq!(writer.calls, 0);
        assert_eq!(
            flush_once(&mut writer, &mut buf, &mut size, 3)
                .unwrap_err()
                .kind(),
            std::io::ErrorKind::WouldBlock
        );
        assert_eq!(writer.calls, 1);
        assert_eq!(buf, [1, 2, 3]);
        assert_eq!(size, 3);
    }
    #[test]
    fn oversized_frame_is_rejected_before_growing_receive_storage() {
        crate::init();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let connection = Connection::new(Some(stream));
        connection.set_frame_limit(16_384);
        let header = Header {
            id: 0,
            opcode: 0,
            seq: 0,
            size: 1_000_000,
            n_fds: 0,
        };
        let mut encoded = [0; 16];
        header.encode(&mut encoded).unwrap();
        peer.write_all(&encoded).unwrap();
        assert_eq!(
            connection.next_message().err().unwrap().kind(),
            std::io::ErrorKind::InvalidData
        );
        assert_eq!(connection.inner.in_buf.read().unwrap().len(), 16_384);
    }

    #[test]
    fn abandoned_partial_native_write_never_flushes_its_remainder() {
        use std::os::fd::AsRawFd;
        crate::init();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        stream.set_nonblocking(true).unwrap();
        let capacity: libc::c_int = 1024;
        // SAFETY: the owned socket is live and capacity points to initialized
        // c_int storage of the exact length passed to setsockopt.
        assert_eq!(
            unsafe {
                libc::setsockopt(
                    stream.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_SNDBUF,
                    (&capacity as *const libc::c_int).cast(),
                    std::mem::size_of_val(&capacity) as libc::socklen_t,
                )
            },
            0
        );
        let connection = Connection::new(Some(stream));
        *connection.inner.out_buf.write().unwrap() = vec![7; 65_536];
        *connection.inner.out_size.write().unwrap() = 65_536;
        let remaining = connection.try_flush_once(65_536).unwrap();
        assert!(remaining > 0 && remaining < 65_536);
        // Cancellation abandons this isolated transport. Closing must not flush
        // the remaining bytes even when the peer resumes consuming data.
        connection.disconnect();
        let mut received = Vec::new();
        peer.read_to_end(&mut received).unwrap();
        assert_eq!(received.len(), 65_536 - remaining);
        assert!(received.iter().all(|byte| *byte == 7));
    }

    #[test]
    fn disconnected_connection_discards_pending_bytes_without_writing() {
        crate::init();
        let (stream, mut peer) = UnixStream::pair().unwrap();
        peer.set_nonblocking(true).unwrap();
        let conn = Connection::new(Some(stream));
        *conn.inner.out_buf.write().unwrap() = vec![1, 2, 3];
        *conn.inner.out_size.write().unwrap() = 3;
        conn.disconnect();
        assert_eq!(*conn.inner.out_size.read().unwrap(), 0);
        assert_eq!(peer.read(&mut [0u8; 8]).unwrap(), 0);
        assert_eq!(
            conn.try_flush_once(32).unwrap_err().kind(),
            std::io::ErrorKind::NotConnected
        );
    }
}
