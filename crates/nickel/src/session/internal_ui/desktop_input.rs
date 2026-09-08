//! Native normalized shell pointer/touch and desktop keyboard routing.
//!
//! Hit testing remains owned by InternalUiRuntime. This adapter retains button
//! edges, touch identities, and seat modifiers before the generic UI path discards them.
//! Relative motion deltas are not owned by this adapter. Button presses
//! claim runtime focus on the desktop only; the keyboard overlay preserves its recipient.

use std::collections::{BTreeSet, HashMap};

use nickel_input::{
    DeviceId, EventOrder, InputEvent, KeyEdge, ModifierState, PointerButton, PointerEvent,
    TouchEvent, TouchId,
};
use nickel_ui::{HostBatch, HostEvent, InternalSurfaceId};

use super::{InternalSurfaceRole, InternalUiRuntime};

pub(crate) enum DesktopPointerAction {
    Motion,
    Axis {
        delta: nickel_input::Vector,
        discrete: Option<(i32, i32)>,
    },
    Button {
        button: PointerButton,
        edge: KeyEdge,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::session::internal_ui::InternalSurfacePlacement;

    fn desktop(runtime: &mut InternalUiRuntime) -> InternalSurfaceId {
        runtime.insert_scene(
            Vec::new(),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Desktop,
                geometry: (-800, -120, 800, 600),
                output: Some("left".into()),
            },
            1.5,
        )
    }

    fn button(edge: KeyEdge) -> DesktopPointerAction {
        DesktopPointerAction::Button {
            button: PointerButton::Secondary,
            edge,
        }
    }

