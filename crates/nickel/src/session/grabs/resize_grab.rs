use crate::session::{
    NickelSession, focus::PointerFocusTarget, grabs::move_grab::WindowPointerOperation,
};
use nickel_core::{
    geometry_authority::GeometryConstraints,
    window_operation::{HorizontalEdge, ResizeEdges, VerticalEdge},
};
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

type ResizeCommit = (
    Window,
    Option<smithay::utils::Serial>,
    Option<Point<i32, Logical>>,
);

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
    last_window_location: Point<i32, Logical>,
    last_window_size: Size<i32, Logical>,
    last_authorization: Option<crate::session::state::InteractiveResizeToken>,
    operation: WindowPointerOperation,
    terminal: bool,
}

impl ResizeSurfaceGrab {
    pub fn start_with_operation(
        start_data: PointerGrabStartData<NickelSession>,
        window: Window,
        edges: ResizeEdge,
        initial_window_rect: Rectangle<i32, Logical>,
        operation: WindowPointerOperation,
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
            last_window_location: initial_rect.loc,
            last_window_size: initial_rect.size,
            last_authorization: None,
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

pub(crate) fn xdg_commit_causality(
    configured: &[(
        smithay::utils::Serial,
        nickel_core::geometry_authority::DesiredRevisions,
        Option<ResizeEdge>,
    )],
    acknowledged: smithay::utils::Serial,
    request: nickel_core::geometry_authority::NativeRequestId,
    current: nickel_core::geometry_authority::DesiredRevisions,
) -> Option<nickel_core::geometry_authority::ObservationCausality> {
    let incorporated = configured
        .iter()
        .rev()
        .find_map(|(serial, revisions, _)| (*serial == acknowledged).then_some(*revisions))?;
    if incorporated == current {
        Some(nickel_core::geometry_authority::ObservationCausality::Correlated(request))
    } else {
        None
    }
}

pub(crate) fn operation_geometry_constraints(window: &Window) -> GeometryConstraints {
    let (min_size, max_size) = if let Some(surface) = window.x11_surface() {
        (
            surface.min_size().unwrap_or_else(|| Size::from((1, 1))),
            surface
                .max_size()
                .unwrap_or_else(|| Size::from((i32::MAX, i32::MAX))),
        )
    } else {
        compositor::with_states(window.toplevel().unwrap().wl_surface(), |states| {
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let data = guard.current();
            (data.min_size, data.max_size)
        })
    };
    GeometryConstraints {
        min_width: min_size.w.max(1),
        min_height: min_size.h.max(1),
        max_width: (max_size.w != 0).then_some(max_size.w.max(1)),
        max_height: (max_size.h != 0).then_some(max_size.h.max(1)),
    }
}

impl PointerGrab<NickelSession> for ResizeSurfaceGrab {
    forward_pointer_grab_events!();

    fn unset(&mut self, data: &mut NickelSession) {
        if self.terminal {
            data.finish_interactive_resize(&self.window);
            if let Some(xdg) = self.window.toplevel() {
                xdg.with_pending_state(|state| {
                    state.states.unset(xdg_toplevel::State::Resizing);
                });
                ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                    state.clear();
                });
            }
            return;
        }
        self.operation.cancel(&mut data.window_operations);
        let compensate = self
            .operation
            .requests_conditional_compensation(&data.window_operations);
        let mut compensation_configure = None;
        if compensate {
            let constraints = operation_geometry_constraints(&self.window);
            if let Some(token) = data.compensate_interactive_resize(&self.window, constraints) {
                compensation_configure = data
                    .apply_authorized_interactive_resize(&self.window, token, false)
                    .flatten();
            }
        }
        data.finish_interactive_resize(&self.window);
        if let Some(xdg) = self.window.toplevel() {
            xdg.with_pending_state(|state| {
                state.states.unset(xdg_toplevel::State::Resizing);
            });
            let terminal_configure =
                compensation_configure.or_else(|| data.send_tracked_xdg_configure(xdg));
            ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                *state = ResizeSurfaceState::WaitingForLastCommit {
                    edges: self.edges,
                    initial_rect: self.initial_rect,
                    terminal_configures: terminal_configure.into_iter().collect(),
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

        let delta = event.location - self.start_data.location;
        let Some(proposal) = self.operation.propose(
            &mut data.window_operations,
            delta.x.round() as i64,
            delta.y.round() as i64,
        ) else {
            return;
        };
        self.last_window_location = Point::from((proposal.x, proposal.y));
        self.last_window_size = Size::from((proposal.width, proposal.height));
        let desired = crate::session::shell_layout::Geometry {
            x: proposal.x,
            y: proposal.y,
            width: proposal.width,
            height: proposal.height,
        };
        let constraints = operation_geometry_constraints(&self.window);
        let Some(token) = data.authorize_interactive_resize(&self.window, desired, constraints)
        else {
            return;
        };
        if data
            .apply_authorized_interactive_resize(&self.window, token, true)
            .is_some()
        {
            self.last_authorization = Some(token);
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
        if !handle.current_pressed().contains(&self.start_data.button) {
            let geometry_authorized = self.operation.complete(&mut data.window_operations);
            // The initiating button released; free the seat before settlement.
            let desired = crate::session::shell_layout::Geometry {
                x: self.last_window_location.x,
                y: self.last_window_location.y,
                width: self.last_window_size.w,
                height: self.last_window_size.h,
            };
            let token = geometry_authorized
                .then_some(self.last_authorization)
                .flatten()
                .or_else(|| {
                    geometry_authorized
                        .then(|| {
                            data.authorize_interactive_resize(
                                &self.window,
                                desired,
                                operation_geometry_constraints(&self.window),
                            )
                        })
                        .flatten()
                });
            if geometry_authorized && self.window.x11_surface().is_some() {
                data.record_x11_interactive_final(&self.window, desired);
            }
            data.finish_interactive_resize(&self.window);
            self.terminal = true;
            handle.unset_grab(self, data, event.serial, event.time, true);

            if let Some(xdg) = self.window.toplevel() {
                let authorized_configure = token
                    .and_then(|token| {
                        data.apply_authorized_interactive_resize(&self.window, token, false)
                    })
                    .flatten();
                let final_configure = authorized_configure.or_else(|| {
                    xdg.with_pending_state(|state| {
                        state.states.unset(xdg_toplevel::State::Resizing);
                    });
                    data.send_tracked_xdg_configure(xdg)
                });
                ResizeSurfaceState::with(xdg.wl_surface(), |state| {
                    *state = ResizeSurfaceState::WaitingForLastCommit {
                        edges: self.edges,
                        initial_rect: self.initial_rect,
                        terminal_configures: final_configure.into_iter().collect(),
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
#[derive(Debug, Clone, Eq, PartialEq, Default)]
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
        terminal_configures: Vec<smithay::utils::Serial>,
    },
}

impl ResizeSurfaceState {
    fn clear(&mut self) {
        *self = Self::Idle;
    }

    fn extend_terminal_correlation(&mut self, serial: smithay::utils::Serial) {
        if let Self::WaitingForLastCommit {
            terminal_configures,
            ..
        } = self
            && !terminal_configures.contains(&serial)
        {
            terminal_configures.push(serial);
        }
    }

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
        match self {
            Self::Resizing {
                edges,
                initial_rect,
            } => Some((*edges, *initial_rect)),
            Self::WaitingForLastCommit {
                edges,
                initial_rect,
                terminal_configures,
            } => {
                if last_acked.is_some_and(|acked| terminal_configures.contains(&acked)) {
                    let result = Some((*edges, *initial_rect));
                    *self = Self::Idle;
                    return result;
                }
                None
            }
            Self::Idle => None,
        }
    }
}

pub(crate) fn current_resize_edges(surface: &WlSurface) -> Option<ResizeEdge> {
    ResizeSurfaceState::with(surface, |state| match state {
        ResizeSurfaceState::Resizing { edges, .. }
        | ResizeSurfaceState::WaitingForLastCommit { edges, .. } => Some(*edges),
        ResizeSurfaceState::Idle => None,
    })
}

pub(crate) fn clear_resize_correlation(surface: &WlSurface) {
    ResizeSurfaceState::with(surface, ResizeSurfaceState::clear);
}

pub(crate) fn extend_terminal_resize_correlation(
    surface: &WlSurface,
    serial: smithay::utils::Serial,
) {
    ResizeSurfaceState::with(surface, |state| {
        state.extend_terminal_correlation(serial);
    });
}

/// Should be called on `WlSurface::commit`
pub fn handle_commit(space: &mut Space<Window>, surface: &WlSurface) -> Option<ResizeCommit> {
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

    let anchor = (new_loc.x.is_some() || new_loc.y.is_some()).then_some(window_loc);
    Some((window, last_acked, anchor))
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
    fn newer_desired_revision_prevents_ack_from_claiming_older_configure() {
        use nickel_core::geometry_authority::{
            GeometryAuthority, GeometryConstraints, NativeRequestId, ObservationCausality,
            Presentation,
        };
        let mut authority = GeometryAuthority::new(
            crate::session::shell_layout::Geometry {
                x: 0,
                y: 0,
                width: 100,
                height: 100,
            },
            Presentation::Normal,
        );
        let incorporated = authority.revisions();
        assert_eq!(
            xdg_commit_causality(
                &[(12_u32.into(), incorporated, None)],
                12_u32.into(),
                NativeRequestId(7),
                authority.revisions(),
            ),
            Some(ObservationCausality::Correlated(NativeRequestId(7)))
        );
        authority.set_placement(
            crate::session::shell_layout::Geometry {
                x: 1,
                y: 0,
                width: 100,
                height: 100,
            },
            GeometryConstraints {
                min_width: 1,
                min_height: 1,
                max_width: None,
                max_height: None,
            },
        );
        assert_eq!(
            xdg_commit_causality(
                &[(12_u32.into(), incorporated, None)],
                12_u32.into(),
                NativeRequestId(7),
                authority.revisions()
            ),
            None
        );
        assert_eq!(
            xdg_commit_causality(
                &[(12_u32.into(), authority.revisions(), None)],
                13_u32.into(),
                NativeRequestId(7),
                authority.revisions(),
            ),
            None,
            "newer acknowledgements are not inferred to incorporate an older configure"
        );
        assert_eq!(
            xdg_commit_causality(
                &[
                    (12_u32.into(), incorporated, Some(ResizeEdge::RIGHT)),
                    (
                        13_u32.into(),
                        authority.revisions(),
                        Some(ResizeEdge::RIGHT)
                    ),
                ],
                13_u32.into(),
                NativeRequestId(7),
                authority.revisions(),
            ),
            Some(ObservationCausality::Correlated(NativeRequestId(7))),
            "a later recorded configure can incorporate the final desired fields"
        );
    }

    #[test]
    fn commit_before_final_configure_ack_does_not_finish_settlement() {
        let mut state = ResizeSurfaceState::WaitingForLastCommit {
            edges: ResizeEdge::LEFT,
            initial_rect: initial_rect(),
            terminal_configures: vec![12_u32.into()],
        };

        assert!(state.commit(Some(11_u32.into())).is_none());
        assert!(matches!(
            state,
            ResizeSurfaceState::WaitingForLastCommit { .. }
        ));
    }

    #[test]
    fn only_exact_recorded_terminal_configure_finishes_settlement() {
        for (acknowledged, recorded, retired) in [
            (12_u32, vec![12_u32.into()], true),
            (13_u32, vec![12_u32.into()], false),
            (13_u32, vec![12_u32.into(), 13_u32.into()], true),
        ] {
            let mut state = ResizeSurfaceState::WaitingForLastCommit {
                edges: ResizeEdge::TOP_LEFT,
                initial_rect: initial_rect(),
                terminal_configures: recorded,
            };

            assert_eq!(state.commit(Some(acknowledged.into())).is_some(), retired);
            assert_eq!(state == ResizeSurfaceState::Idle, retired);
        }
    }

    #[test]
    fn later_configure_can_join_terminal_resize_correlation() {
        let mut state = ResizeSurfaceState::WaitingForLastCommit {
            edges: ResizeEdge::TOP_LEFT,
            initial_rect: initial_rect(),
            terminal_configures: vec![12_u32.into()],
        };
        state.extend_terminal_correlation(13_u32.into());

        assert!(state.commit(Some(13_u32.into())).is_some());
        assert_eq!(state, ResizeSurfaceState::Idle);
    }

    #[test]
    fn unset_cleanup_clears_active_and_waiting_resize_state() {
        let mut states = [
            ResizeSurfaceState::Resizing {
                edges: ResizeEdge::RIGHT,
                initial_rect: initial_rect(),
            },
            ResizeSurfaceState::WaitingForLastCommit {
                edges: ResizeEdge::TOP,
                initial_rect: initial_rect(),
                terminal_configures: vec![12_u32.into()],
            },
        ];
        for state in &mut states {
            state.clear();
            assert_eq!(*state, ResizeSurfaceState::Idle);
        }
    }

    #[test]
    fn maximize_supersedes_active_resize_before_ack_commit_or_later_motion() {
        use nickel_core::{
            geometry::LogicalRect,
            geometry_authority::ControlMode,
            window_operation::{
                BeginRequest, CancellationReason, CompletionBinding, CompletionGesture,
                GeometrySeed, MappingGeneration, NativeLifetimeId, OperationKind, PressEpoch,
                SeatId, Source, SourceGeneration, SourceId, TerminalOutcome, WindowId,
                WindowMapping, WindowOperationReducer,
            },
        };

        let source = Source {
            id: SourceId::new(1),
            generation: SourceGeneration::new(1),
        };
        let subject = WindowMapping {
            window: WindowId::new(7),
            native_lifetime: NativeLifetimeId::new(8),
            generation: MappingGeneration::new(9),
        };
        let mut reducer = WindowOperationReducer::default();
        let operation = WindowPointerOperation::begin_with_geometry(
            &mut reducer,
            BeginRequest {
                seat: SeatId::new(1),
                subject,
                kind: OperationKind::Resize(operation_resize_edges(ResizeEdge::TOP_LEFT).unwrap()),
                control: ControlMode::Cooperative,
                origin: CompletionBinding {
                    source,
                    gesture: CompletionGesture::Button(0x110),
                    press_epoch: PressEpoch::new(1),
                },
                optional_update_sources: Vec::new(),
            },
            GeometrySeed {
                anchor: LogicalRect {
                    x: 10,
                    y: 20,
                    width: 300,
                    height: 200,
                },
                constraints: GeometryConstraints {
                    min_width: 1,
                    min_height: 1,
                    max_width: None,
                    max_height: None,
                },
            },
        )
        .expect("active resize admitted");
        let id = operation.id();
        let mut correlation = ResizeSurfaceState::WaitingForLastCommit {
            edges: ResizeEdge::TOP_LEFT,
            initial_rect: initial_rect(),
            terminal_configures: vec![12_u32.into()],
        };

        let transition = reducer.cancel(id, CancellationReason::Superseded);
        assert_eq!(
            transition.disposition,
            nickel_core::window_operation::Disposition::Applied
        );
        correlation.clear();

        assert_eq!(
            reducer.terminal_outcome(id),
            Some(TerminalOutcome::Cancelled(CancellationReason::Superseded))
        );
        assert!(correlation.commit(Some(12_u32.into())).is_none());
        assert!(
            operation.propose(&mut reducer, 40, 30).is_none(),
            "late pointer motion cannot publish geometry after maximize supersedes the resize"
        );
    }
}
