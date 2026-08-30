//! winit conversions. Application policy does not belong here.

use std::collections::BTreeMap;

use winit::{
    event::{
        ElementState, Ime, KeyEvent as WinitKeyEvent, Modifiers, MouseButton, MouseScrollDelta,
        TouchPhase, WindowEvent,
    },
    keyboard::{
        Key as WinitKey, KeyCode as WinitKeyCode, KeyLocation as WinitLocation, ModifiersKeyState,
        NamedKey as WinitNamedKey, NativeKey as WinitNativeKey, NativeKeyCode,
        PhysicalKey as WinitPhysicalKey,
    },
};

use crate::{
    AggregateModifier, DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation,
    LogicalKey, Modifier, ModifierState, NamedKey, NativeCode, NativeKey, PhysicalKey, Point,
    PointerButton, PointerEvent, TextEvent, TouchEvent, TouchId, Vector,
};

/// Stateful conversion for winit window events.
///
/// Callers assign stable portable device IDs at their native boundary because
/// winit intentionally keeps its platform device representation opaque.
#[derive(Clone, Debug)]
pub struct Adapter {
    next_order: u64,
    modifiers: ModifierState,
    pointer_positions: BTreeMap<DeviceId, Point>,
    scale_factor: f64,
}

impl Default for Adapter {
    fn default() -> Self {
        Self {
            next_order: 0,
            modifiers: ModifierState::default(),
            pointer_positions: BTreeMap::new(),
            scale_factor: 1.0,
        }
    }
}

impl Adapter {
    pub fn set_scale_factor(&mut self, scale_factor: f64) {
        self.scale_factor = valid_scale(scale_factor);
    }

    pub fn normalize(&mut self, device: DeviceId, event: &WindowEvent) -> Vec<InputEvent> {
        self.next_order += 1;
        let order = EventOrder(self.next_order);
        match event {
            WindowEvent::KeyboardInput { event, .. } => {
                let mut key = key_event(device, order, event, self.modifiers.clone());
                if let Some(modifier) = modifier_for_key(&key.physical) {
                    self.modifiers.set(modifier, key.edge == KeyEdge::Pressed);
                    key.modifiers = self.modifiers.clone();
                }
                let mut normalized = vec![InputEvent::Key(key)];
                if event.state == ElementState::Pressed
                    && !event.repeat
                    && let Some(text) = &event.text
                    && !text.is_empty()
                {
                    normalized.push(InputEvent::Text(TextEvent::Commit {
                        device,
                        order,
                        text: text.to_string(),
                    }));
                }
                normalized
            }
            WindowEvent::ModifiersChanged(modifiers) => {
                self.modifiers = modifier_state(modifiers);
                Vec::new()
            }
            WindowEvent::Ime(Ime::Preedit(text, selection)) => {
                vec![InputEvent::Text(TextEvent::Preedit {
                    device,
                    order,
                    text: text.clone(),
                    selection: *selection,
                })]
            }
            WindowEvent::Ime(Ime::Commit(text)) => vec![InputEvent::Text(TextEvent::Commit {
                device,
                order,
                text: text.clone(),
            })],
            WindowEvent::CursorMoved { position, .. } => {
                let position = self.logical_point(position.x, position.y);
                let delta = self
                    .pointer_positions
                    .insert(device, position)
                    .map(|previous| Vector {
                        x: position.x - previous.x,
                        y: position.y - previous.y,
                    });
                vec![InputEvent::Pointer(PointerEvent::Motion {
                    device,
                    order,
                    position,
                    delta,
                })]
            }
            WindowEvent::CursorEntered { .. } => vec![InputEvent::Pointer(PointerEvent::Enter {
                device,
                order,
                position: self
                    .pointer_positions
                    .get(&device)
                    .copied()
                    .unwrap_or(Point { x: 0.0, y: 0.0 }),
            })],
            WindowEvent::CursorLeft { .. } => {
                vec![InputEvent::Pointer(PointerEvent::Leave { device, order })]
            }
            WindowEvent::MouseInput { state, button, .. } => {
                vec![pointer_button_event(
                    device,
                    order,
                    *button,
                    *state,
                    self.pointer_positions.get(&device).copied(),
                )]
            }
            WindowEvent::MouseWheel { delta, .. } => {
                vec![wheel_event(
                    device,
                    order,
                    *delta,
                    self.scale_factor,
                    self.pointer_positions.get(&device).copied(),
                )]
            }
            WindowEvent::Touch(touch) => {
                let position = self.logical_point(touch.location.x, touch.location.y);
                let contact = TouchId(touch.id);
                let event = match touch.phase {
                    TouchPhase::Started => TouchEvent::Started {
                        device,
                        order,
                        contact,
                        position,
                    },
                    TouchPhase::Moved => TouchEvent::Moved {
                        device,
                        order,
                        contact,
                        position,
                    },
                    TouchPhase::Ended => TouchEvent::Ended {
                        device,
                        order,
                        contact,
                        position,
                    },
                    TouchPhase::Cancelled => TouchEvent::Cancelled {
                        device,
                        order,
                        contact,
                    },
                };
                vec![InputEvent::Touch(event)]
            }
            WindowEvent::Focused(true) => vec![InputEvent::FocusGained { order }],
            WindowEvent::Focused(false) | WindowEvent::Destroyed => {
                self.modifiers = ModifierState::default();
                self.pointer_positions.clear();
                vec![InputEvent::FocusLost { order }]
            }
            _ => Vec::new(),
        }
    }

