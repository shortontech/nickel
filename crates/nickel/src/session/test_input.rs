//! Synthetic backend shared by authorized remote input and gated live acceptance tests.
//!
//! Events produced here enter `NickelSession::process_input_event`, exactly like
//! events from winit or libinput. This module must not mutate shell state.

use std::path::PathBuf;

use nickel_session_protocol::{
    InputState, PointerInteraction, RecoveryTargetAction, ResolvedShellTarget, TestControllerAxis,
    TestControllerButton, TestInput, TestKey, TestPointerButton,
};
use smithay::backend::input::{
    AbsolutePositionEvent, Axis, AxisRelativeDirection, AxisSource, ButtonState, Device,
    DeviceCapability, Event, InputBackend, InputEvent, InputTime, KeyState, KeyboardKeyEvent,
    Keycode, PointerAxisEvent, PointerButtonEvent, PointerMotionAbsoluteEvent, PointerMotionEvent,
    UnusedEvent,
};
use smithay::utils::{Logical, Rectangle};

use crate::session::state::NickelSession;

#[cfg(target_os = "linux")]
pub(crate) struct TestController {
    device: evdev::uinput::VirtualDevice,
}

#[cfg(target_os = "linux")]
impl TestController {
    fn connect() -> Result<Self, String> {
        use evdev::{
            AbsInfo, AbsoluteAxisCode, AttributeSet, BusType, InputId, KeyCode, UinputAbsSetup,
            uinput::VirtualDevice,
        };

        let mut buttons = AttributeSet::<KeyCode>::new();
        for button in [
            TestControllerButton::South,
            TestControllerButton::East,
            TestControllerButton::West,
            TestControllerButton::North,
            TestControllerButton::DPadUp,
            TestControllerButton::DPadDown,
            TestControllerButton::DPadLeft,
            TestControllerButton::DPadRight,
            TestControllerButton::LeftShoulder,
            TestControllerButton::RightShoulder,
            TestControllerButton::Select,
            TestControllerButton::Start,
            TestControllerButton::Guide,
        ] {
            buttons.insert(controller_button_code(button));
        }
        let stick = AbsInfo::new(0, i16::MIN.into(), i16::MAX.into(), 128, 256, 1);
        let mut builder = VirtualDevice::builder()
            .map_err(|error| format!("cannot open /dev/uinput: {error}"))?
            .name("Nickel nested test controller")
            .input_id(InputId::new(BusType::BUS_VIRTUAL, 0x4e49, 0x434b, 1))
            .with_keys(&buttons)
            .map_err(|error| format!("cannot configure controller buttons: {error}"))?;
        for axis in [
            AbsoluteAxisCode::ABS_X,
            AbsoluteAxisCode::ABS_Y,
            AbsoluteAxisCode::ABS_RX,
            AbsoluteAxisCode::ABS_RY,
        ] {
            builder = builder
                .with_absolute_axis(&UinputAbsSetup::new(axis, stick))
                .map_err(|error| format!("cannot configure controller axis: {error}"))?;
        }
        let device = builder
            .build()
            .map_err(|error| format!("cannot create virtual controller: {error}"))?;
        Ok(Self { device })
    }

    fn button(&mut self, button: TestControllerButton, state: InputState) -> Result<(), String> {
        use evdev::{EventType, InputEvent};
        self.device
            .emit(&[InputEvent::new(
                EventType::KEY.0,
                controller_button_code(button).code(),
                i32::from(state == InputState::Pressed),
            )])
            .map_err(|error| format!("cannot emit controller button: {error}"))
    }

    fn tap(&mut self, button: TestControllerButton) -> Result<(), String> {
        self.button(button, InputState::Pressed)?;
        self.button(button, InputState::Released)
    }

    fn axis(&mut self, axis: TestControllerAxis, value: i16) -> Result<(), String> {
        use evdev::{AbsoluteAxisEvent, InputEvent};
        let event: InputEvent = *AbsoluteAxisEvent::new(controller_axis_code(axis), value.into());
        self.device
            .emit(&[event])
            .map_err(|error| format!("cannot emit controller axis: {error}"))
    }
}

