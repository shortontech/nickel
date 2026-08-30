//! Gilrs controller conversions.

use ::gilrs::{Axis, Button, Event, EventType};

use crate::{
    KeyEdge, NativeCode,
    controller::{
        ControllerAxis, ControllerButton, ControllerEvent, ControllerId, ControllerIdentity,
    },
};

pub fn event(event: &Event, identity: Option<ControllerIdentity>) -> Option<ControllerEvent> {
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
        EventType::ButtonPressed(button, _) => ControllerEvent::Button {
            id,
            button: button_kind(button),
            edge: KeyEdge::Pressed,
            repeat: false,
        },
        EventType::ButtonRepeated(button, _) => ControllerEvent::Button {
            id,
            button: button_kind(button),
            edge: KeyEdge::Pressed,
            repeat: true,
        },
        EventType::ButtonReleased(button, _) => ControllerEvent::Button {
            id,
            button: button_kind(button),
            edge: KeyEdge::Released,
            repeat: false,
        },
        EventType::AxisChanged(axis, value, _) => ControllerEvent::Axis {
            id,
            axis: axis_kind(axis),
            value,
        },
        _ => return None,
    })
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
}