    pub fn device_removed(&mut self, device: DeviceId) -> InputEvent {
        self.next_order += 1;
        self.pointer_positions.remove(&device);
        self.modifiers = ModifierState::default();
        InputEvent::DeviceRemoved {
            device,
            order: EventOrder(self.next_order),
        }
    }

    fn logical_point(&self, x: f64, y: f64) -> Point {
        Point {
            x: x / self.scale_factor,
            y: y / self.scale_factor,
        }
    }
}

fn valid_scale(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

pub fn key_event(
    device: DeviceId,
    order: EventOrder,
    event: &WinitKeyEvent,
    modifiers: ModifierState,
) -> KeyEvent {
    KeyEvent {
        device,
        order,
        physical: physical_key(event.physical_key),
        logical: logical_key(&event.logical_key),
        location: match event.location {
            WinitLocation::Standard => KeyLocation::Standard,
            WinitLocation::Left => KeyLocation::Left,
            WinitLocation::Right => KeyLocation::Right,
            WinitLocation::Numpad => KeyLocation::Numpad,
        },
        edge: match event.state {
            ElementState::Pressed => KeyEdge::Pressed,
            ElementState::Released => KeyEdge::Released,
        },
        repeat: event.repeat,
        modifiers,
    }
}

fn element_edge(state: ElementState) -> KeyEdge {
    match state {
        ElementState::Pressed => KeyEdge::Pressed,
        ElementState::Released => KeyEdge::Released,
    }
}

fn pointer_button(button: MouseButton) -> PointerButton {
    match button {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::Back => PointerButton::Back,
        MouseButton::Forward => PointerButton::Forward,
        MouseButton::Other(value) => PointerButton::Native(value),
    }
}

fn pointer_button_event(
    device: DeviceId,
    order: EventOrder,
    button: MouseButton,
    state: ElementState,
    position: Option<Point>,
) -> InputEvent {
    InputEvent::Pointer(PointerEvent::Button {
        device,
        order,
        button: pointer_button(button),
        edge: element_edge(state),
        position,
    })
}

fn wheel_event(
    device: DeviceId,
    order: EventOrder,
    delta: MouseScrollDelta,
    scale_factor: f64,
    position: Option<Point>,
) -> InputEvent {
    let (delta, discrete) = match delta {
        MouseScrollDelta::LineDelta(x, y) => (
            Vector {
                x: f64::from(x),
                y: f64::from(y),
            },
            Some((x as i32, y as i32)),
        ),
        MouseScrollDelta::PixelDelta(delta) => {
            let scale = valid_scale(scale_factor);
            (
                Vector {
                    x: delta.x / scale,
                    y: delta.y / scale,
                },
                None,
            )
        }
    };
    InputEvent::Pointer(PointerEvent::Axis {
        device,
        order,
        delta,
        discrete,
        position,
    })
}

fn modifier_for_key(key: &PhysicalKey) -> Option<Modifier> {
    let PhysicalKey::Code(key) = key else {
        return None;
    };
    Some(match key {
        KeyCode::ShiftLeft => Modifier::ShiftLeft,
        KeyCode::ShiftRight => Modifier::ShiftRight,
        KeyCode::ControlLeft => Modifier::ControlLeft,
        KeyCode::ControlRight => Modifier::ControlRight,
        KeyCode::AltLeft => Modifier::AltLeft,
        KeyCode::AltRight => Modifier::AltRight,
        KeyCode::SuperLeft => Modifier::SuperLeft,
        KeyCode::SuperRight => Modifier::SuperRight,
        _ => return None,
    })
}

fn modifier_state(modifiers: &Modifiers) -> ModifierState {
    let candidates = [
        (modifiers.lshift_state(), Modifier::ShiftLeft),
        (modifiers.rshift_state(), Modifier::ShiftRight),
        (modifiers.lcontrol_state(), Modifier::ControlLeft),
        (modifiers.rcontrol_state(), Modifier::ControlRight),
        (modifiers.lalt_state(), Modifier::AltLeft),
        (modifiers.ralt_state(), Modifier::AltRight),
        (modifiers.lsuper_state(), Modifier::SuperLeft),
        (modifiers.rsuper_state(), Modifier::SuperRight),
    ];
    let sides = candidates
        .into_iter()
        .filter_map(|(state, modifier)| (state == ModifiersKeyState::Pressed).then_some(modifier))
        .collect::<Vec<_>>();
    let native = modifiers.state();
    let unsided = [
        (native.shift_key(), AggregateModifier::Shift),
        (native.control_key(), AggregateModifier::Control),
        (native.alt_key(), AggregateModifier::Alt),
        (native.super_key(), AggregateModifier::Super),
    ]
    .into_iter()
    .filter_map(|(pressed, aggregate)| {
        (pressed && !sides.iter().any(|side| side.aggregate() == aggregate)).then_some(aggregate)
    })
    .collect::<Vec<_>>();
    ModifierState::from_sides_and_unsided(sides, unsided)
}

pub fn physical_key(key: WinitPhysicalKey) -> PhysicalKey {
    let key = match key {
        WinitPhysicalKey::Code(key) => key,
        WinitPhysicalKey::Unidentified(native) => return native_physical(native),
    };
    let portable = match key {
        WinitKeyCode::KeyA => KeyCode::KeyA,
        WinitKeyCode::KeyB => KeyCode::KeyB,
        WinitKeyCode::KeyC => KeyCode::KeyC,
        WinitKeyCode::KeyD => KeyCode::KeyD,
        WinitKeyCode::KeyE => KeyCode::KeyE,
        WinitKeyCode::KeyF => KeyCode::KeyF,
        WinitKeyCode::KeyG => KeyCode::KeyG,
        WinitKeyCode::KeyH => KeyCode::KeyH,
        WinitKeyCode::KeyI => KeyCode::KeyI,
        WinitKeyCode::KeyJ => KeyCode::KeyJ,
        WinitKeyCode::KeyK => KeyCode::KeyK,
        WinitKeyCode::KeyL => KeyCode::KeyL,
        WinitKeyCode::KeyM => KeyCode::KeyM,
        WinitKeyCode::KeyN => KeyCode::KeyN,
        WinitKeyCode::KeyO => KeyCode::KeyO,
        WinitKeyCode::KeyP => KeyCode::KeyP,
        WinitKeyCode::KeyQ => KeyCode::KeyQ,
        WinitKeyCode::KeyR => KeyCode::KeyR,
        WinitKeyCode::KeyS => KeyCode::KeyS,
        WinitKeyCode::KeyT => KeyCode::KeyT,
        WinitKeyCode::KeyU => KeyCode::KeyU,
        WinitKeyCode::KeyV => KeyCode::KeyV,
        WinitKeyCode::KeyW => KeyCode::KeyW,
        WinitKeyCode::KeyX => KeyCode::KeyX,
        WinitKeyCode::KeyY => KeyCode::KeyY,
        WinitKeyCode::KeyZ => KeyCode::KeyZ,
        WinitKeyCode::Escape => KeyCode::Escape,
        WinitKeyCode::Tab => KeyCode::Tab,
        WinitKeyCode::Enter => KeyCode::Enter,
        WinitKeyCode::Space => KeyCode::Space,
        WinitKeyCode::Backspace => KeyCode::Backspace,
        WinitKeyCode::Delete => KeyCode::Delete,
        WinitKeyCode::Home => KeyCode::Home,
        WinitKeyCode::End => KeyCode::End,
        WinitKeyCode::ArrowLeft => KeyCode::ArrowLeft,
        WinitKeyCode::ArrowRight => KeyCode::ArrowRight,
        WinitKeyCode::ArrowUp => KeyCode::ArrowUp,
        WinitKeyCode::ArrowDown => KeyCode::ArrowDown,
        WinitKeyCode::ShiftLeft => KeyCode::ShiftLeft,
        WinitKeyCode::ShiftRight => KeyCode::ShiftRight,
        WinitKeyCode::ControlLeft => KeyCode::ControlLeft,
        WinitKeyCode::ControlRight => KeyCode::ControlRight,
        WinitKeyCode::AltLeft => KeyCode::AltLeft,
        WinitKeyCode::AltRight => KeyCode::AltRight,
        WinitKeyCode::SuperLeft => KeyCode::SuperLeft,
        WinitKeyCode::SuperRight => KeyCode::SuperRight,
        WinitKeyCode::PrintScreen => KeyCode::PrintScreen,
        other => {
            return PhysicalKey::Native(NativeKey {
                namespace: "winit-keycode".into(),
                code: NativeCode::Text(format!("{other:?}")),
            });
        }
    };
    PhysicalKey::Code(portable)
}

pub fn logical_key(key: &WinitKey) -> LogicalKey {
    match key {
        WinitKey::Character(value) => LogicalKey::Character(value.to_string()),
        WinitKey::Dead(value) => LogicalKey::Dead(*value),
        WinitKey::Unidentified(native) => native_logical(native),
        WinitKey::Named(named) => named_key(*named).map(LogicalKey::Named).unwrap_or_else(|| {
            LogicalKey::Native(NativeKey {
                namespace: "winit-named-key".into(),
                code: NativeCode::Text(format!("{named:?}")),
            })
        }),
    }
}

fn named_key(key: WinitNamedKey) -> Option<NamedKey> {
    Some(match key {
        WinitNamedKey::Alt => NamedKey::Alt,
        WinitNamedKey::AltGraph => NamedKey::AltGraph,
        WinitNamedKey::Backspace => NamedKey::Backspace,
        WinitNamedKey::CapsLock => NamedKey::CapsLock,
        WinitNamedKey::Control => NamedKey::Control,
        WinitNamedKey::Delete => NamedKey::Delete,
        WinitNamedKey::End => NamedKey::End,
        WinitNamedKey::Enter => NamedKey::Enter,
        WinitNamedKey::Escape => NamedKey::Escape,
        WinitNamedKey::Home => NamedKey::Home,
        WinitNamedKey::Insert => NamedKey::Insert,
        WinitNamedKey::Meta => NamedKey::Meta,
        WinitNamedKey::PageDown => NamedKey::PageDown,
        WinitNamedKey::PageUp => NamedKey::PageUp,
        WinitNamedKey::Shift => NamedKey::Shift,
        WinitNamedKey::Space => NamedKey::Space,
        WinitNamedKey::Tab => NamedKey::Tab,
        WinitNamedKey::ArrowDown => NamedKey::ArrowDown,
        WinitNamedKey::ArrowLeft => NamedKey::ArrowLeft,
        WinitNamedKey::ArrowRight => NamedKey::ArrowRight,
        WinitNamedKey::ArrowUp => NamedKey::ArrowUp,
        WinitNamedKey::PrintScreen => NamedKey::PrintScreen,
        _ => return None,
    })
}

fn native_physical(key: NativeKeyCode) -> PhysicalKey {
    let (namespace, code) = match key {
        NativeKeyCode::Unidentified => ("winit-unidentified", 0),
        NativeKeyCode::Android(code) => ("android-scancode", code as u64),
        NativeKeyCode::MacOS(code) => ("macos-scancode", code as u64),
        NativeKeyCode::Windows(code) => ("windows-scancode", code as u64),
        NativeKeyCode::Xkb(code) => ("xkb-keycode", code as u64),
    };
    PhysicalKey::Native(NativeKey {
        namespace: namespace.into(),
        code: NativeCode::Numeric(code),
    })
}

fn native_logical(key: &WinitNativeKey) -> LogicalKey {
    let (namespace, code) = match key {
        WinitNativeKey::Unidentified => ("winit-unidentified", NativeCode::Numeric(0)),
        WinitNativeKey::Android(code) => ("android-keycode", NativeCode::Numeric(*code as u64)),
        WinitNativeKey::MacOS(code) => ("macos-scancode", NativeCode::Numeric(*code as u64)),
        WinitNativeKey::Windows(code) => ("windows-virtual-key", NativeCode::Numeric(*code as u64)),
        WinitNativeKey::Xkb(code) => ("xkb-keysym", NativeCode::Numeric(*code as u64)),
        WinitNativeKey::Web(code) => ("web-key", NativeCode::Text(code.to_string())),
    };
    LogicalKey::Native(NativeKey {
        namespace: namespace.into(),
        code,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::ModifiersState as WinitModifiersState;

    #[test]
    fn preserves_layout_independent_identity() {
        assert_eq!(
            physical_key(WinitPhysicalKey::Code(WinitKeyCode::KeyQ)),
            PhysicalKey::Code(KeyCode::KeyQ)
        );
        assert_eq!(
            logical_key(&WinitKey::Character("a".into())),
            LogicalKey::Character("a".into())
        );
    }

    #[test]
    fn preserves_unknown_native_code() {
        assert_eq!(
            native_physical(NativeKeyCode::Xkb(777)),
            PhysicalKey::Native(NativeKey {
                namespace: "xkb-keycode".into(),
                code: NativeCode::Numeric(777),
            })
        );
    }

    #[test]
    fn aggregate_modifier_without_side_is_preserved_without_inventing_one() {
        let modifiers = Modifiers::from(WinitModifiersState::CONTROL | WinitModifiersState::ALT);
        let normalized = modifier_state(&modifiers);
        assert!(normalized.aggregate(AggregateModifier::Control));
        assert!(normalized.aggregate(AggregateModifier::Alt));
        assert_eq!(normalized.sides().count(), 0);
        assert_eq!(
            normalized.unsided().collect::<Vec<_>>(),
            [AggregateModifier::Control, AggregateModifier::Alt]
        );
    }

    #[test]
    fn focus_and_device_edges_reset_only_when_the_contract_requires_it() {
        use winit::dpi::PhysicalPosition;

        let mut adapter = Adapter::default();
        let device = DeviceId(9);
        adapter.normalize(
            device,
            &WindowEvent::ModifiersChanged(Modifiers::from(WinitModifiersState::SHIFT)),
        );
        adapter.normalize(
            device,
            &WindowEvent::CursorMoved {
                device_id: winit::event::DeviceId::dummy(),
                position: PhysicalPosition::new(20.0, 30.0),
            },
        );
        adapter.normalize(device, &WindowEvent::Focused(true));
        assert!(adapter.modifiers.aggregate(AggregateModifier::Shift));
        assert!(adapter.pointer_positions.contains_key(&device));

        adapter.normalize(device, &WindowEvent::Focused(false));
        assert!(adapter.modifiers.is_empty());
        assert!(adapter.pointer_positions.is_empty());

        adapter.normalize(
            device,
            &WindowEvent::ModifiersChanged(Modifiers::from(WinitModifiersState::CONTROL)),
        );
        adapter.normalize(
            device,
            &WindowEvent::CursorMoved {
                device_id: winit::event::DeviceId::dummy(),
                position: PhysicalPosition::new(40.0, 50.0),
            },
        );
        assert!(matches!(
            adapter.device_removed(device),
            InputEvent::DeviceRemoved { device: removed, .. } if removed == device
        ));
        assert!(adapter.modifiers.is_empty());
        assert!(!adapter.pointer_positions.contains_key(&device));
    }

    #[cfg(feature = "sdl")]
    #[test]
    fn sdl_and_winit_text_ime_and_focus_fixtures_are_equivalent() {
        use sdl3::event::{Event as SdlEvent, WindowEvent as SdlWindowEvent};

        let mut sdl = crate::sdl::Adapter::default();
        let sdl_events = [
            SdlEvent::Window {
                timestamp: 0,
                window_id: 1,
                win_event: SdlWindowEvent::FocusGained,
            },
            SdlEvent::TextEditing {
                timestamp: 0,
                window_id: 1,
                text: "pre".into(),
                start: 1,
                length: 1,
            },
            SdlEvent::TextInput {
                timestamp: 0,
                window_id: 1,
                text: "commit".into(),
            },
            SdlEvent::Window {
                timestamp: 0,
                window_id: 1,
                win_event: SdlWindowEvent::FocusLost,
            },
        ]
        .iter()
        .filter_map(|event| sdl.normalize(event))
        .collect::<Vec<_>>();

        let mut winit = Adapter::default();
        let winit_events = [
            WindowEvent::Focused(true),
            WindowEvent::Ime(Ime::Preedit("pre".into(), Some((1, 2)))),
            WindowEvent::Ime(Ime::Commit("commit".into())),
            WindowEvent::Focused(false),
        ]
        .iter()
        .flat_map(|event| winit.normalize(DeviceId(0), event))
        .collect::<Vec<_>>();
        assert_eq!(sdl_events, winit_events);
    }

    #[cfg(feature = "sdl")]
    #[test]
    fn sdl_and_winit_pointer_and_wheel_fixtures_are_equivalent() {
        use sdl3::{
            event::Event as SdlEvent,
            mouse::{MouseButton as SdlMouseButton, MouseWheelDirection},
        };

        let mut sdl = crate::sdl::Adapter::default();
        let button = sdl
            .normalize(&SdlEvent::MouseButtonDown {
                timestamp: 0,
                window_id: 1,
                which: 4,
                mouse_btn: SdlMouseButton::Left,
                clicks: 1,
                x: 11.0,
                y: 13.0,
            })
            .unwrap();
        assert_eq!(
            button,
            pointer_button_event(
                DeviceId(4),
                EventOrder(1),
                MouseButton::Left,
                ElementState::Pressed,
                Some(Point { x: 11.0, y: 13.0 }),
            )
        );

        let wheel = sdl
            .normalize(&SdlEvent::MouseWheel {
                timestamp: 0,
                window_id: 1,
                which: 4,
                x: 2.0,
                y: -3.0,
                direction: MouseWheelDirection::Normal,
                mouse_x: 11.0,
                mouse_y: 13.0,
                integer_x: 2,
                integer_y: -3,
            })
            .unwrap();
        assert_eq!(
            wheel,
            wheel_event(
                DeviceId(4),
                EventOrder(2),
                MouseScrollDelta::LineDelta(2.0, -3.0),
                1.0,
                Some(Point { x: 11.0, y: 13.0 }),
            )
        );
    }

    #[cfg(feature = "sdl")]
    #[test]
    fn sdl_and_winit_touch_fixtures_are_equivalent_at_logical_scale() {
        use sdl3::event::Event as SdlEvent;
        use winit::{dpi::PhysicalPosition, event::Touch};

        let sdl_trace = [
            SdlEvent::FingerDown {
                timestamp: 0,
                touch_id: 7,
                finger_id: 11,
                x: 0.25,
                y: 0.75,
                dx: 0.0,
                dy: 0.0,
                pressure: 1.0,
                window_id: 1,
            },
            SdlEvent::FingerMotion {
                timestamp: 0,
                touch_id: 7,
                finger_id: 11,
                x: 0.5,
                y: 1.0,
                dx: 0.25,
                dy: 0.25,
                pressure: 1.0,
                window_id: 1,
            },
            SdlEvent::FingerUp {
                timestamp: 0,
                touch_id: 7,
                finger_id: 11,
                x: 0.5,
                y: 1.0,
                dx: 0.0,
                dy: 0.0,
                pressure: 0.0,
                window_id: 1,
            },
        ];
        let mut sdl = crate::sdl::Adapter::default();
        let sdl_events = sdl_trace
            .iter()
            .filter_map(|event| sdl.normalize(event))
            .collect::<Vec<_>>();

        // Winit reports physical pixels, while SDL touch positions are already logical.
        let mut winit = Adapter::default();
        winit.set_scale_factor(2.0);
        let native_device = winit::event::DeviceId::dummy();
        let winit_trace = [
            (TouchPhase::Started, PhysicalPosition::new(0.5, 1.5)),
            (TouchPhase::Moved, PhysicalPosition::new(1.0, 2.0)),
            (TouchPhase::Ended, PhysicalPosition::new(1.0, 2.0)),
        ];
        let winit_events = winit_trace
            .into_iter()
            .flat_map(|(phase, location)| {
                winit.normalize(
                    DeviceId(7),
                    &WindowEvent::Touch(Touch {
                        device_id: native_device,
                        phase,
                        location,
                        force: None,
                        id: 11,
                    }),
                )
            })
            .collect::<Vec<_>>();

        assert_eq!(sdl_events, winit_events);
    }

    #[cfg(feature = "sdl")]
    #[test]
    fn sdl_and_winit_key_fixtures_keep_physical_logical_repeat_and_modifiers() {
        use sdl3::keyboard::{Keycode as SdlKeycode, Mod, Scancode};

        let sdl = crate::sdl::key_event(
            5,
            7,
            Some(Scancode::Q),
            Some(SdlKeycode::A),
            Mod::LSHIFTMOD,
            KeyEdge::Pressed,
            true,
        );
        assert_eq!(
            sdl.physical,
            physical_key(WinitPhysicalKey::Code(WinitKeyCode::KeyQ))
        );
        assert_eq!(sdl.logical, logical_key(&WinitKey::Character("a".into())));
        assert_eq!(sdl.location, KeyLocation::Standard);
        assert_eq!(sdl.edge, KeyEdge::Pressed);
        assert!(sdl.repeat);
        assert!(sdl.modifiers.contains(Modifier::ShiftLeft));
    }
}
