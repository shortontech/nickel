//! SDL 3 conversions. Application policy does not belong here.

use std::collections::BTreeMap;

use sdl3::{
    event::{Event, WindowEvent},
    gamepad::{Axis as GamepadAxis, Button as GamepadButton},
    keyboard::{Keycode as SdlKeycode, Mod, Scancode},
    mouse::{MouseButton, MouseWheelDirection},
};

use crate::{
    DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation, LogicalKey,
    Modifier, ModifierState, NamedKey, NativeCode, NativeKey, PhysicalKey, Point, PointerButton,
    PointerEvent, TextEvent, TouchEvent, TouchId, Vector,
    controller::{
        ControllerAxis, ControllerButton, ControllerEvent, ControllerId, ControllerIdentity,
    },
};

pub fn controller_event(event: &Event, fingerprint: Option<String>) -> Option<ControllerEvent> {
    Some(match event {
        Event::ControllerDeviceAdded { which, .. } => ControllerEvent::Connected {
            id: ControllerId(*which as u64),
            identity: ControllerIdentity {
                backend: "sdl".into(),
                native: NativeCode::Numeric(*which as u64),
                fingerprint,
            },
        },
        Event::ControllerDeviceRemoved { which, .. } => ControllerEvent::Disconnected {
            id: ControllerId(*which as u64),
        },
        Event::ControllerButtonDown { which, button, .. } => ControllerEvent::Button {
            id: ControllerId(*which as u64),
            button: gamepad_button(*button),
            edge: KeyEdge::Pressed,
            repeat: false,
        },
        Event::ControllerButtonUp { which, button, .. } => ControllerEvent::Button {
            id: ControllerId(*which as u64),
            button: gamepad_button(*button),
            edge: KeyEdge::Released,
            repeat: false,
        },
        Event::ControllerAxisMotion {
            which, axis, value, ..
        } => ControllerEvent::Axis {
            id: ControllerId(*which as u64),
            axis: gamepad_axis(*axis),
            value: gamepad_axis_value(*axis, *value),
        },
        _ => return None,
    })
}

pub fn gamepad_axis_value(axis: GamepadAxis, value: i16) -> f32 {
    let normalized = if value >= 0 {
        value as f32 / i16::MAX as f32
    } else {
        value as f32 / -(i16::MIN as f32)
    };
    if matches!(axis, GamepadAxis::LeftY | GamepadAxis::RightY) {
        -normalized
    } else {
        normalized
    }
}

pub fn gamepad_button(button: GamepadButton) -> ControllerButton {
    match button {
        GamepadButton::South => ControllerButton::South,
        GamepadButton::East => ControllerButton::East,
        GamepadButton::West => ControllerButton::West,
        GamepadButton::North => ControllerButton::North,
        GamepadButton::DPadUp => ControllerButton::DPadUp,
        GamepadButton::DPadDown => ControllerButton::DPadDown,
        GamepadButton::DPadLeft => ControllerButton::DPadLeft,
        GamepadButton::DPadRight => ControllerButton::DPadRight,
        GamepadButton::LeftShoulder => ControllerButton::LeftShoulder,
        GamepadButton::RightShoulder => ControllerButton::RightShoulder,
        GamepadButton::Back => ControllerButton::Select,
        GamepadButton::Start => ControllerButton::Start,
        GamepadButton::Guide => ControllerButton::Guide,
        GamepadButton::LeftStick => ControllerButton::LeftStick,
        GamepadButton::RightStick => ControllerButton::RightStick,
        other => ControllerButton::Native(NativeCode::Numeric(other as u8 as u64)),
    }
}

pub fn gamepad_axis(axis: GamepadAxis) -> ControllerAxis {
    match axis {
        GamepadAxis::LeftX => ControllerAxis::LeftX,
        GamepadAxis::LeftY => ControllerAxis::LeftY,
        GamepadAxis::RightX => ControllerAxis::RightX,
        GamepadAxis::RightY => ControllerAxis::RightY,
        GamepadAxis::TriggerLeft => ControllerAxis::LeftTrigger,
        GamepadAxis::TriggerRight => ControllerAxis::RightTrigger,
    }
}

#[derive(Clone, Debug, Default)]
pub struct Adapter {
    next_order: u64,
    pointer_positions: BTreeMap<DeviceId, Point>,
}