    #[test]
    fn normalized_touch_keeps_device_contacts_capture_and_last_release_position() {
        use crate::session::TouchPhase::*;
        let mut runtime = InternalUiRuntime::default();
        let id = desktop(&mut runtime);
        for source in ["touch-a", "touch-b"] {
            assert!(runtime.normalized_touch_input(source, 0, (-780.0, -40.0), Started, false));
        }
        assert_eq!(runtime.focused(), Some(id));
        let batches = runtime.drain_routed_events();
        let devices = batches
            .iter()
            .filter_map(|(_, batch, _)| match &batch.events[..] {
                [
                    HostEvent::Normalized {
                        input:
                            InputEvent::Touch(TouchEvent::Started {
                                device,
                                contact,
                                position,
                                ..
                            }),
                        ..
                    },
                ] => {
                    assert_eq!(*contact, TouchId(0));
                    assert_eq!(position.x, 20.0);
                    Some(*device)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(devices.len(), 2);
        assert_ne!(devices[0], devices[1]);
        assert!(runtime.normalized_touch_input("touch-a", 0, (20.0, 30.0), Moved, true));
        assert!(runtime.normalized_touch_input("touch-a", 0, (0.0, 0.0), Ended, true));
        let batches = runtime.drain_routed_events();
        assert!(matches!(&batches[1].1.events[..], [HostEvent::Normalized {
            input: InputEvent::Touch(TouchEvent::Ended { position, .. }), ..
        }] if position.x == 820.0 && position.y == 150.0));
        assert_eq!(runtime.desktop_input.touches.len(), 1);
        runtime.remove_desktop_pointer_device("touch-b");
        assert!(runtime.desktop_input.touches.is_empty());
        assert!(matches!(
            &runtime.drain_routed_events()[0].1.events[..],
            [HostEvent::Normalized {
                input: InputEvent::Touch(TouchEvent::Cancelled { .. }),
                ..
            }]
        ));
    }

    #[test]
    fn normalized_keyboard_touch_never_focuses_and_retirement_swallows_old_contact_tail() {
        use crate::session::TouchPhase::*;
        let mut runtime = InternalUiRuntime::default();
        let recipient = desktop(&mut runtime);
        runtime.focus_surface(recipient);
        let keyboard = runtime.insert_scene(
            Vec::new(),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::OnScreenKeyboard,
                geometry: (0, 0, 800, 320),
                output: None,
            },
            1.0,
        );
        assert!(runtime.normalized_touch_input("touch", 3, (30.0, 80.0), Started, true));
        assert_eq!(runtime.focused(), Some(recipient));
        runtime.drain_routed_events();
        runtime.remove(keyboard);
        assert!(matches!(
            &runtime.drain_routed_events()[0].1.events[..],
            [HostEvent::Normalized {
                input: InputEvent::Touch(TouchEvent::Cancelled {
                    contact: TouchId(3),
                    ..
                }),
                ..
            }]
        ));
        assert!(runtime.normalized_touch_input("touch", 3, (-780.0, -40.0), Moved, false));
        assert!(runtime.normalized_touch_input("touch", 3, (0.0, 0.0), Ended, false));
        assert!(runtime.drain_routed_events().is_empty());
        assert!(runtime.desktop_input.touches.is_empty());
        assert!(!runtime.normalized_touch_input("touch", 4, (-780.0, -40.0), Started, true));
    }

    #[test]
    fn keyboard_pointer_preserves_recipient_and_captures_release_over_clients() {
        let mut runtime = InternalUiRuntime::default();
        let recipient = desktop(&mut runtime);
        runtime.focus_surface(recipient);
        let keyboard = runtime.insert_scene(
            Vec::new(),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::OnScreenKeyboard,
                geometry: (0, 0, 800, 320),
                output: None,
            },
            1.0,
        );
        runtime.drain_routed_events();
        for (edge, point) in [
            (KeyEdge::Pressed, (30.0, 80.0)),
            (KeyEdge::Released, (1000.0, 500.0)),
        ] {
            assert!(runtime.desktop_pointer_input(
                "mouse",
                point,
                DesktopPointerAction::Button {
                    button: PointerButton::Primary,
                    edge
                },
                ModifierState::default(),
                true
            ));
            assert_eq!(runtime.focused(), Some(recipient));
        }
        let batches = runtime.drain_routed_events();
        assert_eq!(batches.len(), 2);
        assert!(
            batches
                .iter()
                .all(|(id, batch, _)| *id == keyboard && batch.window_focused.is_none())
        );
        assert!(matches!(&batches[1].1.events[..], [HostEvent::Normalized {
            input: InputEvent::Pointer(PointerEvent::Button { edge: KeyEdge::Released, position: Some(position), .. }), ..
        }] if position.x == 1000.0));
        assert!(runtime.desktop_input.capture.is_none());
        // Legacy touch still uses its adapter, but must never acquire text focus.
        assert!(runtime.touch(1, (30.0, 80.0), crate::session::TouchPhase::Started));
        assert_eq!(runtime.focused(), Some(recipient));
    }

    #[test]
    fn desktop_scroll_routes_without_claiming_focus_or_pointer_capture() {
        let mut runtime = InternalUiRuntime::default();
        let id = desktop(&mut runtime);
        assert!(runtime.desktop_pointer_input(
            "wheel",
            (-780.0, -40.0),
            DesktopPointerAction::Axis {
                delta: nickel_input::Vector { x: 0.0, y: -0.5 },
                discrete: Some((0, 0)),
            },
            Default::default(),
            false
        ));
        assert!(runtime.focused().is_none());
        assert!(runtime.desktop_input.capture.is_none());
        let batches = runtime.drain_routed_events();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].0, id);
        assert!(matches!(&batches[0].1.events[..], [HostEvent::Normalized {
            input: InputEvent::Pointer(PointerEvent::Axis { delta, discrete: Some((0, 0)), .. }), ..
        }] if delta.y == -0.5));
    }

    #[test]
    fn desktop_keyboard_repeat_state_retires_on_focus_loss() {
        let mut runtime = InternalUiRuntime::default();
        let id = desktop(&mut runtime);
        runtime.focus_surface(id);
        runtime.drain_routed_events();
        for (pressed, expected_repeat) in
            [(true, false), (true, true), (false, false), (true, false)]
        {
            assert!(runtime.desktop_keyboard_input(
                "keyboard",
                116,
                pressed,
                |device, order, repeat| {
                    assert_eq!(repeat, expected_repeat);
                    nickel_input::KeyEvent {
                        device,
                        order,
                        repeat,
                        physical: nickel_input::PhysicalKey::Code(nickel_input::KeyCode::ArrowDown),
                        logical: nickel_input::LogicalKey::Named(nickel_input::NamedKey::ArrowDown),
                        location: nickel_input::KeyLocation::Standard,
                        edge: if pressed {
                            KeyEdge::Pressed
                        } else {
                            KeyEdge::Released
                        },
                        modifiers: Default::default(),
                    }
                }
            ));
        }
        let events = runtime.drain_routed_events();
        assert_eq!(events.len(), 4);
        assert!(
            matches!(&events[2].1.events[..], [HostEvent::Normalized { input: InputEvent::Key(key), .. }] if key.edge == KeyEdge::Released)
        );
        assert!(!runtime.desktop_input.pressed_keys.is_empty());
        runtime.clear_focus();
        assert!(
            !runtime.desktop_keyboard_input("keyboard", 116, true, |_, _, _| panic!(
                "unfocused desktop must not normalize keys"
            ))
        );
        assert!(runtime.desktop_input.pressed_keys.is_empty());
    }

    #[test]
    fn desktop_and_panel_share_hover_ownership_with_ordered_departure() {
        let mut runtime = InternalUiRuntime::default();
        let id = desktop(&mut runtime);
        let panel = runtime.insert_scene(
            Vec::new(),
            InternalSurfacePlacement {
                role: InternalSurfaceRole::Panel,
                geometry: (-800, -120, 800, 56),
                output: Some("left".into()),
            },
            1.5,
        );
        runtime.pointer_motion((-780.0, -100.0));
        runtime.drain_routed_events();
        assert!(runtime.desktop_pointer_input(
            "mouse",
            (-780.0, 0.0),
            DesktopPointerAction::Motion,
            Default::default(),
            false
        ));
        let batches = runtime.drain_routed_events();
        assert_eq!(batches.len(), 2);
        assert_eq!(batches[0].0, panel);
        assert!(matches!(
            &batches[0].1.events[..],
            [HostEvent::Ui(nickel_ui::UiEvent::PointerCancelled)]
        ));
        assert_eq!(runtime.hovered, Some(id));
        // The session falls through to generic routing only when desktop routing
        // declines the new target. That path must deliver one normalized departure.
        assert!(!runtime.desktop_pointer_input(
            "mouse",
            (20.0, 20.0),
            DesktopPointerAction::Motion,
            Default::default(),
            true
        ));
        assert!(!runtime.pointer_motion_with_client((20.0, 20.0), true));
        let batches = runtime.drain_routed_events();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].0, id);
        assert!(matches!(
            &batches[0].1.events[..],
            [HostEvent::Normalized {
                input: InputEvent::Pointer(PointerEvent::Leave {
                    order: EventOrder(2),
                    ..
                }),
                ..
            }]
        ));
        assert!(runtime.hovered.is_none());
    }

    #[test]
    fn desktop_pointer_preserves_modifiers_and_capture_over_clients() {
        let mut runtime = InternalUiRuntime::default();
        let id = desktop(&mut runtime);
        let modifiers = ModifierState::from_sides([nickel_input::Modifier::ControlRight]);
        assert!(!runtime.desktop_pointer_input(
            "mouse",
            (-780.0, -40.0),
            button(KeyEdge::Pressed),
            modifiers.clone(),
            true
        ));
        assert!(runtime.drain_routed_events().is_empty());
        assert!(runtime.desktop_pointer_input(
            "mouse",
            (-780.0, -40.0),
            button(KeyEdge::Pressed),
            modifiers.clone(),
            false
        ));
        assert!(runtime.desktop_pointer_input(
            "mouse",
            (200.0, 300.0),
            DesktopPointerAction::Motion,
            modifiers.clone(),
            true
        ));
        assert!(runtime.desktop_pointer_input(
            "mouse",
            (200.0, 300.0),
            button(KeyEdge::Released),
            modifiers.clone(),
            true
        ));
        let mut batches = runtime.drain_routed_events();
        assert_eq!(batches.len(), 4);
        assert_eq!(batches.remove(0).1.window_focused, Some(true));
        assert_eq!(runtime.focused(), Some(id));
        assert!(batches.iter().all(|(target, _, snapshot)| *target == id && snapshot.as_ref() == Some(&modifiers)));
        assert!(matches!(&batches[0].1.events[..], [HostEvent::Normalized {
            input: InputEvent::Pointer(PointerEvent::Button { button: PointerButton::Secondary, edge: KeyEdge::Pressed, position: Some(position), .. }), ..
        }] if *position == nickel_input::Point { x: 20.0, y: 80.0 }));
        assert!(matches!(&batches[1].1.events[..], [HostEvent::Normalized {
            input: InputEvent::Pointer(PointerEvent::Motion { position, .. }), ..
        }] if *position == nickel_input::Point { x: 1000.0, y: 420.0 }));
        assert!(runtime.desktop_input.capture.is_none());
        assert!(!runtime.desktop_pointer_input(
            "mouse",
            (200.0, 300.0),
            DesktopPointerAction::Motion,
            modifiers,
            true
        ));
    }

    #[test]
    fn removed_desktop_consumes_capture_release_without_resurrection() {
        let mut runtime = InternalUiRuntime::default();
        let id = desktop(&mut runtime);
        runtime.desktop_pointer_input(
            "mouse",
            (-780.0, -40.0),
            button(KeyEdge::Pressed),
            Default::default(),
            false,
        );
        runtime.drain_routed_events();
        runtime.remove(id);
        let lifecycle = runtime.drain_routed_events();
        assert_eq!(lifecycle.len(), 1);
        assert_eq!(lifecycle[0].1.window_focused, Some(false));
        assert!(runtime.desktop_pointer_input(
            "mouse",
            (20.0, 20.0),
            button(KeyEdge::Released),
            Default::default(),
            true
        ));
        assert!(runtime.drain_routed_events().is_empty());
        assert!(runtime.desktop_input.capture.is_none());
    }

    #[test]
    fn client_press_blurs_desktop_but_pointer_departure_does_not() {
        let mut runtime = InternalUiRuntime::default();
        let id = desktop(&mut runtime);
        for edge in [KeyEdge::Pressed, KeyEdge::Released] {
            runtime.desktop_pointer_input(
                "mouse",
                (-780.0, -40.0),
                button(edge),
                Default::default(),
                false,
            );
        }
        runtime.drain_routed_events();
        assert!(!runtime.desktop_pointer_input(
            "mouse",
            (20.0, 20.0),
            DesktopPointerAction::Motion,
            Default::default(),
            true
        ));
        runtime.pointer_motion_with_client((20.0, 20.0), true);
        assert_eq!(runtime.focused(), Some(id));
        assert!(
            runtime
                .drain_routed_events()
                .iter()
                .all(|(_, batch, _)| batch.window_focused.is_none())
        );
        assert!(!runtime.pointer_button_with_client((20.0, 20.0), true, true));
        assert_eq!(runtime.focused(), None);
        let batches = runtime.drain_routed_events();
        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].0, id);
        assert_eq!(batches[0].1.window_focused, Some(false));
    }

    #[test]
    fn unrelated_device_removal_does_not_cancel_another_devices_capture() {
        let mut runtime = InternalUiRuntime::default();
        desktop(&mut runtime);
        runtime.desktop_pointer_input(
            "other",
            (-780.0, -40.0),
            DesktopPointerAction::Motion,
            Default::default(),
            false,
        );
        runtime.desktop_pointer_input(
            "owner",
            (-780.0, -40.0),
            button(KeyEdge::Pressed),
            Default::default(),
            false,
        );
        runtime.drain_routed_events();
        runtime.remove_desktop_pointer_device("other");
        assert!(runtime.desktop_input.capture.is_some());
        assert!(runtime.drain_routed_events().is_empty());
        runtime.desktop_pointer_input(
            "owner",
            (20.0, 20.0),
            button(KeyEdge::Released),
            Default::default(),
            true,
        );
        assert!(runtime.desktop_input.capture.is_none());
    }

    #[test]
    fn device_removal_cancels_capture_and_retires_identity() {
        let mut runtime = InternalUiRuntime::default();
        desktop(&mut runtime);
        runtime.desktop_pointer_input(
            "mouse",
            (-780.0, -40.0),
            button(KeyEdge::Pressed),
            Default::default(),
            false,
        );
        runtime.drain_routed_events();
        runtime.remove_desktop_pointer_device("mouse");
        assert!(runtime.desktop_input.capture.is_none());
        assert!(runtime.desktop_input.devices.is_empty());
        let batches = runtime.drain_routed_events();
        assert!(matches!(
            &batches[0].1.events[..],
            [HostEvent::Normalized {
                input: InputEvent::DeviceRemoved { .. },
                ..
            }]
        ));
    }
}

