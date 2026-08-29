//! Explicitly gated nested-session input source used by live acceptance tests.
//!
//! Events produced here enter `NickelSession::process_input_event`, exactly like
//! events from winit or libinput. This module must not mutate shell state.

use std::path::PathBuf;

use nickel_session_protocol::{InputState, TestInput, TestKey, TestPointerButton};
use smithay::backend::input::{
    AbsolutePositionEvent, ButtonState, Device, DeviceCapability, Event, InputBackend, InputEvent,
    InputTime, KeyState, KeyboardKeyEvent, Keycode, PointerButtonEvent, PointerMotionAbsoluteEvent,
    PointerMotionEvent, UnusedEvent,
};

use crate::state::NickelSession;

#[derive(Debug)]
struct TestInputBackend;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct TestInputDevice;

impl Device for TestInputDevice {
    fn id(&self) -> String {
        "nickel-test-control".into()
    }

    fn name(&self) -> String {
        "Nickel nested test control".into()
    }

    fn has_capability(&self, capability: DeviceCapability) -> bool {
        matches!(
            capability,
            DeviceCapability::Keyboard | DeviceCapability::Pointer
        )
    }

    fn usb_id(&self) -> Option<(u32, u32)> {
        None
    }

    fn syspath(&self) -> Option<PathBuf> {
        None
    }
}

#[derive(Clone, Copy, Debug)]
struct TestKeyEvent {
    time: InputTime,
    key_code: u32,
    state: KeyState,
}

impl Event<TestInputBackend> for TestKeyEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> TestInputDevice {
        TestInputDevice
    }
}

impl KeyboardKeyEvent<TestInputBackend> for TestKeyEvent {
    fn key_code(&self) -> Keycode {
        (self.key_code + 8).into()
    }

    fn state(&self) -> KeyState {
        self.state
    }

    fn count(&self) -> u32 {
        u32::from(self.state == KeyState::Pressed)
    }
}

#[derive(Clone, Copy, Debug)]
struct TestPointerMotionEvent {
    time: InputTime,
    x: i32,
    y: i32,
}

impl Event<TestInputBackend> for TestPointerMotionEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> TestInputDevice {
        TestInputDevice
    }
}

impl AbsolutePositionEvent<TestInputBackend> for TestPointerMotionEvent {
    fn x(&self) -> f64 {
        f64::from(self.x)
    }

    fn y(&self) -> f64 {
        f64::from(self.y)
    }

    fn x_transformed(&self, _width: i32) -> f64 {
        f64::from(self.x)
    }

    fn y_transformed(&self, _height: i32) -> f64 {
        f64::from(self.y)
    }
}

impl PointerMotionAbsoluteEvent<TestInputBackend> for TestPointerMotionEvent {}

#[derive(Clone, Copy, Debug)]
struct TestPointerRelativeMotionEvent {
    time: InputTime,
    dx: i32,
    dy: i32,
}

impl Event<TestInputBackend> for TestPointerRelativeMotionEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> TestInputDevice {
        TestInputDevice
    }
}

impl PointerMotionEvent<TestInputBackend> for TestPointerRelativeMotionEvent {
    fn delta_x(&self) -> f64 {
        f64::from(self.dx)
    }

    fn delta_y(&self) -> f64 {
        f64::from(self.dy)
    }

    fn delta_x_unaccel(&self) -> f64 {
        f64::from(self.dx)
    }

    fn delta_y_unaccel(&self) -> f64 {
        f64::from(self.dy)
    }
}

#[derive(Clone, Copy, Debug)]
struct TestPointerButtonEvent {
    time: InputTime,
    button_code: u32,
    state: ButtonState,
}

impl Event<TestInputBackend> for TestPointerButtonEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> TestInputDevice {
        TestInputDevice
    }
}

impl PointerButtonEvent<TestInputBackend> for TestPointerButtonEvent {
    fn button_code(&self) -> u32 {
        self.button_code
    }

    fn state(&self) -> ButtonState {
        self.state
    }
}

