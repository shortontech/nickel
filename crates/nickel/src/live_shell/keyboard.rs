use super::*;
#[cfg(target_os = "linux")]
use nickel_core::on_screen_keyboard::{
    KeyboardKey, Latch, TouchscreenPresence, VirtualModifiers, resolve_enablement,
};
#[cfg(target_os = "linux")]
use nickel_session_protocol::OnScreenKeyboardInput;
use nickel_ui::on_screen_keyboard::KeyboardEffect;

impl LiveShell {
    pub(super) fn refresh_keyboard(&mut self) -> bool {
        #[cfg(target_os = "linux")]
        {
            let Ok(mut snapshot) = platform::on_screen_keyboard_snapshot() else {
                return false;
            };
            let settings = nickel_core::optional_features::OptionalFeatureSettings::load_default();
            let overridden =
                self.keyboard_override != nickel_core::on_screen_keyboard::KeyboardOverride::None;
            let enabled = resolve_enablement(
                settings.on_screen_keyboard,
                self.keyboard_override,
                if snapshot.touchscreen_present {
                    TouchscreenPresence::Present
                } else {
                    TouchscreenPresence::Absent
                },
            )
            .enabled;
            let previous = self.keyboard_recipient.clone();
            if self.keyboard_enabled != enabled
                || snapshot.enabled != enabled
                || snapshot.generation != settings.on_screen_keyboard_generation
                || snapshot.environment_override != overridden
            {
                if platform::configure_on_screen_keyboard(
                    enabled,
                    self.keyboard_visible && enabled,
                    settings.on_screen_keyboard_generation,
                    overridden,
                    self.keyboard_dock_top,
                    self.keyboard_height,
                )
                .is_err()
                {
                    return false;
                }
                self.keyboard_enabled = enabled;
                self.keyboard_visible &= enabled;
                if let Ok(updated) = platform::on_screen_keyboard_snapshot() {
                    snapshot = updated;
                }
            }
            if self.locked && self.keyboard_visible {
                self.set_keyboard_visible(false);
            }
            self.keyboard_visible = snapshot.visible;
            self.keyboard_dock_top = snapshot.dock_top;
            self.keyboard_height = snapshot.height;
            self.keyboard_host
                .application_mut()
                .set_top_docked(snapshot.dock_top);
            if previous.as_ref().is_none_or(|old| {
                old.epoch != snapshot.epoch || old.recipient != snapshot.recipient
            }) {
                self.keyboard_gesture_leases.clear();
                self.keyboard_host
                    .application_mut()
                    .recipient_changed(snapshot.recipient.is_some());
            }
            let changed = previous.as_ref() != Some(&snapshot);
            let auto_show = enabled && !self.keyboard_visible && snapshot.auto_show_requested;
            self.keyboard_recipient = Some(snapshot);
            if auto_show {
                return self.set_keyboard_visible(true) || changed;
            }
            changed
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }

    pub fn set_keyboard_visible(&mut self, visible: bool) -> bool {
        let visible = visible && self.keyboard_enabled && !self.locked;
        let visibility_changed = self.keyboard_visible != visible;
        #[cfg(target_os = "linux")]
        if platform::configure_on_screen_keyboard(
            self.keyboard_enabled,
            visible,
            self.keyboard_recipient
                .as_ref()
                .map_or(0, |snapshot| snapshot.generation),
            self.keyboard_override != nickel_core::on_screen_keyboard::KeyboardOverride::None,
            self.keyboard_dock_top,
            self.keyboard_height,
        )
        .is_err()
        {
            return false;
        }
        self.keyboard_visible = visible;
        if !visible {
            self.keyboard_resize = None;
        }
        if visibility_changed {
            self.keyboard_gesture_leases.clear();
            self.keyboard_host
                .application_mut()
                .recipient_changed(false);
            self.keyboard_recipient = None;
        }
        self.refresh_keyboard();
        true
    }

    pub fn keyboard_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        use nickel_input::{InputEvent, KeyEdge, PointerEvent, TouchEvent};
        // The clear strip at the edge facing the app is a resize grip. Consume the
        // whole gesture so its release cannot type a key after the surface moves.
        let resize_event = match &input {
            InputEvent::Pointer(PointerEvent::Button {
                device,
                button: nickel_input::PointerButton::Primary,
                edge,
                position: Some(position),
                ..
            }) => Some((
                *device,
                None,
                position.y,
                if *edge == KeyEdge::Pressed { 0 } else { 2 },
            )),
            InputEvent::Pointer(PointerEvent::Motion {
                device, position, ..
            }) => Some((*device, None, position.y, 1)),
            InputEvent::Touch(TouchEvent::Started {
                device,
                contact,
                position,
                ..
            }) => Some((*device, Some(*contact), position.y, 0)),
            InputEvent::Touch(TouchEvent::Moved {
                device,
                contact,
                position,
                ..
            }) => Some((*device, Some(*contact), position.y, 1)),
            InputEvent::Touch(TouchEvent::Ended {
                device,
                contact,
                position,
                ..
            }) => Some((*device, Some(*contact), position.y, 2)),
            InputEvent::Touch(TouchEvent::Cancelled { .. })
            | InputEvent::FocusLost { .. }
            | InputEvent::DeviceRemoved { .. } => {
                self.keyboard_resize = None;
                None
            }
            _ => None,
        };
        if let Some((device, contact, y, phase)) = resize_event {
            let edge_distance = if self.keyboard_dock_top {
                f64::from(height) - y
            } else {
                y
            };
            if phase == 0 && self.keyboard_resize.is_none() && (0.0..20.0).contains(&edge_distance)
            {
                self.keyboard_resize = Some((device, contact, edge_distance));
                return true;
            }
            if let Some((owner, finger, offset)) = self.keyboard_resize
                && owner == device
                && finger == contact
            {
                if phase == 2 {
                    self.keyboard_resize = None;
                } else if phase == 1 {
                    let requested = if self.keyboard_dock_top {
                        y + offset
                    } else {
                        f64::from(height) - y + offset
                    };
                    if requested.is_finite() {
                        self.keyboard_height = (requested.round() as u32).clamp(248, 640);
                        self.set_keyboard_visible(true);
                    }
                }
                return true;
            }
        }
        let current = self
            .keyboard_recipient
            .as_ref()
            .map(|snapshot| snapshot.epoch);
        let epoch = match &input {
            InputEvent::Pointer(PointerEvent::Button {
                device,
                edge: KeyEdge::Pressed,
                ..
            }) => {
                if let Some(epoch) = current {
                    self.keyboard_gesture_leases.insert((*device, None), epoch);
                }
                current
            }
            InputEvent::Pointer(PointerEvent::Button {
                device,
                edge: KeyEdge::Released,
                ..
            }) => self.keyboard_gesture_leases.remove(&(*device, None)),
            InputEvent::Touch(TouchEvent::Started {
                device, contact, ..
            }) => {
                if let Some(epoch) = current {
                    self.keyboard_gesture_leases
                        .insert((*device, Some(*contact)), epoch);
                }
                current
            }
            InputEvent::Touch(TouchEvent::Ended {
                device, contact, ..
            }) => self
                .keyboard_gesture_leases
                .remove(&(*device, Some(*contact))),
            InputEvent::Touch(TouchEvent::Cancelled {
                device, contact, ..
            }) => {
                self.keyboard_gesture_leases
                    .remove(&(*device, Some(*contact)));
                None
            }
            InputEvent::FocusLost { .. } | InputEvent::DeviceRemoved { .. } => {
                self.keyboard_gesture_leases.clear();
                None
            }
            _ => current,
        };
        self.keyboard_step(
            HostBatch {
                surface_size: Some((width, height)),
                events: vec![HostEvent::Normalized {
                    input,
                    clipboard_text: None,
                }],
                ..HostBatch::default()
            },
            epoch,
        )
    }