#[derive(Default)]
pub(super) struct DesktopInputState {
    // A contact belongs to its physical device, not merely the backend slot number.
    // Retired targets remain tombstones until Up, preventing release-through clicks.
    touches: HashMap<(DeviceId, TouchId), (Option<InternalSurfaceId>, nickel_input::Point)>,
    // Allocate identities only for devices that reach a normalized shell surface. Removal
    // retires the name mapping; a reconnect receives a fresh, non-aliased identity.
    devices: HashMap<String, DeviceId>,
    next_device: u64,
    order: u64,
    last_device: Option<DeviceId>,
    pressed_keys: BTreeSet<(DeviceId, u32)>,
    // One logical seat pointer owns a drag, even when several physical devices
    // contribute button edges. Track each pair so releases cannot escape to clients.
    capture: Option<(InternalSurfaceId, BTreeSet<(DeviceId, PointerButton)>)>,
}

impl InternalUiRuntime {
    /// Route both key edges only to the focused desktop. The closure performs
    /// backend conversion after device identity/order and repeat are established.
    pub(crate) fn desktop_keyboard_input(
        &mut self,
        source: &str,
        raw: u32,
        pressed: bool,
        normalize: impl FnOnce(DeviceId, EventOrder, bool) -> nickel_input::KeyEvent,
    ) -> bool {
        let Some(id) = self.focused.filter(|id| {
            self.presentation.get(id).is_some_and(|surface| {
                surface.visible
                    && surface.external_scene.is_some()
                    && surface.placement.role == InternalSurfaceRole::Desktop
            })
        }) else {
            return false;
        };
        let state = &mut self.desktop_input;
        let device = *state.devices.entry(source.to_owned()).or_insert_with(|| {
            state.next_device += 1;
            DeviceId(state.next_device)
        });
        state.order = state.order.wrapping_add(1);
        let repeat = if pressed {
            !state.pressed_keys.insert((device, raw))
        } else {
            state.pressed_keys.remove(&(device, raw));
            false
        };
        let key = normalize(device, EventOrder(state.order), repeat);
        self.step(
            id,
            HostBatch {
                events: vec![HostEvent::Normalized {
                    input: InputEvent::Key(key),
                    clipboard_text: None,
                }],
                ..Default::default()
            },
        );
        true
    }