impl Adapter {
    pub fn normalize(&mut self, event: &Event) -> Option<InputEvent> {
        self.next_order += 1;
        let order = EventOrder(self.next_order);
        Some(match event {
            Event::KeyDown {
                which,
                scancode,
                keycode,
                keymod,
                repeat,
                ..
            } => InputEvent::Key(key_event(
                *which,
                order.0,
                *scancode,
                *keycode,
                *keymod,
                KeyEdge::Pressed,
                *repeat,
            )),
            Event::KeyUp {
                which,
                scancode,
                keycode,
                keymod,
                repeat,
                ..
            } => InputEvent::Key(key_event(
                *which,
                order.0,
                *scancode,
                *keycode,
                *keymod,
                KeyEdge::Released,
                *repeat,
            )),
            Event::TextInput { text, .. } => InputEvent::Text(TextEvent::Commit {
                device: DeviceId(0),
                order,
                text: text.clone(),
            }),
            Event::TextEditing {
                text,
                start,
                length,
                ..
            } => InputEvent::Text(TextEvent::Preedit {
                device: DeviceId(0),
                order,
                text: text.clone(),
                selection: selection(*start, *length),
            }),
            Event::MouseMotion {
                which,
                x,
                y,
                xrel,
                yrel,
                ..
            } => {
                let device = DeviceId(*which as u64);
                let position = Point {
                    x: *x as f64,
                    y: *y as f64,
                };
                self.pointer_positions.insert(device, position);
                InputEvent::Pointer(PointerEvent::Motion {
                    device,
                    order,
                    position,
                    delta: Some(Vector {
                        x: *xrel as f64,
                        y: *yrel as f64,
                    }),
                })
            }
            Event::MouseButtonDown {
                which,
                mouse_btn,
                x,
                y,
                ..
            } => InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(*which as u64),
                order,
                button: pointer_button(*mouse_btn),
                edge: KeyEdge::Pressed,
                position: Some(Point {
                    x: *x as f64,
                    y: *y as f64,
                }),
            }),
            Event::MouseButtonUp {
                which,
                mouse_btn,
                x,
                y,
                ..
            } => InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(*which as u64),
                order,
                button: pointer_button(*mouse_btn),
                edge: KeyEdge::Released,
                position: Some(Point {
                    x: *x as f64,
                    y: *y as f64,
                }),
            }),
            Event::MouseWheel {
                which,
                x,
                y,
                integer_x,
                integer_y,
                direction,
                mouse_x,
                mouse_y,
                ..
            } => {
                let sign = if *direction == MouseWheelDirection::Flipped {
                    -1.0
                } else {
                    1.0
                };
                InputEvent::Pointer(PointerEvent::Axis {
                    device: DeviceId(*which as u64),
                    order,
                    delta: Vector {
                        x: *x as f64 * sign,
                        y: *y as f64 * sign,
                    },
                    discrete: Some((
                        (*integer_x as f64 * sign) as i32,
                        (*integer_y as f64 * sign) as i32,
                    )),
                    position: Some(Point {
                        x: *mouse_x as f64,
                        y: *mouse_y as f64,
                    }),
                })
            }
            Event::Window {
                win_event: WindowEvent::FocusGained,
                ..
            } => InputEvent::FocusGained { order },
            Event::Window {
                win_event: WindowEvent::FocusLost,
                ..
            } => {
                self.pointer_positions.clear();
                InputEvent::FocusLost { order }
            }
            Event::Window {
                win_event: WindowEvent::MouseEnter,
                ..
            } => InputEvent::Pointer(PointerEvent::Enter {
                device: DeviceId(0),
                order,
                position: self
                    .pointer_positions
                    .get(&DeviceId(0))
                    .copied()
                    .unwrap_or(Point { x: 0.0, y: 0.0 }),
            }),
            Event::Window {
                win_event: WindowEvent::MouseLeave,
                ..
            } => InputEvent::Pointer(PointerEvent::Leave {
                device: DeviceId(0),
                order,
            }),
            Event::FingerDown {
                touch_id,
                finger_id,
                x,
                y,
                ..
            } => InputEvent::Touch(TouchEvent::Started {
                device: DeviceId(*touch_id),
                order,
                contact: TouchId(*finger_id),
                position: Point {
                    x: *x as f64,
                    y: *y as f64,
                },
            }),
            Event::FingerMotion {
                touch_id,
                finger_id,
                x,
                y,
                ..
            } => InputEvent::Touch(TouchEvent::Moved {
                device: DeviceId(*touch_id),
                order,
                contact: TouchId(*finger_id),
                position: Point {
                    x: *x as f64,
                    y: *y as f64,
                },
            }),
            Event::FingerUp {
                touch_id,
                finger_id,
                x,
                y,
                ..
            } => InputEvent::Touch(TouchEvent::Ended {
                device: DeviceId(*touch_id),
                order,
                contact: TouchId(*finger_id),
                position: Point {
                    x: *x as f64,
                    y: *y as f64,
                },
            }),
            _ => return None,
        })
    }
}