    pub fn keyboard_controller(&mut self, action: ControllerAction) -> bool {
        if action == ControllerAction::Cancel {
            return self.set_keyboard_visible(false);
        }
        let epoch = self
            .keyboard_recipient
            .as_ref()
            .map(|snapshot| snapshot.epoch);
        self.keyboard_step(
            HostBatch {
                events: vec![HostEvent::Controller(action)],
                ..HostBatch::default()
            },
            epoch,
        )
    }

    pub(super) fn keyboard_step(&mut self, batch: HostBatch, epoch: Option<u64>) -> bool {
        // Retain the lease displayed to the user, not a fresh lease acquired after activation.
        let outcome = self.keyboard_host.step(batch);
        for effect in self.keyboard_host.application_mut().take_effects() {
            match effect {
                KeyboardEffect::ResizeBy(delta) => {
                    self.keyboard_height = self
                        .keyboard_height
                        .saturating_add_signed(delta)
                        .clamp(248, 640);
                    self.set_keyboard_visible(true);
                }
                KeyboardEffect::ToggleDock => {
                    self.keyboard_dock_top = !self.keyboard_dock_top;
                    self.set_keyboard_visible(true);
                }
                KeyboardEffect::Hide => {
                    self.set_keyboard_visible(false);
                }
                KeyboardEffect::Input { key, modifiers } => {
                    #[cfg(not(target_os = "linux"))]
                    let _ = (key, modifiers, epoch);
                    #[cfg(target_os = "linux")]
                    if let (Some(epoch), Some(input)) = (epoch, keyboard_input(key, modifiers)) {
                        #[cfg(target_os = "linux")]
                        if platform::deliver_on_screen_keyboard_input(epoch, input).is_err() {
                            self.keyboard_host
                                .application_mut()
                                .recipient_changed(false);
                            self.keyboard_recipient = None;
                        }
                    }
                }
            }
        }
        self.refresh_keyboard();
        outcome.changed
    }
}

