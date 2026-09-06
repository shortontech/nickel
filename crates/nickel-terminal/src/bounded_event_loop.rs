//! Bounded PTY polling loop built on Alacritty Terminal's public TTY contract.
//!
//! The I/O structure follows the Apache-2.0/MIT licensed
//! `alacritty_terminal::event_loop`, but Nickel owns admission and accounting:
//! accepted input bytes remain charged until the platform writer consumes them.

use std::{
    borrow::Cow,
    collections::VecDeque,
    io::{self, ErrorKind, Read, Write},
    num::NonZeroUsize,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError},
    },
    thread::JoinHandle,
    time::Instant,
};

use alacritty_terminal::{
    event::{Event, EventListener, OnResize, WindowSize},
    sync::FairMutex,
    term::Term,
    tty::{self, EventedPty},
    vte::ansi,
};
use polling::{Event as PollingEvent, Events, PollMode, Poller};

const READ_BUFFER_SIZE: usize = 0x10_0000;
const MAX_LOCKED_READ: usize = u16::MAX as usize;
const INPUT_PACKET_CAPACITY: usize = 64;
const PTY_CHILD_EVENT_TOKEN: usize = 1;
#[cfg(unix)]
const PTY_READ_WRITE_TOKEN: usize = 0;
#[cfg(target_os = "windows")]
const PTY_READ_WRITE_TOKEN: usize = 2;

#[derive(Clone)]
pub(crate) struct BoundedEventLoopSender {
    input: SyncSender<Vec<u8>>,
    poller: Arc<Poller>,
    pending_bytes: Arc<AtomicUsize>,
    maximum_pending_bytes: usize,
    resize: Arc<Mutex<Option<WindowSize>>>,
    shutdown: Arc<AtomicBool>,
}

#[derive(Debug)]
pub(crate) enum InputSendError {
    Full,
    Closed,
    Io(io::Error),
}

impl BoundedEventLoopSender {
    #[cfg(test)]
    pub(crate) fn test_channel(
        maximum_pending_bytes: usize,
    ) -> io::Result<(Self, Receiver<Vec<u8>>)> {
        let (input, receiver) = mpsc::sync_channel(INPUT_PACKET_CAPACITY);
        Ok((
            Self {
                input,
                poller: Arc::new(Poller::new()?),
                pending_bytes: Arc::new(AtomicUsize::new(0)),
                maximum_pending_bytes,
                resize: Arc::new(Mutex::new(None)),
                shutdown: Arc::new(AtomicBool::new(false)),
            },
            receiver,
        ))
    }

    pub(crate) fn try_input(&self, bytes: Vec<u8>) -> Result<(), InputSendError> {
        if bytes.is_empty() {
            return Ok(());
        }
        // Callers cannot smuggle disproportionate spare Vec capacity into the
        // retained queue while being charged only for its initialized length.
        let bytes = bytes.into_boxed_slice().into_vec();
        let len = bytes.len();
        let reserved =
            self.pending_bytes
                .fetch_update(Ordering::AcqRel, Ordering::Acquire, |pending| {
                    pending
                        .checked_add(len)
                        .filter(|next| *next <= self.maximum_pending_bytes)
                });
        if reserved.is_err() {
            return Err(InputSendError::Full);
        }
        match self.input.try_send(bytes) {
            Ok(()) => self.poller.notify().map_err(|error| {
                // The packet remains queued and charged even if notification
                // failed; report transport failure without corrupting budget.
                InputSendError::Io(error)
            }),
            Err(TrySendError::Full(_)) => {
                self.pending_bytes.fetch_sub(len, Ordering::AcqRel);
                Err(InputSendError::Full)
            }
            Err(TrySendError::Disconnected(_)) => {
                self.pending_bytes.fetch_sub(len, Ordering::AcqRel);
                Err(InputSendError::Closed)
            }
        }
    }

    pub(crate) fn resize(&self, size: WindowSize) -> io::Result<()> {
        *self
            .resize
            .lock()
            .unwrap_or_else(|error| error.into_inner()) = Some(size);
        self.poller.notify()
    }

    pub(crate) fn shutdown(&self) -> io::Result<()> {
        self.shutdown.store(true, Ordering::Release);
        self.poller.notify()
    }

    #[cfg(test)]
    pub(crate) fn pending_bytes(&self) -> usize {
        self.pending_bytes.load(Ordering::Acquire)
    }
}

