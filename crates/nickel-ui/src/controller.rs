use std::time::{Duration, Instant};

use gilrs::{Axis, Button, EventType, Gilrs};

const STICK_DEAD_ZONE: f32 = 0.55;
const INITIAL_REPEAT_DELAY: Duration = Duration::from_millis(350);
const REPEAT_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LauncherControllerAction {
    Up,
    Down,
    Left,
    Right,
    Confirm,
    Cancel,
}

pub struct LauncherController {
    gilrs: Option<Gilrs>,
    stick: StickRepeater,
}

impl LauncherController {
    pub fn new() -> Self {
        let gilrs = Gilrs::new()
            .map_err(|error| tracing::warn!(%error, "controller input is unavailable"))
            .ok();
        Self {
            gilrs,
            stick: StickRepeater::default(),
        }
    }

    pub fn poll(&mut self, now: Instant) -> Vec<LauncherControllerAction> {
        let mut actions = Vec::new();
        let Some(gilrs) = &mut self.gilrs else {
            return actions;
        };
        while let Some(event) = gilrs.next_event() {
            match event.event {
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

fn button_action(button: Button) -> Option<LauncherControllerAction> {
    match button {
        Button::DPadUp => Some(LauncherControllerAction::Up),
        Button::DPadDown => Some(LauncherControllerAction::Down),
        Button::DPadLeft => Some(LauncherControllerAction::Left),
        Button::DPadRight => Some(LauncherControllerAction::Right),
        Button::South => Some(LauncherControllerAction::Confirm),
        Button::East => Some(LauncherControllerAction::Cancel),
        _ => None,
    }
}

#[derive(Default)]
struct StickRepeater {
    x: f32,
    y: f32,
    direction: Option<LauncherControllerAction>,
    next_repeat: Option<Instant>,
}

impl StickRepeater {
    fn set_x(&mut self, value: f32, now: Instant) -> Option<LauncherControllerAction> {
        self.x = value;
        self.reconcile(now)
    }

    fn set_y(&mut self, value: f32, now: Instant) -> Option<LauncherControllerAction> {
        self.y = value;
        self.reconcile(now)
    }

    fn reconcile(&mut self, now: Instant) -> Option<LauncherControllerAction> {
        let direction = stick_direction(self.x, self.y);
        if direction == self.direction {
            return None;
        }
        self.direction = direction;
        self.next_repeat = direction.map(|_| now + INITIAL_REPEAT_DELAY);
        direction
    }

    fn repeat(&mut self, now: Instant) -> Option<LauncherControllerAction> {
        let direction = self.direction?;
        let deadline = self.next_repeat?;
        if now < deadline {
            return None;
        }
        self.next_repeat = Some(now + REPEAT_INTERVAL);
        Some(direction)
    }
}

fn stick_direction(x: f32, y: f32) -> Option<LauncherControllerAction> {
    if x.abs().max(y.abs()) < STICK_DEAD_ZONE {
        return None;
    }
    if x.abs() > y.abs() {
        Some(if x.is_sign_negative() {
            LauncherControllerAction::Left
        } else {
            LauncherControllerAction::Right
        })
    } else {
        Some(if y.is_sign_negative() {
            LauncherControllerAction::Down
        } else {
            LauncherControllerAction::Up
        })
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use super::{LauncherControllerAction, StickRepeater, stick_direction};

    #[test]
    fn stick_uses_dead_zone_and_dominant_axis() {
        assert_eq!(stick_direction(0.2, 0.3), None);
        assert_eq!(
            stick_direction(0.8, 0.6),
            Some(LauncherControllerAction::Right)
        );
        assert_eq!(
            stick_direction(0.2, -0.9),
            Some(LauncherControllerAction::Down)
        );
    }

    #[test]
    fn held_stick_repeats_after_initial_delay() {
        let now = Instant::now();
        let mut stick = StickRepeater::default();
        assert_eq!(stick.set_x(0.9, now), Some(LauncherControllerAction::Right));
        assert_eq!(stick.repeat(now + Duration::from_millis(349)), None);
        assert_eq!(
            stick.repeat(now + Duration::from_millis(350)),
            Some(LauncherControllerAction::Right)
        );
        assert_eq!(stick.repeat(now + Duration::from_millis(449)), None);
        assert_eq!(
            stick.repeat(now + Duration::from_millis(450)),
            Some(LauncherControllerAction::Right)
        );
    }

    #[test]
    fn returning_to_dead_zone_rearms_the_stick() {
        let now = Instant::now();
        let mut stick = StickRepeater::default();
        assert_eq!(stick.set_y(0.8, now), Some(LauncherControllerAction::Up));
        assert_eq!(stick.set_y(0.0, now), None);
        assert_eq!(stick.set_y(0.8, now), Some(LauncherControllerAction::Up));
    }
}
