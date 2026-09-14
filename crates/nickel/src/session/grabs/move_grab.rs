use crate::session::{NickelSession, focus::PointerFocusTarget};
use nickel_core::window_operation::{
    BeginRequest, CancellationReason, CompletionBinding, Disposition, Effect, GeometrySeed,
    GeometryUpdate, OperationId, ResourceLeaseId, WindowOperationReducer,
};
use smithay::{
    desktop::Window,
    input::pointer::{
        ButtonEvent, GrabStartData as PointerGrabStartData, MotionEvent, PointerGrab,
        PointerInnerHandle,
    },
    utils::{Logical, Point},
};

pub struct MoveSurfaceGrab {
    pub start_data: PointerGrabStartData<NickelSession>,
    pub window: Window,
    pub initial_window_location: Point<i32, Logical>,
    pub last_window_location: Point<i32, Logical>,
    pub restored_from_maximized: bool,
    pub operation: WindowPointerOperation,
}

/// Native-pointer adapter for the shared operation lifecycle.
///
/// Protocol-specific serial/button and focus validation happens before
/// construction. Smithay's pointer grab is the synchronously acquired resource
/// represented by `lease`.
pub struct WindowPointerOperation {
    id: OperationId,
    completion: CompletionBinding,
}

fn crossed_maximized_restore_threshold(delta: Point<f64, Logical>) -> bool {
    delta.x.abs() >= 1.0 || delta.y.abs() >= 1.0
}

impl WindowPointerOperation {
    #[cfg(test)]
    pub fn begin(reducer: &mut WindowOperationReducer, request: BeginRequest) -> Option<Self> {
        let completion = request.origin;
        let (operation, transition) = reducer.begin(request);
        Self::finish_begin(reducer, operation?, transition, completion)
    }

    pub fn begin_with_geometry(
        reducer: &mut WindowOperationReducer,
        request: BeginRequest,
        geometry: GeometrySeed,
    ) -> Option<Self> {
        let completion = request.origin;
        let (operation, transition) = reducer.begin_with_geometry(request, geometry);
        Self::finish_begin(reducer, operation?, transition, completion)
    }

    fn finish_begin(
        reducer: &mut WindowOperationReducer,
        operation: OperationId,
        transition: nickel_core::window_operation::Transition,
        completion: CompletionBinding,
    ) -> Option<Self> {
        let request = transition.effects.iter().find_map(|effect| match effect {
            Effect::Acquire { request, .. } => Some(*request),
            _ => None,
        });
        let Some(request) = request else {
            let _ = reducer.cancel(operation, CancellationReason::RequiredResourceLost);
            return None;
        };
        let lease = ResourceLeaseId::new(request.get());
        if reducer.acquired(operation, request, lease).disposition != Disposition::Applied
            || reducer.activate(operation).disposition != Disposition::Applied
        {
            let _ = reducer.cancel(operation, CancellationReason::RequiredResourceLost);
            return None;
        }
        Some(Self {
            id: operation,
            completion,
        })
    }

    pub(crate) fn propose(
        &self,
        reducer: &mut WindowOperationReducer,
        x: i64,
        y: i64,
    ) -> Option<nickel_core::geometry::LogicalRect> {
        let transition = reducer.update_geometry(
            self.id,
            self.completion.source,
            GeometryUpdate::AbsoluteDisplacement { x, y },
        );
        transition
            .effects
            .into_iter()
            .find_map(|effect| match effect {
                Effect::GeometryProposed { constrained, .. } => Some(constrained),
                _ => None,
            })
    }

    pub(crate) fn rebase(
        &self,
        reducer: &mut WindowOperationReducer,
        anchor: nickel_core::geometry::LogicalRect,
    ) -> bool {
        reducer
            .rebase_geometry(self.id, self.completion.source, anchor)
            .disposition
            == Disposition::Applied
    }

    pub(crate) fn complete(&self, reducer: &mut WindowOperationReducer) -> bool {
        // A higher-priority teardown may already have revoked shared
        // authority. The physical release must still dismantle Smithay's grab.
        if reducer.operation(self.id).is_none() {
            return true;
        }
        let transition = reducer.release(self.id, self.completion);
        transition.disposition == Disposition::Applied
            && transition
                .effects
                .contains(&Effect::SubmitFinalDesiredState { operation: self.id })
    }

    pub(crate) fn cancel(&self, reducer: &mut WindowOperationReducer) {
        self.cancel_for(reducer, CancellationReason::RequiredResourceLost);
    }

    pub(crate) fn cancel_for(
        &self,
        reducer: &mut WindowOperationReducer,
        reason: CancellationReason,
    ) {
        if reducer.operation(self.id).is_some() {
            let _ = reducer.cancel(self.id, reason);
        }
    }

    pub(crate) fn requests_conditional_compensation(
        &self,
        reducer: &WindowOperationReducer,
    ) -> bool {
        reducer.compensation_decision(self.id)
            == Some(nickel_core::window_operation::CompensationDecision::Conditional)
    }

    #[cfg(test)]
    fn id(&self) -> OperationId {
        self.id
    }
}

impl PointerGrab<NickelSession> for MoveSurfaceGrab {
    forward_pointer_grab_events!();