    pub(super) fn clear_desktop_pressed_keys(&mut self) {
        // Releases can go to the next owner; no old press may become a repeat
        // when the desktop is focused again.
        self.desktop_input.pressed_keys.clear();
    }
    /// Translate a seat hover departure into the shell's normalized lifecycle.
    /// This clears hover, not menu ownership or keyboard focus. Captured motion never
    /// invokes departure: its target remains the starting surface until release.
    pub(super) fn desktop_pointer_leave(&mut self, id: InternalSurfaceId) -> bool {
        if !self.presentation.get(&id).is_some_and(|surface| {
            surface.external_scene.is_some()
                && matches!(
                    surface.placement.role,
                    InternalSurfaceRole::Desktop | InternalSurfaceRole::OnScreenKeyboard
                )
        }) {
            return false;
        }
        let Some(device) = self.desktop_input.last_device else {
            return false;
        };
        self.desktop_input.order = self.desktop_input.order.wrapping_add(1);
        self.routed_events.push((
            id,
            HostBatch {
                events: vec![HostEvent::Normalized {
                    input: InputEvent::Pointer(PointerEvent::Leave {
                        device,
                        order: EventOrder(self.desktop_input.order),
                    }),
                    clipboard_text: None,
                }],
                ..Default::default()
            },
            None,
        ));
        true
    }

