use std::{borrow::Cow, sync::Arc};

use smithay::{
    backend::input::{InputTime, KeyState},
    desktop::{PopupKind, Window},
    input::{
        Seat,
        dnd::{DndFocus, OfferData, Source},
        keyboard::{KeyboardTarget, KeysymHandle, ModifiersState},
        pointer::{
            AxisFrame, ButtonEvent, GestureHoldBeginEvent, GestureHoldEndEvent,
            GesturePinchBeginEvent, GesturePinchEndEvent, GesturePinchUpdateEvent,
            GestureSwipeBeginEvent, GestureSwipeEndEvent, GestureSwipeUpdateEvent, MotionEvent,
            PointerTarget, RelativeMotionEvent,
        },
    },
    reexports::wayland_server::{DisplayHandle, protocol::wl_surface::WlSurface},
    utils::{IsAlive, Logical, Point, Serial},
    wayland::seat::WaylandFocus,
    wayland::selection::data_device::WlOfferData,
    xwayland::{X11Surface, xwm::XwmOfferData},
};

use crate::NickelSession;

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum KeyboardFocusTarget {
    Wayland(WlSurface),
    X11(X11Surface),
}

#[derive(Clone, Debug, PartialEq)]
#[allow(clippy::large_enum_variant)]
pub enum PointerFocusTarget {
    Wayland(WlSurface),
    X11(X11Surface),
}

impl IsAlive for PointerFocusTarget {
    fn alive(&self) -> bool {
        match self {
            Self::Wayland(surface) => surface.alive(),
            Self::X11(surface) => surface.alive(),
        }
    }
}

impl WaylandFocus for PointerFocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Wayland(surface) => Some(Cow::Borrowed(surface)),
            Self::X11(surface) => surface.wl_surface().map(Cow::Owned),
        }
    }
}

impl From<KeyboardFocusTarget> for PointerFocusTarget {
    fn from(target: KeyboardFocusTarget) -> Self {
        match target {
            KeyboardFocusTarget::Wayland(surface) => Self::Wayland(surface),
            KeyboardFocusTarget::X11(surface) => Self::X11(surface),
        }
    }
}

macro_rules! delegate_pointer {
    ($self:ident, $method:ident($($arg:expr),* $(,)?)) => {
        match $self {
            PointerFocusTarget::Wayland(surface) => PointerTarget::<NickelSession>::$method(surface, $($arg),*),
            PointerFocusTarget::X11(surface) => PointerTarget::<NickelSession>::$method(surface, $($arg),*),
        }
    };
}

impl PointerTarget<NickelSession> for PointerFocusTarget {
    fn enter(&self, seat: &Seat<NickelSession>, data: &mut NickelSession, event: &MotionEvent) {
        delegate_pointer!(self, enter(seat, data, event))
    }
    fn motion(&self, seat: &Seat<NickelSession>, data: &mut NickelSession, event: &MotionEvent) {
        delegate_pointer!(self, motion(seat, data, event))
    }
    fn relative_motion(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &RelativeMotionEvent,
    ) {
        delegate_pointer!(self, relative_motion(seat, data, event))
    }
    fn button(&self, seat: &Seat<NickelSession>, data: &mut NickelSession, event: &ButtonEvent) {
        delegate_pointer!(self, button(seat, data, event))
    }
    fn axis(&self, seat: &Seat<NickelSession>, data: &mut NickelSession, frame: AxisFrame) {
        delegate_pointer!(self, axis(seat, data, frame))
    }
    fn frame(&self, seat: &Seat<NickelSession>, data: &mut NickelSession) {
        delegate_pointer!(self, frame(seat, data))
    }
    fn gesture_swipe_begin(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &GestureSwipeBeginEvent,
    ) {
        delegate_pointer!(self, gesture_swipe_begin(seat, data, event))
    }
    fn gesture_swipe_update(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &GestureSwipeUpdateEvent,
    ) {
        delegate_pointer!(self, gesture_swipe_update(seat, data, event))
    }
    fn gesture_swipe_end(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &GestureSwipeEndEvent,
    ) {
        delegate_pointer!(self, gesture_swipe_end(seat, data, event))
    }
    fn gesture_pinch_begin(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &GesturePinchBeginEvent,
    ) {
        delegate_pointer!(self, gesture_pinch_begin(seat, data, event))
    }
    fn gesture_pinch_update(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &GesturePinchUpdateEvent,
    ) {
        delegate_pointer!(self, gesture_pinch_update(seat, data, event))
    }
    fn gesture_pinch_end(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &GesturePinchEndEvent,
    ) {
        delegate_pointer!(self, gesture_pinch_end(seat, data, event))
    }
    fn gesture_hold_begin(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &GestureHoldBeginEvent,
    ) {
        delegate_pointer!(self, gesture_hold_begin(seat, data, event))
    }
    fn gesture_hold_end(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        event: &GestureHoldEndEvent,
    ) {
        delegate_pointer!(self, gesture_hold_end(seat, data, event))
    }
    fn leave(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        serial: Serial,
        time: InputTime,
    ) {
        delegate_pointer!(self, leave(seat, data, serial, time))
    }
}