impl InputBackend for TestInputBackend {
    type Device = TestInputDevice;
    type KeyboardKeyEvent = TestKeyEvent;
    type PointerAxisEvent = UnusedEvent;
    type PointerButtonEvent = TestPointerButtonEvent;
    type PointerMotionEvent = TestPointerRelativeMotionEvent;
    type PointerMotionAbsoluteEvent = TestPointerMotionEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = UnusedEvent;
    type TouchUpEvent = UnusedEvent;
    type TouchMotionEvent = UnusedEvent;
    type TouchCancelEvent = UnusedEvent;
    type TouchFrameEvent = UnusedEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = UnusedEvent;
}

impl NickelSession {
    pub(crate) fn inject_test_input(&mut self, input: TestInput) -> Result<(), String> {
        let time = InputTime::now();
        let event = match input {
            TestInput::Key { key, state } => InputEvent::Keyboard {
                event: TestKeyEvent {
                    time,
                    key_code: linux_key_code(key),
                    state: key_state(state),
                },
            },
            TestInput::PointerMove { x, y } => {
                if !self.point_is_on_an_output(x, y) {
                    return Err(format!("pointer position {x},{y} is outside every output"));
                }
                InputEvent::PointerMotionAbsolute {
                    event: TestPointerMotionEvent { time, x, y },
                }
            }
            TestInput::PointerMoveRelative { dx, dy } => InputEvent::PointerMotion {
                event: TestPointerRelativeMotionEvent { time, dx, dy },
            },
            TestInput::PointerButton { button, state } => InputEvent::PointerButton {
                event: TestPointerButtonEvent {
                    time,
                    button_code: pointer_button_code(button),
                    state: button_state(state),
                },
            },
        };
        let _ = self.process_input_event::<TestInputBackend>(event);
        self.display_handle
            .flush_clients()
            .map_err(|error| format!("failed to flush injected input: {error}"))?;
        Ok(())
    }

    fn point_is_on_an_output(&self, x: i32, y: i32) -> bool {
        self.space.outputs().any(|output| {
            self.space
                .output_geometry(output)
                .is_some_and(|geometry| geometry.contains((x, y)))
        })
    }
}

fn key_state(state: InputState) -> KeyState {
    match state {
        InputState::Pressed => KeyState::Pressed,
        InputState::Released => KeyState::Released,
    }
}

fn button_state(state: InputState) -> ButtonState {
    match state {
        InputState::Pressed => ButtonState::Pressed,
        InputState::Released => ButtonState::Released,
    }
}

fn linux_key_code(key: TestKey) -> u32 {
    match key {
        TestKey::A => 30,
        TestKey::C => 46,
        TestKey::P => 25,
        TestKey::Enter => 28,
        TestKey::Tab => 15,
        TestKey::LeftAlt => 56,
        TestKey::LeftShift => 42,
        TestKey::LeftControl => 29,
        TestKey::LeftMeta => 125,
        TestKey::Left => 105,
        TestKey::Right => 106,
        TestKey::F11 => 87,
        TestKey::PrintScreen => 99,
    }
}

fn pointer_button_code(button: TestPointerButton) -> u32 {
    match button {
        TestPointerButton::Left => 0x110,
        TestPointerButton::Right => 0x111,
        TestPointerButton::Middle => 0x112,
    }
}

#[cfg(test)]
mod tests {
    use nickel_session_protocol::{TestKey, TestPointerButton};

    use super::{linux_key_code, pointer_button_code};

    #[test]
    fn semantic_keys_map_to_linux_input_codes_at_the_backend_boundary() {
        assert_eq!(linux_key_code(TestKey::A), 30);
        assert_eq!(linux_key_code(TestKey::C), 46);
        assert_eq!(linux_key_code(TestKey::P), 25);
        assert_eq!(linux_key_code(TestKey::Enter), 28);
        assert_eq!(linux_key_code(TestKey::Tab), 15);
        assert_eq!(linux_key_code(TestKey::LeftAlt), 56);
        assert_eq!(linux_key_code(TestKey::LeftShift), 42);
        assert_eq!(linux_key_code(TestKey::LeftMeta), 125);
        assert_eq!(linux_key_code(TestKey::F11), 87);
        assert_eq!(linux_key_code(TestKey::PrintScreen), 99);
    }

    #[test]
    fn semantic_pointer_buttons_map_at_the_backend_boundary() {
        assert_eq!(pointer_button_code(TestPointerButton::Left), 0x110);
        assert_eq!(pointer_button_code(TestPointerButton::Right), 0x111);
        assert_eq!(pointer_button_code(TestPointerButton::Middle), 0x112);
    }
}
