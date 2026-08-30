use std::time::Instant;

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
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
    PreviousPane,
    NextPane,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum NavigationPane {
    Sidebar,
    #[default]
    Content,
}

#[derive(Default)]
pub struct PaneNavigation {
    pane: NavigationPane,
}

impl PaneNavigation {
    pub fn pane(&self) -> NavigationPane {
        self.pane
    }

    pub fn handle(&mut self, action: ControllerAction) -> bool {
        let next = match action {
            ControllerAction::PreviousPane => NavigationPane::Sidebar,
            ControllerAction::NextPane => NavigationPane::Content,
            _ => return false,
        };
        let changed = self.pane != next;
        self.pane = next;
        changed
    }
}

pub struct ControllerInput {
    gilrs: Option<Gilrs>,
    normalizer: ControllerNormalizer,
    epoch: Instant,
    connected: bool,
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
        if let Some(gilrs) = &gilrs {
            for (id, gamepad) in gilrs
                .gamepads()
                .filter(|(_, gamepad)| gamepad.is_connected())
            {
                normalizer.handle(
                    ControllerEvent::Connected {
                        id: ControllerId(usize::from(id) as u64),
                        identity: ControllerIdentity {
                            backend: "gilrs".into(),
                            native: NativeCode::Numeric(usize::from(id) as u64),
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
        }
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn poll(&mut self, now: Instant) -> Vec<ControllerAction> {
        let mut actions = Vec::new();
        let Some(gilrs) = &mut self.gilrs else {
            return actions;
        };
        while let Some(event) = gilrs.next_event() {
            let identity = matches!(event.event, gilrs::EventType::Connected).then(|| {
                let gamepad = gilrs.gamepad(event.id);
                ControllerIdentity {
                    backend: "gilrs".into(),
                    native: NativeCode::Numeric(usize::from(event.id) as u64),
                    fingerprint: Some(uuid_fingerprint(gamepad.uuid())),
                }
            });
            if let Some(event) = nickel_input::gilrs::event(&event, identity) {
                let now_ms = now.saturating_duration_since(self.epoch).as_millis() as u64;
                actions.extend(
                    self.normalizer
                        .handle(event, now_ms)
                        .into_iter()
                        .filter_map(signal_action),
                );
            }
        }
        self.connected = gilrs.gamepads().any(|(_, gamepad)| gamepad.is_connected());
        let now_ms = now.saturating_duration_since(self.epoch).as_millis() as u64;
        actions.extend(
            self.normalizer
                .tick(now_ms)
                .into_iter()
                .filter_map(signal_action),
        );
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
        ControllerButton::East => Some(ControllerAction::Cancel),
        ControllerButton::LeftShoulder => Some(ControllerAction::PreviousPane),
        ControllerButton::RightShoulder => Some(ControllerAction::NextPane),
        _ => None,
    }
}

fn signal_action(signal: ControllerSignal) -> Option<ControllerAction> {
    match signal {
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

#[cfg(test)]
mod tests {
    use super::{ControllerAction, NavigationPane, PaneNavigation, signal_action};
    use nickel_input::controller::{AxisDirection, ControllerId, ControllerSignal};

    #[test]
    fn shoulder_actions_select_panes() {
        let mut navigation = PaneNavigation::default();
        assert_eq!(navigation.pane(), NavigationPane::Content);
        assert!(navigation.handle(ControllerAction::PreviousPane));
        assert_eq!(navigation.pane(), NavigationPane::Sidebar);
        assert!(navigation.handle(ControllerAction::NextPane));
        assert_eq!(navigation.pane(), NavigationPane::Content);
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
}
