use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        KeyState, KeyboardKeyEvent, PointerAxisEvent, PointerButtonEvent,
    },
    input::{
        keyboard::FilterResult,
        pointer::{AxisFrame, ButtonEvent, MotionEvent},
    },
    reexports::wayland_server::{Resource, protocol::wl_surface::WlSurface},
    utils::SERIAL_COUNTER,
};

use crate::state::NickelSession;

impl NickelSession {
    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) {
        match event {
            InputEvent::Keyboard { event, .. } => {
                const KEY_LEFTMETA: u32 = 125;
                const KEY_RIGHTMETA: u32 = 126;
                if matches!(event.key_code().raw(), KEY_LEFTMETA | KEY_RIGHTMETA) {
                    if event.state() == KeyState::Released {
                        self.toggle_launcher();
                    }
                    return;
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
            InputEvent::PointerMotion { .. } => {}
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
                const DOUBLE_CLICK_MS: u32 = 500;
                const DOUBLE_CLICK_DISTANCE: f64 = 6.0;
                const TITLEBAR_HEIGHT: f64 = 40.0;
                let mut suppress_pointer_event = false;

                if button == BTN_LEFT && button_state == ButtonState::Pressed {
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
            _ => {}
        }
    }
}