fn selection(start: i32, length: i32) -> Option<(usize, usize)> {
    let start = usize::try_from(start).ok()?;
    let length = usize::try_from(length).ok()?;
    Some((start, start.saturating_add(length)))
}

fn pointer_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::X1 => PointerButton::Back,
        MouseButton::X2 => PointerButton::Forward,
        MouseButton::Unknown => PointerButton::Native(0),
    }
}

pub fn key_event(
    device: u32,
    order: u64,
    scancode: Option<Scancode>,
    keycode: Option<SdlKeycode>,
    keymod: Mod,
    edge: KeyEdge,
    repeat: bool,
) -> KeyEvent {
    let physical = scancode.map(physical_key).unwrap_or_else(|| {
        PhysicalKey::Native(NativeKey {
            namespace: "sdl-scancode".into(),
            code: NativeCode::Numeric(0),
        })
    });
    KeyEvent {
        device: DeviceId(device as u64),
        order: EventOrder(order),
        location: location(&physical),
        physical,
        logical: keycode.map(logical_key).unwrap_or_else(|| {
            LogicalKey::Native(NativeKey {
                namespace: "sdl-keycode".into(),
                code: NativeCode::Numeric(0),
            })
        }),
        edge,
        repeat,
        modifiers: modifier_state(keymod),
    }
}

pub fn modifier_state(mods: Mod) -> ModifierState {
    let candidates = [
        (Mod::LSHIFTMOD, Modifier::ShiftLeft),
        (Mod::RSHIFTMOD, Modifier::ShiftRight),
        (Mod::LCTRLMOD, Modifier::ControlLeft),
        (Mod::RCTRLMOD, Modifier::ControlRight),
        (Mod::LALTMOD, Modifier::AltLeft),
        (Mod::RALTMOD, Modifier::AltRight),
        (Mod::LGUIMOD, Modifier::SuperLeft),
        (Mod::RGUIMOD, Modifier::SuperRight),
    ];
    ModifierState::from_sides(
        candidates
            .into_iter()
            .filter_map(|(flag, side)| mods.intersects(flag).then_some(side)),
    )
}

