#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{collections::HashMap, error::Error, path::PathBuf, time::Duration};

use nickel_terminal::{
    TerminalDimensions, TerminalEvent, TerminalExit, TerminalOptions, TerminalProgram,
    TerminalSession, TerminalSnapshot,
};
use nickel_terminal_ui::{
    CellMetrics, PasteDecision, TerminalInputCommand, TerminalPalette, TerminalViewport,
    confirm_paste, prepare_paste, translate_input_with_application_cursor,
};
use nickel_ui::{
    AdapterOutcome, Application, Column, Container, HostAdapter, HostServices, SemanticRole, Text,
    UiHost, View, ViewContext,
};
use winit::event::WindowEvent;

const INITIAL_COLUMNS: u16 = 100;
const INITIAL_LINES: u16 = 30;
const CELL_WIDTH: u16 = 9;
const CELL_HEIGHT: u16 = 19;

struct TerminalApp {
    session: TerminalSession,
    snapshot: TerminalSnapshot,
    palette: TerminalPalette,
    metrics: CellMetrics,
    title: String,
    status: Option<String>,
    paste_confirmation: Option<String>,
    resize_generation: u64,
}

#[derive(Clone)]
enum Message {
    ConfirmPaste,
    CancelPaste,
}

impl TerminalApp {
    fn new(program: Option<TerminalProgram>, cwd: Option<PathBuf>) -> Result<Self, Box<dyn Error>> {
        let dimensions =
            TerminalDimensions::new(INITIAL_COLUMNS, INITIAL_LINES, CELL_WIDTH, CELL_HEIGHT)?;
        let session = TerminalSession::spawn(TerminalOptions {
            program,
            working_directory: cwd,
            environment: HashMap::new(),
            dimensions,
            scrollback_lines: 10_000,
        })?;
        let snapshot = session.snapshot();
        Ok(Self {
            session,
            snapshot,
            palette: TerminalPalette::default(),
            metrics: CellMetrics::integral(14.0, 1.0),
            title: "Nickel Terminal".into(),
            status: None,
            paste_confirmation: None,
            resize_generation: 0,
        })
    }

    fn apply_input(&mut self, command: TerminalInputCommand) -> bool {
        let result = match command {
            TerminalInputCommand::Write(bytes) => self.session.write(bytes),
            TerminalInputCommand::Scroll(scroll) => {
                self.session.scroll(scroll);
                return true;
            }
            TerminalInputCommand::Copy => {
                if let Some(text) = self.session.selected_text() {
                    match arboard::Clipboard::new()
                        .and_then(|mut clipboard| clipboard.set_text(text))
                    {
                        Ok(()) => return false,
                        Err(error) => {
                            self.status = Some(format!("Could not copy selection: {error}"));
                            return true;
                        }
                    }
                }
                return false;
            }
            TerminalInputCommand::PasteRequested => {
                let text = match arboard::Clipboard::new()
                    .and_then(|mut clipboard| clipboard.get_text())
                {
                    Ok(text) => text,
                    Err(error) => {
                        self.status = Some(format!("Could not read clipboard: {error}"));
                        return true;
                    }
                };
                match prepare_paste(text, self.snapshot.bracketed_paste) {
                    PasteDecision::Ready(bytes) => self.session.write(bytes),
                    PasteDecision::ConfirmationRequired(text) => {
                        self.paste_confirmation = Some(text);
                        return true;
                    }
                    PasteDecision::RejectedTooLarge => {
                        self.status = Some("Paste exceeds the 1 MiB safety limit".into());
                        return true;
                    }
                }
            }
            TerminalInputCommand::SelectAll => {
                self.session.select_visible();
                return true;
            }
            TerminalInputCommand::ClearScrollback => {
                self.session.clear_scrollback();
                return true;
            }
            TerminalInputCommand::Focus(focused) => {
                self.session.set_focus(focused);
                return true;
            }
        };
        if let Err(error) = result {
            self.status = Some(error.to_string());
        }
        true
    }

    fn resize(&mut self, width: u32, height: u32) -> bool {
        let usable_height = height.saturating_sub(28) as f32;
        let (columns, lines) = self.metrics.dimensions(width as f32, usable_height);
        if (columns as usize, lines as usize) == (self.snapshot.columns, self.snapshot.lines) {
            return false;
        }
        self.resize_generation = self.resize_generation.wrapping_add(1);
        let dimensions = TerminalDimensions::new(
            columns,
            lines,
            self.metrics.width.round().max(1.0) as u16,
            self.metrics.height.round().max(1.0) as u16,
        );
        match dimensions.and_then(|dimensions| {
            self.session
                .resize(dimensions, self.resize_generation)
                .map(|_| ())
        }) {
            Ok(()) => true,
            Err(error) => {
                self.status = Some(error.to_string());
                true
            }
        }
    }
}

impl Application for TerminalApp {
    type Message = Message;

