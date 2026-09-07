use crate::session::{NickelSession, focus::PointerFocusTarget};
use nickel_ui::InternalSurfaceId;
use smithay::input::pointer::{
    ButtonEvent, GrabStartData, MotionEvent, PointerGrab, PointerInnerHandle,
};
use smithay::utils::{Logical, Point};

pub struct MoveInternalSurfaceGrab {
    pub start_data: GrabStartData<NickelSession>,
    pub surface: InternalSurfaceId,
    pub initial_location: Point<i32, Logical>,
}

impl PointerGrab<NickelSession> for MoveInternalSurfaceGrab {
    forward_pointer_grab_events!();

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
        if !handle.current_pressed().contains(&self.start_data.button) {
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }
}
