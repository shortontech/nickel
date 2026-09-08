//! Native desktop pointer and keyboard routing.
//!
//! Hit testing remains owned by InternalUiRuntime. This adapter retains button
//! edges and seat modifiers before the generic UI path discards those details.
//! Relative motion deltas and scroll are not owned by this adapter. Button presses
//! claim runtime focus; the session reconciles that ownership with the native seat.

use std::collections::{BTreeSet, HashMap};

use nickel_input::{
    DeviceId, EventOrder, InputEvent, KeyEdge, ModifierState, PointerButton, PointerEvent,
};
use nickel_ui::{HostBatch, HostEvent, InternalSurfaceId};

use super::{InternalSurfaceRole, InternalUiRuntime};

pub(crate) enum DesktopPointerAction {
    Motion,
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
    // Allocate identities only for devices that actually reach a desktop. Removal
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
    /// Translate a seat hover departure into the desktop's normalized lifecycle.
    /// This clears hover, not menu ownership or keyboard focus. Captured motion never
    /// invokes departure: its target remains the starting desktop until release.
    pub(super) fn desktop_pointer_leave(&mut self, id: InternalSurfaceId) -> bool {
        if !self.presentation.get(&id).is_some_and(|surface| {
            surface.external_scene.is_some()
                && surface.placement.role == InternalSurfaceRole::Desktop
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

    /// Return true when this event belongs to a desktop, including swallowed
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
        // transfer the release half of an existing desktop gesture to that target.
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
                    && surface.placement.role == InternalSurfaceRole::Desktop
            })
            .map(|surface| surface.placement.clone());
        if !captured && placement.is_none() {
            return false;
        }

        // Share hover ownership with generic widgets. Switching from a panel to
        // a desktop must cancel the panel before delivering the desktop motion.
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
        // Desktop reducers consume whole-surface local coordinates. Panel work-area
        // reservations are already handled by the coordinator's viewport projection.
        let local = nickel_input::Point {
            x: position.0 - f64::from(placement.geometry.0),
            y: position.1 - f64::from(placement.geometry.1),
        };
        let event = match action {
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
        if matches!(
            event,
            PointerEvent::Button {
                edge: KeyEdge::Pressed,
                ..
            }
        ) {
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