#[cfg(target_os = "linux")]
fn controller_button_code(button: TestControllerButton) -> evdev::KeyCode {
    use evdev::KeyCode;
    match button {
        TestControllerButton::South => KeyCode::BTN_SOUTH,
        TestControllerButton::East => KeyCode::BTN_EAST,
        TestControllerButton::West => KeyCode::BTN_WEST,
        TestControllerButton::North => KeyCode::BTN_NORTH,
        TestControllerButton::DPadUp => KeyCode::BTN_DPAD_UP,
        TestControllerButton::DPadDown => KeyCode::BTN_DPAD_DOWN,
        TestControllerButton::DPadLeft => KeyCode::BTN_DPAD_LEFT,
        TestControllerButton::DPadRight => KeyCode::BTN_DPAD_RIGHT,
        TestControllerButton::LeftShoulder => KeyCode::BTN_TL,
        TestControllerButton::RightShoulder => KeyCode::BTN_TR,
        TestControllerButton::Select => KeyCode::BTN_SELECT,
        TestControllerButton::Start => KeyCode::BTN_START,
        TestControllerButton::Guide => KeyCode::BTN_MODE,
    }
}

#[cfg(target_os = "linux")]
fn controller_axis_code(axis: TestControllerAxis) -> evdev::AbsoluteAxisCode {
    use evdev::AbsoluteAxisCode;
    match axis {
        TestControllerAxis::LeftX => AbsoluteAxisCode::ABS_X,
        TestControllerAxis::LeftY => AbsoluteAxisCode::ABS_Y,
        TestControllerAxis::RightX => AbsoluteAxisCode::ABS_RX,
        TestControllerAxis::RightY => AbsoluteAxisCode::ABS_RY,
    }
}

#[derive(Debug)]
struct SyntheticInputBackend;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct SyntheticInputDevice;

impl Device for SyntheticInputDevice {
    fn id(&self) -> String {
        "nickel-synthetic-input".into()
    }

