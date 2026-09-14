use crate::session::{
    NickelSession, focus::PointerFocusTarget, grabs::move_grab::WindowPointerOperation,
};
use nickel_core::window_operation::{HorizontalEdge, ResizeEdges, VerticalEdge};
use smithay::{
    desktop::{Space, Window},
    input::pointer::{
        ButtonEvent, GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab,
        PointerInnerHandle,
    },
    reexports::{
        wayland_protocols::xdg::shell::server::xdg_toplevel,
        wayland_server::protocol::wl_surface::WlSurface,
    },
    utils::{Logical, Point, Rectangle, Size},
    wayland::{
        compositor,
        shell::xdg::{SurfaceCachedState, ToplevelCachedState},
    },
};
use std::cell::RefCell;

bitflags::bitflags! {
    #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
    pub struct ResizeEdge: u32 {
        const TOP          = 0b0001;
        const BOTTOM       = 0b0010;
        const LEFT         = 0b0100;
        const RIGHT        = 0b1000;

        const TOP_LEFT     = Self::TOP.bits() | Self::LEFT.bits();
        const BOTTOM_LEFT  = Self::BOTTOM.bits() | Self::LEFT.bits();

        const TOP_RIGHT    = Self::TOP.bits() | Self::RIGHT.bits();
        const BOTTOM_RIGHT = Self::BOTTOM.bits() | Self::RIGHT.bits();
    }
}

impl From<xdg_toplevel::ResizeEdge> for ResizeEdge {
    #[inline]
    fn from(x: xdg_toplevel::ResizeEdge) -> Self {
        Self::from_bits(x as u32).unwrap()
    }
}

impl From<smithay::xwayland::xwm::ResizeEdge> for ResizeEdge {
    fn from(edge: smithay::xwayland::xwm::ResizeEdge) -> Self {
        use smithay::xwayland::xwm::ResizeEdge as X11Edge;
        match edge {
            X11Edge::Top => Self::TOP,
            X11Edge::Bottom => Self::BOTTOM,
            X11Edge::Left => Self::LEFT,
            X11Edge::TopLeft => Self::TOP_LEFT,
            X11Edge::BottomLeft => Self::BOTTOM_LEFT,
            X11Edge::Right => Self::RIGHT,
            X11Edge::TopRight => Self::TOP_RIGHT,
            X11Edge::BottomRight => Self::BOTTOM_RIGHT,
        }
    }
}

pub struct ResizeSurfaceGrab {
    start_data: PointerGrabStartData<NickelSession>,
    window: Window,

    edges: ResizeEdge,

    initial_rect: Rectangle<i32, Logical>,
    last_window_size: Size<i32, Logical>,
    operation: Option<WindowPointerOperation>,
    terminal: bool,
}

impl ResizeSurfaceGrab {
    pub fn start(
        start_data: PointerGrabStartData<NickelSession>,
        window: Window,
        edges: ResizeEdge,
        initial_window_rect: Rectangle<i32, Logical>,
    ) -> Self {
        Self::start_with_operation(start_data, window, edges, initial_window_rect, None)
    }

    pub fn start_with_operation(
        start_data: PointerGrabStartData<NickelSession>,
        window: Window,
        edges: ResizeEdge,
        initial_window_rect: Rectangle<i32, Logical>,
        operation: Option<WindowPointerOperation>,
    ) -> Self {
        let initial_rect = initial_window_rect;

        if let Some(toplevel) = window.toplevel() {
            ResizeSurfaceState::with(toplevel.wl_surface(), |state| {
                *state = ResizeSurfaceState::Resizing {
                    edges,
                    initial_rect,
                };
            });
        }

        Self {
            start_data,
            window,
            edges,
            initial_rect,
            last_window_size: initial_rect.size,
            operation,
            terminal: false,
        }
    }
}

pub fn operation_resize_edges(edges: ResizeEdge) -> Option<ResizeEdges> {
    let horizontal = if edges.contains(ResizeEdge::LEFT) {
        Some(HorizontalEdge::Left)
    } else if edges.contains(ResizeEdge::RIGHT) {
        Some(HorizontalEdge::Right)
    } else {
        None
    };
    let vertical = if edges.contains(ResizeEdge::TOP) {
        Some(VerticalEdge::Top)
    } else if edges.contains(ResizeEdge::BOTTOM) {
        Some(VerticalEdge::Bottom)
    } else {
        None
    };
    ResizeEdges::new(horizontal, vertical).ok()
}

impl PointerGrab<NickelSession> for ResizeSurfaceGrab {
    forward_pointer_grab_events!();