pub(crate) struct BoundedEventLoop<T: EventedPty, U: EventListener> {
    poller: Arc<Poller>,
    pty: T,
    input: Receiver<Vec<u8>>,
    terminal: Arc<FairMutex<Term<U>>>,
    event_proxy: U,
    pending_bytes: Arc<AtomicUsize>,
    resize: Arc<Mutex<Option<WindowSize>>>,
    shutdown: Arc<AtomicBool>,
    drain_on_exit: bool,
}

impl<T, U> BoundedEventLoop<T, U>
where
    T: EventedPty + OnResize + Send + 'static,
    U: EventListener + Send + 'static,
{
    pub(crate) fn new(
        terminal: Arc<FairMutex<Term<U>>>,
        event_proxy: U,
        pty: T,
        drain_on_exit: bool,
        maximum_pending_bytes: usize,
    ) -> io::Result<(Self, BoundedEventLoopSender)> {
        let (input, receiver) = mpsc::sync_channel(INPUT_PACKET_CAPACITY);
        let poller = Arc::new(Poller::new()?);
        let pending_bytes = Arc::new(AtomicUsize::new(0));
        let resize = Arc::new(Mutex::new(None));
        let shutdown = Arc::new(AtomicBool::new(false));
        let sender = BoundedEventLoopSender {
            input,
            poller: Arc::clone(&poller),
            pending_bytes: Arc::clone(&pending_bytes),
            maximum_pending_bytes,
            resize: Arc::clone(&resize),
            shutdown: Arc::clone(&shutdown),
        };
        Ok((
            Self {
                poller,
                pty,
                input: receiver,
                terminal,
                event_proxy,
                pending_bytes,
                resize,
                shutdown,
                drain_on_exit,
            },
            sender,
        ))
    }

    fn receive_control(&mut self, state: &mut State) -> bool {
        if self.shutdown.load(Ordering::Acquire) {
            return false;
        }
        if let Some(size) = self
            .resize
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .take()
        {
            self.pty.on_resize(size);
        }
        loop {
            match self.input.try_recv() {
                Ok(bytes) => state.write_list.push_back(Writing::new(
                    Cow::Owned(bytes),
                    Arc::clone(&self.pending_bytes),
                )),
                Err(TryRecvError::Empty) => return true,
                Err(TryRecvError::Disconnected) => return false,
            }
        }
    }

    fn pty_read(&mut self, state: &mut State, buffer: &mut [u8]) -> io::Result<()> {
        let mut unprocessed = 0;
        let mut processed = 0;
        let _terminal_lease = self.terminal.lease();
        let mut terminal = None;
        loop {
            match self.pty.reader().read(&mut buffer[unprocessed..]) {
                Ok(0) if unprocessed == 0 => break,
                Ok(read) => unprocessed += read,
                Err(error) => match error.kind() {
                    ErrorKind::Interrupted | ErrorKind::WouldBlock if unprocessed == 0 => break,
                    ErrorKind::Interrupted | ErrorKind::WouldBlock => {}
                    _ => return Err(error),
                },
            }
            let terminal = match &mut terminal {
                Some(terminal) => terminal,
                None => terminal.insert(match self.terminal.try_lock_unfair() {
                    None if unprocessed >= READ_BUFFER_SIZE => self.terminal.lock_unfair(),
                    None => continue,
                    Some(terminal) => terminal,
                }),
            };
            state
                .parser
                .advance(&mut **terminal, &buffer[..unprocessed]);
            processed += unprocessed;
            unprocessed = 0;
            if processed >= MAX_LOCKED_READ {
                break;
            }
        }
        if state.parser.sync_bytes_count() < processed && processed > 0 {
            self.event_proxy.send_event(Event::Wakeup);
        }
        Ok(())
    }

    fn pty_write(&mut self, state: &mut State) -> io::Result<()> {
        state.ensure_next();
        'write_many: while let Some(mut current) = state.writing.take() {
            loop {
                match self.pty.writer().write(current.remaining_bytes()) {
                    Ok(0) => {
                        state.writing = Some(current);
                        break 'write_many;
                    }
                    Ok(written) => {
                        current.advance(written);
                        if current.finished() {
                            state.ensure_next();
                            break;
                        }
                    }
                    Err(error) => {
                        state.writing = Some(current);
                        match error.kind() {
                            ErrorKind::Interrupted | ErrorKind::WouldBlock => break 'write_many,
                            _ => return Err(error),
                        }
                    }
                }
            }
        }
        Ok(())
    }

    pub(crate) fn spawn(mut self) -> io::Result<JoinHandle<()>> {
        std::thread::Builder::new()
            .name("nickel-terminal-pty".into())
            .spawn(move || {
                let mut state = State::default();
                let mut buffer = [0_u8; READ_BUFFER_SIZE];
                let mode = PollMode::Level;
                let mut interest = PollingEvent::readable(0);
                // SAFETY: the PTY remains owned by this loop until deregistration.
                if let Err(error) = unsafe { self.pty.register(&self.poller, interest, mode) } {
                    tracing::error!(%error, "terminal PTY registration failed");
                    return;
                }
                let mut events = Events::with_capacity(NonZeroUsize::new(1024).unwrap());
                'event_loop: loop {
                    let timeout = state
                        .parser
                        .sync_timeout()
                        .sync_timeout()
                        .map(|deadline| deadline.saturating_duration_since(Instant::now()));
                    events.clear();
                    if let Err(error) = self.poller.wait(&mut events, timeout) {
                        if error.kind() == ErrorKind::Interrupted {
                            continue;
                        }
                        tracing::error!(%error, "terminal PTY polling failed");
                        break;
                    }
                    if events.is_empty() {
                        state.parser.stop_sync(&mut *self.terminal.lock());
                        self.event_proxy.send_event(Event::Wakeup);
                    }
                    if !self.receive_control(&mut state) {
                        break;
                    }
                    for event in events.iter() {
                        match event.key {
                            PTY_CHILD_EVENT_TOKEN => {
                                if let Some(tty::ChildEvent::Exited(status)) =
                                    self.pty.next_child_event()
                                {
                                    if let Some(status) = status {
                                        self.event_proxy.send_event(Event::ChildExit(status));
                                    }
                                    if self.drain_on_exit {
                                        let _ = self.pty_read(&mut state, &mut buffer);
                                    }
                                    self.terminal.lock().exit();
                                    self.event_proxy.send_event(Event::Wakeup);
                                    break 'event_loop;
                                }
                            }
                            PTY_READ_WRITE_TOKEN if !event.is_interrupt() => {
                                if event.readable
                                    && let Err(error) = self.pty_read(&mut state, &mut buffer)
                                {
                                    #[cfg(target_os = "linux")]
                                    if error.raw_os_error() == Some(5) {
                                        continue;
                                    }
                                    tracing::error!(%error, "terminal PTY read failed");
                                    break 'event_loop;
                                }
                                if event.writable
                                    && let Err(error) = self.pty_write(&mut state)
                                {
                                    tracing::error!(%error, "terminal PTY write failed");
                                    break 'event_loop;
                                }
                            }
                            _ => {}
                        }
                    }
                    let needs_write = state.writing.is_some() || !state.write_list.is_empty();
                    if needs_write != interest.writable {
                        interest.writable = needs_write;
                        if let Err(error) = self.pty.reregister(&self.poller, interest, mode) {
                            tracing::error!(%error, "terminal PTY registration update failed");
                            break;
                        }
                    }
                }
                let _ = self.pty.deregister(&self.poller);
            })
    }
}

#[derive(Default)]
struct State {
    write_list: VecDeque<Writing>,
    writing: Option<Writing>,
    parser: ansi::Processor,
}

impl State {
    fn ensure_next(&mut self) {
        if self.writing.is_none() {
            self.writing = self.write_list.pop_front();
        }
    }
}

struct Writing {
    source: Cow<'static, [u8]>,
    written: usize,
    pending_bytes: Arc<AtomicUsize>,
}

impl Writing {
    fn new(source: Cow<'static, [u8]>, pending_bytes: Arc<AtomicUsize>) -> Self {
        Self {
            source,
            written: 0,
            pending_bytes,
        }
    }

    fn remaining_bytes(&self) -> &[u8] {
        &self.source[self.written..]
    }

    fn advance(&mut self, written: usize) {
        self.written += written;
        self.pending_bytes.fetch_sub(written, Ordering::AcqRel);
    }

    fn finished(&self) -> bool {
        self.written == self.source.len()
    }
}