    fn name(&self) -> String {
        "Nickel synthetic input".into()
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
struct SyntheticKeyEvent {
    time: InputTime,
    key_code: u32,
    state: KeyState,
}

impl Event<SyntheticInputBackend> for SyntheticKeyEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> SyntheticInputDevice {
        SyntheticInputDevice
    }
}

impl KeyboardKeyEvent<SyntheticInputBackend> for SyntheticKeyEvent {
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
struct SyntheticPointerMotionEvent {
    time: InputTime,
    x: i32,
    y: i32,
}

impl Event<SyntheticInputBackend> for SyntheticPointerMotionEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> SyntheticInputDevice {
        SyntheticInputDevice
    }
}

impl AbsolutePositionEvent<SyntheticInputBackend> for SyntheticPointerMotionEvent {
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

impl PointerMotionAbsoluteEvent<SyntheticInputBackend> for SyntheticPointerMotionEvent {}

#[derive(Clone, Copy, Debug)]
struct SyntheticTouchEvent {
    time: InputTime,
    slot: u32,
    x: i32,
    y: i32,
}
impl Event<SyntheticInputBackend> for SyntheticTouchEvent {
    fn time(&self) -> InputTime {
        self.time
    }
    fn device(&self) -> SyntheticInputDevice {
        SyntheticInputDevice
    }
}
impl AbsolutePositionEvent<SyntheticInputBackend> for SyntheticTouchEvent {
    fn x(&self) -> f64 {
        f64::from(self.x)
    }
    fn y(&self) -> f64 {
        f64::from(self.y)
    }
    fn x_transformed(&self, _: i32) -> f64 {
        self.x()
    }
    fn y_transformed(&self, _: i32) -> f64 {
        self.y()
    }
}
impl smithay::backend::input::TouchEvent<SyntheticInputBackend> for SyntheticTouchEvent {
    fn slot(&self) -> smithay::backend::input::TouchSlot {
        Some(self.slot).into()
    }
}
impl smithay::backend::input::TouchDownEvent<SyntheticInputBackend> for SyntheticTouchEvent {}
impl smithay::backend::input::TouchMotionEvent<SyntheticInputBackend> for SyntheticTouchEvent {}
impl smithay::backend::input::TouchUpEvent<SyntheticInputBackend> for SyntheticTouchEvent {}
impl smithay::backend::input::TouchCancelEvent<SyntheticInputBackend> for SyntheticTouchEvent {}
impl smithay::backend::input::TouchFrameEvent<SyntheticInputBackend> for SyntheticTouchEvent {}

#[derive(Clone, Copy, Debug)]
struct SyntheticPointerRelativeMotionEvent {
    time: InputTime,
    dx: i32,
    dy: i32,
}

impl Event<SyntheticInputBackend> for SyntheticPointerRelativeMotionEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> SyntheticInputDevice {
        SyntheticInputDevice
    }
}

impl PointerMotionEvent<SyntheticInputBackend> for SyntheticPointerRelativeMotionEvent {
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
struct SyntheticPointerButtonEvent {
    time: InputTime,
    button_code: u32,
    state: ButtonState,
}

fn visible_point_in(
    geometry: Rectangle<i32, Logical>,
    mut owns_point: impl FnMut(i32, i32) -> bool,
) -> Option<(i32, i32)> {
    if geometry.size.w <= 0 || geometry.size.h <= 0 {
        return None;
    }
    fn samples(length: i32) -> Vec<i32> {
        let center = length / 2;
        let mut samples = (0..length).step_by(8).collect::<Vec<_>>();
        if samples.last().copied() != Some(length - 1) {
            samples.push(length - 1);
        }
        samples.sort_unstable_by_key(|sample| (sample - center).abs());
        samples
    }
    let xs = samples(geometry.size.w);
    let ys = samples(geometry.size.h);
    for local_y in ys {
        for &local_x in &xs {
            let x = geometry.loc.x + local_x;
            let y = geometry.loc.y + local_y;
            if owns_point(x, y) {
                return Some((x, y));
            }
        }
    }
    None
}

impl Event<SyntheticInputBackend> for SyntheticPointerButtonEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> SyntheticInputDevice {
        SyntheticInputDevice
    }
}

impl PointerButtonEvent<SyntheticInputBackend> for SyntheticPointerButtonEvent {
    fn button_code(&self) -> u32 {
        self.button_code
    }

    fn state(&self) -> ButtonState {
        self.state
    }
}

#[derive(Clone, Copy, Debug)]
struct SyntheticPointerAxisEvent {
    time: InputTime,
    horizontal_v120: i32,
    vertical_v120: i32,
}

impl Event<SyntheticInputBackend> for SyntheticPointerAxisEvent {
    fn time(&self) -> InputTime {
        self.time
    }

    fn device(&self) -> SyntheticInputDevice {
        SyntheticInputDevice
    }
}

impl PointerAxisEvent<SyntheticInputBackend> for SyntheticPointerAxisEvent {
    fn amount(&self, _axis: Axis) -> Option<f64> {
        None
    }

    fn amount_v120(&self, axis: Axis) -> Option<f64> {
        Some(f64::from(match axis {
            Axis::Horizontal => self.horizontal_v120,
            Axis::Vertical => self.vertical_v120,
        }))
    }

    fn source(&self) -> AxisSource {
        AxisSource::Wheel
    }

