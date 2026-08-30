use std::collections::{BTreeMap, BTreeSet};

use crate::{KeyEdge, NativeCode};

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ControllerId(pub u64);

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ControllerIdentity {
    pub backend: String,
    pub native: NativeCode,
    /// Stable hardware identity when a backend can provide one. It is used to
    /// prevent two active backends from delivering the same physical device.
    pub fingerprint: Option<String>,
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ControllerButton {
    South,
    East,
    West,
    North,
    DPadUp,
    DPadDown,
    DPadLeft,
    DPadRight,
    LeftShoulder,
    RightShoulder,
    LeftTrigger,
    RightTrigger,
    Select,
    Start,
    Guide,
    LeftStick,
    RightStick,
    Native(NativeCode),
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum ControllerAxis {
    LeftX,
    LeftY,
    RightX,
    RightY,
    LeftTrigger,
    RightTrigger,
    Native(NativeCode),
}

#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum AxisDirection {
    Up,
    Down,
    Left,
    Right,
}

#[derive(Clone, Debug, PartialEq)]
pub enum ControllerEvent {
    Connected {
        id: ControllerId,
        identity: ControllerIdentity,
    },
    Disconnected {
        id: ControllerId,
    },
    Button {
        id: ControllerId,
        button: ControllerButton,
        edge: KeyEdge,
        repeat: bool,
    },
    Axis {
        id: ControllerId,
        axis: ControllerAxis,
        value: f32,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ControllerSignal {
    Connected(ControllerId),
    DuplicateSuppressed(ControllerId),
    Disconnected(ControllerId),
    Button {
        id: ControllerId,
        button: ControllerButton,
        edge: KeyEdge,
        repeat: bool,
    },
    Direction {
        id: ControllerId,
        direction: AxisDirection,
        edge: KeyEdge,
        repeat: bool,
    },
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ControllerConfig {
    pub press_threshold_milli: u16,
    pub release_threshold_milli: u16,
    pub initial_repeat_ms: u64,
    pub repeat_interval_ms: u64,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            press_threshold_milli: 550,
            release_threshold_milli: 350,
            initial_repeat_ms: 350,
            repeat_interval_ms: 100,
        }
    }
}

#[derive(Clone, Debug, Default)]
struct ControllerState {
    active: bool,
    fingerprint: Option<String>,
    buttons: BTreeSet<ControllerButton>,
    left_x: f32,
    left_y: f32,
    direction: Option<AxisDirection>,
    next_repeat_ms: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct ControllerNormalizer {
    config: ControllerConfig,
    states: BTreeMap<ControllerId, ControllerState>,
    fingerprints: BTreeMap<String, ControllerId>,
}

impl ControllerNormalizer {
    pub fn new(config: ControllerConfig) -> Self {
        Self {
            config,
            states: BTreeMap::new(),
            fingerprints: BTreeMap::new(),
        }
    }

    pub fn handle(&mut self, event: ControllerEvent, now_ms: u64) -> Vec<ControllerSignal> {
        match event {
            ControllerEvent::Connected { id, identity } => {
                if identity
                    .fingerprint
                    .as_ref()
                    .is_some_and(|fingerprint| self.fingerprints.contains_key(fingerprint))
                {
                    self.states.insert(
                        id,
                        ControllerState {
                            fingerprint: identity.fingerprint,
                            ..ControllerState::default()
                        },
                    );
                    return vec![ControllerSignal::DuplicateSuppressed(id)];
                }
                if let Some(fingerprint) = &identity.fingerprint {
                    self.fingerprints.insert(fingerprint.clone(), id);
                }
                self.states.insert(
                    id,
                    ControllerState {
                        active: true,
                        fingerprint: identity.fingerprint,
                        ..ControllerState::default()
                    },
                );
                vec![ControllerSignal::Connected(id)]
            }
            ControllerEvent::Disconnected { id } => {
                let Some(state) = self.states.remove(&id) else {
                    return Vec::new();
                };
                if let Some(fingerprint) = state.fingerprint
                    && self.fingerprints.get(&fingerprint) == Some(&id)
                {
                    self.fingerprints.remove(&fingerprint);
                }
                state
                    .active
                    .then_some(ControllerSignal::Disconnected(id))
                    .into_iter()
                    .collect()
            }
            ControllerEvent::Button {
                id,
                button,
                edge,
                repeat,
            } => {
                let Some(state) = self.states.get_mut(&id).filter(|state| state.active) else {
                    return Vec::new();
                };
                let changed = match edge {
                    KeyEdge::Pressed => state.buttons.insert(button.clone()),
                    KeyEdge::Released => state.buttons.remove(&button),
                };
                if !changed && !repeat {
                    return Vec::new();
                }
                vec![ControllerSignal::Button {
                    id,
                    button,
                    edge,
                    repeat,
                }]
            }
            ControllerEvent::Axis { id, axis, value } => {
                let Some(state) = self.states.get_mut(&id).filter(|state| state.active) else {
                    return Vec::new();
                };
                match axis {
                    ControllerAxis::LeftX => state.left_x = value.clamp(-1.0, 1.0),
                    ControllerAxis::LeftY => state.left_y = value.clamp(-1.0, 1.0),
                    _ => return Vec::new(),
                }
                reconcile_direction(self.config, id, state, now_ms)
            }
        }
    }

    pub fn tick(&mut self, now_ms: u64) -> Vec<ControllerSignal> {
        let mut signals = Vec::new();
        for (id, state) in self.states.iter_mut().filter(|(_, state)| state.active) {
            let (Some(direction), Some(deadline)) = (state.direction, state.next_repeat_ms) else {
                continue;
            };
            if now_ms >= deadline {
                state.next_repeat_ms = Some(now_ms + self.config.repeat_interval_ms);
                signals.push(ControllerSignal::Direction {
                    id: *id,
                    direction,
                    edge: KeyEdge::Pressed,
                    repeat: true,
                });
            }
        }
        signals
    }
}

impl Default for ControllerNormalizer {
    fn default() -> Self {
        Self::new(ControllerConfig::default())
    }
}

fn reconcile_direction(
    config: ControllerConfig,
    id: ControllerId,
    state: &mut ControllerState,
    now_ms: u64,
) -> Vec<ControllerSignal> {
    let threshold = if state.direction.is_some() {
        config.release_threshold_milli as f32 / 1_000.0
    } else {
        config.press_threshold_milli as f32 / 1_000.0
    };
    let next = direction(state.left_x, state.left_y, threshold);
    if next == state.direction {
        return Vec::new();
    }
    let mut signals = Vec::new();
    if let Some(previous) = state.direction {
        signals.push(ControllerSignal::Direction {
            id,
            direction: previous,
            edge: KeyEdge::Released,
            repeat: false,
        });
    }
    state.direction = next;
    state.next_repeat_ms = next.map(|_| now_ms + config.initial_repeat_ms);
    if let Some(next) = next {
        signals.push(ControllerSignal::Direction {
            id,
            direction: next,
            edge: KeyEdge::Pressed,
            repeat: false,
        });
    }
    signals
}

fn direction(x: f32, y: f32, threshold: f32) -> Option<AxisDirection> {
    if x.abs().max(y.abs()) < threshold {
        return None;
    }
    if x.abs() > y.abs() {
        Some(if x.is_sign_negative() {
            AxisDirection::Left
        } else {
            AxisDirection::Right
        })
    } else {
        Some(if y.is_sign_negative() {
            AxisDirection::Down
        } else {
            AxisDirection::Up
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(fingerprint: Option<&str>) -> ControllerIdentity {
        ControllerIdentity {
            backend: "fixture".into(),
            native: NativeCode::Numeric(1),
            fingerprint: fingerprint.map(str::to_owned),
        }
    }

    #[test]
    fn axis_hysteresis_and_repeat_are_deterministic() {
        let id = ControllerId(1);
        let mut normalizer = ControllerNormalizer::default();
        normalizer.handle(
            ControllerEvent::Connected {
                id,
                identity: identity(None),
            },
            0,
        );
        assert!(
            normalizer
                .handle(
                    ControllerEvent::Axis {
                        id,
                        axis: ControllerAxis::LeftX,
                        value: 0.54,
                    },
                    1,
                )
                .is_empty()
        );
        assert!(matches!(
            normalizer
                .handle(
                    ControllerEvent::Axis {
                        id,
                        axis: ControllerAxis::LeftX,
                        value: 0.8,
                    },
                    2,
                )
                .as_slice(),
            [ControllerSignal::Direction {
                direction: AxisDirection::Right,
                repeat: false,
                ..
            }]
        ));
        assert!(
            normalizer
                .handle(
                    ControllerEvent::Axis {
                        id,
                        axis: ControllerAxis::LeftX,
                        value: 0.4,
                    },
                    3,
                )
                .is_empty()
        );
        assert!(normalizer.tick(351).is_empty());
        assert!(matches!(
            normalizer.tick(352).as_slice(),
            [ControllerSignal::Direction { repeat: true, .. }]
        ));
    }

    #[test]
    fn duplicate_backend_device_is_suppressed_and_disconnect_resets() {
        let mut normalizer = ControllerNormalizer::default();
        assert_eq!(
            normalizer.handle(
                ControllerEvent::Connected {
                    id: ControllerId(1),
                    identity: identity(Some("usb-1")),
                },
                0,
            ),
            [ControllerSignal::Connected(ControllerId(1))]
        );
        assert_eq!(
            normalizer.handle(
                ControllerEvent::Connected {
                    id: ControllerId(2),
                    identity: identity(Some("usb-1")),
                },
                1,
            ),
            [ControllerSignal::DuplicateSuppressed(ControllerId(2))]
        );
        assert!(
            normalizer
                .handle(
                    ControllerEvent::Button {
                        id: ControllerId(2),
                        button: ControllerButton::South,
                        edge: KeyEdge::Pressed,
                        repeat: false,
                    },
                    2,
                )
                .is_empty()
        );
        assert_eq!(
            normalizer.handle(
                ControllerEvent::Disconnected {
                    id: ControllerId(1),
                },
                3,
            ),
            [ControllerSignal::Disconnected(ControllerId(1))]
        );
        assert_eq!(
            normalizer.handle(
                ControllerEvent::Connected {
                    id: ControllerId(3),
                    identity: identity(Some("usb-1")),
                },
                4,
            ),
            [ControllerSignal::Connected(ControllerId(3))]
        );
    }

    #[test]
    fn held_buttons_multiple_devices_and_repeat_edges_are_isolated() {
        let mut normalizer = ControllerNormalizer::default();
        for id in [ControllerId(1), ControllerId(2)] {
            normalizer.handle(
                ControllerEvent::Connected {
                    id,
                    identity: identity(None),
                },
                0,
            );
        }
        let press = |id| ControllerEvent::Button {
            id,
            button: ControllerButton::South,
            edge: KeyEdge::Pressed,
            repeat: false,
        };
        assert_eq!(normalizer.handle(press(ControllerId(1)), 1).len(), 1);
        assert!(normalizer.handle(press(ControllerId(1)), 2).is_empty());
        assert_eq!(normalizer.handle(press(ControllerId(2)), 3).len(), 1);
        assert!(matches!(
            normalizer
                .handle(
                    ControllerEvent::Button {
                        id: ControllerId(1),
                        button: ControllerButton::South,
                        edge: KeyEdge::Pressed,
                        repeat: true,
                    },
                    4,
                )
                .as_slice(),
            [ControllerSignal::Button { repeat: true, .. }]
        ));
    }

    #[test]
    fn axis_noise_direction_changes_and_disconnect_cancel_repeat() {
        let id = ControllerId(8);
        let mut normalizer = ControllerNormalizer::default();
        normalizer.handle(
            ControllerEvent::Connected {
                id,
                identity: identity(None),
            },
            0,
        );
        for (time, value) in [(1, 0.02), (2, -0.04), (3, 0.3), (4, -0.34)] {
            assert!(
                normalizer
                    .handle(
                        ControllerEvent::Axis {
                            id,
                            axis: ControllerAxis::LeftX,
                            value,
                        },
                        time,
                    )
                    .is_empty()
            );
        }
        normalizer.handle(
            ControllerEvent::Axis {
                id,
                axis: ControllerAxis::LeftX,
                value: 0.8,
            },
            10,
        );
        let changed = normalizer.handle(
            ControllerEvent::Axis {
                id,
                axis: ControllerAxis::LeftY,
                value: 0.9,
            },
            11,
        );
        assert!(matches!(
            changed.as_slice(),
            [
                ControllerSignal::Direction {
                    direction: AxisDirection::Right,
                    edge: KeyEdge::Released,
                    ..
                },
                ControllerSignal::Direction {
                    direction: AxisDirection::Up,
                    edge: KeyEdge::Pressed,
                    ..
                }
            ]
        ));
        normalizer.handle(ControllerEvent::Disconnected { id }, 12);
        assert!(normalizer.tick(1_000).is_empty());
    }

    #[cfg(all(feature = "sdl", feature = "gilrs"))]
    #[test]
    fn sdl_and_gilrs_standard_axes_and_buttons_normalize_equivalently() {
        use gilrs::{Axis, Button};
        use sdl3::gamepad::{Axis as SdlAxis, Button as SdlButton};

        assert_eq!(
            crate::sdl::gamepad_button(SdlButton::South),
            crate::gilrs::button_kind(Button::South)
        );
        assert_eq!(
            crate::sdl::gamepad_axis(SdlAxis::LeftY),
            crate::gilrs::axis_kind(Axis::LeftStickY)
        );
        let sdl_up = crate::sdl::gamepad_axis_value(SdlAxis::LeftY, i16::MIN);
        let gilrs_up = 1.0_f32;
        assert!((sdl_up - gilrs_up).abs() < f32::EPSILON);
        let sdl_left = crate::sdl::gamepad_axis_value(SdlAxis::LeftX, i16::MIN);
        let gilrs_left = -1.0_f32;
        assert!((sdl_left - gilrs_left).abs() < f32::EPSILON);
    }
}
