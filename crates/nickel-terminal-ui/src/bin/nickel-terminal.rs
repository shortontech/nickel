#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    collections::HashMap,
    error::Error,
    path::PathBuf,
    time::{Duration, Instant},
};

use nickel_core::terminal_settings::TerminalSettings;
use nickel_terminal::{
    TerminalDimensions, TerminalEvent, TerminalExit, TerminalOptions, TerminalProgram,
    TerminalSession, TerminalSnapshot,
};
use nickel_terminal_ui::{
    CellMetrics, PasteDecision, TerminalInputCommand, TerminalPalette, TerminalPointerTranslator,
    TerminalViewport, confirm_paste, prepare_paste, translate_input_with_application_cursor,
};
use nickel_ui::{
    AdapterOutcome, Application, Column, Container, FrameOverlay, HostAdapter, HostServices,
    OverlayAnchor, OverlayMenu, OverlayMenuItem, SemanticRole, Text, UiHost, UiId, View,
    ViewContext,
};
use winit::event::WindowEvent;

const INITIAL_COLUMNS: u16 = 100;
const INITIAL_LINES: u16 = 30;
const CELL_WIDTH: u16 = 9;
const CELL_HEIGHT: u16 = 19;

fn next_poll_delay(previous: Duration, changed: bool) -> Duration {
    if changed {
        Duration::from_millis(16)
    } else {
        previous.saturating_mul(2).min(Duration::from_millis(100))
    }
}

fn close_after_exit(enabled: bool, code: Option<i32>) -> bool {
    enabled && code == Some(0)
}

fn deferred_window_suppressed(delay: Duration) -> bool {
    use std::io::{BufRead, Read};

    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    let _ = std::thread::Builder::new()
        .name("nickel-terminal-window-decision".into())
        .spawn(move || {
            let mut decision = String::new();
            if std::io::stdin()
                .lock()
                .take(64)
                .read_line(&mut decision)
                .is_ok()
                && is_suppress_decision(&decision)
            {
                let _ = sender.try_send(());
            }
        });
    receiver.recv_timeout(delay).is_ok()
}

fn is_suppress_decision(decision: &str) -> bool {
    decision.trim() == "suppress"
}

fn await_deferred_start() -> Instant {
    use std::io::{BufRead, Read};

    let mut line = String::new();
    let _ = std::io::stdin().lock().take(64).read_line(&mut line);
    Instant::now()
}

fn supervise_hidden_session(session: &mut TerminalSession) {
    while matches!(session.exit_state(), TerminalExit::Running) {
        while session.try_event().is_some() {}
        std::thread::sleep(Duration::from_millis(10));
    }
}

struct TerminalApp {
    session: TerminalSession,
    snapshot: TerminalSnapshot,
    palette: TerminalPalette,
    metrics: CellMetrics,
    title: String,
    status: Option<String>,
    paste_confirmation: Option<String>,
    resize_generation: u64,
    poll_delay: Duration,
    close_on_successful_exit: bool,
    exit_requested: bool,
}

#[derive(Clone)]
enum Message {
    ConfirmPaste,
    CancelPaste,
    OpenContextMenu,
    Copy,
    Paste,
    SelectAll,
    ClearScrollback,
}

impl TerminalApp {
    fn new(
        program: Option<TerminalProgram>,
        cwd: Option<PathBuf>,
        settings: &TerminalSettings,
    ) -> Result<Self, Box<dyn Error>> {
        let dimensions =
            TerminalDimensions::new(INITIAL_COLUMNS, INITIAL_LINES, CELL_WIDTH, CELL_HEIGHT)?;
        let program = program.or_else(|| {
            settings
                .default_shell
                .as_ref()
                .map(|executable| TerminalProgram {
                    executable: executable.clone(),
                    arguments: Vec::new(),
                })
        });
        let session = TerminalSession::spawn(TerminalOptions {
            program,
            working_directory: cwd.or_else(|| settings.initial_working_directory.clone()),
            environment: HashMap::new(),
            dimensions,
            scrollback_lines: settings.scrollback_lines,
        })?;
        let snapshot = session.snapshot();
        let palette = TerminalPalette {
            foreground: settings.foreground,
            background: settings.background,
            cursor_style: settings.cursor_style,
            font_family: std::sync::Arc::from(settings.font_family.as_str()),
            ..TerminalPalette::default()
        };
        Ok(Self {
            session,
            snapshot,
            palette,
            metrics: CellMetrics::integral(settings.font_size(), 1.0),
            title: "Nickel Terminal".into(),
            status: None,
            paste_confirmation: None,
            resize_generation: 0,
            poll_delay: Duration::from_millis(16),
            close_on_successful_exit: settings.close_on_successful_exit,
            exit_requested: false,
        })
    }