    fn relative_direction(&self, _axis: Axis) -> AxisRelativeDirection {
        AxisRelativeDirection::Identical
    }
}

impl InputBackend for SyntheticInputBackend {
    type Device = SyntheticInputDevice;
    type KeyboardKeyEvent = SyntheticKeyEvent;
    type PointerAxisEvent = SyntheticPointerAxisEvent;
    type PointerButtonEvent = SyntheticPointerButtonEvent;
    type PointerMotionEvent = SyntheticPointerRelativeMotionEvent;
    type PointerMotionAbsoluteEvent = SyntheticPointerMotionEvent;
    type GestureSwipeBeginEvent = UnusedEvent;
    type GestureSwipeUpdateEvent = UnusedEvent;
    type GestureSwipeEndEvent = UnusedEvent;
    type GesturePinchBeginEvent = UnusedEvent;
    type GesturePinchUpdateEvent = UnusedEvent;
    type GesturePinchEndEvent = UnusedEvent;
    type GestureHoldBeginEvent = UnusedEvent;
    type GestureHoldEndEvent = UnusedEvent;
    type TouchDownEvent = SyntheticTouchEvent;
    type TouchUpEvent = SyntheticTouchEvent;
    type TouchMotionEvent = SyntheticTouchEvent;
    type TouchCancelEvent = SyntheticTouchEvent;
    type TouchFrameEvent = SyntheticTouchEvent;
    type TabletToolAxisEvent = UnusedEvent;
    type TabletToolProximityEvent = UnusedEvent;
    type TabletToolTipEvent = UnusedEvent;
    type TabletToolButtonEvent = UnusedEvent;
    type SwitchToggleEvent = UnusedEvent;
    type SpecialEvent = UnusedEvent;
}

impl NickelSession {
    pub(crate) fn press_controlled_pointer(
        &mut self,
        button: nickel_remote_control::pointer::PointerButton,
    ) -> Result<(), String> {
        self.controlled_pointer_button(button, ButtonState::Pressed);
        self.display_handle
            .flush_clients()
            .map_err(|_| "could not flush pointer input".into())
    }

    pub(crate) fn release_controlled_pointer(
        &mut self,
        button: nickel_remote_control::pointer::PointerButton,
    ) {
        // A client may replace its ordinary click grab with drag-and-drop or a
        // shell move/resize grab. Cancel that replacement before releasing; a
        // cancellation must not become a file drop or finish an unauthorized move.
        if let Some(pointer) = self.seat.get_pointer()
            && pointer
                .with_grab(|_, grab| !grab.is::<smithay::input::pointer::ClickGrab<Self>>())
                .unwrap_or(false)
        {
            pointer.unset_grab(
                self,
                smithay::utils::SERIAL_COUNTER.next_serial(),
                InputTime::now(),
            );
        }
        self.controlled_pointer_button(button, ButtonState::Released);
        let _ = self.display_handle.flush_clients();
    }

    fn controlled_pointer_button(
        &mut self,
        button: nickel_remote_control::pointer::PointerButton,
        state: ButtonState,
    ) {
        use nickel_remote_control::pointer::PointerButton;
        let button_code = match button {
            PointerButton::Left => 0x110,
            PointerButton::Right => 0x111,
            PointerButton::Middle => 0x112,
        };
        let previous = self.remote_input_dispatching;
        self.remote_input_dispatching = true;
        self.process_input_event::<SyntheticInputBackend>(InputEvent::PointerButton {
            event: SyntheticPointerButtonEvent {
                time: InputTime::now(),
                button_code,
                state,
            },
        });
        self.remote_input_dispatching = previous;
    }