    /// Cancel only a transaction involving the removed device. Other devices can
    /// disappear while the seat's pointer is dragging without owning that drag.
    pub(crate) fn remove_desktop_pointer_device(&mut self, source: &str) {
        self.cancel_normalized_touches(Some(source));
        let state = &mut self.desktop_input;
        let Some(device) = state.devices.remove(source) else {
            return;
        };
        state.pressed_keys.retain(|(owner, _)| *owner != device);
        if let Some((id, buttons)) = &mut state.capture {
            if !buttons.iter().any(|(owner, _)| *owner == device) {
                return;
            }
            let target = *id;
            buttons.retain(|(owner, _)| *owner != device);
            if buttons.is_empty() {
                state.capture = None;
            }
            state.order = state.order.wrapping_add(1);
            // Cancellation is ordered with the last device event, even if other
            // devices still have held buttons whose releases must be swallowed.
            self.routed_events.push((
                target,
                HostBatch {
                    events: vec![HostEvent::Normalized {
                        input: InputEvent::DeviceRemoved {
                            device,
                            order: EventOrder(state.order),
                        },
                        clipboard_text: None,
                    }],
                    ..Default::default()
                },
                None,
            ));
        }
    }

    pub(crate) fn normalized_touch_input(
        &mut self,
        source: &str,
        contact: u64,
        position: (f64, f64),
        phase: crate::session::TouchPhase,
        client_present: bool,
    ) -> bool {
        use crate::session::TouchPhase;
        let contact = TouchId(contact);
        let starting = matches!(phase, TouchPhase::Started);
        let target = if starting {
            let Some((id, _)) = self.surface_at(position, client_present) else {
                return false;
            };
            if !self.presentation.get(&id).is_some_and(|surface| {
                surface.external_scene.is_some()
                    && matches!(
                        surface.placement.role,
                        InternalSurfaceRole::Desktop | InternalSurfaceRole::OnScreenKeyboard
                    )
            }) {
                return false;
            }
            Some(id)
        } else {
            None
        };
        let state = &mut self.desktop_input;
        let device = if starting {
            *state.devices.entry(source.to_owned()).or_insert_with(|| {
                state.next_device += 1;
                DeviceId(state.next_device)
            })
        } else {
            let Some(device) = state.devices.get(source).copied() else {
                return false;
            };
            device
        };
        let key = (device, contact);
        let (target, previous) = if starting {
            (target, nickel_input::Point { x: 0.0, y: 0.0 })
        } else {
            let Some(capture) = state.touches.get(&key).copied() else {
                return false;
            };
            capture
        };
        let terminal = matches!(phase, TouchPhase::Ended | TouchPhase::Cancelled);
        if terminal {
            state.touches.remove(&key);
        }
        let Some(id) = target else {
            return true;
        };
        let Some((geometry, role)) = self
            .presentation
            .get(&id)
            .filter(|surface| surface.visible)
            .map(|surface| (surface.placement.geometry, surface.placement.role))
        else {
            return true;
        };
        // Up has no coordinates. Keep the last location from this contact, never
        // a shared pointer position or a new hit target under the release.
        let local = if terminal {
            previous
        } else {
            nickel_input::Point {
                x: position.0 - f64::from(geometry.0),
                y: position.1 - f64::from(geometry.1),
            }
        };
        if !terminal {
            state.touches.insert(key, (Some(id), local));
        }
        state.order = state.order.wrapping_add(1);
        let order = EventOrder(state.order);
        let input = match phase {
            TouchPhase::Started => TouchEvent::Started {
                device,
                contact,
                order,
                position: local,
            },
            TouchPhase::Moved => TouchEvent::Moved {
                device,
                contact,
                order,
                position: local,
            },
            TouchPhase::Ended => TouchEvent::Ended {
                device,
                contact,
                order,
                position: local,
            },
            TouchPhase::Cancelled => TouchEvent::Cancelled {
                device,
                contact,
                order,
            },
        };
        if starting && role == InternalSurfaceRole::Desktop {
            self.focus_surface(id);
        }
        self.step(
            id,
            HostBatch {
                events: vec![HostEvent::Normalized {
                    input: InputEvent::Touch(input),
                    clipboard_text: None,
                }],
                ..Default::default()
            },
        );
        true
    }

