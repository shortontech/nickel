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
    pub initial_location: Point<i32, Logical>,
    pub operation: WindowPointerOperation,
}

pub(crate) fn operation_window(surface: InternalSurfaceId) -> OperationWindowId {
    OperationWindowId::new((1_u64 << 63) | surface.snapshot_token())
}

impl PointerGrab<NickelSession> for MoveInternalSurfaceGrab {
    forward_pointer_grab_events!();

    fn unset(&mut self, data: &mut NickelSession) {
        self.operation.cancel(&mut data.window_operations);
    }

    fn motion(
        &mut self,
        data: &mut NickelSession,
        handle: &mut PointerInnerHandle<'_, NickelSession>,
        _focus: Option<(PointerFocusTarget, Point<f64, Logical>)>,
        event: &MotionEvent,
    ) {
        handle.motion(data, None, event);
        if !self.operation.admits_motion(&mut data.window_operations) {
            return;
        }
        let delta = event.location - self.start_data.location;
        let Some(mut placement) = data.internal_ui.placement(self.surface).cloned() else {
            self.operation.cancel_for(
                &mut data.window_operations,
                CancellationReason::TargetDestroyed,
            );
            handle.unset_grab(self, data, event.serial, event.time, true);
            return;
        };
        placement.geometry.0 = self.initial_location.x + delta.x.round() as i32;
        placement.geometry.1 = self.initial_location.y + delta.y.round() as i32;
        if data.internal_ui.relocate(self.surface, placement) {
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