    fn motion(
        &mut self,
        data: &mut NickelSession,
        handle: &mut PointerInnerHandle<'_, NickelSession>,
        _focus: Option<(PointerFocusTarget, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        // While the grab is active, no client has pointer focus
        handle.motion(data, None, event);

        let drag_delta = event.location - self.start_data.location;
        if !self.restored_from_maximized
            && crossed_maximized_restore_threshold(drag_delta)
            && let Some(location) =
                data.restore_maximized_window_for_drag(&self.window, event.location)
        {
            self.initial_window_location = location;
            self.start_data.location = event.location;
            self.restored_from_maximized = true;
            let size = self.window.geometry().size;
            if !self.operation.rebase(
                &mut data.window_operations,
                nickel_core::geometry::LogicalRect {
                    x: location.x,
                    y: location.y,
                    width: size.w,
                    height: size.h,
                },
            ) {
                return;
            }
        }

        let delta = event.location - self.start_data.location;
        let Some(proposal) = self.operation.propose(
            &mut data.window_operations,
            delta.x.round() as i64,
            delta.y.round() as i64,
        ) else {
            return;
        };
        self.last_window_location = Point::from((proposal.x, proposal.y));
        data.map_compositor_moved_window(self.window.clone(), self.last_window_location, true);
    }

    fn button(
        &mut self,
        data: &mut NickelSession,
        handle: &mut PointerInnerHandle<'_, NickelSession>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);

        let initiating_button_released =
            !handle.current_pressed().contains(&self.start_data.button);
        if initiating_button_released && self.operation.complete(&mut data.window_operations) {
            // The initiating button released and the shared reducer committed.
            if self.window.x11_surface().is_some()
                && let Some(location) = data.space.element_location(&self.window)
            {
                let size = self.window.geometry().size;
                data.record_x11_interactive_final(
                    &self.window,
                    crate::session::shell_layout::Geometry {
                        x: location.x,
                        y: location.y,
                        width: size.w.max(1),
                        height: size.h.max(1),
                    },
                );
            }
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }

    fn unset(&mut self, data: &mut NickelSession) {
        // Covers target loss, grab replacement, and seat/resource teardown.
        // Normal completion has already removed the operation.
        self.operation.cancel(&mut data.window_operations);
        if self
            .operation
            .requests_conditional_compensation(&data.window_operations)
            && data.space.element_location(&self.window) == Some(self.last_window_location)
        {
            data.map_compositor_moved_window(
                self.window.clone(),
                self.initial_window_location,
                true,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::window_operation::{
        CompletionGesture, MappingGeneration, NativeLifetimeId, OperationKind, OperationPhase,
        SeatId, Source, SourceGeneration, SourceId, WindowId, WindowMapping,
    };
    use nickel_core::{
        geometry::LogicalRect,
        geometry_authority::{ControlMode, GeometryConstraints},
    };

    fn request(button: u16) -> BeginRequest {
        BeginRequest {
            seat: SeatId::new(1),
            subject: WindowMapping {
                window: WindowId::new(10),
                native_lifetime: NativeLifetimeId::new(20),
                generation: MappingGeneration::new(30),
            },
            kind: OperationKind::Move,
            control: ControlMode::Enforced,
            origin: CompletionBinding {
                source: Source {
                    id: SourceId::new(1),
                    generation: SourceGeneration::new(1),
                },
                gesture: CompletionGesture::Button(button),
            },
            optional_update_sources: Vec::new(),
        }
    }

    fn geometry() -> GeometrySeed {
        GeometrySeed {
            anchor: LogicalRect {
                x: 20,
                y: 30,
                width: 640,
                height: 480,
            },
            constraints: GeometryConstraints {
                min_width: 1,
                min_height: 1,
                max_width: None,
                max_height: None,
            },
        }
    }

    #[test]
    fn production_adapter_drives_shared_begin_update_and_completion() {
        let mut reducer = WindowOperationReducer::default();
        let operation =
            WindowPointerOperation::begin_with_geometry(&mut reducer, request(0x110), geometry())
                .unwrap();

        assert_eq!(
            reducer.operation(operation.id()).unwrap().phase,
            OperationPhase::Active
        );
        assert_eq!(
            operation.propose(&mut reducer, 12, -4),
            Some(LogicalRect {
                x: 32,
                y: 26,
                width: 640,
                height: 480,
            })
        );
        assert!(operation.complete(&mut reducer));
        assert!(reducer.operation(operation.id()).is_none());
    }

    #[test]
    fn production_adapter_rejects_competing_move_before_native_grab_install() {
        let mut reducer = WindowOperationReducer::default();
        let first = WindowPointerOperation::begin(&mut reducer, request(0x110)).unwrap();

        assert!(WindowPointerOperation::begin(&mut reducer, request(0x111)).is_none());
        assert_eq!(
            reducer.operation(first.id()).unwrap().phase,
            OperationPhase::Active
        );
    }

    #[test]
    fn unexpected_native_unset_cancels_shared_operation_once() {
        let mut reducer = WindowOperationReducer::default();
        let operation = WindowPointerOperation::begin(&mut reducer, request(0x110)).unwrap();

        operation.cancel(&mut reducer);
        operation.cancel(&mut reducer);

        assert!(reducer.operation(operation.id()).is_none());
        assert!(operation.complete(&mut reducer));
    }

    #[test]
    fn maximized_restore_keeps_the_existing_one_logical_unit_threshold() {
        assert!(!crossed_maximized_restore_threshold((0.99, -0.99).into()));
        assert!(crossed_maximized_restore_threshold((1.0, 0.0).into()));
        assert!(crossed_maximized_restore_threshold((0.0, -1.0).into()));
    }
}