    fn update(&mut self, message: Message) {
        match message {
            Message::ConfirmPaste => {
                if let Some(text) = self.paste_confirmation.take()
                    && let PasteDecision::Ready(bytes) =
                        confirm_paste(text, self.snapshot.bracketed_paste)
                    && let Err(error) = self.session.write(bytes)
                {
                    self.status = Some(error.to_string());
                }
            }
            Message::CancelPaste => self.paste_confirmation = None,
        }
    }

    fn view(&self, _: ViewContext) -> impl View<Self::Message> {
        let viewport =
            TerminalViewport::new(&self.snapshot, &self.palette, self.metrics, &self.title).view();
        let status = self
            .status
            .as_deref()
            .or_else(|| match self.session.exit_state() {
                TerminalExit::Running => None,
                state => Some(match state {
                    TerminalExit::Exited(Some(0)) => "Process exited",
                    TerminalExit::Exited(_) => "Process exited with an error",
                    TerminalExit::CloseRequested => "Closing process…",
                    TerminalExit::SpawnFailed => "Process could not start",
                    TerminalExit::IoFailed => "Terminal input/output failed",
                    TerminalExit::Hangup => "Terminal disconnected",
                    TerminalExit::Forced => "Process was forcibly terminated",
                    TerminalExit::Running => unreachable!(),
                }),
            });
        let mut root = Column::new().gap(0.0).child(viewport);
        if let Some(status) = status {
            root = root.child(
                Container::new()
                    .height(28.0)
                    .semantic_role(SemanticRole::Status)
                    .accessibility_label(status)
                    .background(0xff2b2f38)
                    .child(Text::new(status).color(0xffeceff4).scale(0.85)),
            );
        }
        if self.paste_confirmation.is_some() {
            root = root
                .child(nickel_ui::Button::new(
                    Message::ConfirmPaste,
                    "Paste multiple lines",
                ))
                .child(nickel_ui::Button::new(Message::CancelPaste, "Cancel"));
        }
        root
    }

    fn poll(&mut self) -> bool {
        let mut changed = false;
        while let Some(event) = self.session.try_event() {
            match event {
                TerminalEvent::Title(title) if !title.is_empty() => self.title = title,
                TerminalEvent::ChildExited(code) => {
                    self.status = Some(match code {
                        Some(0) => "Process exited".into(),
                        Some(code) => format!("Process exited with status {code}"),
                        None => "Process exited".into(),
                    });
                }
                TerminalEvent::ClipboardStore(text) => {
                    if let Err(error) =
                        arboard::Clipboard::new().and_then(|mut clipboard| clipboard.set_text(text))
                    {
                        self.status = Some(format!("Could not store terminal clipboard: {error}"));
                    }
                }
                TerminalEvent::Bell
                | TerminalEvent::Changed
                | TerminalEvent::Closed
                | TerminalEvent::Title(_) => {}
            }
            changed = true;
        }
        let snapshot = self.session.snapshot();
        if snapshot.generation != self.snapshot.generation {
            self.snapshot = snapshot;
            changed = true;
        }
        changed
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(Duration::from_millis(16))
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn initial_size(&self) -> (u32, u32) {
        (900, 600)
    }
}

#[derive(Default)]
struct TerminalAdapter;

impl HostAdapter<TerminalApp> for TerminalAdapter {
    fn normalized_input(
        &mut self,
        host: &mut UiHost<TerminalApp>,
        input: &nickel_input::InputEvent,
        _: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        let application_cursor = host.application().snapshot.application_cursor;
        let Some(command) = translate_input_with_application_cursor(input, application_cursor)
        else {
            return Ok(AdapterOutcome::default());
        };
        let changed = host.application_mut().apply_input(command);
        Ok(AdapterOutcome {
            changed,
            consume: true,
            exit: false,
        })
    }

    fn event(
        &mut self,
        host: &mut UiHost<TerminalApp>,
        event: &WindowEvent,
        _: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        let changed = match event {
            WindowEvent::Resized(size) => host.application_mut().resize(size.width, size.height),
            _ => false,
        };
        Ok(AdapterOutcome {
            changed,
            ..AdapterOutcome::default()
        })
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    let _log_path = nickel_logging::init("nickel-terminal").ok();
    let mut arguments = std::env::args_os().skip(1);
    let mut cwd = None;
    let mut command = Vec::new();
    while let Some(argument) = arguments.next() {
        if argument == "--cwd" {
            cwd = arguments.next().map(PathBuf::from);
        } else if argument == "--" {
            command.extend(arguments.map(|argument| argument.to_string_lossy().into_owned()));
            break;
        } else {
            return Err(format!("unknown argument: {}", argument.to_string_lossy()).into());
        }
    }
    let program = (!command.is_empty()).then(|| TerminalProgram {
        executable: command.remove(0),
        arguments: command,
    });
    nickel_ui::run_with_adapter(TerminalApp::new(program, cwd)?, TerminalAdapter)
}
