use std::time::{Duration, Instant};

use gilrs::{Axis, Button, EventType, Gilrs};

const STICK_DEAD_ZONE: f32 = 0.55;
const INITIAL_REPEAT_DELAY: Duration = Duration::from_millis(350);
const REPEAT_INTERVAL: Duration = Duration::from_millis(100);

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
    stick: StickRepeater,
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
        Self {
            gilrs,
            stick: StickRepeater::default(),
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
            match event.event {
                EventType::Connected => self.connected = true,
                EventType::Disconnected => {
                    self.connected = gilrs.gamepads().any(|(_, gamepad)| gamepad.is_connected());
                }
                EventType::ButtonPressed(button, _) | EventType::ButtonRepeated(button, _) => {
                    if let Some(action) = button_action(button) {
                        actions.push(action);
                    }
                }
                EventType::AxisChanged(Axis::LeftStickX, value, _) => {
                    actions.extend(self.stick.set_x(value, now));
                }
                EventType::AxisChanged(Axis::LeftStickY, value, _) => {
                    actions.extend(self.stick.set_y(value, now));
                }
                _ => {}
            }
        }
        if let Some(action) = self.stick.repeat(now) {
            actions.push(action);
        }
        actions
    }
}

impl Default for ControllerInput {
    fn default() -> Self {
        Self::new()
    }
}

fn button_action(button: Button) -> Option<ControllerAction> {
    match button {
        Button::DPadUp => Some(ControllerAction::Up),
        Button::DPadDown => Some(ControllerAction::Down),
        Button::DPadLeft => Some(ControllerAction::Left),
        Button::DPadRight => Some(ControllerAction::Right),
        Button::South => Some(ControllerAction::Confirm),
        Button::East => Some(ControllerAction::Cancel),
        Button::LeftTrigger => Some(ControllerAction::PreviousPane),
        Button::RightTrigger => Some(ControllerAction::NextPane),
        _ => None,
    }
}

#[derive(Default)]
struct StickRepeater {
    x: f32,
    y: f32,
    direction: Option<ControllerAction>,
    next_repeat: Option<Instant>,
}

impl StickRepeater {
    fn set_x(&mut self, value: f32, now: Instant) -> Option<ControllerAction> {
        self.x = value;
        self.reconcile(now)
    }

    fn set_y(&mut self, value: f32, now: Instant) -> Option<ControllerAction> {
        self.y = value;
        self.reconcile(now)
    }

    fn reconcile(&mut self, now: Instant) -> Option<ControllerAction> {
        let direction = stick_direction(self.x, self.y);
        if direction == self.direction {
            return None;
        }
        self.direction = direction;
        self.next_repeat = direction.map(|_| now + INITIAL_REPEAT_DELAY);
        direction
    }

    fn repeat(&mut self, now: Instant) -> Option<ControllerAction> {
        let direction = self.direction?;
        let deadline = self.next_repeat?;
        if now < deadline {
            return None;
        }
        self.next_repeat = Some(now + REPEAT_INTERVAL);
        Some(direction)
    }
}

fn stick_direction(x: f32, y: f32) -> Option<ControllerAction> {
    if x.abs().max(y.abs()) < STICK_DEAD_ZONE {
        return None;
    }
    if x.abs() > y.abs() {
        Some(if x.is_sign_negative() {
            ControllerAction::Left
        } else {
            ControllerAction::Right
        })
    } else {
        Some(if y.is_sign_negative() {
            ControllerAction::Down
        } else {
            ControllerAction::Up
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ControllerAction, NavigationPane, PaneNavigation, stick_direction};

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
    fn stick_uses_dead_zone_and_dominant_axis() {
        assert_eq!(stick_direction(0.2, 0.3), None);
        assert_eq!(stick_direction(0.8, 0.6), Some(ControllerAction::Right));
        assert_eq!(stick_direction(0.2, -0.9), Some(ControllerAction::Down));
    }
}
