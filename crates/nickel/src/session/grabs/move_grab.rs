use crate::session::{NickelSession, focus::PointerFocusTarget};
use nickel_core::window_operation::{
    BeginRequest, CancellationReason, CompletionBinding, Disposition, Effect, OperationId,
    ResourceLeaseId, WindowOperationReducer,
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
    pub restored_from_maximized: bool,
    /// Present for client-requested XDG/XWayland operations. Compositor-
    /// initiated frame and modifier moves are migrated separately.
    pub operation: Option<WindowPointerOperation>,
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
    pub fn begin(reducer: &mut WindowOperationReducer, request: BeginRequest) -> Option<Self> {
        let completion = request.origin;
        let (operation, transition) = reducer.begin(request);
        let operation = operation?;
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

    pub(crate) fn admits_motion(&self, reducer: &mut WindowOperationReducer) -> bool {
        let transition = reducer.update(self.id, self.completion.source);
        transition.disposition == Disposition::Applied
            && transition.effects.contains(&Effect::ApplyUpdate {
                operation: self.id,
                source: self.completion.source,
            })
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

        if self
            .operation
            .as_ref()
            .is_some_and(|operation| !operation.admits_motion(&mut data.window_operations))
        {
            return;
        }

        let drag_delta = event.location - self.start_data.location;
        if !self.restored_from_maximized
            && crossed_maximized_restore_threshold(drag_delta)
            && let Some(location) =
                data.restore_maximized_window_for_drag(&self.window, event.location)
        {
            self.initial_window_location = location;
            self.start_data.location = event.location;
            self.restored_from_maximized = true;
        }

        let delta = event.location - self.start_data.location;
        let new_location = self.initial_window_location.to_f64() + delta;
        data.map_compositor_moved_window(self.window.clone(), new_location.to_i32_round(), true);
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
        if initiating_button_released
            && self
                .operation
                .as_ref()
                .is_none_or(|operation| operation.complete(&mut data.window_operations))
        {
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
        if let Some(operation) = &self.operation {
            operation.cancel(&mut data.window_operations);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::geometry_authority::ControlMode;
    use nickel_core::window_operation::{
        CompletionGesture, MappingGeneration, NativeLifetimeId, OperationKind, OperationPhase,
        SeatId, Source, SourceGeneration, SourceId, WindowId, WindowMapping,
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

    #[test]
    fn production_adapter_drives_shared_begin_update_and_completion() {
        let mut reducer = WindowOperationReducer::default();
        let operation = WindowPointerOperation::begin(&mut reducer, request(0x110)).unwrap();

        assert_eq!(
            reducer.operation(operation.id()).unwrap().phase,
            OperationPhase::Active
        );
        assert!(operation.admits_motion(&mut reducer));
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