pub fn physical_key(scancode: Scancode) -> PhysicalKey {
    use KeyCode as K;
    use Scancode as S;
    let portable = match scancode {
        S::A => K::KeyA,
        S::B => K::KeyB,
        S::C => K::KeyC,
        S::D => K::KeyD,
        S::E => K::KeyE,
        S::F => K::KeyF,
        S::G => K::KeyG,
        S::H => K::KeyH,
        S::I => K::KeyI,
        S::J => K::KeyJ,
        S::K => K::KeyK,
        S::L => K::KeyL,
        S::M => K::KeyM,
        S::N => K::KeyN,
        S::O => K::KeyO,
        S::P => K::KeyP,
        S::Q => K::KeyQ,
        S::R => K::KeyR,
        S::S => K::KeyS,
        S::T => K::KeyT,
        S::U => K::KeyU,
        S::V => K::KeyV,
        S::W => K::KeyW,
        S::X => K::KeyX,
        S::Y => K::KeyY,
        S::Z => K::KeyZ,
        S::_0 => K::Digit0,
        S::_1 => K::Digit1,
        S::_2 => K::Digit2,
        S::_3 => K::Digit3,
        S::_4 => K::Digit4,
        S::_5 => K::Digit5,
        S::_6 => K::Digit6,
        S::_7 => K::Digit7,
        S::_8 => K::Digit8,
        S::_9 => K::Digit9,
        S::Escape => K::Escape,
        S::Tab => K::Tab,
        S::CapsLock => K::CapsLock,
        S::Return => K::Enter,
        S::Space => K::Space,
        S::Backspace => K::Backspace,
        S::Delete => K::Delete,
        S::Insert => K::Insert,
        S::Home => K::Home,
        S::End => K::End,
        S::PageUp => K::PageUp,
        S::PageDown => K::PageDown,
        S::Left => K::ArrowLeft,
        S::Right => K::ArrowRight,
        S::Up => K::ArrowUp,
        S::Down => K::ArrowDown,
        S::Grave => K::Backquote,
        S::Minus => K::Minus,
        S::Equals => K::Equal,
        S::LeftBracket => K::BracketLeft,
        S::RightBracket => K::BracketRight,
        S::Backslash => K::Backslash,
        S::Semicolon => K::Semicolon,
        S::Apostrophe => K::Quote,
        S::Comma => K::Comma,
        S::Period => K::Period,
        S::Slash => K::Slash,
        S::LShift => K::ShiftLeft,
        S::RShift => K::ShiftRight,
        S::LCtrl => K::ControlLeft,
        S::RCtrl => K::ControlRight,
        S::LAlt => K::AltLeft,
        S::RAlt => K::AltRight,
        S::LGui => K::SuperLeft,
        S::RGui => K::SuperRight,
        S::PrintScreen => K::PrintScreen,
        S::ScrollLock => K::ScrollLock,
        S::Pause => K::Pause,
        S::F1 => K::F1,
        S::F2 => K::F2,
        S::F3 => K::F3,
        S::F4 => K::F4,
        S::F5 => K::F5,
        S::F6 => K::F6,
        S::F7 => K::F7,
        S::F8 => K::F8,
        S::F9 => K::F9,
        S::F10 => K::F10,
        S::F11 => K::F11,
        S::F12 => K::F12,
        S::Kp0 => K::Numpad0,
        S::Kp1 => K::Numpad1,
        S::Kp2 => K::Numpad2,
        S::Kp3 => K::Numpad3,
        S::Kp4 => K::Numpad4,
        S::Kp5 => K::Numpad5,
        S::Kp6 => K::Numpad6,
        S::Kp7 => K::Numpad7,
        S::Kp8 => K::Numpad8,
        S::Kp9 => K::Numpad9,
        S::KpPlus => K::NumpadAdd,
        S::KpMinus => K::NumpadSubtract,
        S::KpMultiply => K::NumpadMultiply,
        S::KpDivide => K::NumpadDivide,
        S::KpPeriod => K::NumpadDecimal,
        S::KpEnter => K::NumpadEnter,
        other => {
            return PhysicalKey::Native(NativeKey {
                namespace: "sdl-scancode".into(),
                code: NativeCode::Numeric(other as i32 as u64),
            });
        }
    };
    PhysicalKey::Code(portable)
}

pub fn logical_key(keycode: SdlKeycode) -> LogicalKey {
    use NamedKey as N;
    use SdlKeycode as K;
    let named = match keycode {
        K::Escape => N::Escape,
        K::Tab => N::Tab,
        K::Return | K::KpEnter => N::Enter,
        K::Space => N::Space,
        K::Backspace => N::Backspace,
        K::Delete => N::Delete,
        K::Insert => N::Insert,
        K::Home => N::Home,
        K::End => N::End,
        K::PageUp => N::PageUp,
        K::PageDown => N::PageDown,
        K::Left => N::ArrowLeft,
        K::Right => N::ArrowRight,
        K::Up => N::ArrowUp,
        K::Down => N::ArrowDown,
        K::LShift | K::RShift => N::Shift,
        K::LCtrl | K::RCtrl => N::Control,
        K::LAlt | K::RAlt => N::Alt,
        K::LGui | K::RGui => N::Meta,
        K::CapsLock => N::CapsLock,
        K::PrintScreen => N::PrintScreen,
        K::F1 => N::F1,
        K::F2 => N::F2,
        K::F3 => N::F3,
        K::F4 => N::F4,
        K::F5 => N::F5,
        K::F6 => N::F6,
        K::F7 => N::F7,
        K::F8 => N::F8,
        K::F9 => N::F9,
        K::F10 => N::F10,
        K::F11 => N::F11,
        K::F12 => N::F12,
        K::A => return LogicalKey::Character("a".into()),
        K::B => return LogicalKey::Character("b".into()),
        K::C => return LogicalKey::Character("c".into()),
        K::D => return LogicalKey::Character("d".into()),
        K::E => return LogicalKey::Character("e".into()),
        K::F => return LogicalKey::Character("f".into()),
        K::G => return LogicalKey::Character("g".into()),
        K::H => return LogicalKey::Character("h".into()),
        K::I => return LogicalKey::Character("i".into()),
        K::J => return LogicalKey::Character("j".into()),
        K::K => return LogicalKey::Character("k".into()),
        K::L => return LogicalKey::Character("l".into()),
        K::M => return LogicalKey::Character("m".into()),
        K::N => return LogicalKey::Character("n".into()),
        K::O => return LogicalKey::Character("o".into()),
        K::P => return LogicalKey::Character("p".into()),
        K::Q => return LogicalKey::Character("q".into()),
        K::R => return LogicalKey::Character("r".into()),
        K::S => return LogicalKey::Character("s".into()),
        K::T => return LogicalKey::Character("t".into()),
        K::U => return LogicalKey::Character("u".into()),
        K::V => return LogicalKey::Character("v".into()),
        K::W => return LogicalKey::Character("w".into()),
        K::X => return LogicalKey::Character("x".into()),
        K::Y => return LogicalKey::Character("y".into()),
        K::Z => return LogicalKey::Character("z".into()),
        other => {
            return LogicalKey::Native(NativeKey {
                namespace: "sdl-keycode".into(),
                code: NativeCode::Numeric(other as u32 as u64),
            });
        }
    };
    LogicalKey::Named(named)
}

