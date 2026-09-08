//! Native button and absolute-position motion routing for desktop scenes.
//!
//! Hit testing remains owned by InternalUiRuntime. This adapter retains button
//! edges and seat modifiers before the generic UI path discards those details.
//! Relative motion deltas, scroll, hover departure, and keyboard focus are not
//! owned by this adapter; their routing must be coordinated by the session.

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
        let batches = runtime.drain_routed_events();
        assert_eq!(batches.len(), 3);
        assert!(batches.iter().all(|(target, _, snapshot)| *target == id && snapshot.as_ref() == Some(&modifiers)));
        assert!(matches!(&batches[0].1.events[..], [HostEvent::Normalized {
            input: InputEvent::Pointer(PointerEvent::Button { button: PointerButton::Secondary, edge: KeyEdge::Pressed, position: Some(position), .. }), ..
        }] if *position == nickel_input::Point { x: 20.0, y: 80.0 }));
        assert!(matches!(&batches[1].1.events[..], [HostEvent::Normalized {
            input: InputEvent::Pointer(PointerEvent::Motion { position, .. }), ..
        }] if *position == nickel_input::Point { x: 1000.0, y: 420.0 }));
        assert!(runtime.desktop_pointer.capture.is_none());
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
        assert!(runtime.desktop_pointer_input(
            "mouse",
            (20.0, 20.0),
            button(KeyEdge::Released),
            Default::default(),
            true
        ));
        assert!(runtime.drain_routed_events().is_empty());
        assert!(runtime.desktop_pointer.capture.is_none());
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
        assert!(runtime.desktop_pointer.capture.is_some());
        assert!(runtime.drain_routed_events().is_empty());
        runtime.desktop_pointer_input(
            "owner",
            (20.0, 20.0),
            button(KeyEdge::Released),
            Default::default(),
            true,
        );
        assert!(runtime.desktop_pointer.capture.is_none());
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
        assert!(runtime.desktop_pointer.capture.is_none());
        assert!(runtime.desktop_pointer.devices.is_empty());
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
pub(super) struct DesktopPointerState {
    // Allocate identities only for devices that actually reach a desktop. Removal
    // retires the name mapping; a reconnect receives a fresh, non-aliased identity.
    devices: HashMap<String, DeviceId>,
    next_device: u64,
    order: u64,
    // One logical seat pointer owns a drag, even when several physical devices
    // contribute button edges. Track each pair so releases cannot escape to clients.
    capture: Option<(InternalSurfaceId, BTreeSet<(DeviceId, PointerButton)>)>,
}

impl InternalUiRuntime {
    /// Cancel only a transaction involving the removed device. Other devices can
    /// disappear while the seat's pointer is dragging without owning that drag.
    pub(crate) fn remove_desktop_pointer_device(&mut self, source: &str) {
        let state = &mut self.desktop_pointer;
        let Some(device) = state.devices.remove(source) else {
            return;
        };
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
            .desktop_pointer
            .capture
            .as_ref()
            .map(|(id, _)| *id)
            .or_else(|| self.surface_at(position, client_present).map(|(id, _)| id));
        let Some(id) = target else {
            return false;
        };
        let captured = self.desktop_pointer.capture.is_some();
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

        let state = &mut self.desktop_pointer;
        let device = *state.devices.entry(source.to_owned()).or_insert_with(|| {
            state.next_device += 1;
            DeviceId(state.next_device)
        });
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