    /// Caller owns remote authorization and validates the final hit target before each action.
    pub(crate) fn inject_controlled_pointer(
        &mut self,
        target: &nickel_remote_control::pointer::PointerTarget,
        x: i32,
        y: i32,
        action: nickel_remote_control::pointer::PointerAction,
    ) -> Result<(), String> {
        use nickel_remote_control::pointer::{PointerAction, PointerButton};
        action.validate()?;
        if !self.point_is_on_an_output(x, y) {
            return Err("pointer target is outside all outputs".into());
        }
        let origin = self
            .space
            .outputs()
            .next()
            .and_then(|output| self.space.output_geometry(output))
            .ok_or("no output is available")?
            .loc;
        let event = SyntheticPointerMotionEvent {
            time: InputTime::now(),
            x: x.checked_sub(origin.x)
                .ok_or("pointer coordinate overflow")?,
            y: y.checked_sub(origin.y)
                .ok_or("pointer coordinate overflow")?,
        };
        self.process_input_event::<SyntheticInputBackend>(InputEvent::PointerMotionAbsolute {
            event,
        });
        let pointer = self.seat.get_pointer().ok_or("pointer unavailable")?;
        let location = pointer.current_location();
        if location.x != f64::from(x) || location.y != f64::from(y) {
            return Err("pointer constraint prevented the requested position".into());
        }
        if !self.remote_pointer_target_matches(target, x, y) {
            return Err("pointer target changed during motion".into());
        }
        match action {
            PointerAction::DragStart { .. }
            | PointerAction::DragMove
            | PointerAction::DragEnd
            | PointerAction::DragCancel => return Err("drag requires gesture ownership".into()),
            PointerAction::Move => {}
            PointerAction::Click { button } | PointerAction::DoubleClick { button } => {
                let button_code = match button {
                    PointerButton::Left => 0x110,
                    PointerButton::Right => 0x111,
                    PointerButton::Middle => 0x112,
                };
                let clicks = if matches!(action, PointerAction::DoubleClick { .. }) {
                    2
                } else {
                    1
                };
                // Complete each press/release in the same owner dispatch. No remotely held
                // button survives either a successful click or a rejected second target.
                for _ in 0..clicks {
                    if !self.remote_pointer_target_matches(target, x, y) {
                        return Err("pointer target changed between clicks".into());
                    }
                    for state in [ButtonState::Pressed, ButtonState::Released] {
                        self.process_input_event::<SyntheticInputBackend>(
                            InputEvent::PointerButton {
                                event: SyntheticPointerButtonEvent {
                                    time: InputTime::now(),
                                    button_code,
                                    state,
                                },
                            },
                        );
                    }
                }
            }
            PointerAction::Scroll {
                horizontal_v120,
                vertical_v120,
            } => {
                self.process_input_event::<SyntheticInputBackend>(InputEvent::PointerAxis {
                    event: SyntheticPointerAxisEvent {
                        time: InputTime::now(),
                        horizontal_v120,
                        vertical_v120,
                    },
                });
            }
        }
        self.display_handle
            .flush_clients()
            .map_err(|error| format!("could not flush pointer input: {error}"))
    }