#[cfg(target_os = "linux")]
fn keyboard_input(key: KeyboardKey, modifiers: VirtualModifiers) -> Option<OnScreenKeyboardInput> {
    use nickel_input::NamedKey::*;
    let chord = modifiers.control != Latch::Off
        || modifiers.alt != Latch::Off
        || modifiers.super_key != Latch::Off
        || modifiers.alt_gr != Latch::Off;
    let keysym = match key {
        KeyboardKey::Character { normal, shifted } => {
            if !chord {
                return Some(OnScreenKeyboardInput::Text {
                    text: if modifiers.shifted(normal.is_alphabetic()) {
                        shifted
                    } else {
                        normal
                    }
                    .to_string(),
                });
            }
            normal as u32
        }
        KeyboardKey::Named(Space) if !chord => {
            return Some(OnScreenKeyboardInput::Text { text: " ".into() });
        }
        KeyboardKey::Named(named) => match named {
            Backspace => 0xff08,
            Tab => 0xff09,
            Enter => 0xff0d,
            Escape => 0xff1b,
            Home => 0xff50,
            ArrowLeft => 0xff51,
            ArrowUp => 0xff52,
            ArrowRight => 0xff53,
            ArrowDown => 0xff54,
            PageUp => 0xff55,
            PageDown => 0xff56,
            End => 0xff57,
            Insert => 0xff63,
            Delete => 0xffff,
            Space => 0x20,
            F1 => 0xffbe,
            F2 => 0xffbf,
            F3 => 0xffc0,
            F4 => 0xffc1,
            F5 => 0xffc2,
            F6 => 0xffc3,
            F7 => 0xffc4,
            F8 => 0xffc5,
            F9 => 0xffc6,
            F10 => 0xffc7,
            F11 => 0xffc8,
            F12 => 0xffc9,
            _ => return None,
        },
        _ => return None,
    };
    let modifiers = [
        (modifiers.shift, 0xffe1),
        (modifiers.control, 0xffe3),
        (modifiers.alt, 0xffe9),
        (modifiers.super_key, 0xffeb),
        (modifiers.alt_gr, 0xfe03),
    ]
    .into_iter()
    .filter_map(|(latch, key)| (latch != Latch::Off).then_some(key))
    .collect();
    Some(OnScreenKeyboardInput::Key { keysym, modifiers })
}
