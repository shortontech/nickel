//! Shared focused-input policy for Nickel UI component trees.

use nickel_input::{
    AggregateModifier, InputEvent, KeyCode, KeyEdge, LogicalKey, NamedKey, PhysicalKey,
    PointerButton, PointerEvent, TextEvent, TouchEvent, TouchId,
};

use crate::{Point, Shortcut, UiEvent};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct InputContext {
    pub text_focused: bool,
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
                delta, position, ..
            }) => {
                if let Some(position) = position {
                    self.pointer = Point {
                        x: position.x as f32,
                        y: position.y as f32,
                    };
                }
                let mut commands = Vec::new();
                if delta.x != 0.0 {
                    commands.push(InputCommand::Ui(UiEvent::ScrollHorizontal {
                        point: self.pointer,
                        delta_x: -(delta.x as f32) * 42.0,
                    }));
                }
                if delta.y != 0.0 {
                    commands.push(InputCommand::Ui(UiEvent::Scroll {
                        point: self.pointer,
                        delta_y: -(delta.y as f32) * 42.0,
                    }));
                }
                commands
            }
            InputEvent::FocusGained { .. } => vec![InputCommand::Ui(UiEvent::FocusGained)],
            InputEvent::FocusLost { .. } => vec![InputCommand::Ui(UiEvent::FocusLost)],
            InputEvent::DeviceRemoved { .. } => vec![InputCommand::Ui(UiEvent::DeviceRemoved)],
            InputEvent::Key(event) if event.edge == KeyEdge::Pressed => {
                let shift = event.modifiers.aggregate(AggregateModifier::Shift);
                let control = event.modifiers.aggregate(AggregateModifier::Control);
                let command = if cfg!(target_os = "macos") {
                    event.modifiers.aggregate(AggregateModifier::Super)
                } else {
                    control
                };
                let command = match (&event.logical, &event.physical) {
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
                    (LogicalKey::Named(NamedKey::Enter), _) if shift && context.text_focused => {
                        InputCommand::Ui(UiEvent::TextInput("\n".into()))
                    }
                    (LogicalKey::Named(NamedKey::Enter), _) if shift && !event.repeat => {
                        InputCommand::Application {
                            shortcut: Shortcut::Newline,
                            fallback: Some(UiEvent::KeyboardActivate),
                        }
                    }
                    (LogicalKey::Named(NamedKey::Enter), _) if !event.repeat => {
                        InputCommand::Application {
                            shortcut: Shortcut::Submit,
                            fallback: Some(UiEvent::KeyboardActivate),
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
                    (LogicalKey::Named(NamedKey::Tab), _) if shift => {
                        InputCommand::Ui(UiEvent::FocusPrevious)
                    }
                    (LogicalKey::Named(NamedKey::Tab), _) => InputCommand::Ui(UiEvent::FocusNext),
                    (LogicalKey::Named(NamedKey::Backspace), _) if control => {
                        InputCommand::Ui(UiEvent::TextBackspaceWord)
                    }
                    (LogicalKey::Named(NamedKey::Backspace), _) => {
                        InputCommand::Ui(UiEvent::TextBackspace)
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
                        InputCommand::Ui(UiEvent::TextMoveLeft {
                            extend_selection: shift,
                        })
                    }
                    (LogicalKey::Named(NamedKey::ArrowRight), _) if control => {
                        InputCommand::Ui(UiEvent::TextMoveWordRight {
                            extend_selection: shift,
                        })
                    }
                    (LogicalKey::Named(NamedKey::ArrowRight), _) => {
                        InputCommand::Ui(UiEvent::TextMoveRight {
                            extend_selection: shift,
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
                        return vec![InputCommand::Copy];
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyC)) if command => {
                        return vec![InputCommand::Copy];
                    }
                    (LogicalKey::Character(value), _)
                        if command && value.eq_ignore_ascii_case("x") =>
                    {
                        return vec![InputCommand::Cut];
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyX)) if command => {
                        return vec![InputCommand::Cut];
                    }
                    (LogicalKey::Character(value), _)
                        if command && value.eq_ignore_ascii_case("v") =>
                    {
                        return vec![InputCommand::Paste];
                    }
                    (_, PhysicalKey::Code(KeyCode::KeyV)) if command => {
                        return vec![InputCommand::Paste];
                    }
                    _ => return Vec::new(),
                };
                if event.repeat
                    && matches!(
                        command,
                        InputCommand::Ui(
                            UiEvent::FocusNext
                                | UiEvent::FocusPrevious
                                | UiEvent::KeyboardActivate
                                | UiEvent::SelectionClear
                        )
                    )
                {
                    Vec::new()
                } else {
                    vec![match command {
                        InputCommand::Ui(_) | InputCommand::Application { .. } => command,
                        InputCommand::Copy | InputCommand::Cut | InputCommand::Paste => {
                            unreachable!()
                        }
                    }]
                }
            }
            InputEvent::Key(_) | InputEvent::Pointer(_) | InputEvent::Touch(_) => Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use nickel_input::{
        DeviceId, EventOrder, KeyEvent, KeyLocation, Modifier, ModifierState, NativeCode, NativeKey,
    };

    use super::*;

    fn key(logical: LogicalKey, physical: KeyCode, sides: &[Modifier]) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(1),
            physical: PhysicalKey::Code(physical),
            logical,
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers: ModifierState::from_sides(sides.iter().copied()),
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
}