pub enum NickelOfferData<S: Source> {
    Wayland(WlOfferData<S>),
    X11(XwmOfferData<S>),
}

impl<S: Source> OfferData for NickelOfferData<S> {
    fn disable(&self) {
        match self {
            Self::Wayland(data) => data.disable(),
            Self::X11(data) => data.disable(),
        }
    }
    fn drop(&self) {
        match self {
            Self::Wayland(data) => data.drop(),
            Self::X11(data) => data.drop(),
        }
    }
    fn validated(&self) -> bool {
        match self {
            Self::Wayland(data) => data.validated(),
            Self::X11(data) => data.validated(),
        }
    }
}

impl DndFocus<NickelSession> for PointerFocusTarget {
    type OfferData<S: Source> = NickelOfferData<S>;

    fn enter<S: Source>(
        &self,
        data: &mut NickelSession,
        dh: &DisplayHandle,
        source: Arc<S>,
        seat: &Seat<NickelSession>,
        location: Point<f64, Logical>,
        serial: &Serial,
    ) -> Option<Self::OfferData<S>> {
        match self {
            Self::Wayland(surface) => {
                DndFocus::enter(surface, data, dh, source, seat, location, serial)
                    .map(NickelOfferData::Wayland)
            }
            Self::X11(surface) => {
                DndFocus::enter(surface, data, dh, source, seat, location, serial)
                    .map(NickelOfferData::X11)
            }
        }
    }
    fn motion<S: Source>(
        &self,
        data: &mut NickelSession,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<NickelSession>,
        location: Point<f64, Logical>,
        time: InputTime,
    ) {
        match (self, offer) {
            (Self::Wayland(surface), Some(NickelOfferData::Wayland(offer))) => {
                DndFocus::motion(surface, data, Some(offer), seat, location, time)
            }
            (Self::X11(surface), Some(NickelOfferData::X11(offer))) => {
                DndFocus::motion(surface, data, Some(offer), seat, location, time)
            }
            (Self::Wayland(surface), None) => {
                DndFocus::motion::<S>(surface, data, None, seat, location, time)
            }
            (Self::X11(surface), None) => {
                DndFocus::motion::<S>(surface, data, None, seat, location, time)
            }
            _ => {}
        }
    }
    fn leave<S: Source>(
        &self,
        data: &mut NickelSession,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<NickelSession>,
    ) {
        match (self, offer) {
            (Self::Wayland(surface), Some(NickelOfferData::Wayland(offer))) => {
                DndFocus::leave(surface, data, Some(offer), seat)
            }
            (Self::X11(surface), Some(NickelOfferData::X11(offer))) => {
                DndFocus::leave(surface, data, Some(offer), seat)
            }
            (Self::Wayland(surface), None) => DndFocus::leave::<S>(surface, data, None, seat),
            (Self::X11(surface), None) => DndFocus::leave::<S>(surface, data, None, seat),
            _ => {}
        }
    }
    fn drop<S: Source>(
        &self,
        data: &mut NickelSession,
        offer: Option<&mut Self::OfferData<S>>,
        seat: &Seat<NickelSession>,
    ) {
        match (self, offer) {
            (Self::Wayland(surface), Some(NickelOfferData::Wayland(offer))) => {
                DndFocus::drop(surface, data, Some(offer), seat)
            }
            (Self::X11(surface), Some(NickelOfferData::X11(offer))) => {
                DndFocus::drop(surface, data, Some(offer), seat)
            }
            (Self::Wayland(surface), None) => DndFocus::drop::<S>(surface, data, None, seat),
            (Self::X11(surface), None) => DndFocus::drop::<S>(surface, data, None, seat),
            _ => {}
        }
    }
}

