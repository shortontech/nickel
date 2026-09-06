//! Bounded, renderer-independent terminal emulation and PTY ownership for Nickel.

pub mod deferred;

use std::{
    borrow::Cow,
    collections::HashMap,
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{Receiver, SyncSender, TryRecvError, TrySendError, sync_channel},
    },
    thread::JoinHandle,
};

use alacritty_terminal::{
    event::{Event, EventListener, WindowSize},
    event_loop::{EventLoop, EventLoopSender, Msg},
    grid::Scroll,
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{Config, Term, TermMode, cell::Flags, test::TermSize},
    tty::{self, Shell},
    vte::ansi,
};

const EVENT_CAPACITY: usize = 128;
const MAX_WRITE_BYTES: usize = 64 * 1024;
const MAX_TITLE_BYTES: usize = 4 * 1024;
const MAX_SCROLLBACK: usize = 100_000;
const MAX_COLUMNS: u16 = 500;
const MAX_LINES: u16 = 200;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalDimensions {
    pub columns: u16,
    pub lines: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl TerminalDimensions {
    pub fn new(
        columns: u16,
        lines: u16,
        cell_width: u16,
        cell_height: u16,
    ) -> Result<Self, TerminalError> {
        if columns == 0 || lines == 0 || cell_width == 0 || cell_height == 0 {
            return Err(TerminalError::InvalidDimensions);
        }
        if columns > MAX_COLUMNS || lines > MAX_LINES {
            return Err(TerminalError::DimensionsTooLarge);
        }
        Ok(Self {
            columns,
            lines,
            cell_width,
            cell_height,
        })
    }

    fn term_size(self) -> TermSize {
        TermSize::new(self.columns as usize, self.lines as usize)
    }

    fn window_size(self) -> WindowSize {
        WindowSize {
            num_lines: self.lines,
            num_cols: self.columns,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalProgram {
    pub executable: String,
    pub arguments: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct TerminalOptions {
    pub program: Option<TerminalProgram>,
    pub working_directory: Option<PathBuf>,
    pub environment: HashMap<String, String>,
    pub dimensions: TerminalDimensions,
    pub scrollback_lines: usize,
}

impl TerminalOptions {
    pub fn validate(&self) -> Result<(), TerminalError> {
        if self.scrollback_lines > MAX_SCROLLBACK {
            return Err(TerminalError::ScrollbackTooLarge);
        }
        if let Some(path) = &self.working_directory
            && !path.is_dir()
        {
            return Err(TerminalError::InvalidWorkingDirectory(path.clone()));
        }
        if self
            .program
            .as_ref()
            .is_some_and(|program| program.executable.is_empty())
        {
            return Err(TerminalError::EmptyExecutable);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalColor {
    Named(String),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalCell {
    pub character: char,
    pub combining: Vec<char>,
    pub foreground: TerminalColor,
    pub background: TerminalColor,
    pub bold: bool,
    pub dim: bool,
    pub italic: bool,
    pub underline: bool,
    pub inverse: bool,
    pub concealed: bool,
    pub wide: bool,
    pub wide_spacer: bool,
    pub selected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalCursor {
    pub line: usize,
    pub column: usize,
    pub visible: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalSelectionKind {
    Simple,
    Block,
    Semantic,
    Lines,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TerminalPoint {
    pub line: i32,
    pub column: usize,
}

impl TerminalPoint {
    fn upstream(self) -> Point {
        Point::new(Line(self.line), Column(self.column))
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalScroll {
    Lines(i32),
    PageUp,
    PageDown,
    Top,
    Bottom,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalSnapshot {
    pub generation: u64,
    pub columns: usize,
    pub lines: usize,
    pub cells: Vec<TerminalCell>,
    pub cursor: TerminalCursor,
    pub alternate_screen: bool,
    pub bracketed_paste: bool,
    pub mouse_reporting: bool,
    pub sgr_mouse: bool,
    pub application_cursor: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalEvent {
    Changed,
    Title(String),
    Bell,
    ChildExited(Option<i32>),
    ClipboardStore(String),
    Closed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TerminalExit {
    Running,
    Exited(Option<i32>),
    CloseRequested,
    SpawnFailed,
    IoFailed,
    Hangup,
    Forced,
}

#[derive(Debug, thiserror::Error)]
pub enum TerminalError {
    #[error("terminal dimensions must be nonzero")]
    InvalidDimensions,
    #[error("terminal dimensions exceed the bounded grid maximum")]
    DimensionsTooLarge,
    #[error("terminal scrollback exceeds the bounded maximum")]
    ScrollbackTooLarge,
    #[error("terminal executable is empty")]
    EmptyExecutable,
    #[error("terminal working directory is not a directory: {0}")]
    InvalidWorkingDirectory(PathBuf),
    #[error("terminal input exceeds the bounded write size")]
    WriteTooLarge,
    #[error("terminal session is closed")]
    Closed,
    #[error("could not create terminal PTY: {0}")]
    Spawn(std::io::Error),
    #[error("could not send terminal command: {0}")]
    Send(String),
}

#[derive(Clone)]
struct Proxy {
    events: SyncSender<TerminalEvent>,
    generation: Arc<AtomicU64>,
    wake_pending: Arc<AtomicBool>,
    pty_sender: Arc<Mutex<Option<EventLoopSender>>>,
}

impl EventListener for Proxy {
    fn send_event(&self, event: Event) {
        self.generation.fetch_add(1, Ordering::Release);
        let projected = match event {
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {
                if self.wake_pending.swap(true, Ordering::AcqRel) {
                    return;
                }
                TerminalEvent::Changed
            }
            Event::Title(title) => TerminalEvent::Title(truncate_utf8(title, MAX_TITLE_BYTES)),
            Event::ResetTitle => TerminalEvent::Title(String::new()),
            Event::Bell => TerminalEvent::Bell,
            Event::ChildExit(status) => TerminalEvent::ChildExited(status.code()),
            Event::ClipboardStore(_, text) => {
                TerminalEvent::ClipboardStore(truncate_utf8(text, MAX_WRITE_BYTES))
            }
            Event::Exit => TerminalEvent::Closed,
            Event::PtyWrite(text) => {
                if let Some(sender) = self.pty_sender.lock().unwrap().as_ref() {
                    let bytes = truncate_utf8(text, MAX_WRITE_BYTES).into_bytes();
                    let _ = sender.send(Msg::Input(Cow::Owned(bytes)));
                }
                return;
            }
            Event::ClipboardLoad(_, _)
            | Event::ColorRequest(_, _)
            | Event::TextAreaSizeRequest(_) => return,
        };
        if matches!(
            projected,
            TerminalEvent::ChildExited(_) | TerminalEvent::Closed
        ) {
            // These lifecycle events are emitted at most once each and must not be lost behind
            // coalescible redraw/title noise. Blocking after child exit cannot stall PTY output.
            let _ = self.events.send(projected);
            return;
        }
        match self.events.try_send(projected) {
            Ok(()) | Err(TrySendError::Full(_)) | Err(TrySendError::Disconnected(_)) => {}
        }
    }
}

/// A renderer-independent parser useful for deterministic fixtures and workbench models.
pub struct TerminalEngine {
    terminal: Term<Proxy>,
    parser: ansi::Processor,
    generation: Arc<AtomicU64>,
    wake_pending: Arc<AtomicBool>,
    events: Receiver<TerminalEvent>,
    dimensions: TerminalDimensions,
    resize_generation: u64,
}

impl TerminalEngine {
    pub fn new(
        dimensions: TerminalDimensions,
        scrollback_lines: usize,
    ) -> Result<Self, TerminalError> {
        if scrollback_lines > MAX_SCROLLBACK {
            return Err(TerminalError::ScrollbackTooLarge);
        }
        let ProxyParts {
            proxy,
            events,
            generation,
            wake_pending,
            ..
        } = proxy();
        let config = Config {
            scrolling_history: scrollback_lines,
            ..Config::default()
        };
        Ok(Self {
            terminal: Term::new(config, &dimensions.term_size(), proxy),
            parser: ansi::Processor::new(),
            generation,
            wake_pending,
            events,
            dimensions,
            resize_generation: 0,
        })
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.terminal, bytes);
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn resize(&mut self, dimensions: TerminalDimensions, generation: u64) -> bool {
        if generation <= self.resize_generation {
            return false;
        }
        self.dimensions = dimensions;
        self.terminal.resize(dimensions.term_size());
        self.resize_generation = generation;
        self.generation.fetch_add(1, Ordering::Release);
        true
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        snapshot(
            &self.terminal,
            self.generation.load(Ordering::Acquire),
            self.dimensions,
        )
    }

    pub fn scroll(&mut self, scroll: TerminalScroll) {
        self.terminal.scroll_display(upstream_scroll(scroll));
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn begin_selection(&mut self, kind: TerminalSelectionKind, point: TerminalPoint) {
        self.terminal.selection = Some(Selection::new(
            selection_kind(kind),
            point.upstream(),
            Side::Left,
        ));
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn update_selection(&mut self, point: TerminalPoint) {
        if let Some(selection) = &mut self.terminal.selection {
            selection.update(point.upstream(), Side::Right);
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    pub fn clear_selection(&mut self) {
        if self.terminal.selection.take().is_some() {
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.terminal.selection_to_string()
    }

    pub fn try_event(&self) -> Option<TerminalEvent> {
        match self.events.try_recv() {
            Ok(event) => {
                if event == TerminalEvent::Changed {
                    self.wake_pending.store(false, Ordering::Release);
                }
                Some(event)
            }
            Err(TryRecvError::Empty | TryRecvError::Disconnected) => None,
        }
    }
}

/// Owns one child process, PTY event loop, and terminal model.
pub struct TerminalSession {
    terminal: Arc<FairMutex<Term<Proxy>>>,
    sender: EventLoopSender,
    events: Receiver<TerminalEvent>,
    generation: Arc<AtomicU64>,
    wake_pending: Arc<AtomicBool>,
    dimensions: TerminalDimensions,
    resize_generation: u64,
    exit: TerminalExit,
    worker: Option<JoinHandle<()>>,
}

impl TerminalSession {
    pub fn spawn(options: TerminalOptions) -> Result<Self, TerminalError> {
        options.validate()?;
        let ProxyParts {
            proxy,
            events,
            generation,
            wake_pending,
            pty_sender,
        } = proxy();
        let config = Config {
            scrolling_history: options.scrollback_lines,
            ..Config::default()
        };
        let terminal = Arc::new(FairMutex::new(Term::new(
            config,
            &options.dimensions.term_size(),
            proxy.clone(),
        )));
        let tty_options = tty::Options {
            shell: options
                .program
                .map(|program| Shell::new(program.executable, program.arguments)),
            working_directory: options.working_directory,
            drain_on_exit: true,
            env: options.environment,
            #[cfg(target_os = "windows")]
            escape_args: true,
        };
        let pty = tty::new(&tty_options, options.dimensions.window_size(), 0)
            .map_err(TerminalError::Spawn)?;
        let event_loop = EventLoop::new(Arc::clone(&terminal), proxy, pty, true, false)
            .map_err(TerminalError::Spawn)?;
        let sender = event_loop.channel();
        *pty_sender.lock().unwrap() = Some(sender.clone());
        let worker = std::thread::Builder::new()
            .name("nickel-terminal-pty".into())
            .spawn(move || {
                let _ = event_loop.spawn().join();
            })
            .map_err(TerminalError::Spawn)?;
        Ok(Self {
            terminal,
            sender,
            events,
            generation,
            wake_pending,
            dimensions: options.dimensions,
            resize_generation: 0,
            exit: TerminalExit::Running,
            worker: Some(worker),
        })
    }

    pub fn write(&self, bytes: Vec<u8>) -> Result<(), TerminalError> {
        if bytes.len() > MAX_WRITE_BYTES {
            return Err(TerminalError::WriteTooLarge);
        }
        self.sender
            .send(Msg::Input(Cow::Owned(bytes)))
            .map_err(|error| TerminalError::Send(error.to_string()))
    }

    pub fn resize(
        &mut self,
        dimensions: TerminalDimensions,
        generation: u64,
    ) -> Result<bool, TerminalError> {
        if generation <= self.resize_generation {
            return Ok(false);
        }
        self.sender
            .send(Msg::Resize(dimensions.window_size()))
            .map_err(|error| TerminalError::Send(error.to_string()))?;
        self.terminal.lock().resize(dimensions.term_size());
        self.dimensions = dimensions;
        self.resize_generation = generation;
        self.generation.fetch_add(1, Ordering::Release);
        Ok(true)
    }

    pub fn snapshot(&self) -> TerminalSnapshot {
        snapshot(
            &self.terminal.lock(),
            self.generation.load(Ordering::Acquire),
            self.dimensions,
        )
    }

    pub fn scroll(&mut self, scroll: TerminalScroll) {
        self.terminal.lock().scroll_display(upstream_scroll(scroll));
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn begin_selection(&mut self, kind: TerminalSelectionKind, point: TerminalPoint) {
        self.terminal.lock().selection = Some(Selection::new(
            selection_kind(kind),
            point.upstream(),
            Side::Left,
        ));
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn update_selection(&mut self, point: TerminalPoint) {
        let mut terminal = self.terminal.lock();
        if let Some(selection) = &mut terminal.selection {
            selection.update(point.upstream(), Side::Right);
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    pub fn clear_selection(&mut self) {
        if self.terminal.lock().selection.take().is_some() {
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.terminal.lock().selection_to_string()
    }

    pub fn set_focus(&mut self, focused: bool) {
        let mut terminal = self.terminal.lock();
        if terminal.is_focused != focused {
            terminal.is_focused = focused;
            self.generation.fetch_add(1, Ordering::Release);
        }
    }

    pub fn clear_scrollback(&mut self) {
        self.terminal.lock().grid_mut().clear_history();
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn select_visible(&mut self) {
        let mut terminal = self.terminal.lock();
        let start = Point::new(Line(0), Column(0));
        let end = Point::new(
            Line(i32::from(self.dimensions.lines) - 1),
            Column(usize::from(self.dimensions.columns) - 1),
        );
        let mut selection = Selection::new(SelectionType::Simple, start, Side::Left);
        selection.update(end, Side::Right);
        terminal.selection = Some(selection);
        self.generation.fetch_add(1, Ordering::Release);
    }

    pub fn try_event(&mut self) -> Option<TerminalEvent> {
        let event = self.events.try_recv().ok()?;
        if event == TerminalEvent::Changed {
            self.wake_pending.store(false, Ordering::Release);
        }
        match event {
            TerminalEvent::ChildExited(code) => {
                self.exit = TerminalExit::Exited(code);
                Some(TerminalEvent::ChildExited(code))
            }
            TerminalEvent::Closed => {
                if self.exit == TerminalExit::Running {
                    self.exit = TerminalExit::Hangup;
                }
                Some(TerminalEvent::Closed)
            }
            event => Some(event),
        }
    }

    pub fn exit_state(&self) -> &TerminalExit {
        &self.exit
    }

    pub fn request_close(&mut self) -> Result<(), TerminalError> {
        if self.exit != TerminalExit::Running {
            return Ok(());
        }
        self.exit = TerminalExit::CloseRequested;
        self.sender
            .send(Msg::Shutdown)
            .map_err(|error| TerminalError::Send(error.to_string()))
    }
}

impl Drop for TerminalSession {
    fn drop(&mut self) {
        let _ = self.sender.send(Msg::Shutdown);
        // Joining a stuck platform PTY would block the UI/drop path. Dropping detaches the bounded
        // worker; explicit shutdown observation can join it in a future process supervisor.
        let _ = self.worker.take();
    }
}

struct ProxyParts {
    proxy: Proxy,
    events: Receiver<TerminalEvent>,
    generation: Arc<AtomicU64>,
    wake_pending: Arc<AtomicBool>,
    pty_sender: Arc<Mutex<Option<EventLoopSender>>>,
}

fn proxy() -> ProxyParts {
    let (events, receiver) = sync_channel(EVENT_CAPACITY);
    let generation = Arc::new(AtomicU64::new(1));
    let wake_pending = Arc::new(AtomicBool::new(false));
    let pty_sender = Arc::new(Mutex::new(None));
    ProxyParts {
        proxy: Proxy {
            events,
            generation: Arc::clone(&generation),
            wake_pending: Arc::clone(&wake_pending),
            pty_sender: Arc::clone(&pty_sender),
        },
        events: receiver,
        generation,
        wake_pending,
        pty_sender,
    }
}

fn snapshot(
    terminal: &Term<Proxy>,
    generation: u64,
    dimensions: TerminalDimensions,
) -> TerminalSnapshot {
    let content = terminal.renderable_content();
    let cursor = TerminalCursor {
        line: content.cursor.point.line.0.max(0) as usize,
        column: content.cursor.point.column.0,
        visible: !matches!(content.cursor.shape, ansi::CursorShape::Hidden),
    };
    let cells = content
        .display_iter
        .map(|indexed| {
            let selected = content
                .selection
                .is_some_and(|selection| selection.contains(indexed.point));
            let cell = indexed.cell;
            TerminalCell {
                character: cell.c,
                combining: cell.zerowidth().unwrap_or_default().to_vec(),
                foreground: color(cell.fg),
                background: color(cell.bg),
                bold: cell
                    .flags
                    .intersects(Flags::BOLD | Flags::BOLD_ITALIC | Flags::DIM_BOLD),
                dim: cell.flags.intersects(Flags::DIM | Flags::DIM_BOLD),
                italic: cell.flags.intersects(Flags::ITALIC | Flags::BOLD_ITALIC),
                underline: cell.flags.intersects(Flags::ALL_UNDERLINES),
                inverse: cell.flags.contains(Flags::INVERSE),
                concealed: cell.flags.contains(Flags::HIDDEN),
                wide: cell.flags.contains(Flags::WIDE_CHAR),
                wide_spacer: cell.flags.contains(Flags::WIDE_CHAR_SPACER),
                selected,
            }
        })
        .collect();
    TerminalSnapshot {
        generation,
        columns: dimensions.columns as usize,
        lines: dimensions.lines as usize,
        cells,
        cursor,
        alternate_screen: content.mode.contains(TermMode::ALT_SCREEN),
        bracketed_paste: content.mode.contains(TermMode::BRACKETED_PASTE),
        mouse_reporting: content.mode.intersects(TermMode::MOUSE_MODE),
        sgr_mouse: content.mode.contains(TermMode::SGR_MOUSE),
        application_cursor: content.mode.contains(TermMode::APP_CURSOR),
    }
}

fn color(color: ansi::Color) -> TerminalColor {
    match color {
        ansi::Color::Named(value) => TerminalColor::Named(format!("{value:?}")),
        ansi::Color::Indexed(value) => TerminalColor::Indexed(value),
        ansi::Color::Spec(value) => TerminalColor::Rgb(value.r, value.g, value.b),
    }
}

fn selection_kind(kind: TerminalSelectionKind) -> SelectionType {
    match kind {
        TerminalSelectionKind::Simple => SelectionType::Simple,
        TerminalSelectionKind::Block => SelectionType::Block,
        TerminalSelectionKind::Semantic => SelectionType::Semantic,
        TerminalSelectionKind::Lines => SelectionType::Lines,
    }
}

fn upstream_scroll(scroll: TerminalScroll) -> Scroll {
    match scroll {
        TerminalScroll::Lines(lines) => Scroll::Delta(lines),
        TerminalScroll::PageUp => Scroll::PageUp,
        TerminalScroll::PageDown => Scroll::PageDown,
        TerminalScroll::Top => Scroll::Top,
        TerminalScroll::Bottom => Scroll::Bottom,
    }
}

fn truncate_utf8(mut value: String, maximum: usize) -> String {
    if value.len() <= maximum {
        return value;
    }
    let mut end = maximum;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn dimensions(columns: u16, lines: u16) -> TerminalDimensions {
        TerminalDimensions::new(columns, lines, 8, 16).unwrap()
    }

    #[test]
    fn fixture_projects_text_unicode_modes_and_attributes() {
        let mut engine = TerminalEngine::new(dimensions(12, 3), 50).unwrap();
        engine
            .process(b"hello \x1b[1;3;4;31mred\x1b[0m\r\nwide: \xe7\x95\x8c\x1b[?2004h\x1b[?1000h");
        let snapshot = engine.snapshot();
        assert_eq!(snapshot.columns, 12);
        assert!(
            snapshot
                .cells
                .iter()
                .any(|cell| cell.character == '\u{754c}' && cell.wide)
        );
        let red = snapshot
            .cells
            .iter()
            .find(|cell| cell.character == 'r')
            .unwrap();
        assert!(red.bold && red.italic && red.underline);
        assert!(snapshot.bracketed_paste);
        assert!(snapshot.mouse_reporting);
    }

    #[test]
    fn alternate_screen_and_reset_are_engine_owned() {
        let mut engine = TerminalEngine::new(dimensions(8, 2), 10).unwrap();
        engine.process(b"primary\x1b[?1049halternate");
        assert!(engine.snapshot().alternate_screen);
        engine.process(b"\x1b[?1049l");
        let snapshot = engine.snapshot();
        assert!(!snapshot.alternate_screen);
        assert!(snapshot.cells.iter().any(|cell| cell.character == 'p'));
    }

    #[test]
    fn resize_rejects_stale_generations() {
        let mut engine = TerminalEngine::new(dimensions(8, 2), 10).unwrap();
        engine.process(b"output can advance the independent change token");
        assert!(engine.resize(dimensions(20, 4), 10));
        assert!(!engine.resize(dimensions(5, 1), 9));
        assert_eq!(
            (engine.snapshot().columns, engine.snapshot().lines),
            (20, 4)
        );
    }

    #[test]
    fn selection_and_scrollback_are_projected_without_upstream_types() {
        let mut engine = TerminalEngine::new(dimensions(8, 2), 10).unwrap();
        engine.process(b"one\r\ntwo\r\nthree");
        engine.scroll(TerminalScroll::Top);
        engine.begin_selection(
            TerminalSelectionKind::Lines,
            TerminalPoint {
                line: -1,
                column: 0,
            },
        );
        engine.update_selection(TerminalPoint {
            line: -1,
            column: 2,
        });
        assert!(
            engine
                .selected_text()
                .is_some_and(|text| text.contains("one"))
        );
        assert!(engine.snapshot().cells.iter().any(|cell| cell.selected));
        engine.clear_selection();
        assert!(engine.selected_text().is_none());
    }

    #[test]
    fn bounds_configuration_and_preserves_argument_vectors() {
        let temporary = tempfile::tempdir().unwrap();
        let options = TerminalOptions {
            program: Some(TerminalProgram {
                executable: "/bin/printf".into(),
                arguments: vec!["%s".into(), "space value".into()],
            }),
            working_directory: Some(temporary.path().to_owned()),
            environment: HashMap::from([("NICKEL_TEST".into(), "1".into())]),
            dimensions: dimensions(80, 24),
            scrollback_lines: 1_000,
        };
        options.validate().unwrap();
        assert_eq!(options.program.unwrap().arguments[1], "space value");
        assert!(TerminalEngine::new(dimensions(80, 24), MAX_SCROLLBACK + 1).is_err());
        assert!(TerminalDimensions::new(MAX_COLUMNS + 1, 24, 8, 16).is_err());
    }

    #[test]
    fn lifecycle_event_survives_a_full_noise_queue() {
        let ProxyParts { proxy, events, .. } = proxy();
        for index in 0..EVENT_CAPACITY {
            proxy.send_event(Event::Title(format!("title-{index}")));
        }
        let producer = std::thread::spawn(move || proxy.send_event(Event::Exit));
        let mut closed = false;
        for _ in 0..=EVENT_CAPACITY {
            if events.recv().unwrap() == TerminalEvent::Closed {
                closed = true;
                break;
            }
        }
        producer.join().unwrap();
        assert!(closed, "the terminal close event must never be dropped");
    }

    #[cfg(unix)]
    #[test]
    fn live_pty_preserves_output_and_reports_child_exit() {
        let mut session = TerminalSession::spawn(TerminalOptions {
            program: Some(TerminalProgram {
                executable: "/bin/sh".into(),
                arguments: vec!["-c".into(), "printf 'nickel-pty-✓'".into()],
            }),
            working_directory: None,
            environment: HashMap::new(),
            dimensions: dimensions(40, 4),
            scrollback_lines: 100,
        })
        .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut exited = false;
        while Instant::now() < deadline {
            while let Some(event) = session.try_event() {
                exited |= matches!(event, TerminalEvent::ChildExited(_));
            }
            let text = session
                .snapshot()
                .cells
                .into_iter()
                .map(|cell| cell.character)
                .collect::<String>();
            if exited && text.contains("nickel-pty-✓") {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("PTY did not preserve output and exit within the bounded deadline");
    }
}