    pub(crate) fn inject_test_input(&mut self, input: TestInput) -> Result<(), String> {
        #[cfg(target_os = "linux")]
        match &input {
            TestInput::ControllerConnect => {
                if self.test_controller.is_none() {
                    self.test_controller = Some(TestController::connect()?);
                }
                return Ok(());
            }
            TestInput::ControllerDisconnect => {
                self.test_controller = None;
                return Ok(());
            }
            TestInput::ControllerButton { button, state } => {
                return self
                    .test_controller
                    .as_mut()
                    .ok_or_else(|| "test controller is not connected".to_owned())?
                    .button(*button, *state);
            }
            TestInput::ControllerTap { button } => {
                return self
                    .test_controller
                    .as_mut()
                    .ok_or_else(|| "test controller is not connected".to_owned())?
                    .tap(*button);
            }
            TestInput::ControllerAxis { axis, value } => {
                return self
                    .test_controller
                    .as_mut()
                    .ok_or_else(|| "test controller is not connected".to_owned())?
                    .axis(*axis, *value);
            }
            _ => {}
        }
        #[cfg(not(target_os = "linux"))]
        if matches!(
            input,
            TestInput::ControllerConnect
                | TestInput::ControllerDisconnect
                | TestInput::ControllerButton { .. }
                | TestInput::ControllerTap { .. }
                | TestInput::ControllerAxis { .. }
        ) {
            return Err("virtual controller test input is available only on Linux".into());
        }
        let time = InputTime::now();
        let event = match input {
            TestInput::TouchDown { slot, x, y } | TestInput::TouchMotion { slot, x, y } => {
                if slot > 31 || !self.point_is_on_an_output(x, y) {
                    return Err("invalid test touch slot or position".into());
                }
                let event = SyntheticTouchEvent { time, slot, x, y };
                if matches!(input, TestInput::TouchDown { .. }) {
                    InputEvent::TouchDown { event }
                } else {
                    InputEvent::TouchMotion { event }
                }
            }
            TestInput::TouchUp { slot } | TestInput::TouchCancel { slot } => {
                if slot > 31 {
                    return Err("invalid test touch slot".into());
                }
                let event = SyntheticTouchEvent {
                    time,
                    slot,
                    x: 0,
                    y: 0,
                };
                if matches!(input, TestInput::TouchUp { .. }) {
                    InputEvent::TouchUp { event }
                } else {
                    InputEvent::TouchCancel { event }
                }
            }
            TestInput::TouchFrame => InputEvent::TouchFrame {
                event: SyntheticTouchEvent {
                    time,
                    slot: 0,
                    x: 0,
                    y: 0,
                },
            },
            TestInput::Key { key, state } => InputEvent::Keyboard {
                event: SyntheticKeyEvent {
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
                    event: SyntheticPointerMotionEvent { time, x, y },
                }
            }
            TestInput::PointerMoveRelative { dx, dy } => InputEvent::PointerMotion {
                event: SyntheticPointerRelativeMotionEvent { time, dx, dy },
            },
            TestInput::PointerButton { button, state } => InputEvent::PointerButton {
                event: SyntheticPointerButtonEvent {
                    time,
                    button_code: pointer_button_code(button),
                    state: button_state(state),
                },
            },
            TestInput::PointerAxis {
                horizontal_v120,
                vertical_v120,
            } => InputEvent::PointerAxis {
                event: SyntheticPointerAxisEvent {
                    time,
                    horizontal_v120,
                    vertical_v120,
                },
            },
            TestInput::ShellPointer { target } => return self.inject_shell_pointer(target),
            TestInput::RecoveryPointer { action, output } => {
                return self.inject_recovery_pointer(action, output.as_deref());
            }
            TestInput::WindowPointer {
                window,
                interaction,
            } => return self.inject_window_pointer(window, interaction),
            TestInput::ControllerConnect
            | TestInput::ControllerDisconnect
            | TestInput::ControllerButton { .. }
            | TestInput::ControllerTap { .. }
            | TestInput::ControllerAxis { .. } => unreachable!(),
        };
        let _ = self.process_input_event::<SyntheticInputBackend>(event);
        self.display_handle
            .flush_clients()
            .map_err(|error| format!("failed to flush injected input: {error}"))?;
        Ok(())
    }

    fn inject_shell_pointer(&mut self, target: ResolvedShellTarget) -> Result<(), String> {
        let surface = self
            .protocol_shell_surfaces()
            .into_iter()
            .find(|surface| {
                surface.role == target.role
                    && target
                        .output
                        .as_ref()
                        .is_none_or(|output| surface.output.as_ref() == Some(output))
            })
            .ok_or_else(|| format!("shell surface {:?} is not mapped", target.role))?;
        let geometry = surface
            .geometry
            .ok_or_else(|| format!("shell surface {:?} has no geometry", target.role))?;
        if target.x < 0 || target.y < 0 || target.x >= geometry.width || target.y >= geometry.height
        {
            return Err(format!(
                "shell-local target {},{} is outside {:?} geometry {}x{}",
                target.x, target.y, target.role, geometry.width, geometry.height
            ));
        }
        let x = geometry.x + target.x;
        let y = geometry.y + target.y;
        let current = self
            .seat
            .get_pointer()
            .map(|pointer| pointer.current_location());
        if geometry.width > 1
            && current.is_some_and(|current| {
                current.x.round() as i32 == x && current.y.round() as i32 == y
            })
        {
            // An absolute backend may coalesce a move to its current point.
            // Approach from another valid point on the same shell surface so
            // repeated semantic hovers still traverse ordinary hit testing.
            let approach_x = if target.x + 1 < geometry.width {
                x + 1
            } else {
                x - 1
            };
            self.inject_test_input(TestInput::PointerMove { x: approach_x, y })?;
        }
        self.inject_test_input(TestInput::PointerMove { x, y })?;
        let (button, click_count) = match target.interaction {
            PointerInteraction::Hover => return Ok(()),
            PointerInteraction::LeftClick => (TestPointerButton::Left, 1),
            PointerInteraction::LeftDoubleClick => (TestPointerButton::Left, 2),
            PointerInteraction::RightClick => (TestPointerButton::Right, 1),
            PointerInteraction::LeftPress => {
                return self.inject_test_input(TestInput::PointerButton {
                    button: TestPointerButton::Left,
                    state: InputState::Pressed,
                });
            }
            PointerInteraction::LeftRelease => {
                return self.inject_test_input(TestInput::PointerButton {
                    button: TestPointerButton::Left,
                    state: InputState::Released,
                });
            }
        };
        for _ in 0..click_count {
            self.inject_test_input(TestInput::PointerButton {
                button,
                state: InputState::Pressed,
            })?;
            self.inject_test_input(TestInput::PointerButton {
                button,
                state: InputState::Released,
            })?;
        }
        Ok(())
    }