fn location(key: &PhysicalKey) -> KeyLocation {
    match key {
        PhysicalKey::Code(
            KeyCode::ShiftLeft | KeyCode::ControlLeft | KeyCode::AltLeft | KeyCode::SuperLeft,
        ) => KeyLocation::Left,
        PhysicalKey::Code(
            KeyCode::ShiftRight | KeyCode::ControlRight | KeyCode::AltRight | KeyCode::SuperRight,
        ) => KeyLocation::Right,
        PhysicalKey::Code(
            KeyCode::Numpad0
            | KeyCode::Numpad1
            | KeyCode::Numpad2
            | KeyCode::Numpad3
            | KeyCode::Numpad4
            | KeyCode::Numpad5
            | KeyCode::Numpad6
            | KeyCode::Numpad7
            | KeyCode::Numpad8
            | KeyCode::Numpad9
            | KeyCode::NumpadAdd
            | KeyCode::NumpadSubtract
            | KeyCode::NumpadMultiply
            | KeyCode::NumpadDivide
            | KeyCode::NumpadDecimal
            | KeyCode::NumpadEnter,
        ) => KeyLocation::Numpad,
        PhysicalKey::Code(_) => KeyLocation::Standard,
        PhysicalKey::Native(_) => KeyLocation::Unknown,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AggregateModifier;

    #[test]
    fn preserves_sides_physical_and_logical_identity() {
        let event = key_event(
            4,
            7,
            Some(Scancode::Q),
            Some(SdlKeycode::A),
            Mod::LSHIFTMOD,
            KeyEdge::Pressed,
            false,
        );
        assert_eq!(event.physical, PhysicalKey::Code(KeyCode::KeyQ));
        assert_eq!(event.logical, LogicalKey::Character("a".into()));
        assert!(event.modifiers.contains(Modifier::ShiftLeft));
        assert!(event.modifiers.aggregate(AggregateModifier::Shift));
    }

    #[test]
    fn unknown_values_are_not_relabelled() {
        assert!(matches!(
            physical_key(Scancode::Unknown),
            PhysicalKey::Native(_)
        ));
        assert!(matches!(
            logical_key(SdlKeycode::Unknown),
            LogicalKey::Native(_)
        ));
    }

    #[test]
    fn focus_loss_clears_sdl_pointer_history() {
        let mut adapter = Adapter::default();
        assert!(matches!(
            adapter.normalize(&Event::MouseMotion {
                timestamp: 0,
                window_id: 1,
                which: 4,
                mousestate: sdl3::mouse::MouseState::from_sdl_state(0),
                x: 10.0,
                y: 20.0,
                xrel: 1.0,
                yrel: 2.0,
            }),
            Some(InputEvent::Pointer(PointerEvent::Motion { .. }))
        ));
        assert!(adapter.pointer_positions.contains_key(&DeviceId(4)));
        assert_eq!(
            adapter.normalize(&Event::Window {
                timestamp: 0,
                window_id: 1,
                win_event: WindowEvent::FocusLost,
            }),
            Some(InputEvent::FocusLost {
                order: EventOrder(2)
            })
        );
        assert!(adapter.pointer_positions.is_empty());
    }
}
