use std::{collections::BTreeMap, time::Instant};

use gilrs::Gilrs;
use nickel_input::{
    NativeCode,
    controller::{
        AxisDirection, ControllerButton, ControllerEvent, ControllerId, ControllerIdentity,
        ControllerNormalizer, ControllerSignal,
    },
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ControllerAction {
    Launcher,
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
    ContextMenu,
    PreviousPane,
    NextPane,
}

#[derive(Clone, Copy, Debug, Default, Eq, Hash, PartialEq)]
pub enum ControllerFamily {
    PlayStation,
    Xbox,
    Switch,
    #[default]
    Generic,
}

impl ControllerFamily {
    /// Resolves only backend-reported identity. Button layout is deliberately
    /// not used as a branding guess.
    pub fn from_reported_name(name: &str) -> Self {
        let name = name.to_ascii_lowercase();
        if ["playstation", "dualshock", "dualsense", "sony"]
            .iter()
            .any(|needle| name.contains(needle))
        {
            Self::PlayStation
        } else if ["xbox", "xinput", "microsoft"]
            .iter()
            .any(|needle| name.contains(needle))
        {
            Self::Xbox
        } else if ["nintendo", "joy-con", "switch pro"]
            .iter()
            .any(|needle| name.contains(needle))
        {
            Self::Switch
        } else {
            Self::Generic
        }
    }
}

pub struct ControllerInput {
    gilrs: Option<Gilrs>,
    normalizer: ControllerNormalizer,
    epoch: Instant,
    connected: bool,
    families: BTreeMap<ControllerId, ControllerFamily>,
    active_family: Option<ControllerFamily>,
}

impl ControllerInput {
    pub fn new() -> Self {
        let gilrs = Gilrs::new()
            .map_err(|error| tracing::warn!(%error, "controller input is unavailable"))
            .ok();
        let connected = gilrs
            .as_ref()
            .is_some_and(|gilrs| gilrs.gamepads().any(|(_, gamepad)| gamepad.is_connected()));
        let mut normalizer = ControllerNormalizer::default();
        let mut families = BTreeMap::new();
        if let Some(gilrs) = &gilrs {
            for (id, gamepad) in gilrs
                .gamepads()
                .filter(|(_, gamepad)| gamepad.is_connected())
            {
                let id = ControllerId(usize::from(id) as u64);
                families.insert(id, ControllerFamily::from_reported_name(gamepad.name()));
                normalizer.handle(
                    ControllerEvent::Connected {
                        id,
                        identity: ControllerIdentity {
                            backend: "gilrs".into(),
                            native: NativeCode::Numeric(id.0),
                            fingerprint: Some(uuid_fingerprint(gamepad.uuid())),
                        },
                    },
                    0,
                );
            }
        }
        Self {
            gilrs,
            normalizer,
            epoch: Instant::now(),
            connected,
            families,
            active_family: None,
        }
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    /// Family of the controller that most recently produced meaningful input.
    /// Merely connecting a device does not choose a family or input modality.
    pub fn active_family(&self) -> Option<ControllerFamily> {
        self.active_family
    }

    /// Polls controller input for a window. Events are drained but never emitted while the
    /// window is unfocused, preventing stale input from being replayed when focus returns.
    pub fn poll(&mut self, now: Instant, window_focused: bool) -> Vec<ControllerAction> {
        let actions = self.poll_global(now);
        if window_focused { actions } else { Vec::new() }
    }

    /// Polls controller input for a session-global owner such as the desktop shell.
    /// Ordinary applications should use [`Self::poll`] so background input is discarded.
    pub fn poll_global(&mut self, now: Instant) -> Vec<ControllerAction> {
        let mut actions = Vec::new();
        let Some(gilrs) = &mut self.gilrs else {
            return actions;
        };
        while let Some(event) = gilrs.next_event() {
            let identity = matches!(event.event, gilrs::EventType::Connected).then(|| {
                let gamepad = gilrs.gamepad(event.id);
                self.families.insert(
                    ControllerId(usize::from(event.id) as u64),
                    ControllerFamily::from_reported_name(gamepad.name()),
                );
                ControllerIdentity {
                    backend: "gilrs".into(),
                    native: NativeCode::Numeric(usize::from(event.id) as u64),
                    fingerprint: Some(uuid_fingerprint(gamepad.uuid())),
                }
            });
            let disconnected = matches!(event.event, gilrs::EventType::Disconnected)
                .then_some(ControllerId(usize::from(event.id) as u64));
            if let Some(event) = nickel_input::gilrs::event(&event, identity) {
                let now_ms = now.saturating_duration_since(self.epoch).as_millis() as u64;
                for signal in self.normalizer.handle(event, now_ms) {
                    if let Some(action) = signal_action(signal.clone()) {
                        self.active_family = signal_id(&signal)
                            .and_then(|id| self.families.get(&id).copied())
                            .or(Some(ControllerFamily::Generic));
                        actions.push(action);
                    }
                }
            }
            if let Some(id) = disconnected {
                self.families.remove(&id);
            }
        }
        self.connected = gilrs.gamepads().any(|(_, gamepad)| gamepad.is_connected());
        let now_ms = now.saturating_duration_since(self.epoch).as_millis() as u64;
        for signal in self.normalizer.tick(now_ms) {
            if let Some(action) = signal_action(signal.clone()) {
                self.active_family = signal_id(&signal)
                    .and_then(|id| self.families.get(&id).copied())
                    .or(Some(ControllerFamily::Generic));
                actions.push(action);
            }
        }
        actions
    }
}

fn uuid_fingerprint(uuid: [u8; 16]) -> String {
    use std::fmt::Write;

    let mut result = String::with_capacity(32);
    for byte in uuid {
        let _ = write!(result, "{byte:02x}");
    }
    result
}

impl Default for ControllerInput {
    fn default() -> Self {
        Self::new()
    }
}

fn button_action(button: &ControllerButton) -> Option<ControllerAction> {
    match button {
        ControllerButton::DPadUp => Some(ControllerAction::Up),
        ControllerButton::DPadDown => Some(ControllerAction::Down),
        ControllerButton::DPadLeft => Some(ControllerAction::Left),
        ControllerButton::DPadRight => Some(ControllerAction::Right),
        ControllerButton::South => Some(ControllerAction::Confirm),
        ControllerButton::East | ControllerButton::Select => Some(ControllerAction::Cancel),
        ControllerButton::Start => Some(ControllerAction::ContextMenu),
        ControllerButton::LeftShoulder => Some(ControllerAction::PreviousPane),
        ControllerButton::RightShoulder => Some(ControllerAction::NextPane),
        ControllerButton::Guide => Some(ControllerAction::Launcher),
        _ => None,
    }
}

fn signal_action(signal: ControllerSignal) -> Option<ControllerAction> {
    match signal {
        ControllerSignal::Button {
            button: ControllerButton::Start,
            edge: nickel_input::KeyEdge::Pressed,
            repeat: true,
            ..
        } => None,
        ControllerSignal::Button {
            button,
            edge: nickel_input::KeyEdge::Pressed,
            ..
        } => button_action(&button),
        ControllerSignal::Direction {
            direction,
            edge: nickel_input::KeyEdge::Pressed,
            ..
        } => Some(match direction {
            AxisDirection::Up => ControllerAction::Up,
            AxisDirection::Down => ControllerAction::Down,
            AxisDirection::Left => ControllerAction::Left,
            AxisDirection::Right => ControllerAction::Right,
        }),
        _ => None,
    }
}

fn signal_id(signal: &ControllerSignal) -> Option<ControllerId> {
    match signal {
        ControllerSignal::Button { id, .. } | ControllerSignal::Direction { id, .. } => Some(*id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::{ControllerAction, ControllerFamily, signal_action};
    use nickel_input::NativeCode;
    use nickel_input::controller::{
        AxisDirection, ControllerAxis, ControllerButton, ControllerEvent, ControllerId,
        ControllerIdentity, ControllerNormalizer, ControllerSignal,
    };

    fn identity(name: &str, fingerprint: Option<&str>) -> ControllerIdentity {
        ControllerIdentity {
            backend: name.into(),
            native: NativeCode::Text(name.into()),
            fingerprint: fingerprint.map(str::to_owned),
        }
    }

    fn context_trace(
        normalizer: &mut ControllerNormalizer,
        id: ControllerId,
        identity: ControllerIdentity,
    ) -> Vec<Option<ControllerAction>> {
        let mut actions = Vec::new();
        for (time, event) in [
            (0, ControllerEvent::Connected { id, identity }),
            (
                1,
                ControllerEvent::Button {
                    id,
                    button: ControllerButton::Start,
                    edge: nickel_input::KeyEdge::Pressed,
                    repeat: false,
                },
            ),
            (
                2,
                ControllerEvent::Button {
                    id,
                    button: ControllerButton::Start,
                    edge: nickel_input::KeyEdge::Pressed,
                    repeat: true,
                },
            ),
            (
                3,
                ControllerEvent::Button {
                    id,
                    button: ControllerButton::Start,
                    edge: nickel_input::KeyEdge::Released,
                    repeat: false,
                },
            ),
        ] {
            actions.extend(
                normalizer
                    .handle(event, time)
                    .into_iter()
                    .map(signal_action),
            );
        }
        actions
    }

    #[test]
    fn normalized_directions_remain_consumer_owned_actions() {
        assert_eq!(
            signal_action(ControllerSignal::Direction {
                id: ControllerId(1),
                direction: AxisDirection::Right,
                edge: nickel_input::KeyEdge::Pressed,
                repeat: false,
            }),
            Some(ControllerAction::Right)
        );
    }

    #[test]
    fn guide_button_is_a_launcher_action() {
        assert_eq!(
            signal_action(ControllerSignal::Button {
                id: ControllerId(1),
                button: ControllerButton::Guide,
                edge: nickel_input::KeyEdge::Pressed,
                repeat: false,
            }),
            Some(ControllerAction::Launcher)
        );
    }

    #[test]
    fn start_button_is_the_semantic_context_menu_action() {
        assert_eq!(
            signal_action(ControllerSignal::Button {
                id: ControllerId(1),
                button: ControllerButton::Start,
                edge: nickel_input::KeyEdge::Pressed,
                repeat: false,
            }),
            Some(ControllerAction::ContextMenu)
        );
    }

    #[test]
    fn reported_controller_names_resolve_without_layout_guessing() {
        assert_eq!(
            ControllerFamily::from_reported_name("Sony Interactive Entertainment DualSense"),
            ControllerFamily::PlayStation
        );
        assert_eq!(
            ControllerFamily::from_reported_name("Microsoft X-Box One pad"),
            ControllerFamily::Xbox
        );
        assert_eq!(
            ControllerFamily::from_reported_name("Nintendo Switch Pro Controller"),
            ControllerFamily::Switch
        );
        assert_eq!(
            ControllerFamily::from_reported_name("USB game controller"),
            ControllerFamily::Generic
        );
    }

    #[test]
    fn family_context_traces_normalize_press_only_and_survive_hotplug() {
        for (index, (name, family)) in [
            ("Sony DualSense", ControllerFamily::PlayStation),
            ("Microsoft Xbox Wireless Controller", ControllerFamily::Xbox),
            ("Nintendo Switch Pro Controller", ControllerFamily::Switch),
            ("Unknown USB HID", ControllerFamily::Generic),
        ]
        .into_iter()
        .enumerate()
        {
            assert_eq!(ControllerFamily::from_reported_name(name), family);
            let mut normalizer = ControllerNormalizer::default();
            let id = ControllerId(index as u64 + 1);
            assert_eq!(
                context_trace(&mut normalizer, id, identity(name, None)),
                [None, Some(ControllerAction::ContextMenu), None, None]
            );
            assert_eq!(
                normalizer.handle(ControllerEvent::Disconnected { id }, 4),
                [ControllerSignal::Disconnected(id)]
            );
            let replacement = ControllerId(id.0 + 100);
            assert_eq!(
                context_trace(&mut normalizer, replacement, identity(name, None)),
                [None, Some(ControllerAction::ContextMenu), None, None]
            );
        }
    }

    #[test]
    fn duplicate_backend_context_delivery_is_suppressed_by_hardware_identity() {
        let mut normalizer = ControllerNormalizer::default();
        let primary = ControllerId(1);
        let duplicate = ControllerId(2);
        normalizer.handle(
            ControllerEvent::Connected {
                id: primary,
                identity: identity("gilrs", Some("physical-pad")),
            },
            0,
        );
        assert_eq!(
            normalizer.handle(
                ControllerEvent::Connected {
                    id: duplicate,
                    identity: identity("sdl", Some("physical-pad")),
                },
                1,
            ),
            [ControllerSignal::DuplicateSuppressed(duplicate)]
        );
        let press = |id| ControllerEvent::Button {
            id,
            button: ControllerButton::Start,
            edge: nickel_input::KeyEdge::Pressed,
            repeat: false,
        };
        assert!(normalizer.handle(press(duplicate), 2).is_empty());
        assert_eq!(
            normalizer
                .handle(press(primary), 3)
                .into_iter()
                .filter_map(signal_action)
                .collect::<Vec<_>>(),
            [ControllerAction::ContextMenu]
        );
    }

    #[test]
    fn connection_and_stick_drift_do_not_produce_modality_changing_actions() {
        let id = ControllerId(7);
        let mut normalizer = ControllerNormalizer::default();
        let mut signals = normalizer.handle(
            ControllerEvent::Connected {
                id,
                identity: ControllerIdentity {
                    backend: "fixture".into(),
                    native: NativeCode::Numeric(7),
                    fingerprint: None,
                },
            },
            0,
        );
        for (time, value) in [(1, 0.01), (2, -0.08), (3, 0.22), (4, -0.31)] {
            signals.extend(normalizer.handle(
                ControllerEvent::Axis {
                    id,
                    axis: ControllerAxis::LeftX,
                    value,
                },
                time,
            ));
        }

        assert!(
            signals
                .into_iter()
                .all(|signal| signal_action(signal).is_none()),
            "only a meaningful normalized controller action may select controller modality"
        );
    }
}
