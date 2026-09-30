use super::*;
use nickel_core::on_screen_keyboard::{
    KeyboardKey, Latch, TouchscreenPresence, VirtualModifiers, resolve_enablement,
};
use nickel_session_protocol::OnScreenKeyboardInput;
use nickel_ui::on_screen_keyboard::KeyboardEffect;

impl LiveShell {
    pub(crate) fn keyboard_placement_preferences(&self) -> (bool, u32) {
        (self.keyboard_dock_top, self.keyboard_height)
    }
    pub(super) fn keyboard_service_snapshot(&self) -> serde_json::Value {
        let rows = if self.keyboard_visible {
            self.keyboard_host.application().display_rows(true)
        } else {
            Vec::new()
        };
        serde_json::json!({
            "available": self.keyboard_enabled && self.keyboard_visible && !self.locked && self.active_shell_declares("keyboard"),
            "generation": self.keyboard_service_generation,
            "height": self.keyboard_height,
            "dockTop": self.keyboard_dock_top,
            "recipientAvailable": self.keyboard_recipient.as_ref().is_some_and(|snapshot| snapshot.has_recipient()),
            "rows": rows,
        })
    }

    pub(super) fn apply_keyboard_plugin_effect(
        &mut self,
        effect: crate::plugin_panel::PluginEffect,
        epoch: Option<u64>,
    ) -> bool {
        use crate::plugin_panel::PluginEffect;
        use nickel_ui::on_screen_keyboard::KeyboardMessage;
        let PluginEffect::Keyboard { plugin_id, effect } = effect else {
            return false;
        };
        if !self.public_native_granted(
            &plugin_id,
            nickel_core::plugins::PluginCapability::OnScreenKeyboardInput,
        ) || effect.validate(&self.keyboard_service_snapshot()).is_err()
            || !self.plugin_surface_matches(&self.active_shell_surface_key("keyboard"))
        {
            return false;
        }
        let valid_epoch = epoch.filter(|epoch| {
            self.keyboard_recipient
                .as_ref()
                .is_some_and(|snapshot| snapshot.has_recipient() && snapshot.epoch == *epoch)
        });
        match effect.operation.as_str() {
            "keyboard.press" => {
                if valid_epoch.is_none()
                    || !self
                        .keyboard_host
                        .application_mut()
                        .press_display_key(true, effect.id.as_deref().unwrap_or(""))
                {
                    return false;
                }
            }
            "keyboard.hide" => self
                .keyboard_host
                .application_mut()
                .update(KeyboardMessage::Hide),
            "keyboard.toggleDock" => self
                .keyboard_host
                .application_mut()
                .update(KeyboardMessage::ToggleDock),
            "keyboard.holdModifiers" => self
                .keyboard_host
                .application_mut()
                .update(KeyboardMessage::PersistentModifiers),
            "keyboard.resize" => self
                .keyboard_host
                .application_mut()
                .update(KeyboardMessage::ResizeBy(effect.delta.unwrap_or(0))),
            _ => return false,
        }
        self.keyboard_service_generation = self.keyboard_service_generation.saturating_add(1);
        self.keyboard_step(
            HostBatch {
                application_changed: true,
                ..HostBatch::default()
            },
            valid_epoch,
        );
        true
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn apply_windows_keyboard_settings(
        &mut self,
        settings: &nickel_core::optional_features::OptionalFeatureSettings,
    ) -> bool {
        use nickel_core::on_screen_keyboard::{TouchscreenPresence, resolve_enablement};

        let enabled = resolve_enablement(
            settings.on_screen_keyboard,
            self.keyboard_override,
            if self.keyboard_touchscreen_present {
                TouchscreenPresence::Present
            } else {
                TouchscreenPresence::Absent
            },
        )
        .enabled;
        let changed = self.keyboard_generation != settings.on_screen_keyboard_generation
            || self.keyboard_enabled != enabled;
        self.keyboard_generation = settings.on_screen_keyboard_generation;
        self.keyboard_enabled = enabled;
        if !enabled {
            self.keyboard_visible = false;
            self.keyboard_resize = None;
            self.keyboard_gesture_leases.clear();
            self.keyboard_recipient = None;
            self.keyboard_host
                .application_mut()
                .recipient_changed(false);
        }
        changed
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn windows_keyboard_snapshot(
        &self,
    ) -> nickel_session_protocol::OnScreenKeyboardSnapshot {
        nickel_session_protocol::OnScreenKeyboardSnapshot {
            generation: self.keyboard_generation,
            enabled: self.keyboard_enabled,
            visible: self.keyboard_visible,
            touchscreen_present: self.keyboard_touchscreen_present,
            environment_override: self.keyboard_override
                != nickel_core::on_screen_keyboard::KeyboardOverride::None,
            dock_top: self.keyboard_dock_top,
            height: self.keyboard_height,
            ..Default::default()
        }
    }

    pub(super) fn refresh_keyboard(&mut self) -> bool {
        #[cfg(target_os = "linux")]
        {
            let Ok(mut snapshot) = self.session_host.keyboard_snapshot() else {
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
                if self
                    .session_host
                    .configure_keyboard(
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
                if let Ok(updated) = self.session_host.keyboard_snapshot() {
                    snapshot = updated;
                }
            }
            if self.locked && self.keyboard_visible {
                self.set_keyboard_visible(false);
            }
            if snapshot.visible && (!self.active_shell_declares("keyboard") || self.locked) {
                self.set_keyboard_visible(false);
                snapshot.visible = false;
            }
            self.keyboard_visible = snapshot.visible;
            if self.active_shell_declares("keyboard") {
                self.set_default_shell_surface_visible("keyboard", self.keyboard_visible);
            }
            self.keyboard_dock_top = snapshot.dock_top;
            self.keyboard_height = snapshot.height;
            self.keyboard_host
                .application_mut()
                .set_top_docked(snapshot.dock_top);
            if previous.as_ref().is_none_or(|old| {
                old.epoch != snapshot.epoch
                    || old.recipient != snapshot.recipient
                    || old.internal_recipient != snapshot.internal_recipient
            }) {
                self.keyboard_gesture_leases.clear();
                self.keyboard_resize = None;
                self.keyboard_host
                    .application_mut()
                    .recipient_changed(snapshot.has_recipient());
            }
            let changed = previous.as_ref() != Some(&snapshot);
            if changed {
                self.keyboard_service_generation =
                    self.keyboard_service_generation.saturating_add(1);
            }
            let auto_show = enabled && !self.keyboard_visible && snapshot.auto_show_requested;
            self.keyboard_recipient = Some(snapshot);
            if auto_show {
                return self.set_keyboard_visible(true) || changed;
            }
            changed
        }
        #[cfg(not(target_os = "linux"))]
        {
            let Ok(snapshot) = self.session_host.keyboard_snapshot() else {
                return false;
            };
            let previous = self.keyboard_recipient.as_ref();
            let changed = previous != Some(&snapshot);
            if previous.is_none_or(|old| {
                old.epoch != snapshot.epoch || old.recipient != snapshot.recipient
            }) {
                self.keyboard_gesture_leases.clear();
                self.keyboard_resize = None;
                self.keyboard_host
                    .application_mut()
                    .recipient_changed(snapshot.has_recipient());
            }
            if changed {
                self.keyboard_service_generation =
                    self.keyboard_service_generation.saturating_add(1);
            }
            self.keyboard_recipient = Some(snapshot);
            changed
        }
    }

    pub fn set_keyboard_visible(&mut self, visible: bool) -> bool {
        let visible = visible
            && self.keyboard_enabled
            && !self.locked
            && self.active_shell_declares("keyboard");
        let visibility_changed = self.keyboard_visible != visible;
        #[cfg(target_os = "linux")]
        if self
            .session_host
            .configure_keyboard(
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
        if self.active_shell_declares("keyboard") {
            self.set_default_shell_surface_visible("keyboard", visible);
        }
        if !visible {
            self.keyboard_resize = None;
        }
        if visibility_changed {
            self.keyboard_service_generation = self.keyboard_service_generation.saturating_add(1);
            self.keyboard_gesture_leases.clear();
            self.keyboard_host
                .application_mut()
                .recipient_changed(false);
            self.keyboard_recipient = None;
        }
        #[cfg(target_os = "windows")]
        if visible {
            self.refresh_keyboard();
        }
        // Native configuration is queued. Re-reading the pre-command snapshot here
        // can recursively re-enter auto-show; the authority wakes us on completion.
        true
    }

    pub fn keyboard_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        let (ingress, authority) = internal_normalized_ingress(
            input,
            None,
            "on-screen-keyboard",
            self.keyboard_host.inspect(),
            None,
        );
        self.keyboard_host_event_authorized(ingress, width, height, Some(authority))
    }

    pub(crate) fn keyboard_host_event_authorized(
        &mut self,
        ingress: HostEvent,
        width: u32,
        height: u32,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> bool {
        use nickel_input::{InputEvent, KeyEdge, PointerEvent, TouchEvent};
        let input = normalized_input(&ingress)
            .expect("keyboard host event must be normalized")
            .clone();
        // Only primary clicks activate keyboard keys. Other buttons must neither
        // replace the primary lease nor consume its release-time recipient epoch.
        if matches!(&input, InputEvent::Pointer(PointerEvent::Button { button, .. })
            if *button != nickel_input::PointerButton::Primary)
        {
            return false;
        }
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
        let epoch = self.keyboard_gesture_epoch(&input);
        self.keyboard_step(
            HostBatch {
                surface_size: Some((width, height)),
                events: vec![ingress],
                normalized_authorities: authority.into_iter().collect(),
                ..HostBatch::default()
            },
            epoch,
        )
    }

    pub(super) fn keyboard_gesture_epoch(
        &mut self,
        input: &nickel_input::InputEvent,
    ) -> Option<u64> {
        use nickel_input::{InputEvent, KeyEdge, PointerEvent, TouchEvent};
        let current = self
            .keyboard_recipient
            .as_ref()
            .map(|snapshot| snapshot.epoch);
        match input {
            InputEvent::Pointer(PointerEvent::Button {
                device,
                button: nickel_input::PointerButton::Primary,
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
                button: nickel_input::PointerButton::Primary,
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
        }
    }

    #[cfg(target_os = "linux")]
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

    #[cfg(target_os = "linux")]
    pub(crate) fn cancel_keyboard_gestures(&mut self) {
        self.keyboard_gesture_leases.clear();
        self.keyboard_resize = None;
        self.keyboard_host.step(HostBatch {
            window_focused: Some(false),
            ..HostBatch::default()
        });
        let available = self
            .keyboard_recipient
            .as_ref()
            .is_some_and(|snapshot| snapshot.has_recipient());
        self.keyboard_host
            .application_mut()
            .recipient_changed(available);
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
                    if let (Some(epoch), Some(input)) = (epoch, keyboard_input(key, modifiers)) {
                        if self.session_host.keyboard_input(epoch, input).is_err() {
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

#[cfg(all(test, target_os = "windows"))]
mod windows_tests {
    use super::*;
    use nickel_session_protocol::{OnScreenKeyboardSnapshot, WindowId};
    use std::sync::{Arc, Mutex};

    struct Host {
        snapshot: Mutex<OnScreenKeyboardSnapshot>,
        inputs: Mutex<Vec<(u64, OnScreenKeyboardInput)>>,
    }

    impl crate::session_host::SessionHost for Host {
        fn dispatch(&self, _: platform::ShellCommand) -> Result<(), platform::SessionRequestError> {
            Ok(())
        }

        fn keyboard_snapshot(
            &self,
        ) -> Result<OnScreenKeyboardSnapshot, platform::SessionRequestError> {
            Ok(self.snapshot.lock().unwrap().clone())
        }

        fn keyboard_input(
            &self,
            epoch: u64,
            input: OnScreenKeyboardInput,
        ) -> Result<(), platform::SessionRequestError> {
            self.inputs.lock().unwrap().push((epoch, input));
            Ok(())
        }
    }

    #[test]
    fn jsx_keyboard_delivers_with_windows_recipient_lease() {
        let host = Arc::new(Host {
            snapshot: Mutex::new(OnScreenKeyboardSnapshot {
                epoch: 19,
                recipient: Some(WindowId(7)),
                ..Default::default()
            }),
            inputs: Mutex::new(Vec::new()),
        });
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.keyboard_enabled = true;
        shell.set_keyboard_visible(true);
        assert!(shell.native_surface_visible(
            SurfaceRole::Panel,
            Some(&shell.active_shell_surface_key("keyboard"))
        ));
        assert!(!shell.surface_visible(SurfaceRole::OnScreenKeyboard));
        let effect = crate::plugin_panel::PluginEffect::Keyboard {
            plugin_id: "nickel-default".into(),
            effect: crate::keyboard_capabilities::KeyboardRequest {
                operation: "keyboard.press".into(),
                id: Some("osk-char-97".into()),
                generation: shell.keyboard_service_generation,
                delta: None,
            },
        };
        assert!(shell.apply_keyboard_plugin_effect(effect.clone(), Some(19)));
        assert_eq!(
            *host.inputs.lock().unwrap(),
            vec![(19, OnScreenKeyboardInput::Text { text: "a".into() })]
        );
        host.snapshot.lock().unwrap().epoch = 20;
        shell.refresh_keyboard();
        let stale = crate::plugin_panel::PluginEffect::Keyboard {
            plugin_id: "nickel-default".into(),
            effect: crate::keyboard_capabilities::KeyboardRequest {
                operation: "keyboard.press".into(),
                id: Some("osk-char-97".into()),
                generation: shell.keyboard_service_generation,
                delta: None,
            },
        };
        assert!(!shell.apply_keyboard_plugin_effect(stale, Some(19)));
        assert_eq!(host.inputs.lock().unwrap().len(), 1);
    }
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;
    use nickel_input::{DeviceId, EventOrder, InputEvent, KeyEdge, PointerButton, PointerEvent};
    use nickel_session_protocol::{OnScreenKeyboardSnapshot, ShellSemanticTarget, WindowId};
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Host(Mutex<Vec<(u64, OnScreenKeyboardInput)>>);
    impl crate::session_host::SessionHost for Host {
        fn dispatch(&self, _: platform::ShellCommand) -> Result<(), platform::SessionRequestError> {
            Ok(())
        }
        fn configure_keyboard(
            &self,
            _: bool,
            _: bool,
            _: u64,
            _: bool,
            _: bool,
            _: u32,
        ) -> Result<(), platform::SessionRequestError> {
            Ok(())
        }
        fn keyboard_input(
            &self,
            epoch: u64,
            input: OnScreenKeyboardInput,
        ) -> Result<(), platform::SessionRequestError> {
            self.0.lock().unwrap().push((epoch, input));
            Ok(())
        }
    }

    #[test]
    fn primary_keyboard_gesture_keeps_epoch_through_other_buttons_and_cancels_on_topology() {
        let host = Arc::new(Host::default());
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.keyboard_recipient = Some(OnScreenKeyboardSnapshot {
            epoch: 19,
            generation: 1,
            recipient: Some(WindowId(7)),
            enabled: true,
            visible: true,
            ..Default::default()
        });
        shell
            .keyboard_host
            .application_mut()
            .recipient_changed(true);
        shell.scene(SurfaceRole::OnScreenKeyboard, 1280, 368);
        let target = shell
            .resolve_semantic_target(&ShellSemanticTarget::OnScreenKeyboard {
                key: "osk-char-97".into(),
            })
            .unwrap();
        let event = |button, edge| {
            InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(1),
                order: EventOrder(1),
                button,
                edge,
                position: Some(nickel_input::Point {
                    x: f64::from(target.x),
                    y: f64::from(target.y),
                }),
            })
        };
        for (button, edge) in [
            (PointerButton::Primary, KeyEdge::Pressed),
            (PointerButton::Secondary, KeyEdge::Pressed),
            (PointerButton::Secondary, KeyEdge::Released),
            (PointerButton::Primary, KeyEdge::Released),
        ] {
            shell.keyboard_host_input(event(button, edge), 1280, 368);
        }
        assert_eq!(
            *host.0.lock().unwrap(),
            vec![(19, OnScreenKeyboardInput::Text { text: "a".into() })]
        );
        shell.keyboard_host_input(event(PointerButton::Primary, KeyEdge::Pressed), 1280, 368);
        // A newer snapshot cannot retarget an already-started gesture. Keeping
        // the old epoch lets the session authority reject the stale command.
        shell.keyboard_recipient.as_mut().unwrap().epoch = 20;
        shell.keyboard_host_input(event(PointerButton::Primary, KeyEdge::Released), 1280, 368);
        assert_eq!(host.0.lock().unwrap().last().unwrap().0, 19);
        shell.keyboard_host_input(event(PointerButton::Primary, KeyEdge::Pressed), 1280, 368);
        shell.keyboard_resize = Some((DeviceId(1), None, 2.0));
        shell.cancel_keyboard_gestures();
        assert!(shell.keyboard_resize.is_none());
        assert!(shell.keyboard_gesture_leases.is_empty());
        shell.keyboard_host_input(event(PointerButton::Primary, KeyEdge::Released), 1280, 368);
        assert_eq!(host.0.lock().unwrap().len(), 2);
    }

    #[test]
    fn plugin_keyboard_effect_rejects_a_recipient_changed_after_press() {
        let host = Arc::new(Host::default());
        let mut shell = LiveShell::new_with_session_host(host.clone()).unwrap();
        shell.keyboard_enabled = true;
        assert!(
            shell.active_shell_declares("keyboard"),
            "{:?}",
            shell.plugin_registry.get("nickel-default")
        );
        assert!(shell.set_keyboard_visible(true));
        assert!(
            shell.plugin_surface_matches(&shell.active_shell_surface_key("keyboard")),
            "{:?}",
            shell.show_plugin_window("nickel-default", "keyboard")
        );
        shell.keyboard_recipient = Some(OnScreenKeyboardSnapshot {
            epoch: 19,
            generation: 1,
            recipient: Some(WindowId(7)),
            enabled: true,
            visible: true,
            ..Default::default()
        });
        shell
            .keyboard_host
            .application_mut()
            .recipient_changed(true);
        let input = |edge| {
            InputEvent::Pointer(PointerEvent::Button {
                device: DeviceId(4),
                order: EventOrder(1),
                button: PointerButton::Primary,
                edge,
                position: Some(nickel_input::Point { x: 50.0, y: 50.0 }),
            })
        };
        assert_eq!(
            shell.keyboard_gesture_epoch(&input(KeyEdge::Pressed)),
            Some(19)
        );
        shell.keyboard_recipient.as_mut().unwrap().epoch = 20;
        let pressed_epoch = shell.keyboard_gesture_epoch(&input(KeyEdge::Released));
        assert_eq!(pressed_epoch, Some(19));
        assert!(!shell.apply_keyboard_plugin_effect(
            crate::plugin_panel::PluginEffect::Keyboard {
                plugin_id: "nickel-default".into(),
                effect: crate::keyboard_capabilities::KeyboardRequest {
                    operation: "keyboard.press".into(),
                    id: Some("osk-char-97".into()),
                    generation: shell.keyboard_service_generation,
                    delta: None
                }
            },
            pressed_epoch,
        ));
        assert!(host.0.lock().unwrap().is_empty());
        assert!(shell.apply_keyboard_plugin_effect(
            crate::plugin_panel::PluginEffect::Keyboard {
                plugin_id: "nickel-default".into(),
                effect: crate::keyboard_capabilities::KeyboardRequest {
                    operation: "keyboard.press".into(),
                    id: Some("osk-char-97".into()),
                    generation: shell.keyboard_service_generation,
                    delta: None
                }
            },
            Some(20),
        ));
        assert_eq!(
            *host.0.lock().unwrap(),
            vec![(20, OnScreenKeyboardInput::Text { text: "a".into() })]
        );
        let current = crate::plugin_panel::PluginEffect::Keyboard {
            plugin_id: "nickel-default".into(),
            effect: crate::keyboard_capabilities::KeyboardRequest {
                operation: "keyboard.press".into(),
                id: Some("osk-char-97".into()),
                generation: shell.keyboard_service_generation,
                delta: None,
            },
        };
        shell.locked = true;
        assert!(!shell.apply_keyboard_plugin_effect(current.clone(), Some(20)));
        shell.locked = false;
        shell
            .plugin_registry
            .set_enabled("nickel-default", false)
            .unwrap();
        assert!(!shell.apply_keyboard_plugin_effect(current, Some(20)));
        assert_eq!(host.0.lock().unwrap().len(), 1);
    }
}
