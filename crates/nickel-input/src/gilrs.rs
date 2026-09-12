//! Gilrs controller conversions.

use ::gilrs::{Axis, Button, Event, EventType};

use crate::{
    KeyEdge, NativeCode,
    controller::{
        ControllerAxis, ControllerButton, ControllerEvent, ControllerId, ControllerIdentity,
    },
};

pub fn event(event: &Event, identity: Option<ControllerIdentity>) -> Option<ControllerEvent> {
    event_for_reported_name(event, identity, "")
}

/// Convert a backend event while undoing Gilrs's sideways single-Joy-Con
/// orientation. Nickel treats a connected left/right half as a vertically held
/// split controller, matching the physical orientation beside a display.
pub fn event_for_reported_name(
    event: &Event,
    identity: Option<ControllerIdentity>,
    reported_name: &str,
) -> Option<ControllerEvent> {
    let id = ControllerId(usize::from(event.id) as u64);
    Some(match event.event {
        EventType::Connected => ControllerEvent::Connected {
            id,
            identity: identity.unwrap_or_else(|| ControllerIdentity {
                backend: "gilrs".into(),
                native: NativeCode::Numeric(id.0),
                fingerprint: None,
            }),
        },
        EventType::Disconnected => ControllerEvent::Disconnected { id },
        EventType::ButtonPressed(button, code) => ControllerEvent::Button {
            id,
            button: button_kind_for_device(button, code.into_u32(), reported_name),
            edge: KeyEdge::Pressed,
            repeat: false,
        },
        EventType::ButtonRepeated(button, code) => ControllerEvent::Button {
            id,
            button: button_kind_for_device(button, code.into_u32(), reported_name),
            edge: KeyEdge::Pressed,
            repeat: true,
        },
        EventType::ButtonReleased(button, code) => ControllerEvent::Button {
            id,
            button: button_kind_for_device(button, code.into_u32(), reported_name),
            edge: KeyEdge::Released,
            repeat: false,
        },
        EventType::AxisChanged(axis, value, _) => {
            let (axis, value) = joycon_axis(reported_name, axis_kind(axis), value);
            ControllerEvent::Axis { id, axis, value }
        }
        _ => return None,
    })
}

fn button_kind_for_device(
    button: Button,
    native_code: u32,
    reported_name: &str,
) -> ControllerButton {
    #[cfg(target_os = "linux")]
    if is_left_joycon(reported_name) {
        const EV_KEY: u32 = 1 << 16;
        return match native_code {
            code if code == EV_KEY | 544 => ControllerButton::DPadUp,
            code if code == EV_KEY | 545 => ControllerButton::DPadDown,
            code if code == EV_KEY | 546 => ControllerButton::DPadLeft,
            code if code == EV_KEY | 547 => ControllerButton::DPadRight,
            _ => button_kind_with_code(button, native_code),
        };
    }
    button_kind_with_code(button, native_code)
}

fn joycon_axis(reported_name: &str, axis: ControllerAxis, value: f32) -> (ControllerAxis, f32) {
    if is_left_joycon(reported_name) {
        match axis {
            ControllerAxis::LeftX => (ControllerAxis::LeftY, -value),
            ControllerAxis::LeftY => (ControllerAxis::LeftX, value),
            other => (other, value),
        }
    } else if is_right_joycon(reported_name) {
        match axis {
            ControllerAxis::LeftX => (ControllerAxis::LeftY, value),
            ControllerAxis::LeftY => (ControllerAxis::LeftX, -value),
            other => (other, value),
        }
    } else {
        (axis, value)
    }
}

fn is_left_joycon(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("joy-con") && (name.contains("left") || name.contains("(l)"))
}

fn is_right_joycon(name: &str) -> bool {
    let name = name.to_ascii_lowercase();
    name.contains("joy-con") && (name.contains("right") || name.contains("(r)"))
}

pub fn button_kind(button: Button) -> ControllerButton {
    match button {
        Button::South => ControllerButton::South,
        Button::East => ControllerButton::East,
        Button::West => ControllerButton::West,
        Button::North => ControllerButton::North,
        Button::DPadUp => ControllerButton::DPadUp,
        Button::DPadDown => ControllerButton::DPadDown,
        Button::DPadLeft => ControllerButton::DPadLeft,
        Button::DPadRight => ControllerButton::DPadRight,
        Button::LeftTrigger => ControllerButton::LeftShoulder,
        Button::RightTrigger => ControllerButton::RightShoulder,
        Button::LeftTrigger2 => ControllerButton::LeftTrigger,
        Button::RightTrigger2 => ControllerButton::RightTrigger,
        Button::Select => ControllerButton::Select,
        Button::Start => ControllerButton::Start,
        Button::Mode => ControllerButton::Guide,
        Button::LeftThumb => ControllerButton::LeftStick,
        Button::RightThumb => ControllerButton::RightStick,
        other => ControllerButton::Native(NativeCode::Numeric(other as u16 as u64)),
    }
}

fn button_kind_with_code(button: Button, native_code: u32) -> ControllerButton {
    #[cfg(target_os = "linux")]
    {
        // Individual Joy-Con mappings in SDL_GameControllerDB omit `guide`, but
        // hid-nintendo still reports Home as EV_KEY/BTN_MODE. Preserve the
        // semantic shell action when the higher-level mapping is incomplete.
        const EV_KEY_BTN_MODE: u32 = (1 << 16) | 0x13c;
        if native_code == EV_KEY_BTN_MODE {
            return ControllerButton::Guide;
        }
    }
    button_kind(button)
}

pub fn axis_kind(axis: Axis) -> ControllerAxis {
    match axis {
        Axis::LeftStickX => ControllerAxis::LeftX,
        Axis::LeftStickY => ControllerAxis::LeftY,
        Axis::RightStickX => ControllerAxis::RightX,
        Axis::RightStickY => ControllerAxis::RightY,
        Axis::LeftZ => ControllerAxis::LeftTrigger,
        Axis::RightZ => ControllerAxis::RightTrigger,
        other => ControllerAxis::Native(NativeCode::Numeric(other as u16 as u64)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_controls_map_without_application_actions() {
        assert_eq!(button_kind(Button::South), ControllerButton::South);
        assert_eq!(axis_kind(Axis::LeftStickX), ControllerAxis::LeftX);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_btn_mode_remains_guide_when_the_device_mapping_omits_it() {
        assert_eq!(
            button_kind_with_code(Button::Unknown, (1 << 16) | 0x13c),
            ControllerButton::Guide
        );
    }

    #[test]
    fn split_joycon_axes_are_rotated_back_to_vertical_orientation() {
        assert_eq!(
            joycon_axis("Nintendo Switch Left Joy-Con", ControllerAxis::LeftX, 1.0),
            (ControllerAxis::LeftY, -1.0)
        );
        assert_eq!(
            joycon_axis("Nintendo Switch Right Joy-Con", ControllerAxis::LeftX, -1.0),
            (ControllerAxis::LeftY, -1.0)
        );
        assert_eq!(
            joycon_axis("Xbox Series Controller", ControllerAxis::LeftX, 1.0),
            (ControllerAxis::LeftX, 1.0)
        );
    }
}
