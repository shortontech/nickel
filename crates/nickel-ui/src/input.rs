//! Shared focused-input policy for Nickel UI component trees.

use nickel_input::{
    AggregateModifier, DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, LogicalKey, NamedKey,
    PhysicalKey, PointerButton, PointerEvent, TextEvent, TouchEvent, TouchId,
};
use std::collections::VecDeque;

use crate::{Point, Shortcut, UiEvent};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputContext {
    pub text_focused: bool,
    /// A retained controller/tree target takes precedence over a stale text
    /// focus when keyboard keys are serving as the controller proxy.
    pub navigation_active: bool,
    pub selection_owned: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputCommand {
    Ui(UiEvent),
    Application {
        shortcut: Shortcut,
        fallback: Option<UiEvent>,
    },
    Copy,
    Cut,
    Paste,
}

#[derive(Clone, Debug, Default)]
pub struct FocusedInputDispatcher {
    pointer: Point,
    active_touch: Option<TouchId>,
    consumed_text: VecDeque<ConsumedTextTransaction>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ConsumedTextTransaction {
    device: DeviceId,
    order: EventOrder,
}

impl FocusedInputDispatcher {
    pub fn dispatch(&mut self, event: &InputEvent) -> Vec<InputCommand> {
        self.dispatch_with_context(event, InputContext::default())
    }

    pub fn dispatch_with_context(
        &mut self,
        event: &InputEvent,
        context: InputContext,
    ) -> Vec<InputCommand> {
        if let InputEvent::Text(text_event) = event {
            let (device, order, committed) = match text_event {
                TextEvent::Commit { device, order, .. } => (*device, *order, true),
                TextEvent::Preedit { device, order, .. } => (*device, *order, false),
            };
            let correlated = ConsumedTextTransaction { device, order };
            if let Some(index) = self
                .consumed_text
                .iter()
                .position(|transaction| *transaction == correlated)
            {
                // One composition transaction may contain multiple preedit
                // updates followed by a commit. Keep suppressing until the
                // commit closes that exact device/order transaction.
                if committed {
                    self.consumed_text.remove(index);
                }
                return Vec::new();
            }
            // A backend is allowed to omit correlated text events. A later
            // transaction from the same device proves older suppressions can
            // no longer match, so retire them without disturbing transactions
            // still pending on another keyboard/IME device.
            self.consumed_text.retain(|transaction| {
                transaction.device != device || transaction.order.0 >= order.0
            });
        }

        match event {
            InputEvent::Text(TextEvent::Commit { text, .. }) => {
                vec![InputCommand::Ui(UiEvent::TextInput(text.clone()))]
            }
            InputEvent::Text(TextEvent::Preedit { text, .. }) => {
                vec![InputCommand::Ui(UiEvent::ImePreedit(text.clone()))]
            }
            InputEvent::Pointer(PointerEvent::Motion { position, .. }) => {
                self.pointer = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                vec![InputCommand::Ui(UiEvent::PointerMoved(self.pointer))]
            }
            InputEvent::Touch(TouchEvent::Started {
                contact, position, ..
            }) if self.active_touch.is_none() => {
                self.active_touch = Some(*contact);
                self.pointer = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                vec![
                    InputCommand::Ui(UiEvent::PointerMoved(self.pointer)),
                    InputCommand::Ui(UiEvent::PointerPressed(self.pointer)),
                ]
            }
            InputEvent::Touch(TouchEvent::Moved {
                contact, position, ..
            }) if self.active_touch == Some(*contact) => {
                self.pointer = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                vec![InputCommand::Ui(UiEvent::PointerMoved(self.pointer))]
            }
            InputEvent::Touch(TouchEvent::Ended {
                contact, position, ..
            }) if self.active_touch == Some(*contact) => {
                self.active_touch = None;
                self.pointer = Point {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                vec![InputCommand::Ui(UiEvent::PointerReleased(self.pointer))]
            }
            InputEvent::Touch(TouchEvent::Cancelled { contact, .. })
                if self.active_touch == Some(*contact) =>
            {
                self.active_touch = None;
                vec![InputCommand::Ui(UiEvent::PointerCancelled)]
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Primary,
                edge,
                position,
                ..
            }) => {
                if let Some(position) = position {
                    self.pointer = Point {
                        x: position.x as f32,
                        y: position.y as f32,
                    };
                }
                vec![InputCommand::Ui(match edge {
                    KeyEdge::Pressed => UiEvent::PointerPressed(self.pointer),
                    KeyEdge::Released => UiEvent::PointerReleased(self.pointer),
                })]
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Secondary,
                edge: KeyEdge::Pressed,
                position,
                ..
            }) => {
                if let Some(position) = position {
                    self.pointer = Point {
                        x: position.x as f32,
                        y: position.y as f32,
                    };
                }
                vec![InputCommand::Ui(UiEvent::PointerContext(self.pointer))]
            }
            InputEvent::Pointer(PointerEvent::Axis {
                delta,
                discrete,
                position,
                ..
            }) => {
                if let Some(position) = position {
                    self.pointer = Point {
                        x: position.x as f32,
                        y: position.y as f32,
                    };
                }
                let mut commands = Vec::new();
                let multiplier = if discrete.is_some() { 42.0 } else { 1.0 };
                if delta.x != 0.0 {
                    commands.push(InputCommand::Ui(UiEvent::ScrollHorizontal {
                        point: self.pointer,
                        delta_x: -(delta.x as f32) * multiplier,
                    }));
                }
                if delta.y != 0.0 {
                    commands.push(InputCommand::Ui(UiEvent::Scroll {
                        point: self.pointer,
                        delta_y: -(delta.y as f32) * multiplier,
                    }));
                }
                commands
            }
            InputEvent::FocusGained { .. } => {
                self.consumed_text.clear();
                vec![InputCommand::Ui(UiEvent::FocusGained)]
            }
            InputEvent::FocusLost { .. } => {
                self.consumed_text.clear();
                vec![InputCommand::Ui(UiEvent::FocusLost)]
            }
            InputEvent::DeviceRemoved { device, .. } => {
                self.consumed_text
                    .retain(|transaction| transaction.device != *device);
                vec![InputCommand::Ui(UiEvent::DeviceRemoved)]
            }
            InputEvent::Key(event) if event.edge == KeyEdge::Pressed => {
                let shift = event.modifiers.aggregate(AggregateModifier::Shift);
                let control = event.modifiers.aggregate(AggregateModifier::Control);
                let text_editing = context.text_focused && !context.navigation_active;
                let command_modifier = control;
                // AltGr is commonly reported as Control+Alt. It produces text and must not be
                // mistaken for an editing shortcut merely because Control is present.
                let command =
                    command_modifier && !event.modifiers.aggregate(AggregateModifier::Alt);
                let command = match (&event.logical, &event.physical) {
                    (LogicalKey::Named(NamedKey::ContextMenu), _)
                    | (_, PhysicalKey::Code(KeyCode::ContextMenu))
                        if !event.repeat =>
                    {
                        InputCommand::Ui(UiEvent::KeyboardContextMenu)
                    }
                    (_, PhysicalKey::Code(KeyCode::F10)) if shift && !event.repeat => {
                        InputCommand::Ui(UiEvent::KeyboardContextMenu)
                    }
                    (LogicalKey::Character(value), _)
                        if command && value.eq_ignore_ascii_case("r") && !event.repeat =>
                    {
                        InputCommand::Application {
                            shortcut: Shortcut::Reload,
                            fallback: None,
                        }
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyR)) if command && !event.repeat => {
                        InputCommand::Application {
                            shortcut: Shortcut::Reload,
                            fallback: None,
                        }
                    }
                    (LogicalKey::Named(NamedKey::ArrowLeft), _)
                        if event.modifiers.aggregate(AggregateModifier::Alt) && !event.repeat =>
                    {
                        InputCommand::Application {
                            shortcut: Shortcut::Back,
                            fallback: None,
                        }
                    }
                    (LogicalKey::Named(NamedKey::ArrowRight), _)
                        if event.modifiers.aggregate(AggregateModifier::Alt) && !event.repeat =>
                    {
                        InputCommand::Application {
                            shortcut: Shortcut::Forward,
                            fallback: None,
                        }
                    }
                    (LogicalKey::Named(NamedKey::Enter), _) if shift && text_editing => {
                        InputCommand::Ui(UiEvent::TextInput("\n".into()))
                    }
                    (LogicalKey::Named(NamedKey::Enter), _) if shift && !event.repeat => {
                        InputCommand::Application {
                            shortcut: Shortcut::Newline,
                            fallback: Some(UiEvent::KeyboardActivate),
                        }
                    }
                    (LogicalKey::Named(NamedKey::Enter), _)
                        if context.navigation_active && !event.repeat =>
                    {
                        InputCommand::Ui(UiEvent::KeyboardNavigateActivate)
                    }
                    (LogicalKey::Named(NamedKey::Enter), _) if !event.repeat => {
                        InputCommand::Application {
                            shortcut: Shortcut::Submit,
                            fallback: Some(if text_editing {
                                UiEvent::KeyboardActivate
                            } else {
                                UiEvent::KeyboardNavigateActivate
                            }),
                        }
                    }
                    (LogicalKey::Named(NamedKey::Space), _) if !event.repeat => {
                        InputCommand::Ui(UiEvent::KeyboardActivate)
                    }
                    (LogicalKey::Named(NamedKey::Escape), _) if context.selection_owned => {
                        InputCommand::Ui(UiEvent::SelectionClear)
                    }
                    (LogicalKey::Named(NamedKey::Escape), _) if !event.repeat => {
                        InputCommand::Application {
                            shortcut: Shortcut::Escape,
                            fallback: Some(UiEvent::Dismiss),
                        }
                    }
                    (LogicalKey::Named(NamedKey::Home), _) if context.navigation_active => {
                        InputCommand::Ui(UiEvent::KeyboardNavigateStart)
                    }
                    (LogicalKey::Named(NamedKey::End), _) if context.navigation_active => {
                        InputCommand::Ui(UiEvent::KeyboardNavigateEnd)
                    }
                    (LogicalKey::Named(NamedKey::PageUp), _) if context.navigation_active => {
                        InputCommand::Ui(UiEvent::KeyboardNavigatePageUp)
                    }
                    (LogicalKey::Named(NamedKey::PageDown), _) if context.navigation_active => {
                        InputCommand::Ui(UiEvent::KeyboardNavigatePageDown)
                    }
                    (LogicalKey::Named(NamedKey::Home), _) => InputCommand::Application {
                        shortcut: Shortcut::DocumentStart,
                        fallback: Some(if control {
                            UiEvent::TextMoveDocumentHome {
                                extend_selection: shift,
                            }
                        } else {
                            UiEvent::TextMoveHome {
                                extend_selection: shift,
                            }
                        }),
                    },
                    (LogicalKey::Named(NamedKey::End), _) => InputCommand::Application {
                        shortcut: Shortcut::DocumentEnd,
                        fallback: Some(if control {
                            UiEvent::TextMoveDocumentEnd {
                                extend_selection: shift,
                            }
                        } else {
                            UiEvent::TextMoveEnd {
                                extend_selection: shift,
                            }
                        }),
                    },
                    (LogicalKey::Named(NamedKey::PageUp), _) => {
                        InputCommand::Ui(UiEvent::KeyboardNavigatePageUp)
                    }
                    (LogicalKey::Named(NamedKey::PageDown), _) => {
                        InputCommand::Ui(UiEvent::KeyboardNavigatePageDown)
                    }
                    (LogicalKey::Named(NamedKey::Tab), _) if shift => {
                        InputCommand::Ui(UiEvent::FocusPrevious)
                    }
                    (LogicalKey::Named(NamedKey::Tab), _) => InputCommand::Ui(UiEvent::FocusNext),
                    (LogicalKey::Named(NamedKey::Backspace), _) if control => {
                        InputCommand::Ui(UiEvent::TextBackspaceWord)
                    }
                    (LogicalKey::Named(NamedKey::Backspace), _) => {
                        InputCommand::Ui(if text_editing {
                            UiEvent::TextBackspace
                        } else {
                            UiEvent::KeyboardNavigateBack
                        })
                    }
                    (LogicalKey::Named(NamedKey::Delete), _) => {
                        InputCommand::Ui(UiEvent::TextDelete)
                    }
                    (LogicalKey::Named(NamedKey::ArrowLeft), _) if control => {
                        InputCommand::Ui(UiEvent::TextMoveWordLeft {
                            extend_selection: shift,
                        })
                    }
                    (LogicalKey::Named(NamedKey::ArrowLeft), _) => {
                        InputCommand::Ui(if text_editing {
                            UiEvent::TextMoveLeft {
                                extend_selection: shift,
                            }
                        } else {
                            UiEvent::KeyboardNavigateLeft
                        })
                    }
                    (LogicalKey::Named(NamedKey::ArrowRight), _) if control => {
                        InputCommand::Ui(UiEvent::TextMoveWordRight {
                            extend_selection: shift,
                        })
                    }
                    (LogicalKey::Named(NamedKey::ArrowRight), _) => {
                        InputCommand::Ui(if text_editing {
                            UiEvent::TextMoveRight {
                                extend_selection: shift,
                            }
                        } else {
                            UiEvent::KeyboardNavigateRight
                        })
                    }
                    (LogicalKey::Named(NamedKey::ArrowUp), _) => {
                        InputCommand::Ui(UiEvent::KeyboardNavigateUp)
                    }
                    (LogicalKey::Named(NamedKey::ArrowDown), _) => {
                        InputCommand::Ui(UiEvent::KeyboardNavigateDown)
                    }
                    (LogicalKey::Character(value), _)
                        if command && text_editing && value.eq_ignore_ascii_case("z") =>
                    {
                        InputCommand::Ui(if shift {
                            UiEvent::TextRedo
                        } else {
                            UiEvent::TextUndo
                        })
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyZ)) if command && text_editing => {
                        InputCommand::Ui(if shift {
                            UiEvent::TextRedo
                        } else {
                            UiEvent::TextUndo
                        })
                    }
                    (LogicalKey::Character(value), _)
                        if command && value.eq_ignore_ascii_case("a") =>
                    {
                        InputCommand::Ui(UiEvent::TextSelectAll)
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyA)) if command => {
                        InputCommand::Ui(UiEvent::TextSelectAll)
                    }
                    (LogicalKey::Character(value), _)
                        if command && value.eq_ignore_ascii_case("c") =>
                    {
                        InputCommand::Copy
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyC)) if command => InputCommand::Copy,
                    (LogicalKey::Character(value), _)
                        if command && value.eq_ignore_ascii_case("x") =>
                    {
                        InputCommand::Cut
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyX)) if command => InputCommand::Cut,
                    (LogicalKey::Character(value), _)
                        if command && value.eq_ignore_ascii_case("v") =>
                    {
                        InputCommand::Paste
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyV)) if command => InputCommand::Paste,
                    _ => return Vec::new(),
                };
                if consumes_correlated_text(&command) {
                    self.consume_text_from(event);
                }
                if event.repeat && suppresses_repeat(&command) {
                    Vec::new()
                } else {
                    vec![command]
                }
            }
            InputEvent::Key(_) | InputEvent::Pointer(_) | InputEvent::Touch(_) => Vec::new(),
        }
    }

    fn consume_text_from(&mut self, event: &nickel_input::KeyEvent) {
        let transaction = ConsumedTextTransaction {
            device: event.device,
            order: event.order,
        };
        if !self.consumed_text.contains(&transaction) {
            // A missing native text callback must not grow this queue forever.
            // Thirty-two outstanding command transactions is already far past
            // plausible interactive delivery latency and keeps diagnostics and
            // memory bounded under a broken backend.
            if self.consumed_text.len() == 32 {
                self.consumed_text.pop_front();
            }
            self.consumed_text.push_back(transaction);
        }
    }
}