    fn apply_input(&mut self, command: TerminalInputCommand) -> bool {
        self.poll_delay = Duration::from_millis(16);
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
            TerminalInputCommand::BeginSelection(kind, point) => {
                self.session.begin_selection(kind, point);
                return true;
            }
            TerminalInputCommand::UpdateSelection(point) => {
                self.session.update_selection(point);
                return true;
            }
            TerminalInputCommand::UpdateSelectionAndScroll(point, lines) => {
                self.session
                    .scroll(nickel_terminal::TerminalScroll::Lines(lines));
                self.session.update_selection(point);
                return true;
            }
            TerminalInputCommand::ClearSelection => {
                self.session.clear_selection();
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
            Message::OpenContextMenu => {}
            Message::Copy => {
                self.apply_input(TerminalInputCommand::Copy);
            }
            Message::Paste => {
                self.apply_input(TerminalInputCommand::PasteRequested);
            }
            Message::SelectAll => {
                self.apply_input(TerminalInputCommand::SelectAll);
            }
            Message::ClearScrollback => {
                self.apply_input(TerminalInputCommand::ClearScrollback);
            }
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
        let mut root = Column::new().gap(0.0).child(
            Container::new()
                .id("terminal-interaction")
                .context_message(Message::OpenContextMenu)
                .child(viewport),
        );
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

    fn frame_overlays(&self, _: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let copy = if self.session.selected_text().is_some() {
            OverlayMenuItem::action("copy", "Copy", Message::Copy).shortcut("Ctrl+Shift+C")
        } else {
            OverlayMenuItem::disabled_with_reason("copy", "Copy", "No text is selected")
        };
        let mut menu = OverlayMenu::new(
            "terminal-context-menu",
            OverlayAnchor::InvocationTarget(UiId::new("terminal-interaction")),
        )
        .item(copy)
        .item(OverlayMenuItem::action("paste", "Paste", Message::Paste).shortcut("Ctrl+Shift+V"))
        .item(
            OverlayMenuItem::action("select-all", "Select All", Message::SelectAll)
                .shortcut("Ctrl+Shift+A")
                .separator_before(true),
        )
        .item(OverlayMenuItem::action(
            "clear-scrollback",
            "Clear Scrollback",
            Message::ClearScrollback,
        ));
        menu.background = self.palette.background;
        menu.border = self.palette.foreground;
        menu.foreground = self.palette.foreground;
        menu.item_hover = Some(self.palette.selection);
        menu.item_selected = Some(self.palette.selection);
        vec![FrameOverlay::Menu(menu)]
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
                    self.exit_requested = close_after_exit(self.close_on_successful_exit, code);
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
        if self.session.generation() != self.snapshot.generation {
            self.snapshot = self.session.snapshot();
            changed = true;
        }
        self.poll_delay = next_poll_delay(self.poll_delay, changed);
        changed
    }

    fn poll_interval(&self) -> Option<Duration> {
        Some(self.poll_delay)
    }

    fn title(&self) -> &str {
        &self.title
    }

    fn initial_size(&self) -> (u32, u32) {
        (900, 600)
    }
}

struct TerminalAdapter {
    pointer: TerminalPointerTranslator,
    started: Instant,
    monitor_exit: bool,
}

impl Default for TerminalAdapter {
    fn default() -> Self {
        Self {
            pointer: TerminalPointerTranslator::default(),
            started: Instant::now(),
            monitor_exit: false,
        }
    }
}

impl HostAdapter<TerminalApp> for TerminalAdapter {
    fn poll_interval(&self) -> Option<Duration> {
        self.monitor_exit.then_some(Duration::from_millis(100))
    }

    fn poll(
        &mut self,
        host: &mut UiHost<TerminalApp>,
        _: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        Ok(if host.application().exit_requested {
            AdapterOutcome::exit()
        } else {
            AdapterOutcome::default()
        })
    }

    fn normalized_input(
        &mut self,
        host: &mut UiHost<TerminalApp>,
        input: &nickel_input::InputEvent,
        _: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn Error>> {
        let pointer_command = self.pointer.translate(
            input,
            &host.application().snapshot,
            host.application().metrics,
            self.started.elapsed().as_millis().min(u128::from(u64::MAX)) as u64,
        );
        let application_cursor = host.application().snapshot.application_cursor;
        let Some(command) = pointer_command
            .or_else(|| translate_input_with_application_cursor(input, application_cursor))
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
            WindowEvent::CloseRequested => {
                if let Err(error) = host.application_mut().session.request_close() {
                    host.application_mut().status = Some(error.to_string());
                }
                // Let the shared runtime finish closing the native window after the graceful PTY
                // shutdown request has crossed the session boundary.
                true
            }
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
    let mut deferred_window = Duration::ZERO;
    while let Some(argument) = arguments.next() {
        if argument == "--working-directory" {
            cwd = Some(PathBuf::from(
                arguments
                    .next()
                    .ok_or("--working-directory requires a path")?,
            ));
        } else if argument == "--defer-window-for-child-ms" {
            let value = arguments
                .next()
                .ok_or("--defer-window-for-child-ms requires a value")?
                .to_string_lossy()
                .parse::<u64>()?;
            deferred_window = Duration::from_millis(value);
            if deferred_window > nickel_terminal::deferred::MAX_CLASSIFICATION_DELAY {
                return Err("deferred window delay exceeds 2000 ms".into());
            }
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
    // The trusted launcher opens the classification interval before this process spawns the PTY.
    // PTY allocation and parsing then start before any visibility delay; absent an attributed
    // suppression signal, the conservative decision is to construct the terminal UI at expiry.
    let deferred_started = (!deferred_window.is_zero()).then(await_deferred_start);
    let settings = TerminalSettings::load_default();
    let mut app = TerminalApp::new(program, cwd, &settings)?;
    let remaining = deferred_started
        .map(|started| deferred_window.saturating_sub(started.elapsed()))
        .unwrap_or_default();
    if !remaining.is_zero() && deferred_window_suppressed(remaining) {
        supervise_hidden_session(&mut app.session);
        return Ok(());
    }
    nickel_ui::run_with_adapter(
        app,
        TerminalAdapter {
            monitor_exit: settings.close_on_successful_exit,
            ..TerminalAdapter::default()
        },
    )
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn context_menu_exposes_only_implemented_terminal_actions() {
        let app = TerminalApp::new(
            Some(TerminalProgram {
                executable: "/bin/sh".into(),
                arguments: vec!["-c".into(), "exit 0".into()],
            }),
            None,
            &TerminalSettings::default(),
        )
        .expect("fixture PTY");
        let mut host = UiHost::new(app, 900, 600);
        let target = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id.as_str().ends_with("/terminal-interaction"))
            .expect("terminal context target");
        let opened = host.perform_semantic_action(
            target.id,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::ContextMenu),
        );
        assert!(opened.changed);
        let names = host
            .semantic_nodes()
            .into_iter()
            .filter_map(|node| node.name)
            .collect::<Vec<_>>();

        for expected in ["Copy", "Paste", "Select All", "Clear Scrollback"] {
            assert!(
                names.iter().any(|name| name == expected),
                "missing {expected}: {names:?}"
            );
        }
        assert!(!names.iter().any(|name| name == "Cut" || name == "Delete"));
    }

    #[test]
    fn idle_polling_backs_off_but_output_returns_to_low_latency() {
        let mut delay = Duration::from_millis(16);
        for _ in 0..20 {
            delay = next_poll_delay(delay, false);
        }
        assert_eq!(delay, Duration::from_millis(100));
        assert_eq!(next_poll_delay(delay, true), Duration::from_millis(16));
    }

    #[test]
    fn exit_policy_closes_only_after_an_explicit_success() {
        assert!(close_after_exit(true, Some(0)));
        assert!(!close_after_exit(false, Some(0)));
        assert!(!close_after_exit(true, Some(1)));
        assert!(!close_after_exit(true, None));
    }

    #[test]
    fn deferred_control_pipe_accepts_only_the_exact_suppression_word() {
        assert!(is_suppress_decision("suppress\n"));
        assert!(!is_suppress_decision("start\n"));
        assert!(!is_suppress_decision("suppress now\n"));
    }
}
