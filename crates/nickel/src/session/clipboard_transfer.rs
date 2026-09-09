//! Bounded clipboard I/O primitives. Callers retain recipient authority; workers
//! own only their descriptor, admission permit, and bounded text payload.

use smithay::reexports::rustix::{
    event::{PollFd, PollFlags, poll},
    fs::{OFlags, fcntl_getfl, fcntl_setfl},
    io::{read, write},
};
use std::{
    os::fd::OwnedFd,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

#[derive(Clone)]
pub(super) struct TransferGate(Arc<AtomicUsize>);
pub(super) struct TransferPermit(Arc<AtomicUsize>);

impl Default for TransferGate {
    fn default() -> Self {
        Self(Arc::new(AtomicUsize::new(0)))
    }
}

impl TransferGate {
    pub(super) fn acquire(&self, maximum: usize) -> Option<TransferPermit> {
        self.0
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                (active < maximum).then_some(active + 1)
            })
            .ok()?;
        Some(TransferPermit(self.0.clone()))
    }
}

impl Drop for TransferPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn ready(fd: &OwnedFd, interest: PollFlags, deadline: Instant) -> Result<(), &'static str> {
    let remaining = deadline
        .checked_duration_since(Instant::now())
        .ok_or("clipboard transfer timed out")?;
    let timeout = smithay::reexports::rustix::event::Timespec::try_from(remaining)
        .map_err(|_| "invalid clipboard transfer timeout")?;
    let mut descriptors = [PollFd::new(fd, interest)];
    let count =
        poll(&mut descriptors, Some(&timeout)).map_err(|_| "clipboard descriptor poll failed")?;
    if count == 0 {
        Err("clipboard transfer timed out")
    } else {
        Ok(())
    }
}

fn nonblocking(fd: &OwnedFd) -> Result<(), &'static str> {
    let flags = fcntl_getfl(fd).map_err(|_| "clipboard descriptor flags unavailable")?;
    fcntl_setfl(fd, flags | OFlags::NONBLOCK)
        .map_err(|_| "clipboard descriptor cannot be made nonblocking")
}

pub(super) fn read_text(
    fd: OwnedFd,
    maximum: usize,
    duration: Duration,
) -> Result<String, &'static str> {
    nonblocking(&fd)?;
    let deadline = Instant::now() + duration;
    let mut bytes = Vec::new();
    let mut scratch = [0u8; 4096];
    loop {
        ready(&fd, PollFlags::IN, deadline)?;
        match read(&fd, &mut scratch) {
            Ok(0) => {
                return String::from_utf8(bytes).map_err(|_| "clipboard text is not valid UTF-8");
            }
            Ok(length) => {
                if length > maximum.saturating_sub(bytes.len()) {
                    return Err("clipboard text exceeds transfer limit");
                }
                // Vec's usual geometric growth can exceed the admission limit
                // even when its length does not. Bound retained capacity too.
                if bytes.capacity().saturating_sub(bytes.len()) < length {
                    let capacity = bytes.capacity().saturating_mul(2).max(4096).min(maximum);
                    bytes.reserve_exact(capacity - bytes.len());
                }
                bytes.extend_from_slice(&scratch[..length]);
            }
            Err(smithay::reexports::rustix::io::Errno::AGAIN) => continue,
            Err(_) => return Err("clipboard read failed"),
        }
    }
}

pub(super) fn read_bytes(
    fd: OwnedFd,
    maximum: usize,
    duration: Duration,
) -> Result<Vec<u8>, &'static str> {
    nonblocking(&fd)?;
    let deadline = Instant::now() + duration;
    let mut bytes = Vec::new();
    let mut scratch = [0u8; 16 * 1024];
    loop {
        ready(&fd, PollFlags::IN, deadline)?;
        match read(&fd, &mut scratch) {
            Ok(0) => return Ok(bytes),
            Ok(length) => {
                if length > maximum.saturating_sub(bytes.len()) {
                    return Err("clipboard payload exceeds transfer limit");
                }
                bytes.extend_from_slice(&scratch[..length]);
            }
            Err(smithay::reexports::rustix::io::Errno::AGAIN) => continue,
            Err(_) => return Err("clipboard read failed"),
        }
    }
}

pub(super) fn write_text(
    fd: OwnedFd,
    text: Arc<String>,
    duration: Duration,
) -> Result<(), &'static str> {
    write_bytes(fd, text.as_bytes(), duration)
}

pub(super) fn write_bytes(
    fd: OwnedFd,
    bytes: &[u8],
    duration: Duration,
) -> Result<(), &'static str> {
    nonblocking(&fd)?;
    let deadline = Instant::now() + duration;
    let mut remaining = bytes;
    while !remaining.is_empty() {
        ready(&fd, PollFlags::OUT, deadline)?;
        match write(&fd, remaining) {
            Ok(0) => return Err("clipboard recipient stopped reading"),
            Ok(length) => remaining = &remaining[length..],
            Err(smithay::reexports::rustix::io::Errno::AGAIN) => continue,
            Err(_) => return Err("clipboard write failed"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Write, os::unix::net::UnixStream};

    #[test]
    fn transfer_gate_bounds_active_workers_and_releases_on_drop() {
        let gate = TransferGate::default();
        let permit = gate.acquire(1).unwrap();
        assert!(gate.acquire(1).is_none());
        drop(permit);
        assert!(gate.acquire(1).is_some());
    }

    #[test]
    fn reads_reject_oversize_invalid_utf8_and_stalled_sources() {
        for (payload, expected) in [
            (&b"large"[..], "clipboard text exceeds transfer limit"),
            (&b"\xff"[..], "clipboard text is not valid UTF-8"),
        ] {
            let (reader, mut writer) = UnixStream::pair().unwrap();
            writer.write_all(payload).unwrap();
            drop(writer);
            assert_eq!(
                read_text(reader.into(), 4, Duration::from_secs(1)),
                Err(expected)
            );
        }
        let (reader, _writer) = UnixStream::pair().unwrap();
        assert_eq!(
            read_text(reader.into(), 4, Duration::from_millis(10)),
            Err("clipboard transfer timed out")
        );
    }

    #[test]
    fn bounded_text_roundtrips_without_a_compositor_thread() {
        let (reader, writer) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            write_text(
                writer.into(),
                Arc::new("hello".into()),
                Duration::from_secs(1),
            )
        });
        assert_eq!(
            read_text(reader.into(), 5, Duration::from_secs(1)).unwrap(),
            "hello"
        );
        worker.join().unwrap().unwrap();
    }

    #[test]
    fn byte_reads_are_bounded() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        writer.write_all(b"12345").unwrap();
        drop(writer);
        assert_eq!(
            read_bytes(reader.into(), 4, Duration::from_secs(1)),
            Err("clipboard payload exceeds transfer limit")
        );
    }
}