    fn unset(&mut self, data: &mut NickelSession) {
        if self.terminal {
            return;
        }
        if let Some(operation) = &self.operation {
            operation.cancel(&mut data.window_operations);
        }
        if let Some(x11) = self.window.x11_surface() {
            if let Err(error) = x11.configure(self.initial_rect) {
                tracing::warn!(?error, "X11 cancelled resize compensation failed");
            }
            data.space
                .map_element(self.window.clone(), self.initial_rect.loc, false);
        } else if let Some(xdg) = self.window.toplevel() {
            xdg.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Resizing);
                state.size = Some(self.initial_rect.size);
            });
            let final_configure = xdg.send_pending_configure();
            ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                *state = ResizeSurfaceState::WaitingForLastCommit {
                    edges: self.edges,
                    initial_rect: self.initial_rect,
                    final_configure,
                };
            });
        }
    }

    fn motion(
        &mut self,
        data: &mut NickelSession,
        handle: &mut PointerInnerHandle<'_, NickelSession>,
        _focus: Option<(PointerFocusTarget, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        if self
            .operation
            .as_ref()
            .is_some_and(|operation| !operation.admits_motion(&mut data.window_operations))
        {
            return;
        }

        let mut delta = event.location - self.start_data.location;

        let mut new_window_width = self.initial_rect.size.w;
        let mut new_window_height = self.initial_rect.size.h;

        if self.edges.intersects(ResizeEdge::LEFT | ResizeEdge::RIGHT) {
            if self.edges.intersects(ResizeEdge::LEFT) {
                delta.x = -delta.x;
            }

            new_window_width = (self.initial_rect.size.w as f64 + delta.x) as i32;
        }

        if self.edges.intersects(ResizeEdge::TOP | ResizeEdge::BOTTOM) {
            if self.edges.intersects(ResizeEdge::TOP) {
                delta.y = -delta.y;
            }

            new_window_height = (self.initial_rect.size.h as f64 + delta.y) as i32;
        }

        let (min_size, max_size) = if let Some(surface) = self.window.x11_surface() {
            (
                surface.min_size().unwrap_or_else(|| Size::from((1, 1))),
                surface
                    .max_size()
                    .unwrap_or_else(|| Size::from((i32::MAX, i32::MAX))),
            )
        } else {
            compositor::with_states(self.window.toplevel().unwrap().wl_surface(), |states| {
                let mut guard = states.cached_state.get::<SurfaceCachedState>();
                let data = guard.current();
                (data.min_size, data.max_size)
            })
        };

        let min_width = min_size.w.max(1);
        let min_height = min_size.h.max(1);

        let max_width = if max_size.w == 0 {
            i32::MAX
        } else {
            max_size.w
        };
        let max_height = if max_size.h == 0 {
            i32::MAX
        } else {
            max_size.h
        };

        self.last_window_size = Size::from((
            new_window_width.max(min_width).min(max_width),
            new_window_height.max(min_height).min(max_height),
        ));

        if let Some(x11) = self.window.x11_surface() {
            let mut location = self.initial_rect.loc;
            if self.edges.contains(ResizeEdge::LEFT) {
                location.x += self.initial_rect.size.w - self.last_window_size.w;
            }
            if self.edges.contains(ResizeEdge::TOP) {
                location.y += self.initial_rect.size.h - self.last_window_size.h;
            }
            let geometry = Rectangle::new(location, self.last_window_size);
            if let Err(error) = x11.configure(geometry) {
                tracing::warn!(?error, "X11 interactive resize failed");
            }
            data.space.map_element(self.window.clone(), location, false);
        } else {
            let xdg = self.window.toplevel().unwrap();
            xdg.with_pending_state(|state| {
                state.states.set(xdg_toplevel::State::Resizing);
                state.size = Some(self.last_window_size);
            });
            xdg.send_pending_configure();
        }
    }

    fn button(
        &mut self,
        data: &mut NickelSession,
        handle: &mut PointerInnerHandle<'_, NickelSession>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        // The button is a button code as defined in the
        if !handle.current_pressed().contains(&self.start_data.button)
            && self
                .operation
                .as_ref()
                .is_none_or(|operation| operation.complete(&mut data.window_operations))
        {
            // The initiating button released; free the seat before settlement.
            if self.window.x11_surface().is_some() {
                let mut location = self.initial_rect.loc;
                if self.edges.contains(ResizeEdge::LEFT) {
                    location.x += self.initial_rect.size.w - self.last_window_size.w;
                }
                if self.edges.contains(ResizeEdge::TOP) {
                    location.y += self.initial_rect.size.h - self.last_window_size.h;
                }
                data.record_x11_interactive_final(
                    &self.window,
                    crate::session::shell_layout::Geometry {
                        x: location.x,
                        y: location.y,
                        width: self.last_window_size.w,
                        height: self.last_window_size.h,
                    },
                );
            }
            self.terminal = true;
            handle.unset_grab(self, data, event.serial, event.time, true);

            if let Some(xdg) = self.window.toplevel() {
                xdg.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Resizing);
                    state.size = Some(self.last_window_size);
                });
                let final_configure = xdg.send_pending_configure();
                ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                    *state = ResizeSurfaceState::WaitingForLastCommit {
                        edges: self.edges,
                        initial_rect: self.initial_rect,
                        final_configure,
                    };
                });
            }
        }
    }
}

