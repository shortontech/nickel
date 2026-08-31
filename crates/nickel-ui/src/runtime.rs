use std::{
    error::Error,
    time::{Duration, Instant},
};

use sdl3::{
    event::{Event, WindowEvent},
    mouse::{Cursor as MouseCursor, SystemCursor},
};

use crate::{
    ControllerAction, ControllerInput, FocusedInputDispatcher, InputCommand, InputContext,
    PointerIcon, Rect, SdlCanvasPresenter, UiEvent, UiStateStore, UiTree, View,
};

#[derive(Debug, Default)]
struct FrameScheduler {
    dirty: bool,
}

impl FrameScheduler {
    fn invalidate(&mut self) {
        self.dirty = true;
    }

    fn take_rebuild(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shortcut {
    Submit,
    Newline,
    Escape,
    Reload,
    Back,
    Forward,
    DocumentStart,
    DocumentEnd,
}

pub trait Application: Sized {
    type Message: Clone;

    fn update(&mut self, message: Self::Message);

    fn view(&self) -> impl View<Self::Message>;

    /// Poll application-owned background work without introducing another UI runtime.
    /// Return `true` when new state requires a redraw.
    fn poll(&mut self) -> bool {
        false
    }

    /// Handle application-level keyboard semantics before ordinary component activation.
    fn shortcut(&mut self, _shortcut: Shortcut) -> bool {
        false
    }

    fn title(&self) -> &str {
        "Nickel UI"
    }

    fn initial_size(&self) -> (u32, u32) {
        (800, 600)
    }

    /// Handles controller actions with application-level meaning, such as a shell launcher.
    /// Returning `false` lets the runtime apply ordinary component navigation.
    fn controller_action(&mut self, _action: ControllerAction) -> bool {
        false
    }
}

fn controller_ui_event(action: ControllerAction) -> Option<UiEvent> {
    match action {
        ControllerAction::Up => Some(UiEvent::ControllerPrevious),
        ControllerAction::Down => Some(UiEvent::ControllerNext),
        ControllerAction::Left => Some(UiEvent::ControllerAdjust(-1.0)),
        ControllerAction::Right => Some(UiEvent::ControllerAdjust(1.0)),
        ControllerAction::Confirm => Some(UiEvent::ControllerActivate),
        ControllerAction::Cancel => Some(UiEvent::ControllerBack),
        ControllerAction::PreviousPane => Some(UiEvent::ControllerPreviousPane),
        ControllerAction::NextPane => Some(UiEvent::ControllerNextPane),
        ControllerAction::Launcher => None,
    }
}

pub struct ApplicationHost<A: Application> {
    application: A,
    state: UiStateStore,
    tree: UiTree<A::Message>,
    bounds: Rect,
    input_dispatcher: FocusedInputDispatcher,
}

#[derive(Default)]
pub struct HostEventOutcome {
    pub changed: bool,
    pub clipboard_text: Option<String>,
}

impl<A: Application> ApplicationHost<A> {
    pub fn new(application: A, width: u32, height: u32) -> Self {
        let bounds = Rect::new(0.0, 0.0, width as f32, height as f32);
        let mut state = UiStateStore::default();
        let tree = UiTree::layout_with_state(application.view(), bounds, &mut state);
        Self {
            application,
            state,
            tree,
            bounds,
            input_dispatcher: FocusedInputDispatcher::default(),
        }
    }

    pub fn application_mut(&mut self) -> &mut A {
        &mut self.application
    }

    pub fn commands(&self) -> &[crate::PaintCommand] {
        self.tree.commands()
    }

    pub fn input_context(&self) -> crate::InputContext {
        crate::InputContext {
            text_focused: self.state.focused().is_some(),
            selection_owned: self.state.selection_owner().is_some(),
        }
    }

    pub fn selected_text(&self) -> Option<String> {
        self.tree.selected_text(&self.state)
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        self.bounds = Rect::new(0.0, 0.0, width as f32, height as f32);
        self.rebuild();
    }

    pub fn poll(&mut self) -> bool {
        if !self.application.poll() {
            return false;
        }
        self.rebuild();
        true
    }

    pub fn handle_event(&mut self, event: UiEvent) -> HostEventOutcome {
        let outcome = self.tree.handle_event(&mut self.state, event);
        let changed =
            outcome.invalidation != crate::Invalidation::None || !outcome.messages.is_empty();
        for message in outcome.messages {
            self.application.update(message);
        }
        let clipboard_text = outcome.clipboard_text;
        if changed {
            self.rebuild();
        }
        HostEventOutcome {
            changed,
            clipboard_text,
        }
    }

    pub fn handle_controller_action(&mut self, action: ControllerAction) -> HostEventOutcome {
        if !self.state.window_focused() {
            return HostEventOutcome::default();
        }
        if self.application.controller_action(action) {
            self.rebuild();
            return HostEventOutcome {
                changed: true,
                ..HostEventOutcome::default()
            };
        }
        controller_ui_event(action)
            .map(|event| self.handle_event(event))
            .unwrap_or_default()
    }

    pub fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        if !self.application.shortcut(shortcut) {
            return false;
        }
        self.rebuild();
        true
    }

    /// Dispatch a normalized event through the same focused-input contract used by standalone
    /// Nickel UI applications. Embedded hosts provide clipboard text only when paste is allowed;
    /// copy and cut return replacement clipboard text in the outcome.
    pub fn handle_input(
        &mut self,
        input: &nickel_input::InputEvent,
        clipboard_text: Option<&str>,
    ) -> HostEventOutcome {
        let context = self.input_context();
        let commands = self.input_dispatcher.dispatch_with_context(input, context);
        let mut combined = HostEventOutcome::default();
        for command in commands {
            let event = match command {
                InputCommand::Ui(event) => Some(event),
                InputCommand::Application { shortcut, fallback } => {
                    if self.shortcut(shortcut) {
                        combined.changed = true;
                        None
                    } else {
                        fallback
                    }
                }
                InputCommand::Copy => Some(UiEvent::TextCopy),
                InputCommand::Cut => Some(UiEvent::TextCut),
                InputCommand::Paste => clipboard_text.map(|text| UiEvent::TextPaste(text.into())),
            };
            let Some(event) = event else {
                continue;
            };
            let outcome = self.handle_event(event);
            combined.changed |= outcome.changed;
            if outcome.clipboard_text.is_some() {
                combined.clipboard_text = outcome.clipboard_text;
            }
        }
        combined
    }

    fn rebuild(&mut self) {
        self.tree =
            UiTree::layout_with_state(self.application.view(), self.bounds, &mut self.state);
    }
}

pub fn run<A: Application>(mut application: A) -> Result<(), Box<dyn Error>> {
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let clipboard = video.clipboard();
    let mut events = sdl.event_pump()?;
    let (width, height) = application.initial_size();
    let window = video
        .window(application.title(), width, height)
        .position_centered()
        .resizable()
        .high_pixel_density()
        .build()?;
    let mut presenter = SdlCanvasPresenter::new(window)?;
    let text_input = video.text_input();
    text_input.start(presenter.window());
    let mut state = UiStateStore::default();
    let default_cursor = MouseCursor::from_system(SystemCursor::Arrow).ok();
    let hand_cursor = MouseCursor::from_system(SystemCursor::Hand).ok();
    let text_cursor = MouseCursor::from_system(SystemCursor::IBeam).ok();
    let mut pointer_icon = PointerIcon::Default;
    let mut running = true;
    let (logical_width, logical_height) = presenter.window().size();
    let pixel_width = presenter.window().size_in_pixels().0;
    let mut scale = pixel_width as f32 / logical_width.max(1) as f32;
    let mut tree = UiTree::layout_with_state(
        application.view(),
        Rect::new(0.0, 0.0, logical_width as f32, logical_height as f32),
        &mut state,
    );
    presenter.present_accelerated(tree.commands(), scale)?;
    let mut scheduler = FrameScheduler::default();
    let mut input_adapter = nickel_input::sdl::Adapter::default();
    let mut input_dispatcher = FocusedInputDispatcher::default();
    let mut controller = ControllerInput::new();
    let mut next_caret_blink = Instant::now() + Duration::from_millis(500);

    while running {
        let caret_tick = Instant::now() >= next_caret_blink;
        if caret_tick {
            next_caret_blink = Instant::now() + Duration::from_millis(500);
            if state.toggle_caret() != crate::Invalidation::None {
                scheduler.invalidate();
            }
        }
        if application.poll() {
            scheduler.invalidate();
        }
        for action in controller.poll(Instant::now(), state.window_focused()) {
            if application.controller_action(action) {
                scheduler.invalidate();
                continue;
            }
            let Some(event) = controller_ui_event(action) else {
                continue;
            };
            let outcome = tree.handle_event(&mut state, event);
            for message in outcome.messages {
                application.update(message);
            }
            if outcome.invalidation != crate::Invalidation::None {
                scheduler.invalidate();
            }
        }
        if scheduler.take_rebuild() {
            let (logical_width, logical_height) = presenter.window().size();
            let pixel_width = presenter.window().size_in_pixels().0;
            scale = pixel_width as f32 / logical_width.max(1) as f32;
            tree = UiTree::layout_with_state(
                application.view(),
                Rect::new(0.0, 0.0, logical_width as f32, logical_height as f32),
                &mut state,
            );
            presenter.present_accelerated(tree.commands(), scale)?;
        }

        let Some(event) = events.wait_event_timeout(Duration::from_millis(16)) else {
            continue;
        };
        let mut pending = vec![event];
        pending.extend(events.poll_iter());
        for event in pending {
            match &event {
                Event::Quit { .. }
                | Event::Window {
                    win_event: WindowEvent::CloseRequested,
                    ..
                } => {
                    running = false;
                    continue;
                }
                Event::Window {
                    win_event: WindowEvent::Resized(_, _) | WindowEvent::PixelSizeChanged(_, _),
                    ..
                } => {
                    scheduler.invalidate();
                    continue;
                }
                _ => {}
            }
            let Some(normalized) = input_adapter.normalize(&event) else {
                continue;
            };
            let context = InputContext {
                text_focused: state.focused().is_some(),
                selection_owned: state.selection_owner().is_some(),
            };
            let commands = input_dispatcher.dispatch_with_context(&normalized, context);
            for command in commands {
                let event = match command {
                    InputCommand::Ui(event) => event,
                    InputCommand::Application { shortcut, fallback } => {
                        if application.shortcut(shortcut) {
                            scheduler.invalidate();
                            continue;
                        }
                        let Some(fallback) = fallback else {
                            continue;
                        };
                        fallback
                    }
                    InputCommand::Copy => UiEvent::TextCopy,
                    InputCommand::Cut => {
                        let Some(selected) = tree.selected_text(&state) else {
                            continue;
                        };
                        if clipboard.set_clipboard_text(&selected).is_err() {
                            continue;
                        }
                        UiEvent::TextCut
                    }
                    InputCommand::Paste => {
                        let Ok(text) = clipboard.clipboard_text() else {
                            continue;
                        };
                        UiEvent::TextPaste(text)
                    }
                };
                if let UiEvent::PointerMoved(point)
                | UiEvent::PointerPressed(point)
                | UiEvent::PointerReleased(point) = event
                {
                    let cursor = point;
                    let next_icon = tree.pointer_icon_at(cursor);
                    if next_icon != pointer_icon {
                        if let Some(cursor) = match next_icon {
                            PointerIcon::Default => default_cursor.as_ref(),
                            PointerIcon::Hand => hand_cursor.as_ref(),
                            PointerIcon::Text => text_cursor.as_ref(),
                        } {
                            cursor.set();
                        }
                        pointer_icon = next_icon;
                    }
                }
                let resets_caret = matches!(
                    &event,
                    UiEvent::PointerPressed(_)
                        | UiEvent::PointerMoved(_)
                        | UiEvent::TextInput(_)
                        | UiEvent::TextBackspace
                        | UiEvent::TextBackspaceWord
                        | UiEvent::TextDelete
                        | UiEvent::TextMoveLeft { .. }
                        | UiEvent::TextMoveRight { .. }
                        | UiEvent::TextMoveWordLeft { .. }
                        | UiEvent::TextMoveWordRight { .. }
                        | UiEvent::TextMoveHome { .. }
                        | UiEvent::TextMoveEnd { .. }
                        | UiEvent::TextMoveDocumentHome { .. }
                        | UiEvent::TextMoveDocumentEnd { .. }
                        | UiEvent::TextSelectAll
                        | UiEvent::TextCut
                        | UiEvent::TextPaste(_)
                );
                let outcome = tree.handle_event(&mut state, event);
                if resets_caret {
                    next_caret_blink = Instant::now() + Duration::from_millis(500);
                }
                for message in outcome.messages {
                    application.update(message);
                }
                if let Some(text) = outcome.clipboard_text {
                    let _ = clipboard.set_clipboard_text(&text);
                }
                if outcome.invalidation != crate::Invalidation::None {
                    scheduler.invalidate();
                }
            }
        }
    }
    text_input.stop(presenter.window());
    state.destroy();
    Ok(())
}

#[cfg(test)]
mod tests {
    use nickel_input::{
        DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation, LogicalKey,
        Modifier, ModifierState, NamedKey, PhysicalKey, Point, PointerButton, PointerEvent,
        TextEvent,
    };