    /// Cancel device-owned contacts without affecting another touchscreen's equal slots.
    pub(crate) fn cancel_normalized_touches(&mut self, source: Option<&str>) -> bool {
        let device = source.and_then(|source| self.desktop_input.devices.get(source).copied());
        if source.is_some() && device.is_none() {
            return false;
        }
        let keys = self
            .desktop_input
            .touches
            .keys()
            .copied()
            .filter(|(owner, _)| device.is_none_or(|device| device == *owner))
            .collect::<Vec<_>>();
        let handled = !keys.is_empty();
        for (device, contact) in keys {
            let (target, _) = self
                .desktop_input
                .touches
                .remove(&(device, contact))
                .unwrap();
            if let Some(target) = target {
                self.dispatch_touch_cancel(target, device, contact);
            }
        }
        handled
    }

    pub(super) fn retire_normalized_touch_surface(&mut self, id: InternalSurfaceId) {
        let keys = self
            .desktop_input
            .touches
            .iter()
            .filter_map(|(key, (target, _))| (*target == Some(id)).then_some(*key))
            .collect::<Vec<_>>();
        for (device, contact) in keys {
            self.desktop_input
                .touches
                .get_mut(&(device, contact))
                .unwrap()
                .0 = None;
            self.dispatch_touch_cancel(id, device, contact);
        }
    }