    fn inject_recovery_pointer(
        &mut self,
        action: RecoveryTargetAction,
        output_name: Option<&str>,
    ) -> Result<(), String> {
        if !self.shell_recovery_visible() {
            return Err("compositor recovery is not visible".into());
        }
        self.space
            .outputs()
            .find(|output| output_name.is_none_or(|name| output.name() == name))
            .ok_or_else(|| match output_name {
                Some(name) => format!("unknown output {name:?}"),
                None => "session has no output".into(),
            })?;
        let action = match action {
            RecoveryTargetAction::Retry => crate::session::recovery_ui::RecoveryAction::Retry,
            RecoveryTargetAction::Exit => crate::session::recovery_ui::RecoveryAction::Exit,
        };
        let action = self
            .recovery_ui
            .activate(action)
            .ok_or("recovery semantic target did not activate")?;
        self.apply_recovery_action(action);
        Ok(())
    }

    fn inject_window_pointer(
        &mut self,
        window: nickel_session_protocol::WindowId,
        interaction: PointerInteraction,
    ) -> Result<(), String> {
        let window = self
            .window_for_registry_id(crate::session::window_registry::WindowId(window.0))
            .ok_or("managed window is not mapped")?;
        let geometry = self
            .space
            .element_bbox(&window)
            .ok_or("managed window has no production geometry")?;
        let (x, y) = visible_point_in(geometry, |x, y| {
            self.space
                .element_under((f64::from(x), f64::from(y)))
                .is_some_and(|(candidate, _)| candidate == &window)
        })
        .ok_or("managed window has no visible production input point")?;
        self.inject_test_input(TestInput::PointerMove { x, y })?;
        let (button, click_count) = match interaction {
            PointerInteraction::Hover => return Ok(()),
            PointerInteraction::LeftClick => (TestPointerButton::Left, 1),
            PointerInteraction::LeftDoubleClick => (TestPointerButton::Left, 2),
            PointerInteraction::RightClick => (TestPointerButton::Right, 1),
            PointerInteraction::LeftPress => {
                return self.inject_test_input(TestInput::PointerButton {
                    button: TestPointerButton::Left,
                    state: InputState::Pressed,
                });
            }
            PointerInteraction::LeftRelease => {
                return self.inject_test_input(TestInput::PointerButton {
                    button: TestPointerButton::Left,
                    state: InputState::Released,
                });
            }
        };
        for _ in 0..click_count {
            self.inject_test_input(TestInput::PointerButton {
                button,
                state: InputState::Pressed,
            })?;
            self.inject_test_input(TestInput::PointerButton {
                button,
                state: InputState::Released,
            })?;
        }
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
        TestKey::V => 47,
        TestKey::X => 45,
        TestKey::Enter => 28,
        TestKey::Escape => 1,
        TestKey::Tab => 15,
        TestKey::LeftAlt => 56,
        TestKey::LeftShift => 42,
        TestKey::LeftControl => 29,
        TestKey::LeftMeta => 125,
        TestKey::Left => 105,
        TestKey::Right => 106,
        TestKey::Up => 103,
        TestKey::Down => 108,
        TestKey::Space => 57,
        TestKey::Backspace => 14,
        TestKey::Delete => 111,
        TestKey::F11 => 87,
        TestKey::PrintScreen => 99,
        TestKey::VolumeMute => 113,
        TestKey::VolumeDown => 114,
        TestKey::VolumeUp => 115,
        TestKey::MediaNext => 163,
        TestKey::MediaPlayPause => 164,
        TestKey::MediaPrevious => 165,
        TestKey::MediaStop => 166,
        TestKey::MediaRewind => 168,
        TestKey::MediaPause => 201,
        TestKey::MediaPlay => 207,
        TestKey::MediaFastForward => 208,
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
    use smithay::backend::input::{Axis, AxisRelativeDirection, AxisSource, PointerAxisEvent};
    use smithay::utils::Rectangle;

    use super::{
        SyntheticInputBackend, SyntheticPointerAxisEvent, linux_key_code, pointer_button_code,
        visible_point_in,
    };

    #[test]
    fn semantic_keys_map_to_linux_input_codes_at_the_backend_boundary() {
        assert_eq!(linux_key_code(TestKey::A), 30);
        assert_eq!(linux_key_code(TestKey::C), 46);
        assert_eq!(linux_key_code(TestKey::P), 25);
        assert_eq!(linux_key_code(TestKey::V), 47);
        assert_eq!(linux_key_code(TestKey::X), 45);
        assert_eq!(linux_key_code(TestKey::Enter), 28);
        assert_eq!(linux_key_code(TestKey::Escape), 1);
        assert_eq!(linux_key_code(TestKey::Tab), 15);
        assert_eq!(linux_key_code(TestKey::LeftAlt), 56);
        assert_eq!(linux_key_code(TestKey::LeftShift), 42);
        assert_eq!(linux_key_code(TestKey::LeftMeta), 125);
        assert_eq!(linux_key_code(TestKey::Up), 103);
        assert_eq!(linux_key_code(TestKey::Down), 108);
        assert_eq!(linux_key_code(TestKey::Space), 57);
        assert_eq!(linux_key_code(TestKey::Backspace), 14);
        assert_eq!(linux_key_code(TestKey::Delete), 111);
        assert_eq!(linux_key_code(TestKey::F11), 87);
        assert_eq!(linux_key_code(TestKey::PrintScreen), 99);
    }

    #[test]
    fn semantic_pointer_buttons_map_at_the_backend_boundary() {
        assert_eq!(pointer_button_code(TestPointerButton::Left), 0x110);
        assert_eq!(pointer_button_code(TestPointerButton::Right), 0x111);
        assert_eq!(pointer_button_code(TestPointerButton::Middle), 0x112);
    }

    #[test]
    fn wheel_deltas_map_to_v120_axes_at_the_backend_boundary() {
        let event = SyntheticPointerAxisEvent {
            time: smithay::backend::input::InputTime::now(),
            horizontal_v120: 120,
            vertical_v120: -240,
        };
        assert_eq!(
            <SyntheticPointerAxisEvent as PointerAxisEvent<SyntheticInputBackend>>::amount_v120(
                &event,
                Axis::Horizontal
            ),
            Some(120.0)
        );
        assert_eq!(
            <SyntheticPointerAxisEvent as PointerAxisEvent<SyntheticInputBackend>>::amount_v120(
                &event,
                Axis::Vertical
            ),
            Some(-240.0)
        );
        assert_eq!(event.source(), AxisSource::Wheel);
        assert_eq!(
            event.relative_direction(Axis::Vertical),
            AxisRelativeDirection::Identical
        );
    }

    #[test]
    fn semantic_window_point_selects_a_visible_sliver_instead_of_an_occluded_center() {
        let geometry = Rectangle::new((100, 200).into(), (100, 80).into());
        let point = visible_point_in(geometry, |x, _| x < 108);
        assert!(point.is_some_and(|(x, y)| (100..108).contains(&x) && (200..280).contains(&y)));
        assert_eq!(visible_point_in(geometry, |_, _| false), None);
    }
}
