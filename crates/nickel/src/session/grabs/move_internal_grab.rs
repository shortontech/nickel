use crate::session::{
    NickelSession, focus::PointerFocusTarget, grabs::move_grab::WindowPointerOperation,
};
use nickel_core::window_operation::CancellationReason;
use nickel_core::window_operation::WindowId as OperationWindowId;
use nickel_ui::InternalSurfaceId;
use smithay::input::pointer::{
    ButtonEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
};
use smithay::utils::{Logical, Point};

pub struct MoveInternalSurfaceGrab {
    pub start_data: GrabStartData<NickelSession>,
    pub surface: InternalSurfaceId,
    pub operation: WindowPointerOperation,
}

pub(crate) fn operation_window(surface: InternalSurfaceId) -> OperationWindowId {
    OperationWindowId::new((1_u64 << 63) | surface.snapshot_token())
}

impl PointerGrab<NickelSession> for MoveInternalSurfaceGrab {
    forward_pointer_grab_events!();

    fn unset(&mut self, data: &mut NickelSession) {
        self.operation.cancel(&mut data.window_operations);
        let compensate = self
            .operation
            .requests_conditional_compensation(&data.window_operations);
        if data.finish_internal_move(self.surface, compensate) {
            data.request_output_redraw();
        }
    }

    fn motion(
        &mut self,
        data: &mut NickelSession,
        handle: &mut PointerInnerHandle<'_, NickelSession>,
        _focus: Option<(PointerFocusTarget, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        let delta = event.location - self.start_data.location;
        let Some(mut placement) = data.internal_ui.placement(self.surface).cloned() else {
            self.operation.cancel_for(
                &mut data.window_operations,
                CancellationReason::TargetDestroyed,
            );
            handle.unset_grab(self, data, event.serial, event.time, true);
            return;
        };
        let Some(proposal) = self.operation.propose(
            &mut data.window_operations,
            delta.x.round() as i64,
            delta.y.round() as i64,
        ) else {
            return;
        };
        placement.geometry.0 = proposal.x;
        placement.geometry.1 = proposal.y;
        if data.apply_internal_move(self.surface, placement) {
            data.request_output_redraw();
        }
    }

    fn button(
        &mut self,
        data: &mut NickelSession,
        handle: &mut PointerInnerHandle<'_, NickelSession>,
        event: &ButtonEvent,
    ) {
        handle.button(data, event);
        if !handle.current_pressed().contains(&self.start_data.button)
            && self.operation.complete(&mut data.window_operations)
        {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }
}