    pub(crate) fn retire_normalized_touch_surfaces(&mut self) {
        let ids = self.presentation.keys().copied().collect::<Vec<_>>();
        for id in ids {
            self.retire_normalized_touch_surface(id);
        }
    }

    fn dispatch_touch_cancel(&mut self, id: InternalSurfaceId, device: DeviceId, contact: TouchId) {
        self.desktop_input.order = self.desktop_input.order.wrapping_add(1);
        self.step(
            id,
            HostBatch {
                events: vec![HostEvent::Normalized {
                    input: InputEvent::Touch(TouchEvent::Cancelled {
                        device,
                        contact,
                        order: EventOrder(self.desktop_input.order),
                    }),
                    clipboard_text: None,
                }],
                ..Default::default()
            },
        );
    }

    /// Return true when this event belongs to a normalized shell surface, including swallowed
    /// releases for retired captures. False leaves routing to the generic UI/client
    /// path. Positions are compositor-logical; scale must not be applied again.
    pub(crate) fn desktop_pointer_input(
        &mut self,
        source: &str,
        position: (f64, f64),
        action: DesktopPointerAction,
        modifiers: ModifierState,
        client_present: bool,
    ) -> bool {
        // Capture precedes hit testing so crossing a client or panel does not
        // transfer the release half of an existing shell gesture to that target.
        let target = self
            .desktop_input
            .capture
            .as_ref()
            .map(|(id, _)| *id)
            .or_else(|| self.surface_at(position, client_present).map(|(id, _)| id));
        let Some(id) = target else {
            return false;
        };
        let captured = self.desktop_input.capture.is_some();
        let placement = self
            .presentation
            .get(&id)
            .filter(|surface| {
                surface.visible
                    && surface.external_scene.is_some()
                    && matches!(
                        surface.placement.role,
                        InternalSurfaceRole::Desktop | InternalSurfaceRole::OnScreenKeyboard
                    )
            })
            .map(|surface| surface.placement.clone());
        if !captured && placement.is_none() {
            return false;
        }

        // Share hover ownership with generic widgets. Switching from a panel to
        // a normalized surface must cancel the panel before delivering motion.
        if placement.is_some() && self.hovered != Some(id) {
            if let Some(previous) = self.hovered {
                self.dispatch_ui(previous, nickel_ui::UiEvent::PointerCancelled);
            }
            self.hovered = Some(id);
        }

        let state = &mut self.desktop_input;
        let device = *state.devices.entry(source.to_owned()).or_insert_with(|| {
            state.next_device += 1;
            DeviceId(state.next_device)
        });
        state.last_device = Some(device);
        state.order = state.order.wrapping_add(1);
        let order = EventOrder(state.order);
        if let DesktopPointerAction::Button { button, edge } = &action {
            if *edge == KeyEdge::Pressed {
                state
                    .capture
                    .get_or_insert_with(|| (id, BTreeSet::new()))
                    .1
                    .insert((device, button.clone()));
            } else if let Some((_, buttons)) = &mut state.capture {
                buttons.remove(&(device, button.clone()));
                if buttons.is_empty() {
                    state.capture = None;
                }
            }
        }
        // A removed/hidden target still owns its outstanding releases. Consume
        // them without resurrecting its scene or clicking the client underneath.
        let Some(placement) = placement else {
            return true;
        };
        // Shell reducers consume whole-surface local coordinates. Panel work-area
        // reservations are already handled by the coordinator's viewport projection.
        let local = nickel_input::Point {
            x: position.0 - f64::from(placement.geometry.0),
            y: position.1 - f64::from(placement.geometry.1),
        };
        let event = match action {
            DesktopPointerAction::Axis { delta, discrete } => PointerEvent::Axis {
                device,
                order,
                delta,
                discrete,
                position: Some(local),
            },
            DesktopPointerAction::Motion => PointerEvent::Motion {
                device,
                order,
                position: local,
                delta: None,
            },
            DesktopPointerAction::Button { button, edge } => PointerEvent::Button {
                device,
                order,
                position: Some(local),
                button,
                edge,
            },
        };
        // A desktop interaction needs a real focus owner so client activation or
        // Alt-Tab can blur it later. Merely painting a menu cannot establish this.
        if placement.role == InternalSurfaceRole::Desktop
            && matches!(
                event,
                PointerEvent::Button {
                    edge: KeyEdge::Pressed,
                    ..
                }
            )
        {
            self.focus_surface(id);
        }
        // Snapshot modifiers with the event: reading them later during queue drain
        // could apply a newer key state to an earlier Ctrl/Shift-click.
        self.routed_events.push((
            id,
            HostBatch {
                events: vec![HostEvent::Normalized {
                    input: InputEvent::Pointer(event),
                    clipboard_text: None,
                }],
                ..Default::default()
            },
            Some(modifiers),
        ));
        true
    }
}
