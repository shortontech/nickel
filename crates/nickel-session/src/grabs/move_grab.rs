use crate::{NickelSession, focus::PointerFocusTarget};
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
            && (drag_delta.x.abs() >= 1.0 || drag_delta.y.abs() >= 1.0)
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

        if !handle.current_pressed().contains(&self.start_data.button) {
            // No more buttons are pressed, release the grab.
            handle.unset_grab(self, data, event.serial, event.time, true);
        }
    }
}
