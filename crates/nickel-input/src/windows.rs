//! Pure translation and recognition state for the Windows input boundary.
//!
//! The Win32 hook and message loop remain in the platform crate; this module
//! deliberately accepts plain values so translation and shortcut behavior can
//! be tested on every development host.

use crate::{
    Binding, DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation, LogicalKey,
    Modifier, ModifierState, NativeCode, NativeKey, PhysicalKey, PointerButton, ShortcutEngine,
    ShortcutOutcome,
};

pub const WINDOWS_KEYBOARD_DEVICE: DeviceId = DeviceId(0x5749_4e4b_4244);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct NativeKeyboardEvent {
    pub virtual_key: u32,
    pub scan_code: u32,
    pub extended: bool,
    pub edge: KeyEdge,
    pub injected: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum InjectedEventPolicy {
    Ignore,
    Accept,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WindowsDispatch<A> {
    pub normalized: KeyEvent,
    pub outcomes: Vec<ShortcutOutcome<A>>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SuperPointerGesture {
    Move,
    Resize,
}

#[derive(Clone, Debug)]
pub struct WindowsInputAdapter<A> {
    engine: ShortcutEngine<A>,
    next_order: u64,
    injected_policy: InjectedEventPolicy,
    alt_graph_active: bool,
}

impl<A: Clone> WindowsInputAdapter<A> {
    pub fn new(bindings: impl IntoIterator<Item = Binding<A>>) -> Self {
        Self {
            engine: ShortcutEngine::new(bindings),
            next_order: 0,
            injected_policy: InjectedEventPolicy::Ignore,
            alt_graph_active: false,
        }
    }

    pub fn with_injected_policy(mut self, policy: InjectedEventPolicy) -> Self {
        self.injected_policy = policy;
        self
    }

    pub fn handle_native(&mut self, event: NativeKeyboardEvent) -> Option<WindowsDispatch<A>> {
        if event.injected && self.injected_policy == InjectedEventPolicy::Ignore {
            return None;
        }
        // Win32 reports AltGr as an artificial left-Control followed by right
        // Alt. Remove that Control state before shortcut recognition.
        if event.virtual_key == 0xa5 && event.edge == KeyEdge::Pressed {
            if self.engine.modifiers().contains(Modifier::ControlLeft) {
                self.engine.reconcile_modifier(
                    WINDOWS_KEYBOARD_DEVICE,
                    Modifier::ControlLeft,
                    false,
                );
                self.alt_graph_active = true;
            }
        } else if event.virtual_key == 0xa5 && event.edge == KeyEdge::Released {
            self.alt_graph_active = false;
        }
        let normalized = self.normalize(event);
        let outcomes = self.engine.handle(&InputEvent::Key(normalized.clone()));
        Some(WindowsDispatch {
            normalized,
            outcomes,
        })
    }

    pub fn handle_key_code(&mut self, key: KeyCode, edge: KeyEdge) -> WindowsDispatch<A> {
        self.next_order += 1;
        let normalized = key_event(key, edge, self.next_order, self.engine.modifiers());
        let outcomes = self.engine.handle(&InputEvent::Key(normalized.clone()));
        WindowsDispatch {
            normalized,
            outcomes,
        }
    }

    pub fn observe_key_code(&mut self, key: KeyCode, edge: KeyEdge) {
        let _ = self.handle_key_code(key, edge);
    }

    pub fn begin_pointer_chord(&mut self) -> bool {
        if !self
            .engine
            .modifiers()
            .aggregate(crate::AggregateModifier::Super)
        {
            return false;
        }
        self.engine.chord_held_modifiers();
        true
    }

    pub fn begin_pointer_gesture(&mut self, button: PointerButton) -> Option<SuperPointerGesture> {
        let gesture = match button {
            PointerButton::Primary => SuperPointerGesture::Move,
            PointerButton::Secondary => SuperPointerGesture::Resize,
            _ => return None,
        };
        if !self.begin_pointer_chord() {
            return None;
        }
        Some(gesture)
    }

    pub fn modifier_held(&self, modifier: crate::AggregateModifier) -> bool {
        self.engine.modifiers().aggregate(modifier)
    }

    pub fn key_held(&self, key: KeyCode) -> bool {
        self.engine
            .pressed_keys(WINDOWS_KEYBOARD_DEVICE)
            .any(|pressed| pressed == &PhysicalKey::Code(key))
    }

    pub fn reset(&mut self) {
        self.engine.reset();
        self.alt_graph_active = false;
    }

    fn normalize(&mut self, event: NativeKeyboardEvent) -> KeyEvent {
        self.next_order += 1;
        let physical = physical_key(event.virtual_key, event.scan_code, event.extended);
        let location = key_location(&physical);
        let logical = if self.alt_graph_active && event.virtual_key == 0xa5 {
            LogicalKey::Named(crate::NamedKey::AltGraph)
        } else {
            logical_key(event.virtual_key)
        };
        let repeat = event.edge == KeyEdge::Pressed
            && self
                .engine
                .pressed_keys(WINDOWS_KEYBOARD_DEVICE)
                .any(|pressed| pressed == &physical);
        KeyEvent {
            device: WINDOWS_KEYBOARD_DEVICE,
            order: EventOrder(self.next_order),
            physical,
            logical,
            location,
            edge: event.edge,
            repeat,
            modifiers: self.engine.modifiers(),
        }
    }
}

fn key_event(key: KeyCode, edge: KeyEdge, order: u64, modifiers: ModifierState) -> KeyEvent {
    KeyEvent {
        device: WINDOWS_KEYBOARD_DEVICE,
        order: EventOrder(order),
        physical: PhysicalKey::Code(key),
        logical: logical_for_code(key),
        location: key_location(&PhysicalKey::Code(key)),
        edge,
        repeat: false,
        modifiers,
    }
}

pub fn physical_key(virtual_key: u32, scan_code: u32, extended: bool) -> PhysicalKey {
    // The OEM grave key is identified physically because its translated VK can
    // become VK_HANJA under Alt on some layouts.
    if scan_code == 0x29 {
        return PhysicalKey::Code(KeyCode::Backquote);
    }
    if scan_code == 0x1c && extended {
        return PhysicalKey::Code(KeyCode::NumpadEnter);
    }
    virtual_key_to_key_code(virtual_key, extended).map_or_else(
        || {
            PhysicalKey::Native(NativeKey {
                namespace: "windows.scan-code".into(),
                code: NativeCode::Numeric(((extended as u64) << 32) | u64::from(scan_code)),
            })
        },
        PhysicalKey::Code,
    )
}

pub fn virtual_key_to_key_code(vk: u32, extended: bool) -> Option<KeyCode> {
    Some(match vk {
        0x41..=0x5a => ALL_LETTERS[(vk - 0x41) as usize],
        0x30..=0x39 => ALL_DIGITS[(vk - 0x30) as usize],
        0x5b => KeyCode::SuperLeft,
        0x5c => KeyCode::SuperRight,
        0xa0 => KeyCode::ShiftLeft,
        0xa1 => KeyCode::ShiftRight,
        0xa2 => KeyCode::ControlLeft,
        0xa3 => KeyCode::ControlRight,
        0xa4 => KeyCode::AltLeft,
        0xa5 => KeyCode::AltRight,
        0x10 => KeyCode::ShiftLeft,
        0x11 => {
            if extended {
                KeyCode::ControlRight
            } else {
                KeyCode::ControlLeft
            }
        }
        0x12 => {
            if extended {
                KeyCode::AltRight
            } else {
                KeyCode::AltLeft
            }
        }
        0x08 => KeyCode::Backspace,
        0x09 => KeyCode::Tab,
        0x0d => {
            if extended {
                KeyCode::NumpadEnter
            } else {
                KeyCode::Enter
            }
        }
        0x1b => KeyCode::Escape,
        0x20 => KeyCode::Space,
        0x21 => KeyCode::PageUp,
        0x22 => KeyCode::PageDown,
        0x23 => KeyCode::End,
        0x24 => KeyCode::Home,
        0x25 => KeyCode::ArrowLeft,
        0x26 => KeyCode::ArrowUp,
        0x27 => KeyCode::ArrowRight,
        0x28 => KeyCode::ArrowDown,
        0x2c => KeyCode::PrintScreen,
        0x2d => KeyCode::Insert,
        0x2e => KeyCode::Delete,
        0x5d => KeyCode::ContextMenu,
        0x60..=0x69 => ALL_NUMPAD[(vk - 0x60) as usize],
        0x6a => KeyCode::NumpadMultiply,
        0x6b => KeyCode::NumpadAdd,
        0x6d => KeyCode::NumpadSubtract,
        0x6e => KeyCode::NumpadDecimal,
        0x6f => KeyCode::NumpadDivide,
        0x70..=0x7b => ALL_FUNCTIONS[(vk - 0x70) as usize],
        0xba => KeyCode::Semicolon,
        0xbb => KeyCode::Equal,
        0xbc => KeyCode::Comma,
        0xbd => KeyCode::Minus,
        0xbe => KeyCode::Period,
        0xbf => KeyCode::Slash,
        0xc0 => KeyCode::Backquote,
        0xdb => KeyCode::BracketLeft,
        0xdc => KeyCode::Backslash,
        0xdd => KeyCode::BracketRight,
        0xde => KeyCode::Quote,
        0xad => KeyCode::AudioVolumeMute,
        0xae => KeyCode::AudioVolumeDown,
        0xaf => KeyCode::AudioVolumeUp,
        0xb0 => KeyCode::MediaTrackNext,
        0xb1 => KeyCode::MediaTrackPrevious,
        0xb2 => KeyCode::MediaStop,
        0xb3 => KeyCode::MediaPlayPause,
        _ => return None,
    })
}

fn logical_key(vk: u32) -> LogicalKey {
    virtual_key_to_key_code(vk, false).map_or_else(
        || {
            LogicalKey::Native(NativeKey {
                namespace: "windows.virtual-key".into(),
                code: NativeCode::Numeric(u64::from(vk)),
            })
        },
        logical_for_code,
    )
}

fn logical_for_code(key: KeyCode) -> LogicalKey {
    let named = match key {
        KeyCode::Tab => Some(crate::NamedKey::Tab),
        KeyCode::Enter | KeyCode::NumpadEnter => Some(crate::NamedKey::Enter),
        KeyCode::Escape => Some(crate::NamedKey::Escape),
        KeyCode::Backspace => Some(crate::NamedKey::Backspace),
        KeyCode::Delete => Some(crate::NamedKey::Delete),
        KeyCode::ArrowLeft => Some(crate::NamedKey::ArrowLeft),
        KeyCode::ArrowRight => Some(crate::NamedKey::ArrowRight),
        KeyCode::ArrowUp => Some(crate::NamedKey::ArrowUp),
        KeyCode::ArrowDown => Some(crate::NamedKey::ArrowDown),
        KeyCode::PrintScreen => Some(crate::NamedKey::PrintScreen),
        _ => None,
    };
    named.map(LogicalKey::Named).unwrap_or_else(|| {
        LogicalKey::Native(NativeKey {
            namespace: "windows.key-code".into(),
            code: NativeCode::Numeric(key as u64),
        })
    })
}

fn key_location(key: &PhysicalKey) -> KeyLocation {
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

const ALL_LETTERS: [KeyCode; 26] = [
    KeyCode::KeyA,
    KeyCode::KeyB,
    KeyCode::KeyC,
    KeyCode::KeyD,
    KeyCode::KeyE,
    KeyCode::KeyF,
    KeyCode::KeyG,
    KeyCode::KeyH,
    KeyCode::KeyI,
    KeyCode::KeyJ,
    KeyCode::KeyK,
    KeyCode::KeyL,
    KeyCode::KeyM,
    KeyCode::KeyN,
    KeyCode::KeyO,
    KeyCode::KeyP,
    KeyCode::KeyQ,
    KeyCode::KeyR,
    KeyCode::KeyS,
    KeyCode::KeyT,
    KeyCode::KeyU,
    KeyCode::KeyV,
    KeyCode::KeyW,
    KeyCode::KeyX,
    KeyCode::KeyY,
    KeyCode::KeyZ,
];
const ALL_DIGITS: [KeyCode; 10] = [
    KeyCode::Digit0,
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];
const ALL_NUMPAD: [KeyCode; 10] = [
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
];
const ALL_FUNCTIONS: [KeyCode; 12] = [
    KeyCode::F1,
    KeyCode::F2,
    KeyCode::F3,
    KeyCode::F4,
    KeyCode::F5,
    KeyCode::F6,
    KeyCode::F7,
    KeyCode::F8,
    KeyCode::F9,
    KeyCode::F10,
    KeyCode::F11,
    KeyCode::F12,
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{AggregateModifier, Shortcut, ShortcutKey, ShortcutTrigger};
    use std::collections::BTreeSet;

    #[test]
    fn translates_modifier_sides_layout_stable_grave_and_unknown_keys() {
        assert_eq!(
            physical_key(0x11, 0x1d, false),
            PhysicalKey::Code(KeyCode::ControlLeft)
        );
        assert_eq!(
            physical_key(0x11, 0x1d, true),
            PhysicalKey::Code(KeyCode::ControlRight)
        );
        assert_eq!(
            physical_key(0x19, 0x29, false),
            PhysicalKey::Code(KeyCode::Backquote)
        );
        assert_eq!(
            physical_key(0x5d, 0x5d, true),
            PhysicalKey::Code(KeyCode::ContextMenu)
        );
        assert!(matches!(
            physical_key(0xff, 0x77, true),
            PhysicalKey::Native(_)
        ));
    }

    #[test]
    fn ignores_injected_events_and_suppresses_a_chord_once() {
        let binding = Binding {
            shortcut: Shortcut {
                key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::KeyA)),
                modifiers: [AggregateModifier::Control]
                    .into_iter()
                    .collect::<BTreeSet<_>>(),
                trigger: ShortcutTrigger::Pressed,
            },
            action: "select-all",
            suppress: true,
        };
        let mut adapter = WindowsInputAdapter::new([binding]);
        let native = |vk, scan, edge| NativeKeyboardEvent {
            virtual_key: vk,
            scan_code: scan,
            extended: false,
            edge,
            injected: false,
        };
        adapter.handle_native(native(0xa2, 0x1d, KeyEdge::Pressed));
        let first = adapter
            .handle_native(native(0x41, 0x1e, KeyEdge::Pressed))
            .unwrap();
        assert_eq!(first.outcomes.len(), 1);
        assert!(first.outcomes[0].suppress);
        assert!(
            adapter
                .handle_native(native(0x41, 0x1e, KeyEdge::Pressed))
                .unwrap()
                .outcomes
                .is_empty()
        );
        assert!(
            adapter
                .handle_native(NativeKeyboardEvent {
                    injected: true,
                    ..native(0x41, 0x1e, KeyEdge::Pressed)
                })
                .is_none()
        );
    }

    #[test]
    fn pointer_gesture_chords_super_and_reset_clears_state() {
        let mut adapter = WindowsInputAdapter::<()>::new([]);
        adapter.handle_key_code(KeyCode::SuperLeft, KeyEdge::Pressed);
        assert_eq!(
            adapter.begin_pointer_gesture(PointerButton::Primary),
            Some(SuperPointerGesture::Move)
        );
        assert!(
            adapter
                .handle_key_code(KeyCode::SuperLeft, KeyEdge::Released)
                .outcomes
                .is_empty()
        );
        adapter.handle_key_code(KeyCode::AltRight, KeyEdge::Pressed);
        adapter.reset();
        assert!(!adapter.modifier_held(AggregateModifier::Alt));
    }

    #[test]
    fn alt_graph_does_not_become_a_control_alt_shortcut() {
        let binding = Binding {
            shortcut: Shortcut {
                key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::KeyL)),
                modifiers: [AggregateModifier::Control, AggregateModifier::Alt]
                    .into_iter()
                    .collect(),
                trigger: ShortcutTrigger::Pressed,
            },
            action: "lock",
            suppress: true,
        };
        let mut adapter = WindowsInputAdapter::new([binding]);
        let native = |virtual_key, scan_code, extended| NativeKeyboardEvent {
            virtual_key,
            scan_code,
            extended,
            edge: KeyEdge::Pressed,
            injected: false,
        };
        adapter.handle_native(native(0xa2, 0x1d, false));
        let alt_graph = adapter.handle_native(native(0xa5, 0x38, true)).unwrap();
        assert_eq!(
            alt_graph.normalized.logical,
            LogicalKey::Named(crate::NamedKey::AltGraph)
        );
        assert!(
            adapter
                .handle_native(native(0x4c, 0x26, false))
                .unwrap()
                .outcomes
                .is_empty()
        );
    }
}
