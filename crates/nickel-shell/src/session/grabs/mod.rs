/// Implements the pointer events that interactive grabs pass through unchanged.
///
/// Motion and button handling remain in each grab because those events drive the
/// operation itself. Keeping the mechanical forwarding here prevents the two
/// implementations from drifting as Smithay's pointer protocol evolves.
macro_rules! forward_pointer_grab_events {
    () => {
        fn relative_motion(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            focus: Option<(
                crate::session::focus::PointerFocusTarget,
                smithay::utils::Point<f64, smithay::utils::Logical>,
            )>,
            event: &smithay::input::pointer::RelativeMotionEvent,
        ) {
            handle.relative_motion(data, focus, event);
        }

        fn axis(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            details: smithay::input::pointer::AxisFrame,
        ) {
            handle.axis(data, details)
        }

        fn frame(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
        ) {
            handle.frame(data);
        }

        fn gesture_swipe_begin(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            event: &smithay::input::pointer::GestureSwipeBeginEvent,
        ) {
            handle.gesture_swipe_begin(data, event)
        }

        fn gesture_swipe_update(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            event: &smithay::input::pointer::GestureSwipeUpdateEvent,
        ) {
            handle.gesture_swipe_update(data, event)
        }

        fn gesture_swipe_end(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            event: &smithay::input::pointer::GestureSwipeEndEvent,
        ) {
            handle.gesture_swipe_end(data, event)
        }

        fn gesture_pinch_begin(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            event: &smithay::input::pointer::GesturePinchBeginEvent,
        ) {
            handle.gesture_pinch_begin(data, event)
        }

        fn gesture_pinch_update(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            event: &smithay::input::pointer::GesturePinchUpdateEvent,
        ) {
            handle.gesture_pinch_update(data, event)
        }

        fn gesture_pinch_end(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            event: &smithay::input::pointer::GesturePinchEndEvent,
        ) {
            handle.gesture_pinch_end(data, event)
        }

        fn gesture_hold_begin(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            event: &smithay::input::pointer::GestureHoldBeginEvent,
        ) {
            handle.gesture_hold_begin(data, event)
        }

        fn gesture_hold_end(
            &mut self,
            data: &mut crate::session::NickelSession,
            handle: &mut smithay::input::pointer::PointerInnerHandle<
                '_,
                crate::session::NickelSession,
            >,
            event: &smithay::input::pointer::GestureHoldEndEvent,
        ) {
            handle.gesture_hold_end(data, event)
        }

        fn start_data(
            &self,
        ) -> &smithay::input::pointer::GrabStartData<crate::session::NickelSession> {
            &self.start_data
        }

        fn unset(&mut self, _data: &mut crate::session::NickelSession) {}
    };
}

pub mod move_grab;
pub use move_grab::MoveSurfaceGrab;

pub mod resize_grab;
pub use resize_grab::{ResizeEdge, ResizeSurfaceGrab};