fn consumes_correlated_text(command: &InputCommand) -> bool {
    matches!(
        command,
        InputCommand::Copy | InputCommand::Cut | InputCommand::Paste
    ) || matches!(
        command,
        InputCommand::Ui(
            UiEvent::TextSelectAll
                | UiEvent::TextUndo
                | UiEvent::TextRedo
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
        ) | InputCommand::Application {
            shortcut: Shortcut::Reload,
            ..
        }
    )
}

fn suppresses_repeat(command: &InputCommand) -> bool {
    matches!(
        command,
        InputCommand::Copy | InputCommand::Cut | InputCommand::Paste
    ) || matches!(
        command,
        InputCommand::Ui(
            UiEvent::FocusNext
                | UiEvent::FocusPrevious
                | UiEvent::KeyboardActivate
                | UiEvent::SelectionClear
                | UiEvent::TextSelectAll
                | UiEvent::TextUndo
                | UiEvent::TextRedo
        )
    )
}

#[cfg(test)]
mod tests {
    use nickel_input::{
        DeviceId, EventOrder, KeyEvent, KeyLocation, Modifier, ModifierState, NativeCode, NativeKey,
    };

    use super::*;

    fn key_at(
        order: u64,
        logical: LogicalKey,
        physical: KeyCode,
        sides: &[Modifier],
    ) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(physical),
            logical,
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers: ModifierState::from_sides(sides.iter().copied()),
        })
    }

    fn key(logical: LogicalKey, physical: KeyCode, sides: &[Modifier]) -> InputEvent {
        key_at(1, logical, physical, sides)
    }

    fn commit(device: u64, order: u64, text: &str) -> InputEvent {
        InputEvent::Text(TextEvent::Commit {
            device: DeviceId(device),
            order: EventOrder(order),
            text: text.into(),
        })
    }

    fn preedit(device: u64, order: u64, text: &str) -> InputEvent {
        InputEvent::Text(TextEvent::Preedit {
            device: DeviceId(device),
            order: EventOrder(order),
            text: text.into(),
            selection: None,
        })
    }

    #[test]
    fn layout_logical_navigation_and_physical_command_fallback_share_dispatch() {
        let mut dispatch = FocusedInputDispatcher::default();
        assert_eq!(
            dispatch.dispatch(&key(
                LogicalKey::Named(NamedKey::Tab),
                KeyCode::Tab,
                &[Modifier::ShiftRight]
            )),
            [InputCommand::Ui(UiEvent::FocusPrevious)]
        );
        assert_eq!(
            dispatch.dispatch(&key(
                LogicalKey::Native(NativeKey {
                    namespace: "fixture".into(),
                    code: NativeCode::Numeric(9),
                }),
                KeyCode::KeyC,
                &[Modifier::ControlLeft]
            )),
            [InputCommand::Copy]
        );
    }

    #[test]
    fn release_does_not_activate() {
        let mut event = key(LogicalKey::Named(NamedKey::Enter), KeyCode::Enter, &[]);
        let InputEvent::Key(key_event) = &mut event else {
            unreachable!()
        };
        key_event.edge = KeyEdge::Released;
        assert!(
            FocusedInputDispatcher::default()
                .dispatch(&event)
                .is_empty()
        );
    }

    #[test]
    fn shift_f10_dispatches_the_shared_context_action() {
        assert_eq!(
            FocusedInputDispatcher::default().dispatch(&key(
                LogicalKey::Native(NativeKey {
                    namespace: "fixture".into(),
                    code: NativeCode::Numeric(10),
                }),
                KeyCode::F10,
                &[Modifier::ShiftLeft]
            )),
            [InputCommand::Ui(UiEvent::KeyboardContextMenu)]
        );
    }

    #[test]
    fn context_menu_key_dispatches_the_shared_context_action() {
        assert_eq!(
            FocusedInputDispatcher::default().dispatch(&key(
                LogicalKey::Named(NamedKey::ContextMenu),
                KeyCode::ContextMenu,
                &[]
            )),
            [InputCommand::Ui(UiEvent::KeyboardContextMenu)]
        );
    }

    #[test]
    fn command_z_routes_undo_and_redo_only_for_an_editor() {
        let mut dispatch = FocusedInputDispatcher::default();
        let editor = InputContext {
            text_focused: true,
            ..InputContext::default()
        };
        assert_eq!(
            dispatch.dispatch_with_context(
                &key(
                    LogicalKey::Character("z".into()),
                    KeyCode::KeyZ,
                    &[Modifier::ControlLeft]
                ),
                editor,
            ),
            [InputCommand::Ui(UiEvent::TextUndo)]
        );
        assert_eq!(
            dispatch.dispatch_with_context(
                &key(
                    LogicalKey::Character("z".into()),
                    KeyCode::KeyZ,
                    &[Modifier::ControlLeft, Modifier::ShiftLeft],
                ),
                editor,
            ),
            [InputCommand::Ui(UiEvent::TextRedo)]
        );
    }

    #[test]
    fn retained_navigation_target_wins_over_stale_text_focus() {
        let context = InputContext {
            text_focused: true,
            navigation_active: true,
            selection_owned: false,
        };
        let mut dispatch = FocusedInputDispatcher::default();

        assert_eq!(
            dispatch.dispatch_with_context(
                &key(
                    LogicalKey::Named(NamedKey::ArrowRight),
                    KeyCode::ArrowRight,
                    &[]
                ),
                context,
            ),
            [InputCommand::Ui(UiEvent::KeyboardNavigateRight)]
        );
        assert_eq!(
            dispatch.dispatch_with_context(
                &key(
                    LogicalKey::Named(NamedKey::Backspace),
                    KeyCode::Backspace,
                    &[]
                ),
                context,
            ),
            [InputCommand::Ui(UiEvent::KeyboardNavigateBack)]
        );
        assert_eq!(
            dispatch.dispatch_with_context(
                &key(LogicalKey::Named(NamedKey::Enter), KeyCode::Enter, &[]),
                context,
            ),
            [InputCommand::Ui(UiEvent::KeyboardNavigateActivate)]
        );
    }

    #[test]
    fn navigation_page_and_boundary_keys_remain_semantic() {
        let context = InputContext {
            text_focused: true,
            navigation_active: true,
            selection_owned: false,
        };
        let cases = [
            (
                NamedKey::PageUp,
                KeyCode::PageUp,
                UiEvent::KeyboardNavigatePageUp,
            ),
            (
                NamedKey::PageDown,
                KeyCode::PageDown,
                UiEvent::KeyboardNavigatePageDown,
            ),
            (
                NamedKey::Home,
                KeyCode::Home,
                UiEvent::KeyboardNavigateStart,
            ),
            (NamedKey::End, KeyCode::End, UiEvent::KeyboardNavigateEnd),
        ];
        for (logical, physical, expected) in cases {
            assert_eq!(
                FocusedInputDispatcher::default().dispatch_with_context(
                    &key(LogicalKey::Named(logical), physical, &[]),
                    context,
                ),
                [InputCommand::Ui(expected)]
            );
        }
    }

    #[test]
    fn wheel_lines_are_normalized_but_trackpad_pixels_are_preserved() {
        let axis = |delta_y, discrete| {
            InputEvent::Pointer(PointerEvent::Axis {
                device: DeviceId(1),
                order: EventOrder(1),
                delta: nickel_input::Vector { x: 0.0, y: delta_y },
                discrete,
                position: Some(nickel_input::Point { x: 8.0, y: 9.0 }),
            })
        };
        assert_eq!(
            FocusedInputDispatcher::default().dispatch(&axis(2.0, Some((0, 2)))),
            [InputCommand::Ui(UiEvent::Scroll {
                point: Point { x: 8.0, y: 9.0 },
                delta_y: -84.0,
            })]
        );
        assert_eq!(
            FocusedInputDispatcher::default().dispatch(&axis(2.5, None)),
            [InputCommand::Ui(UiEvent::Scroll {
                point: Point { x: 8.0, y: 9.0 },
                delta_y: -2.5,
            })]
        );
    }

    #[test]
    fn one_touch_contact_uses_the_canonical_pointer_transition_and_cancel_path() {
        let mut dispatch = FocusedInputDispatcher::default();
        let started = InputEvent::Touch(TouchEvent::Started {
            device: DeviceId(7),
            order: EventOrder(1),
            contact: TouchId(9),
            position: nickel_input::Point { x: 12.0, y: 18.0 },
        });
        assert_eq!(
            dispatch.dispatch(&started),
            [
                InputCommand::Ui(UiEvent::PointerMoved(Point { x: 12.0, y: 18.0 })),
                InputCommand::Ui(UiEvent::PointerPressed(Point { x: 12.0, y: 18.0 })),
            ]
        );
        let cancelled = InputEvent::Touch(TouchEvent::Cancelled {
            device: DeviceId(7),
            order: EventOrder(2),
            contact: TouchId(9),
        });
        assert_eq!(
            dispatch.dispatch(&cancelled),
            [InputCommand::Ui(UiEvent::PointerCancelled)]
        );
    }

    #[test]
    fn select_all_suppresses_only_its_correlated_text_commit() {
        let mut dispatch = FocusedInputDispatcher::default();
        let control_a = key_at(
            7,
            LogicalKey::Character("a".into()),
            KeyCode::KeyA,
            &[Modifier::ControlLeft],
        );

        assert_eq!(
            dispatch.dispatch_with_context(
                &control_a,
                InputContext {
                    text_focused: true,
                    ..InputContext::default()
                }
            ),
            [InputCommand::Ui(UiEvent::TextSelectAll)]
        );
        assert!(dispatch.dispatch(&commit(1, 7, "a")).is_empty());
        assert_eq!(
            dispatch.dispatch(&commit(1, 8, "a")),
            [InputCommand::Ui(UiEvent::TextInput("a".into()))]
        );
    }

    #[test]
    fn consumed_text_is_correlated_by_device_order_and_cleared_on_focus_loss() {
        let mut dispatch = FocusedInputDispatcher::default();
        dispatch.dispatch(&key_at(
            3,
            LogicalKey::Character("x".into()),
            KeyCode::KeyX,
            &[Modifier::ControlLeft],
        ));
        assert_eq!(
            dispatch.dispatch(&commit(2, 3, "x")),
            [InputCommand::Ui(UiEvent::TextInput("x".into()))]
        );
        assert!(dispatch.dispatch(&commit(1, 3, "x")).is_empty());

        dispatch.dispatch(&key_at(
            4,
            LogicalKey::Character("a".into()),
            KeyCode::KeyA,
            &[Modifier::ControlLeft],
        ));
        dispatch.dispatch(&InputEvent::FocusLost {
            order: EventOrder(5),
        });
        assert_eq!(
            dispatch.dispatch(&commit(1, 4, "a")),
            [InputCommand::Ui(UiEvent::TextInput("a".into()))]
        );
    }

    #[test]
    fn concurrent_devices_keep_independent_consumed_text_transactions() {
        let mut dispatch = FocusedInputDispatcher::default();
        let context = InputContext {
            text_focused: true,
            ..InputContext::default()
        };
        for device in [1, 2] {
            assert_eq!(
                dispatch.dispatch_with_context(
                    &InputEvent::Key(nickel_input::KeyEvent {
                        device: DeviceId(device),
                        order: EventOrder(10),
                        physical: PhysicalKey::Code(KeyCode::KeyA),
                        logical: LogicalKey::Character("a".into()),
                        location: nickel_input::KeyLocation::Standard,
                        edge: KeyEdge::Pressed,
                        repeat: false,
                        modifiers: nickel_input::ModifierState::from_sides(
                            [Modifier::ControlLeft,]
                        ),
                    }),
                    context,
                ),
                [InputCommand::Ui(UiEvent::TextSelectAll)]
            );
        }

        assert!(dispatch.dispatch(&commit(2, 10, "a")).is_empty());
        assert!(dispatch.dispatch(&commit(1, 10, "a")).is_empty());
        assert_eq!(
            dispatch.dispatch(&commit(1, 11, "b")),
            [InputCommand::Ui(UiEvent::TextInput("b".into()))]
        );
    }

    #[test]
    fn consecutive_consumed_commands_do_not_overwrite_each_other() {
        let mut dispatch = FocusedInputDispatcher::default();
        let context = InputContext {
            text_focused: true,
            ..InputContext::default()
        };
        for order in [20, 21] {
            dispatch.dispatch_with_context(
                &key_at(
                    order,
                    LogicalKey::Character("a".into()),
                    KeyCode::KeyA,
                    &[Modifier::ControlLeft],
                ),
                context,
            );
        }

        // Native text/IME queues may report the callbacks after both key
        // callbacks; both exact transactions remain consumed.
        assert!(dispatch.dispatch(&commit(1, 20, "a")).is_empty());
        assert!(dispatch.dispatch(&commit(1, 21, "a")).is_empty());
    }

    #[test]
    fn altgr_text_and_ime_transactions_are_not_suppressed() {
        let mut dispatch = FocusedInputDispatcher::default();
        assert!(
            dispatch
                .dispatch(&key_at(
                    9,
                    LogicalKey::Character("@".into()),
                    KeyCode::KeyQ,
                    &[Modifier::ControlLeft, Modifier::AltRight],
                ))
                .is_empty()
        );
        assert_eq!(
            dispatch.dispatch(&commit(1, 9, "@")),
            [InputCommand::Ui(UiEvent::TextInput("@".into()))]
        );
        let preedit = InputEvent::Text(TextEvent::Preedit {
            device: DeviceId(1),
            order: EventOrder(10),
            text: "世".into(),
            selection: Some((0, 3)),
        });
        assert_eq!(
            dispatch.dispatch(&preedit),
            [InputCommand::Ui(UiEvent::ImePreedit("世".into()))]
        );
        assert_eq!(
            dispatch.dispatch(&commit(1, 11, "世界")),
            [InputCommand::Ui(UiEvent::TextInput("世界".into()))]
        );
    }

    #[test]
    fn every_clipboard_shortcut_consumes_its_key_generated_text() {
        let mut dispatch = FocusedInputDispatcher::default();
        for (order, code, character, expected) in [
            (20, KeyCode::KeyC, "c", InputCommand::Copy),
            (21, KeyCode::KeyX, "x", InputCommand::Cut),
            (22, KeyCode::KeyV, "v", InputCommand::Paste),
        ] {
            assert_eq!(
                dispatch.dispatch(&key_at(
                    order,
                    LogicalKey::Character(character.into()),
                    code,
                    &[Modifier::ControlLeft],
                )),
                [expected]
            );
            assert!(dispatch.dispatch(&commit(1, order, character)).is_empty());
        }
    }

    #[test]
    fn editing_shortcut_repeats_are_consumed_without_reexecuting_commands() {
        let mut dispatch = FocusedInputDispatcher::default();
        let context = InputContext {
            text_focused: true,
            ..InputContext::default()
        };
        for (order, code, character) in [
            (24, KeyCode::KeyA, "a"),
            (25, KeyCode::KeyC, "c"),
            (26, KeyCode::KeyX, "x"),
            (27, KeyCode::KeyV, "v"),
            (28, KeyCode::KeyZ, "z"),
        ] {
            let InputEvent::Key(mut repeat) = key_at(
                order,
                LogicalKey::Character(character.into()),
                code,
                &[Modifier::ControlLeft],
            ) else {
                unreachable!()
            };
            repeat.repeat = true;
            assert!(
                dispatch
                    .dispatch_with_context(&InputEvent::Key(repeat), context)
                    .is_empty()
            );
            assert!(dispatch.dispatch(&commit(1, order, character)).is_empty());
        }
    }

    #[test]
    fn consumed_composition_suppresses_every_preedit_until_its_commit() {
        let mut dispatch = FocusedInputDispatcher::default();
        dispatch.dispatch_with_context(
            &key_at(
                30,
                LogicalKey::Character("a".into()),
                KeyCode::KeyA,
                &[Modifier::ControlLeft],
            ),
            InputContext {
                text_focused: true,
                ..InputContext::default()
            },
        );
        assert!(dispatch.dispatch(&preedit(1, 30, "a")).is_empty());
        assert!(dispatch.dispatch(&preedit(1, 30, "A")).is_empty());
        assert!(dispatch.dispatch(&commit(1, 30, "A")).is_empty());
        assert_eq!(
            dispatch.dispatch(&commit(1, 31, "b")),
            [InputCommand::Ui(UiEvent::TextInput("b".into()))]
        );
    }

    #[test]
    fn undo_redo_and_reload_suppress_correlated_layout_text() {
        let mut dispatch = FocusedInputDispatcher::default();
        let context = InputContext {
            text_focused: true,
            ..InputContext::default()
        };
        for (order, sides, expected) in [
            (
                40,
                vec![Modifier::ControlLeft],
                InputCommand::Ui(UiEvent::TextUndo),
            ),
            (
                41,
                vec![Modifier::ControlLeft, Modifier::ShiftRight],
                InputCommand::Ui(UiEvent::TextRedo),
            ),
        ] {
            assert_eq!(
                dispatch.dispatch_with_context(
                    &key_at(
                        order,
                        LogicalKey::Character("z".into()),
                        KeyCode::KeyZ,
                        &sides,
                    ),
                    context,
                ),
                [expected]
            );
            assert!(dispatch.dispatch(&commit(1, order, "z")).is_empty());
        }
        assert!(matches!(
            dispatch.dispatch_with_context(
                &key_at(
                    42,
                    LogicalKey::Character("r".into()),
                    KeyCode::KeyR,
                    &[Modifier::ControlLeft],
                ),
                context,
            )[..],
            [InputCommand::Application {
                shortcut: Shortcut::Reload,
                ..
            }]
        ));
        assert!(dispatch.dispatch(&commit(1, 42, "r")).is_empty());
    }

    #[test]
    fn omitted_backend_commit_cannot_leave_sticky_suppression() {
        let mut dispatch = FocusedInputDispatcher::default();
        dispatch.dispatch(&key_at(
            50,
            LogicalKey::Character("x".into()),
            KeyCode::KeyX,
            &[Modifier::ControlLeft],
        ));
        assert_eq!(
            dispatch.dispatch(&commit(1, 51, "q")),
            [InputCommand::Ui(UiEvent::TextInput("q".into()))]
        );
        assert_eq!(
            dispatch.dispatch(&commit(1, 50, "x")),
            [InputCommand::Ui(UiEvent::TextInput("x".into()))],
            "the retired transaction must not suppress an old delayed event"
        );
    }
}