impl KeyboardFocusTarget {
    pub(crate) fn for_window(window: &Window) -> Option<Self> {
        if let Some(surface) = window.x11_surface() {
            Some(Self::X11(surface.clone()))
        } else {
            window.wl_surface().map(Cow::into_owned).map(Self::Wayland)
        }
    }
}

impl From<WlSurface> for KeyboardFocusTarget {
    fn from(surface: WlSurface) -> Self {
        Self::Wayland(surface)
    }
}

impl From<PopupKind> for KeyboardFocusTarget {
    fn from(popup: PopupKind) -> Self {
        Self::Wayland(popup.into())
    }
}

impl From<KeyboardFocusTarget> for WlSurface {
    fn from(focus: KeyboardFocusTarget) -> Self {
        match focus {
            KeyboardFocusTarget::Wayland(surface) => surface,
            KeyboardFocusTarget::X11(surface) => surface
                .wl_surface()
                .expect("an X11 popup grab requires an associated Wayland surface"),
        }
    }
}

impl IsAlive for KeyboardFocusTarget {
    fn alive(&self) -> bool {
        match self {
            Self::Wayland(surface) => surface.alive(),
            Self::X11(surface) => surface.alive(),
        }
    }
}

impl WaylandFocus for KeyboardFocusTarget {
    fn wl_surface(&self) -> Option<Cow<'_, WlSurface>> {
        match self {
            Self::Wayland(surface) => Some(Cow::Borrowed(surface)),
            Self::X11(surface) => WaylandFocus::wl_surface(surface),
        }
    }
}

impl KeyboardTarget<NickelSession> for KeyboardFocusTarget {
    fn enter(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        keys: Vec<KeysymHandle<'_>>,
        serial: Serial,
    ) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::<NickelSession>::enter(surface, seat, data, keys, serial)
            }
            Self::X11(surface) => {
                KeyboardTarget::<NickelSession>::enter(surface, seat, data, keys, serial)
            }
        }
    }

    fn leave(&self, seat: &Seat<NickelSession>, data: &mut NickelSession, serial: Serial) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::<NickelSession>::leave(surface, seat, data, serial)
            }
            Self::X11(surface) => {
                KeyboardTarget::<NickelSession>::leave(surface, seat, data, serial)
            }
        }
    }

    fn key(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        key: KeysymHandle<'_>,
        state: KeyState,
        serial: Serial,
        time: InputTime,
    ) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::<NickelSession>::key(surface, seat, data, key, state, serial, time)
            }
            Self::X11(surface) => {
                KeyboardTarget::<NickelSession>::key(surface, seat, data, key, state, serial, time)
            }
        }
    }

    fn modifiers(
        &self,
        seat: &Seat<NickelSession>,
        data: &mut NickelSession,
        modifiers: ModifiersState,
        serial: Serial,
    ) {
        match self {
            Self::Wayland(surface) => {
                KeyboardTarget::<NickelSession>::modifiers(surface, seat, data, modifiers, serial)
            }
            Self::X11(surface) => {
                KeyboardTarget::<NickelSession>::modifiers(surface, seat, data, modifiers, serial)
            }
        }
    }
}
