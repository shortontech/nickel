mod compatibility;
mod compositor;
mod xdg_activation;
mod xdg_shell;
mod xwayland;

use crate::{
    NickelSession,
    focus::{KeyboardFocusTarget, PointerFocusTarget},
};

//
// Wl Seat
//

use smithay::input::{
    Seat, SeatHandler, SeatState,
    dnd::{DnDGrab, DndGrabHandler, DndTarget, GrabType, Source},
    pointer::Focus,
};
use smithay::reexports::wayland_server::Resource;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::wayland::output::OutputHandler;
use smithay::wayland::seat::WaylandFocus;
use smithay::wayland::selection::data_device::{
    DataDeviceHandler, DataDeviceState, WaylandDndGrabHandler, set_data_device_focus,
};
use smithay::wayland::selection::primary_selection::set_primary_focus;
use smithay::wayland::selection::{SelectionHandler, SelectionSource, SelectionTarget};
use smithay::{
    delegate_dispatch2,
    utils::{Logical, Point, Serial},
};

const MAX_SELECTION_MIME_TYPES: usize = 64;
const MAX_SELECTION_MIME_TYPE_BYTES: usize = 256;

fn bounded_selection_mime_types(mime_types: Vec<String>) -> Vec<String> {
    let mut bounded = Vec::with_capacity(mime_types.len().min(MAX_SELECTION_MIME_TYPES));
    for mime_type in mime_types {
        if !mime_type.is_empty()
            && mime_type.len() <= MAX_SELECTION_MIME_TYPE_BYTES
            && !bounded.contains(&mime_type)
        {
            bounded.push(mime_type);
            if bounded.len() == MAX_SELECTION_MIME_TYPES {
                break;
            }
        }
    }
    bounded
}

impl SeatHandler for NickelSession {
    type KeyboardFocus = KeyboardFocusTarget;
    type PointerFocus = PointerFocusTarget;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<NickelSession> {
        &mut self.seat_state
    }

    fn cursor_image(
        &mut self,
        _seat: &Seat<Self>,
        _image: smithay::input::pointer::CursorImageStatus,
    ) {
    }

    fn focus_changed(&mut self, seat: &Seat<Self>, focused: Option<&KeyboardFocusTarget>) {
        let dh = &self.display_handle;
        let focused_surface = focused.and_then(|focus| focus.wl_surface());
        let client = focused_surface
            .as_deref()
            .and_then(|surface| dh.get_client(surface.id()).ok());
        set_data_device_focus(dh, seat, client);
        let client = focused_surface
            .as_deref()
            .and_then(|surface| dh.get_client(surface.id()).ok());
        set_primary_focus(dh, seat, client);
        self.launcher_keyboard_focus_changed(focused_surface.as_deref());
    }
}

//
// Wl Data Device
//

#[derive(Clone)]
pub enum SelectionOwner {
    XWayland(smithay::xwayland::xwm::XwmId),
}

impl SelectionHandler for NickelSession {
    type SelectionUserData = SelectionOwner;

    fn new_selection(
        &mut self,
        target: SelectionTarget,
        source: Option<SelectionSource>,
        _seat: Seat<Self>,
    ) {
        if let Some((_, xwm)) = self.xwm.as_mut() {
            let mime_types = source.map(|source| bounded_selection_mime_types(source.mime_types()));
            if let Err(error) = xwm.new_selection(target, mime_types) {
                tracing::warn!(
                    ?error,
                    ?target,
                    "failed to mirror Wayland selection to XWayland"
                );
            }
        }
    }

    fn send_selection(
        &mut self,
        target: SelectionTarget,
        mime_type: String,
        fd: std::os::fd::OwnedFd,
        _seat: Seat<Self>,
        owner: &Self::SelectionUserData,
    ) {
        let SelectionOwner::XWayland(owner_id) = owner;
        let Some((xwm_id, xwm)) = self.xwm.as_mut() else {
            return;
        };
        if xwm_id == owner_id
            && let Err(error) = xwm.send_selection(target, mime_type, fd)
        {
            tracing::warn!(?error, ?target, "failed to read XWayland selection");
        }
    }
}

impl DataDeviceHandler for NickelSession {
    fn data_device_state(&mut self) -> &mut DataDeviceState {
        &mut self.data_device_state
    }
}

impl WaylandDndGrabHandler for NickelSession {
    fn dnd_requested<S: Source>(
        &mut self,
        source: S,
        icon: Option<WlSurface>,
        seat: Seat<Self>,
        serial: Serial,
        grab_type: GrabType,
    ) {
        self.dnd_icon = icon;
        match grab_type {
            GrabType::Pointer => {
                let pointer = seat.get_pointer().expect("pointer drag requires a pointer");
                let start_data = pointer
                    .grab_start_data()
                    .expect("pointer drag requires grab start data");
                pointer.set_grab(
                    self,
                    DnDGrab::new_pointer(&self.display_handle, start_data, source, seat),
                    serial,
                    Focus::Keep,
                );
            }
            GrabType::Touch => {
                let touch = seat
                    .get_touch()
                    .expect("touch drag requires a touch device");
                let start_data = touch
                    .grab_start_data()
                    .expect("touch drag requires grab start data");
                touch.set_grab(
                    self,
                    DnDGrab::new_touch(&self.display_handle, start_data, source, seat),
                    serial,
                );
            }
        }
    }
}

impl DndGrabHandler for NickelSession {
    fn dropped(
        &mut self,
        _target: Option<DndTarget<'_, Self>>,
        _validated: bool,
        _seat: Seat<Self>,
        _location: Point<f64, Logical>,
    ) {
        self.dnd_icon = None;
    }
}

#[cfg(test)]
mod selection_policy_tests {
    use super::{
        MAX_SELECTION_MIME_TYPE_BYTES, MAX_SELECTION_MIME_TYPES, bounded_selection_mime_types,
    };

    #[test]
    fn selection_bridge_bounds_and_deduplicates_advertised_types() {
        let mut offered = vec![String::new(), "text/plain".into(), "text/plain".into()];
        offered.push("x".repeat(MAX_SELECTION_MIME_TYPE_BYTES + 1));
        offered.extend((0..MAX_SELECTION_MIME_TYPES + 5).map(|index| format!("type/{index}")));

        let bounded = bounded_selection_mime_types(offered);

        assert_eq!(bounded.len(), MAX_SELECTION_MIME_TYPES);
        assert_eq!(bounded[0], "text/plain");
        assert!(bounded.iter().all(|mime_type| {
            !mime_type.is_empty() && mime_type.len() <= MAX_SELECTION_MIME_TYPE_BYTES
        }));
    }
}

//
// Wl Output & Xdg Output
//

impl OutputHandler for NickelSession {}

delegate_dispatch2!(NickelSession);
