use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent, PointerMotionEvent,
        TouchEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent},
        touch::{DownEvent, MotionEvent as TouchMotion, UpEvent},
    },
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::{Logical, Rectangle, SERIAL_COUNTER},
};

use crate::{
    grabs::{MoveSurfaceGrab, ResizeEdge, ResizeSurfaceGrab},
    state::NickelSession,
};

impl NickelSession {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) -> Option<i32> {
        match event {
            InputEvent::Keyboard { event, .. } => {
                // libinput's Smithay backend exposes XKB keycodes (evdev + 8).
                const KEY_LEFTCTRL: u32 = 37;
                const KEY_LEFTALT: u32 = 64;
                const KEY_RIGHTCTRL: u32 = 105;
                const KEY_RIGHTALT: u32 = 108;
                const KEY_LEFTMETA: u32 = 133;
                const KEY_RIGHTMETA: u32 = 134;
                let key_code = event.key_code().raw();
                if matches!(
                    key_code,
                    KEY_LEFTCTRL
                        | KEY_LEFTALT
                        | KEY_RIGHTCTRL
                        | KEY_RIGHTALT
                        | KEY_LEFTMETA
                        | KEY_RIGHTMETA
                ) {
                    match event.state() {
                        KeyState::Pressed => {
                            self.pressed_modifier_keys.insert(key_code);
                        }
                        KeyState::Released => {
                            self.pressed_modifier_keys.remove(&key_code);
                        }
                    }
                }
                let control = self
                    .pressed_modifier_keys
                    .iter()
                    .any(|key| matches!(*key, KEY_LEFTCTRL | KEY_RIGHTCTRL));
                let alt = self
                    .pressed_modifier_keys
                    .iter()
                    .any(|key| matches!(*key, KEY_LEFTALT | KEY_RIGHTALT));
                if control
                    && alt
                    && let Some(vt) = vt_from_key_code(key_code)
                {
                    return (event.state() == KeyState::Pressed).then_some(vt);
                }
                if matches!(key_code, KEY_LEFTMETA | KEY_RIGHTMETA) {
                    if event.state() == KeyState::Released {
                        if !self.super_chorded {
                            self.toggle_launcher();
                        }
                        self.super_chorded = false;
                    }
                    return None;
                }
                let serial = SERIAL_COUNTER.next_serial();
                let time = Event::time_msec(&event);

                self.seat.get_keyboard().unwrap().input::<(), _>(
                    self,
                    event.key_code(),
                    event.state(),
                    serial,
                    time,
                    |_, _, _| FilterResult::Forward,
                );
            }
            InputEvent::PointerMotion { event, .. } => {
                let current = self.seat.get_pointer().unwrap().current_location();
                let max_x = self
                    .space
                    .outputs()
                    .filter_map(|output| self.space.output_geometry(output))
                    .map(|geometry| geometry.loc.x + geometry.size.w)
                    .max()
                    .unwrap_or(1);
                let max_y = self
                    .space
                    .outputs()
                    .filter_map(|output| self.space.output_geometry(output))
                    .map(|geometry| geometry.loc.y + geometry.size.h)
                    .max()
                    .unwrap_or(1);
                let delta = event.delta();
                let position = (
                    (current.x + delta.x).clamp(0.0, f64::from(max_x.saturating_sub(1))),
                    (current.y + delta.y).clamp(0.0, f64::from(max_y.saturating_sub(1))),
                )
                    .into();
                let pointer = self.seat.get_pointer().unwrap();
                pointer.motion(
                    self,
                    self.surface_under(position),
                    &MotionEvent {
                        location: position,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();

                let output_geo = self.space.output_geometry(output).unwrap();

                let pos = event.position_transformed(output_geo.size) + output_geo.loc.to_f64();

                let serial = SERIAL_COUNTER.next_serial();

                let pointer = self.seat.get_pointer().unwrap();

                let under = self.surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time_msec(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();

                const BTN_LEFT: u32 = 0x110;
                const BTN_RIGHT: u32 = 0x111;
                const DOUBLE_CLICK_MS: u32 = 500;
                const DOUBLE_CLICK_DISTANCE: f64 = 6.0;
                const TITLEBAR_HEIGHT: f64 = 40.0;
                let mut suppress_pointer_event = false;
                let super_pressed = self
                    .pressed_modifier_keys
                    .iter()
                    .any(|key| matches!(*key, 133 | 134));

                if button == BTN_LEFT && button_state == ButtonState::Pressed && !super_pressed {
                    let location = pointer.current_location();
                    let titlebar_window = self
                        .space
                        .element_under(location)
                        .map(|(window, _)| window.clone())
                        .filter(|window| {
                            self.panel_window.as_ref() != Some(window)
                                && self.launcher_window.as_ref() != Some(window)
                        })
                        .filter(|window| {
                            self.space.element_bbox(window).is_some_and(|bbox| {
                                location.y >= f64::from(bbox.loc.y)
                                    && location.y < f64::from(bbox.loc.y) + TITLEBAR_HEIGHT
                            })
                        });

                    if let Some(window) = titlebar_window {
                        let surface = window.toplevel().unwrap().clone();
                        let id = surface.wl_surface().id();
                        let is_double_click = self.last_titlebar_click.as_ref().is_some_and(
                            |(previous_id, previous_time, previous_location)| {
                                previous_id == &id
                                    && event.time_msec().wrapping_sub(*previous_time)
                                        <= DOUBLE_CLICK_MS
                                    && (location.x - previous_location.x).abs()
                                        <= DOUBLE_CLICK_DISTANCE
                                    && (location.y - previous_location.y).abs()
                                        <= DOUBLE_CLICK_DISTANCE
                            },
                        );

                        if is_double_click {
                            self.last_titlebar_click = None;
                            self.suppress_left_button_release = true;
                            suppress_pointer_event = true;
                            self.toggle_maximized_toplevel(&surface);
                        } else {
                            self.last_titlebar_click = Some((id, event.time_msec(), location));
                        }
                    } else {
                        self.last_titlebar_click = None;
                    }
                } else if button == BTN_LEFT
                    && button_state == ButtonState::Released
                    && self.suppress_left_button_release
                {
                    self.suppress_left_button_release = false;
                    suppress_pointer_event = true;
                }

                if ButtonState::Pressed == button_state && !pointer.is_grabbed() {
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.space.raise_element(&window, true);
                        if let Some(id) = self
                            .surface_windows
                            .get(&window.toplevel().unwrap().wl_surface().id())
                            .copied()
                            .filter(|_| {
                                self.panel_window.as_ref() != Some(&window)
                                    && self.launcher_window.as_ref() != Some(&window)
                            })
                        {
                            self.windows.raise(id);
                        }
                        if self.panel_window.as_ref() != Some(&window) {
                            keyboard.set_focus(
                                self,
                                Some(window.toplevel().unwrap().wl_surface().clone()),
                                serial,
                            );
                            self.space.elements().for_each(|window| {
                                window.toplevel().unwrap().send_pending_configure();
                            });
                        }
                    } else {
                        self.space.elements().for_each(|window| {
                            window.set_activated(false);
                            window.toplevel().unwrap().send_pending_configure();
                        });
                        keyboard.set_focus(self, Option::<WlSurface>::None, serial);
                    }
                };

                if button == BTN_LEFT
                    && button_state == ButtonState::Pressed
                    && super_pressed
                    && !pointer.is_grabbed()
                    && let Some((window, _)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(window, location)| (window.clone(), location))
                        .filter(|(window, _)| {
                            self.panel_window.as_ref() != Some(window)
                                && self.launcher_window.as_ref() != Some(window)
                                && self.context_menu_window.as_ref() != Some(window)
                        })
                {
                    self.super_chorded = true;
                    let location = pointer.current_location();
                    let initial_window_location = self.space.element_location(&window).unwrap();
                    let start_data = GrabStartData {
                        focus: self.surface_under(location),
                        button,
                        location,
                    };
                    pointer.set_grab(
                        self,
                        MoveSurfaceGrab {
                            start_data,
                            window,
                            initial_window_location,
                        },
                        serial,
                        Focus::Clear,
                    );
                }

                if button == BTN_RIGHT
                    && button_state == ButtonState::Pressed
                    && super_pressed
                    && !pointer.is_grabbed()
                    && let Some((window, _)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(window, location)| (window.clone(), location))
                        .filter(|(window, _)| {
                            self.panel_window.as_ref() != Some(window)
                                && self.launcher_window.as_ref() != Some(window)
                                && self.context_menu_window.as_ref() != Some(window)
                        })
                {
                    self.super_chorded = true;
                    let location = pointer.current_location();
                    let initial_window_location = self.space.element_location(&window).unwrap();
                    let initial_rect =
                        Rectangle::new(initial_window_location, window.geometry().size);
                    let edges = resize_edges_at(location, initial_rect);
                    let start_data = GrabStartData {
                        focus: self.surface_under(location),
                        button,
                        location,
                    };
                    pointer.set_grab(
                        self,
                        ResizeSurfaceGrab::start(start_data, window, edges, initial_rect),
                        serial,
                        Focus::Clear,
                    );
                }

                if !suppress_pointer_event {
                    pointer.button(
                        self,
                        &ButtonEvent {
                            button,
                            state: button_state,
                            serial,
                            time: event.time_msec(),
                        },
                    );
                }
                pointer.frame(self);
            }
            InputEvent::PointerAxis { event, .. } => {
                let source = event.source();

                let horizontal_amount = event.amount(Axis::Horizontal).unwrap_or_else(|| {
                    event.amount_v120(Axis::Horizontal).unwrap_or(0.0) * 15.0 / 120.
                });
                let vertical_amount = event.amount(Axis::Vertical).unwrap_or_else(|| {
                    event.amount_v120(Axis::Vertical).unwrap_or(0.0) * 15.0 / 120.
                });
                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);

                let mut frame = AxisFrame::new(event.time_msec()).source(source);
                if horizontal_amount != 0.0 {
                    frame = frame.value(Axis::Horizontal, horizontal_amount);
                    if let Some(discrete) = horizontal_amount_discrete {
                        frame = frame.v120(Axis::Horizontal, discrete as i32);
                    }
                }
                if vertical_amount != 0.0 {
                    frame = frame.value(Axis::Vertical, vertical_amount);
                    if let Some(discrete) = vertical_amount_discrete {
                        frame = frame.v120(Axis::Vertical, discrete as i32);
                    }
                }

                if source == AxisSource::Finger {
                    if event.amount(Axis::Horizontal) == Some(0.0) {
                        frame = frame.stop(Axis::Horizontal);
                    }
                    if event.amount(Axis::Vertical) == Some(0.0) {
                        frame = frame.stop(Axis::Vertical);
                    }
                }

                let pointer = self.seat.get_pointer().unwrap();
                pointer.axis(self, frame);
                pointer.frame(self);
            }
            InputEvent::TouchDown { event, .. } => {
                let output = self.space.outputs().next()?;
                let geometry = self.space.output_geometry(output)?;
                let location = event.position_transformed(geometry.size) + geometry.loc.to_f64();
                let touch = self.seat.get_touch().unwrap();
                touch.down(
                    self,
                    self.surface_under(location),
                    &DownEvent {
                        slot: event.slot(),
                        location,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::TouchMotion { event, .. } => {
                let output = self.space.outputs().next()?;
                let geometry = self.space.output_geometry(output)?;
                let location = event.position_transformed(geometry.size) + geometry.loc.to_f64();
                let touch = self.seat.get_touch().unwrap();
                touch.motion(
                    self,
                    self.surface_under(location),
                    &TouchMotion {
                        slot: event.slot(),
                        location,
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::TouchUp { event, .. } => {
                let touch = self.seat.get_touch().unwrap();
                touch.up(
                    self,
                    &UpEvent {
                        slot: event.slot(),
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time_msec(),
                    },
                );
            }
            InputEvent::TouchFrame { .. } => self.seat.get_touch().unwrap().frame(self),
            InputEvent::TouchCancel { .. } => self.seat.get_touch().unwrap().cancel(self),
            _ => {}
        }
        None
    }
}

fn resize_edges_at(
    pointer: smithay::utils::Point<f64, Logical>,
    window: Rectangle<i32, Logical>,
) -> ResizeEdge {
    let local_x = pointer.x - f64::from(window.loc.x);
    let local_y = pointer.y - f64::from(window.loc.y);
    let width = f64::from(window.size.w.max(1));
    let height = f64::from(window.size.h.max(1));

    let horizontal = if local_x < width / 3.0 {
        ResizeEdge::LEFT
    } else if local_x > width * 2.0 / 3.0 {
        ResizeEdge::RIGHT
    } else {
        ResizeEdge::empty()
    };
    let vertical = if local_y < height / 3.0 {
        ResizeEdge::TOP
    } else if local_y > height * 2.0 / 3.0 {
        ResizeEdge::BOTTOM
    } else {
        ResizeEdge::empty()
    };

    if horizontal.is_empty() && vertical.is_empty() {
        let distances = [
            (local_x, ResizeEdge::LEFT),
            (width - local_x, ResizeEdge::RIGHT),
            (local_y, ResizeEdge::TOP),
            (height - local_y, ResizeEdge::BOTTOM),
        ];
        distances
            .into_iter()
            .min_by(|left, right| left.0.total_cmp(&right.0))
            .map(|(_, edge)| edge)
            .unwrap_or(ResizeEdge::BOTTOM_RIGHT)
    } else {
        horizontal | vertical
    }
}

fn vt_from_key_code(key_code: u32) -> Option<i32> {
    match key_code {
        67..=76 => Some((key_code - 66) as i32),
        95 => Some(11),
        96 => Some(12),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use smithay::utils::{Point, Rectangle};

    use super::{ResizeEdge, resize_edges_at, vt_from_key_code};

    #[test]
    fn resize_edges_follow_pointer_region() {
        let window = Rectangle::new((100, 100).into(), (900, 600).into());

        assert_eq!(
            resize_edges_at(Point::from((150.0, 150.0)), window),
            ResizeEdge::TOP_LEFT
        );
        assert_eq!(
            resize_edges_at(Point::from((950.0, 650.0)), window),
            ResizeEdge::BOTTOM_RIGHT
        );
        assert_eq!(
            resize_edges_at(Point::from((550.0, 110.0)), window),
            ResizeEdge::TOP
        );
    }

    #[test]
    fn xkb_function_keys_map_to_linux_virtual_terminals() {
        assert_eq!(vt_from_key_code(67), Some(1));
        assert_eq!(vt_from_key_code(76), Some(10));
        assert_eq!(vt_from_key_code(95), Some(11));
        assert_eq!(vt_from_key_code(96), Some(12));
        assert_eq!(vt_from_key_code(1), None);
    }
}