/// State of the resize operation.
///
/// It is stored inside of WlSurface,
/// and can be accessed using [`ResizeSurfaceState::with`]
#[derive(Debug, Clone, Copy, Eq, PartialEq, Default)]
enum ResizeSurfaceState {
    #[default]
    Idle,
    Resizing {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
    },
    /// Resize is done, we are now waiting for last commit, to do the final move
    WaitingForLastCommit {
        edges: ResizeEdge,
        /// The initial window size and location.
        initial_rect: Rectangle<i32, Logical>,
        final_configure: Option<smithay::utils::Serial>,
    },
}

impl ResizeSurfaceState {
    fn with<F, T>(surface: &WlSurface, cb: F) -> T
    where
        F: FnOnce(&mut Self) -> T,
    {
        compositor::with_states(surface, |states| {
            states.data_map.insert_if_missing(RefCell::<Self>::default);
            let state = states.data_map.get::<RefCell<Self>>().unwrap();

            cb(&mut state.borrow_mut())
        })
    }

    fn commit(
        &mut self,
        last_acked: Option<smithay::utils::Serial>,
    ) -> Option<(ResizeEdge, Rectangle<i32, Logical>)> {
        match *self {
            Self::Resizing {
                edges,
                initial_rect,
            } => Some((edges, initial_rect)),
            Self::WaitingForLastCommit {
                edges,
                initial_rect,
                final_configure,
            } => {
                if final_configure.is_none_or(|final_configure| {
                    last_acked.is_some_and(|acked| acked.is_no_older_than(&final_configure))
                }) {
                    *self = Self::Idle;
                }

                Some((edges, initial_rect))
            }
            Self::Idle => None,
        }
    }
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(space: &mut Space<Window>, surface: &WlSurface) -> Option<()> {
    let window = space
        .elements()
        .find(|window| {
            window
                .toplevel()
                .is_some_and(|toplevel| toplevel.wl_surface() == surface)
        })
        .cloned()?;

    let mut window_loc = space.element_location(&window)?;
    let geometry = window.geometry();

    let last_acked = compositor::with_states(surface, |states| {
        states
            .cached_state
            .get::<ToplevelCachedState>()
            .current()
            .last_acked
            .as_ref()
            .map(|configure| configure.serial)
    });
    let new_loc: Point<Option<i32>, Logical> = ResizeSurfaceState::with(surface, |state| {
        state
            .commit(last_acked)
            .and_then(|(edges, initial_rect)| {
                // If the window is being resized by top or left, its location must be adjusted
                // accordingly.
                edges.intersects(ResizeEdge::TOP_LEFT).then(|| {
                    let new_x = edges
                        .intersects(ResizeEdge::LEFT)
                        .then_some(initial_rect.loc.x + (initial_rect.size.w - geometry.size.w));

                    let new_y = edges
                        .intersects(ResizeEdge::TOP)
                        .then_some(initial_rect.loc.y + (initial_rect.size.h - geometry.size.h));

                    (new_x, new_y).into()
                })
            })
            .unwrap_or_default()
    });

    if let Some(new_x) = new_loc.x {
        window_loc.x = new_x;
    }
    if let Some(new_y) = new_loc.y {
        window_loc.y = new_y;
    }

    if new_loc.x.is_some() || new_loc.y.is_some() {
        // If TOP or LEFT side of the window got resized, we have to move it
        space.map_element(window, window_loc, false);
    }

    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn initial_rect() -> Rectangle<i32, Logical> {
        Rectangle::new((10, 20).into(), (300, 200).into())
    }

    #[test]
    fn every_native_resize_edge_maps_to_a_valid_shared_kind() {
        for edges in [
            ResizeEdge::TOP,
            ResizeEdge::BOTTOM,
            ResizeEdge::LEFT,
            ResizeEdge::RIGHT,
            ResizeEdge::TOP_LEFT,
            ResizeEdge::TOP_RIGHT,
            ResizeEdge::BOTTOM_LEFT,
            ResizeEdge::BOTTOM_RIGHT,
        ] {
            assert!(operation_resize_edges(edges).is_some());
        }
        assert!(operation_resize_edges(ResizeEdge::empty()).is_none());
    }

    #[test]
    fn commit_before_final_configure_ack_does_not_finish_settlement() {
        let mut state = ResizeSurfaceState::WaitingForLastCommit {
            edges: ResizeEdge::LEFT,
            initial_rect: initial_rect(),
            final_configure: Some(12_u32.into()),
        };

        assert!(state.commit(Some(11_u32.into())).is_some());
        assert!(matches!(
            state,
            ResizeSurfaceState::WaitingForLastCommit { .. }
        ));
    }

    #[test]
    fn final_or_newer_configure_ack_finishes_settlement() {
        for acknowledged in [12_u32, 13_u32] {
            let mut state = ResizeSurfaceState::WaitingForLastCommit {
                edges: ResizeEdge::TOP_LEFT,
                initial_rect: initial_rect(),
                final_configure: Some(12_u32.into()),
            };

            assert!(state.commit(Some(acknowledged.into())).is_some());
            assert_eq!(state, ResizeSurfaceState::Idle);
        }
    }
}