    use super::{Application, ApplicationHost, FrameScheduler, Shortcut};
    use crate::{ControllerAction, Invalidation, TextField, UiStateStore};

    #[derive(Clone)]
    enum Message {
        Changed(String),
    }

    #[derive(Default)]
    struct InputApplication {
        text: String,
        submits: usize,
    }

    impl Application for InputApplication {
        type Message = Message;

        fn update(&mut self, message: Self::Message) {
            match message {
                Message::Changed(text) => self.text = text,
            }
        }

        fn view(&self) -> impl crate::View<Self::Message> {
            TextField::on_change(&self.text, Message::Changed)
        }

        fn shortcut(&mut self, shortcut: Shortcut) -> bool {
            if shortcut != Shortcut::Submit {
                return false;
            }
            self.submits += 1;
            true
        }

        fn controller_action(&mut self, _action: ControllerAction) -> bool {
            true
        }
    }

    fn key(order: u64, repeat: bool) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(KeyCode::Enter),
            logical: LogicalKey::Named(NamedKey::Enter),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat,
            modifiers: ModifierState::default(),
        })
    }

    fn command_key(order: u64, physical: KeyCode, logical: &str) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(physical),
            logical: LogicalKey::Character(logical.into()),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers: ModifierState::from_sides([Modifier::ControlLeft]),
        })
    }

    fn focus_event() -> InputEvent {
        InputEvent::Pointer(PointerEvent::Button {
            device: DeviceId(2),
            order: EventOrder(1),
            button: PointerButton::Primary,
            edge: KeyEdge::Pressed,
            position: Some(Point { x: 4.0, y: 4.0 }),
        })
    }

    #[test]
    fn idle_frames_do_not_rebuild_and_event_batches_coalesce() {
        let mut scheduler = FrameScheduler::default();
        assert!(!scheduler.take_rebuild());
        scheduler.invalidate();
        scheduler.invalidate();
        scheduler.invalidate();
        assert!(scheduler.take_rebuild());
        assert!(!scheduler.take_rebuild());
    }

    #[test]
    fn an_unfocused_caret_tick_does_not_invalidate_the_window() {
        let mut state = UiStateStore::default();

        assert_eq!(state.toggle_caret(), Invalidation::None);
    }

    #[test]
    fn embedded_controller_dispatch_respects_window_focus() {
        let mut host = ApplicationHost::new(InputApplication::default(), 320, 48);
        host.handle_event(crate::UiEvent::FocusGained);
        assert!(
            host.handle_controller_action(ControllerAction::Down)
                .changed
        );
        host.handle_event(crate::UiEvent::FocusLost);
        assert!(
            !host
                .handle_controller_action(ControllerAction::Down)
                .changed
        );
        host.handle_event(crate::UiEvent::FocusGained);
        assert!(
            host.handle_controller_action(ControllerAction::Down)
                .changed
        );
    }

    #[test]
    fn embedded_host_dispatches_normalized_text_ime_and_submit_once() {
        let mut host = ApplicationHost::new(InputApplication::default(), 320, 48);
        assert!(host.handle_input(&focus_event(), None).changed);
        assert!(host.input_context().text_focused);
        assert!(
            !host
                .handle_input(
                    &InputEvent::FocusGained {
                        order: EventOrder(2),
                    },
                    None,
                )
                .changed
        );
        assert!(host.input_context().text_focused);

        let preedit = InputEvent::Text(TextEvent::Preedit {
            device: DeviceId(1),
            order: EventOrder(3),
            text: "世".into(),
            selection: Some((0, 3)),
        });
        assert!(host.handle_input(&preedit, None).changed);
        assert!(host.application_mut().text.is_empty());

        let commit = InputEvent::Text(TextEvent::Commit {
            device: DeviceId(1),
            order: EventOrder(4),
            text: "世界".into(),
        });
        assert!(host.handle_input(&commit, None).changed);
        assert_eq!(host.application_mut().text, "世界");

        assert!(host.handle_input(&key(5, false), None).changed);
        assert_eq!(host.application_mut().submits, 1);
        assert!(!host.handle_input(&key(6, true), None).changed);
        assert_eq!(host.application_mut().submits, 1);
    }

    #[test]
    fn embedded_host_owns_one_clipboard_command_path() {
        let mut host = ApplicationHost::new(InputApplication::default(), 320, 48);
        host.handle_input(&focus_event(), None);
        host.handle_input(
            &InputEvent::Text(TextEvent::Commit {
                device: DeviceId(1),
                order: EventOrder(2),
                text: "copy me".into(),
            }),
            None,
        );
        host.handle_input(&command_key(3, KeyCode::KeyA, "a"), None);

        let copied = host.handle_input(&command_key(4, KeyCode::KeyC, "c"), None);
        assert_eq!(copied.clipboard_text.as_deref(), Some("copy me"));
        assert_eq!(host.application_mut().text, "copy me");

        let cut = host.handle_input(&command_key(5, KeyCode::KeyX, "x"), None);
        assert_eq!(cut.clipboard_text.as_deref(), Some("copy me"));
        assert!(host.application_mut().text.is_empty());

        assert!(
            host.handle_input(&command_key(6, KeyCode::KeyV, "v"), Some("pasted"))
                .changed
        );
        assert_eq!(host.application_mut().text, "pasted");
    }
}
