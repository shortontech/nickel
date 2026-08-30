//! Backend-neutral input events and deterministic shortcut recognition.
//!
//! This crate deliberately contains no application actions or native runtime
//! types. Adapters retain responsibility for converting native events, while
//! consumers bind normalized shortcuts to their own typed actions.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};

pub mod controller;
#[cfg(feature = "gilrs")]
pub mod gilrs;
pub mod global;
#[cfg(feature = "sdl")]
pub mod sdl;
#[cfg(feature = "winit")]
pub mod winit;

/// Identifies an input device within a backend instance.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct DeviceId(pub u64);

/// A monotonically increasing event order assigned by an adapter.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct EventOrder(pub u64);

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum KeyEdge {
    Pressed,
    Released,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum KeyLocation {
    Standard,
    Left,
    Right,
    Numpad,
    Unknown,
}

/// A backend's lossless native key identity when no portable identity exists.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct NativeKey {
    pub namespace: String,
    pub code: NativeCode,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum NativeCode {
    Numeric(u64),
    Text(String),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PhysicalKey {
    Code(KeyCode),
    Native(NativeKey),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum KeyCode {
    KeyA,
    KeyB,
    KeyC,
    KeyD,
    KeyE,
    KeyF,
    KeyG,
    KeyH,
    KeyI,
    KeyJ,
    KeyK,
    KeyL,
    KeyM,
    KeyN,
    KeyO,
    KeyP,
    KeyQ,
    KeyR,
    KeyS,
    KeyT,
    KeyU,
    KeyV,
    KeyW,
    KeyX,
    KeyY,
    KeyZ,
    Digit0,
    Digit1,
    Digit2,
    Digit3,
    Digit4,
    Digit5,
    Digit6,
    Digit7,
    Digit8,
    Digit9,
    Escape,
    Tab,
    CapsLock,
    Enter,
    Space,
    Backspace,
    Delete,
    Insert,
    Home,
    End,
    PageUp,
    PageDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    ArrowDown,
    Backquote,
    Minus,
    Equal,
    BracketLeft,
    BracketRight,
    Backslash,
    Semicolon,
    Quote,
    Comma,
    Period,
    Slash,
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,
    PrintScreen,
    ScrollLock,
    Pause,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
    Numpad0,
    Numpad1,
    Numpad2,
    Numpad3,
    Numpad4,
    Numpad5,
    Numpad6,
    Numpad7,
    Numpad8,
    Numpad9,
    NumpadAdd,
    NumpadSubtract,
    NumpadMultiply,
    NumpadDivide,
    NumpadDecimal,
    NumpadEnter,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum LogicalKey {
    Named(NamedKey),
    Character(String),
    Dead(Option<char>),
    Native(NativeKey),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
#[non_exhaustive]
pub enum NamedKey {
    Alt,
    AltGraph,
    Backspace,
    CapsLock,
    Control,
    Delete,
    End,
    Enter,
    Escape,
    Home,
    Insert,
    Meta,
    PageDown,
    PageUp,
    Shift,
    Space,
    Tab,
    ArrowDown,
    ArrowLeft,
    ArrowRight,
    ArrowUp,
    PrintScreen,
    F1,
    F2,
    F3,
    F4,
    F5,
    F6,
    F7,
    F8,
    F9,
    F10,
    F11,
    F12,
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Modifier {
    ShiftLeft,
    ShiftRight,
    ControlLeft,
    ControlRight,
    AltLeft,
    AltRight,
    SuperLeft,
    SuperRight,
}

impl Modifier {
    pub fn aggregate(self) -> AggregateModifier {
        match self {
            Self::ShiftLeft | Self::ShiftRight => AggregateModifier::Shift,
            Self::ControlLeft | Self::ControlRight => AggregateModifier::Control,
            Self::AltLeft | Self::AltRight => AggregateModifier::Alt,
            Self::SuperLeft | Self::SuperRight => AggregateModifier::Super,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AggregateModifier {
    Shift,
    Control,
    Alt,
    Super,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ModifierState {
    sides: BTreeSet<Modifier>,
    unsided: BTreeSet<AggregateModifier>,
}

impl ModifierState {
    pub fn from_sides(sides: impl IntoIterator<Item = Modifier>) -> Self {
        Self {
            sides: sides.into_iter().collect(),
            unsided: BTreeSet::new(),
        }
    }
    pub fn from_sides_and_unsided(
        sides: impl IntoIterator<Item = Modifier>,
        unsided: impl IntoIterator<Item = AggregateModifier>,
    ) -> Self {
        Self {
            sides: sides.into_iter().collect(),
            unsided: unsided.into_iter().collect(),
        }
    }
    pub fn contains(&self, modifier: Modifier) -> bool {
        self.sides.contains(&modifier)
    }
    pub fn aggregate(&self, modifier: AggregateModifier) -> bool {
        self.unsided.contains(&modifier)
            || self.sides.iter().any(|side| side.aggregate() == modifier)
    }
    pub fn sides(&self) -> impl Iterator<Item = Modifier> + '_ {
        self.sides.iter().copied()
    }
    pub fn unsided(&self) -> impl Iterator<Item = AggregateModifier> + '_ {
        self.unsided.iter().copied()
    }
    pub fn is_empty(&self) -> bool {
        self.sides.is_empty() && self.unsided.is_empty()
    }
    fn set(&mut self, modifier: Modifier, pressed: bool) {
        if pressed {
            self.sides.insert(modifier);
        } else {
            self.sides.remove(&modifier);
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyEvent {
    pub device: DeviceId,
    pub order: EventOrder,
    pub physical: PhysicalKey,
    pub logical: LogicalKey,
    pub location: KeyLocation,
    pub edge: KeyEdge,
    pub repeat: bool,
    /// Adapter-observed state after this edge. The shortcut engine independently
    /// derives authoritative pressed state so focus loss can reset it.
    pub modifiers: ModifierState,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TextEvent {
    Preedit {
        device: DeviceId,
        order: EventOrder,
        text: String,
        selection: Option<(usize, usize)>,
    },
    Commit {
        device: DeviceId,
        order: EventOrder,
        text: String,
    },
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Vector {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum PointerButton {
    Primary,
    Secondary,
    Middle,
    Back,
    Forward,
    Native(u16),
}

#[derive(Clone, Debug, PartialEq)]
pub enum PointerEvent {
    Motion {
        device: DeviceId,
        order: EventOrder,
        position: Point,
        delta: Option<Vector>,
    },
    Button {
        device: DeviceId,
        order: EventOrder,
        button: PointerButton,
        edge: KeyEdge,
        position: Option<Point>,
    },
    Axis {
        device: DeviceId,
        order: EventOrder,
        delta: Vector,
        discrete: Option<(i32, i32)>,
        position: Option<Point>,
    },
    Enter {
        device: DeviceId,
        order: EventOrder,
        position: Point,
    },
    Leave {
        device: DeviceId,
        order: EventOrder,
    },
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TouchId(pub u64);

#[derive(Clone, Debug, PartialEq)]
pub enum TouchEvent {
    Started {
        device: DeviceId,
        order: EventOrder,
        contact: TouchId,
        position: Point,
    },
    Moved {
        device: DeviceId,
        order: EventOrder,
        contact: TouchId,
        position: Point,
    },
    Ended {
        device: DeviceId,
        order: EventOrder,
        contact: TouchId,
        position: Point,
    },
    Cancelled {
        device: DeviceId,
        order: EventOrder,
        contact: TouchId,
    },
}

#[derive(Clone, Debug, PartialEq)]
pub enum InputEvent {
    Key(KeyEvent),
    Text(TextEvent),
    Pointer(PointerEvent),
    Touch(TouchEvent),
    FocusGained { order: EventOrder },
    FocusLost { order: EventOrder },
    DeviceRemoved { device: DeviceId, order: EventOrder },
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ShortcutKey {
    Physical(PhysicalKey),
    Logical(LogicalKey),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Shortcut {
    pub key: ShortcutKey,
    pub modifiers: BTreeSet<AggregateModifier>,
    pub trigger: ShortcutTrigger,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ShortcutTrigger {
    Pressed,
    ModifierReleased(Modifier),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ShortcutOutcome<A> {
    pub action: A,
    pub suppress: bool,
}

#[derive(Clone, Debug)]
pub struct Binding<A> {
    pub shortcut: Shortcut,
    pub action: A,
    pub suppress: bool,
}

#[derive(Clone, Debug, Default)]
struct DeviceState {
    pressed: BTreeSet<PhysicalKey>,
    modifiers: ModifierState,
    chorded_modifiers: BTreeSet<Modifier>,
}

/// Deterministic, device-aware shortcut recognizer.
#[derive(Clone, Debug)]
pub struct ShortcutEngine<A> {
    bindings: Vec<Binding<A>>,
    devices: BTreeMap<DeviceId, DeviceState>,
    last_order: Option<EventOrder>,
}

impl<A> Default for ShortcutEngine<A> {
    fn default() -> Self {
        Self {
            bindings: Vec::new(),
            devices: BTreeMap::new(),
            last_order: None,
        }
    }
}

impl<A: Clone> ShortcutEngine<A> {
    pub fn new(bindings: impl IntoIterator<Item = Binding<A>>) -> Self {
        Self {
            bindings: bindings.into_iter().collect(),
            ..Self::default()
        }
    }

    pub fn handle(&mut self, event: &InputEvent) -> Vec<ShortcutOutcome<A>> {
        match event {
            InputEvent::Key(event) => self.handle_key(event),
            InputEvent::FocusGained { order } => {
                self.observe_order(*order);
                Vec::new()
            }
            InputEvent::FocusLost { order } => {
                self.observe_order(*order);
                self.reset();
                Vec::new()
            }
            InputEvent::DeviceRemoved { device, order } => {
                self.observe_order(*order);
                self.devices.remove(device);
                Vec::new()
            }
            InputEvent::Text(_) | InputEvent::Pointer(_) | InputEvent::Touch(_) => Vec::new(),
        }
    }

    pub fn reset(&mut self) {
        self.devices.clear();
    }
    pub fn pressed_keys(&self, device: DeviceId) -> impl Iterator<Item = &PhysicalKey> {
        self.devices
            .get(&device)
            .into_iter()
            .flat_map(|state| state.pressed.iter())
    }
    pub fn modifiers(&self) -> ModifierState {
        let mut result = ModifierState::default();
        for state in self.devices.values() {
            for side in state.modifiers.sides() {
                result.set(side, true);
            }
            result.unsided.extend(state.modifiers.unsided());
        }
        result
    }

    fn observe_order(&mut self, order: EventOrder) {
        if let Some(previous) = self.last_order {
            debug_assert!(
                order > previous,
                "input event order must be strictly monotonic"
            );
        }
        self.last_order = Some(order);
    }

    fn handle_key(&mut self, event: &KeyEvent) -> Vec<ShortcutOutcome<A>> {
        self.observe_order(event.order);
        let modifier = modifier_for(&event.physical);
        let (genuine_press, modifier_was_chorded) = {
            let state = self.devices.entry(event.device).or_default();
            let was_pressed = state.pressed.contains(&event.physical);
            let genuine_press = event.edge == KeyEdge::Pressed && !was_pressed && !event.repeat;

            if event.edge == KeyEdge::Pressed {
                state.pressed.insert(event.physical.clone());
                if let Some(side) = modifier {
                    state.modifiers.set(side, true);
                }
                if genuine_press {
                    let held = state.modifiers.sides().collect::<Vec<_>>();
                    for held in held {
                        if Some(held) != modifier {
                            state.chorded_modifiers.insert(held);
                        }
                    }
                }
            }
            let modifier_was_chorded =
                modifier.is_some_and(|side| state.chorded_modifiers.contains(&side));
            (genuine_press, modifier_was_chorded)
        };

        if genuine_press && modifier.is_none() {
            for state in self.devices.values_mut() {
                state.chorded_modifiers.extend(state.modifiers.sides());
            }
        }

        let aggregate = self.modifiers();
        let mut modifiers_after_release = aggregate.clone();
        if event.edge == KeyEdge::Released
            && let Some(side) = modifier
        {
            modifiers_after_release.set(side, false);
        }
        let mut outcomes = Vec::new();
        for binding in &self.bindings {
            let triggered = match binding.shortcut.trigger {
                ShortcutTrigger::Pressed => {
                    genuine_press
                        && shortcut_key_matches(&binding.shortcut.key, event)
                        && exact_modifiers(&binding.shortcut.modifiers, &aggregate)
                }
                ShortcutTrigger::ModifierReleased(side) => {
                    event.edge == KeyEdge::Released
                        && modifier == Some(side)
                        && !modifier_was_chorded
                        && exact_modifiers(&binding.shortcut.modifiers, &modifiers_after_release)
                        && shortcut_key_matches(&binding.shortcut.key, event)
                }
            };
            if triggered {
                outcomes.push(ShortcutOutcome {
                    action: binding.action.clone(),
                    suppress: binding.suppress,
                });
            }
        }

        if event.edge == KeyEdge::Released {
            let state = self.devices.entry(event.device).or_default();
            state.pressed.remove(&event.physical);
            if let Some(side) = modifier {
                state.modifiers.set(side, false);
                state.chorded_modifiers.remove(&side);
            }
        }
        outcomes
    }
}

fn exact_modifiers(required: &BTreeSet<AggregateModifier>, actual: &ModifierState) -> bool {
    [
        AggregateModifier::Shift,
        AggregateModifier::Control,
        AggregateModifier::Alt,
        AggregateModifier::Super,
    ]
    .into_iter()
    .all(|modifier| required.contains(&modifier) == actual.aggregate(modifier))
}

fn shortcut_key_matches(key: &ShortcutKey, event: &KeyEvent) -> bool {
    match key {
        ShortcutKey::Physical(key) => key == &event.physical,
        ShortcutKey::Logical(key) => key == &event.logical,
    }
}

pub fn modifier_for(key: &PhysicalKey) -> Option<Modifier> {
    let PhysicalKey::Code(code) = key else {
        return None;
    };
    Some(match code {
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

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn key(device: u64, order: u64, code: KeyCode, edge: KeyEdge, repeat: bool) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(device),
            order: EventOrder(order),
            physical: PhysicalKey::Code(code),
            logical: LogicalKey::Native(NativeKey {
                namespace: "test".into(),
                code: NativeCode::Numeric(code as u64),
            }),
            location: KeyLocation::Standard,
            edge,
            repeat,
            modifiers: ModifierState::default(),
        })
    }
    fn set(items: impl IntoIterator<Item = AggregateModifier>) -> BTreeSet<AggregateModifier> {
        items.into_iter().collect()
    }

    #[test]
    fn physical_logical_and_text_are_independent() {
        let physical = PhysicalKey::Code(KeyCode::KeyQ);
        let logical = LogicalKey::Character("a".into());
        let text = TextEvent::Commit {
            device: DeviceId(1),
            order: EventOrder(2),
            text: "å".into(),
        };
        assert_ne!(
            ShortcutKey::Physical(physical),
            ShortcutKey::Logical(logical)
        );
        assert!(matches!(text, TextEvent::Commit { text, .. } if text == "å"));
    }

    #[test]
    fn unknown_and_dead_keys_are_preserved() {
        let native = NativeKey {
            namespace: "fixture".into(),
            code: NativeCode::Numeric(0xdead),
        };
        assert_eq!(
            PhysicalKey::Native(native.clone()),
            PhysicalKey::Native(native)
        );
        assert_eq!(LogicalKey::Dead(Some('^')), LogicalKey::Dead(Some('^')));
    }

    #[test]
    fn chord_triggers_once_and_distinguishes_modifier_sides() {
        let shortcut = Shortcut {
            key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::KeyR)),
            modifiers: set([AggregateModifier::Super]),
            trigger: ShortcutTrigger::Pressed,
        };
        let mut engine = ShortcutEngine::new([Binding {
            shortcut,
            action: "run",
            suppress: true,
        }]);
        assert!(
            engine
                .handle(&key(1, 1, KeyCode::SuperLeft, KeyEdge::Pressed, false))
                .is_empty()
        );
        assert_eq!(
            engine.handle(&key(1, 2, KeyCode::KeyR, KeyEdge::Pressed, false)),
            [ShortcutOutcome {
                action: "run",
                suppress: true
            }]
        );
        assert!(
            engine
                .handle(&key(1, 3, KeyCode::KeyR, KeyEdge::Pressed, true))
                .is_empty()
        );
        assert!(engine.modifiers().contains(Modifier::SuperLeft));
        assert!(!engine.modifiers().contains(Modifier::SuperRight));
    }

    #[test]
    fn modifier_only_binding_fires_on_unchorded_release() {
        let shortcut = Shortcut {
            key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::SuperLeft)),
            modifiers: set([]),
            trigger: ShortcutTrigger::ModifierReleased(Modifier::SuperLeft),
        };
        let binding = Binding {
            shortcut,
            action: "launcher",
            suppress: false,
        };
        let mut bare = ShortcutEngine::new([binding.clone()]);
        bare.handle(&key(1, 1, KeyCode::SuperLeft, KeyEdge::Pressed, false));
        assert_eq!(
            bare.handle(&key(1, 2, KeyCode::SuperLeft, KeyEdge::Released, false))
                .len(),
            1
        );
        let mut chorded = ShortcutEngine::new([binding]);
        chorded.handle(&key(1, 1, KeyCode::SuperLeft, KeyEdge::Pressed, false));
        chorded.handle(&key(1, 2, KeyCode::KeyR, KeyEdge::Pressed, false));
        assert!(
            chorded
                .handle(&key(1, 3, KeyCode::SuperLeft, KeyEdge::Released, false))
                .is_empty()
        );
    }

    #[test]
    fn a_key_on_another_device_chords_a_held_modifier() {
        let shortcut = Shortcut {
            key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::SuperLeft)),
            modifiers: set([]),
            trigger: ShortcutTrigger::ModifierReleased(Modifier::SuperLeft),
        };
        let mut engine = ShortcutEngine::new([Binding {
            shortcut,
            action: "launcher",
            suppress: false,
        }]);
        engine.handle(&key(1, 1, KeyCode::SuperLeft, KeyEdge::Pressed, false));
        engine.handle(&key(2, 2, KeyCode::KeyR, KeyEdge::Pressed, false));
        assert!(
            engine
                .handle(&key(1, 3, KeyCode::SuperLeft, KeyEdge::Released, false))
                .is_empty()
        );
    }

    #[test]
    fn focus_loss_and_device_removal_reset_without_actions() {
        let mut engine = ShortcutEngine::<()>::new([]);
        engine.handle(&key(1, 1, KeyCode::ShiftLeft, KeyEdge::Pressed, false));
        assert!(
            engine
                .handle(&InputEvent::FocusGained {
                    order: EventOrder(2),
                })
                .is_empty()
        );
        assert!(engine.modifiers().aggregate(AggregateModifier::Shift));
        engine.handle(&key(2, 3, KeyCode::ControlRight, KeyEdge::Pressed, false));
        assert!(engine.modifiers().aggregate(AggregateModifier::Shift));
        engine.handle(&InputEvent::DeviceRemoved {
            device: DeviceId(1),
            order: EventOrder(4),
        });
        assert!(!engine.modifiers().aggregate(AggregateModifier::Shift));
        engine.handle(&InputEvent::FocusLost {
            order: EventOrder(5),
        });
        assert!(engine.modifiers().is_empty());
    }

    proptest! {
        #[test]
        fn arbitrary_edges_and_resets_never_leave_non_modifier_modifier_state(ops in prop::collection::vec((0u8..4, any::<bool>(), any::<bool>()), 0..200)) {
            let mut engine = ShortcutEngine::<()>::new([]);
            let mut order = 0;
            for (code, pressed, reset) in ops {
                order += 1;
                if reset { engine.handle(&InputEvent::FocusLost { order: EventOrder(order) }); continue; }
                let keycode = [KeyCode::ShiftLeft, KeyCode::ControlRight, KeyCode::KeyA, KeyCode::AltLeft][code as usize];
                engine.handle(&key(1, order, keycode, if pressed { KeyEdge::Pressed } else { KeyEdge::Released }, false));
                for side in engine.modifiers().sides() {
                    prop_assert!(matches!(side, Modifier::ShiftLeft | Modifier::ShiftRight | Modifier::ControlLeft | Modifier::ControlRight | Modifier::AltLeft | Modifier::AltRight | Modifier::SuperLeft | Modifier::SuperRight));
                }
            }
        }

        #[test]
        fn replay_is_deterministic(ops in prop::collection::vec((0u8..3, any::<bool>()), 0..100)) {
            let binding = Binding {
                shortcut: Shortcut { key: ShortcutKey::Physical(PhysicalKey::Code(KeyCode::KeyR)), modifiers: set([AggregateModifier::Super]), trigger: ShortcutTrigger::Pressed },
                action: 7u8, suppress: true,
            };
            let replay = |ops: &[(u8, bool)]| {
                let mut engine = ShortcutEngine::new([binding.clone()]);
                let mut output = Vec::new();
                for (index, (code, pressed)) in ops.iter().enumerate() {
                    let keycode = [KeyCode::SuperLeft, KeyCode::KeyR, KeyCode::ShiftRight][*code as usize];
                    output.extend(engine.handle(&key(1, index as u64 + 1, keycode, if *pressed { KeyEdge::Pressed } else { KeyEdge::Released }, false)));
                }
                output
            };
            prop_assert_eq!(replay(&ops), replay(&ops));
        }
    }
}
