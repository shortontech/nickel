use nickel_core::hotkeys::{Hotkey, HotkeyAction, KeyEdge};
use nickel_core::launcher::LauncherPointerTarget;
use nickel_core::window_input::{
    PointerPosition, WindowGeometry, WindowPointerEffect, WindowSurface, hit_test,
    reduce_pointer_press,
};
use smithay::{
    backend::input::{
        AbsolutePositionEvent, Axis, AxisSource, ButtonState, Event, InputBackend, InputEvent,
        InputTime, KeyState, KeyboardKeyEvent, MouseButton, PointerAxisEvent, PointerButtonEvent,
        PointerMotionEvent, TouchEvent,
    },
    input::{
        keyboard::{FilterResult, Keysym, keysyms},
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent, RelativeMotionEvent},
        touch::{DownEvent, MotionEvent as TouchMotion, UpEvent},
    },
    reexports::wayland_server::Resource,
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
};

use crate::{
    grabs::{MoveSurfaceGrab, ResizeEdge, ResizeSurfaceGrab},
    state::NickelSession,
    window_frame::{self, FramePart},
};

impl NickelSession {
    pub fn release_pressed_keys_on_host_focus_loss(&mut self) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let pressed = keyboard.pressed_keys();
        for keycode in pressed {
            keyboard.input::<Option<i32>, _>(
                self,
                keycode,
                KeyState::Released,
                SERIAL_COUNTER.next_serial(),
                InputTime::now(),
                |_session, _modifiers, _handle| FilterResult::Forward,
            );
        }
        self.hotkeys.reconcile_super(false);
        let _ = self.hotkeys.reconcile_alt(false);
    }

    fn update_frame_cursor(&mut self, position: smithay::utils::Point<f64, Logical>) {
        self.frame_cursor = self
            .space
            .elements()
            .filter(|window| {
                !self.shell_windows().any(|shell| shell == *window)
                    && !self.is_fullscreen_window(window)
                    && self.is_server_decorated(window)
            })
            .filter_map(|window| {
                let bounds = self.space.element_bbox(window)?;
                let geometry = crate::shell_layout::Geometry {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                };
                window_frame::hit_test(
                    geometry,
                    position.x.round() as i32,
                    position.y.round() as i32,
                )
            })
            .next_back()
            .map(FramePart::cursor)
            .unwrap_or_default();
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) -> Option<i32> {
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time();
                let state = event.state();
                let keyboard = self.seat.get_keyboard().unwrap();
                return keyboard
                    .input::<Option<i32>, _>(
                        self,
                        event.key_code(),
                        state,
                        serial,
                        time,
                        move |session, modifiers, handle| {
                            let sym = handle.modified_sym();
                            if modifiers.ctrl
                                && modifiers.alt
                                && let Some(vt) = vt_from_keysym(sym)
                            {
                                return FilterResult::Intercept(
                                    (state == KeyState::Pressed).then_some(vt),
                                );
                            }
                            let key = hotkey_from_keysym(sym);
                            let edge = if state == KeyState::Pressed {
                                KeyEdge::Pressed
                            } else {
                                KeyEdge::Released
                            };
                            let outcome = session.hotkeys.handle(key, edge);
                            match outcome.action {
                                Some(HotkeyAction::ShowLauncher) => {
                                    session.set_launcher_visible(true)
                                }
                                Some(HotkeyAction::HideLauncher) => {
                                    session.set_launcher_visible(false)
                                }
                                Some(
                                    action @ (HotkeyAction::SwitchNext
                                    | HotkeyAction::SwitchPrevious
                                    | HotkeyAction::SwitchGroupNext
                                    | HotkeyAction::SwitchGroupPrevious
                                    | HotkeyAction::CommitSwitch),
                                ) => session.apply_task_switch_action(action),
                                Some(HotkeyAction::SwitchWorkspacePrevious) => session
                                    .switch_workspace_direction(
                                        nickel_core::workspaces::WorkspaceDirection::Previous,
                                    ),
                                Some(HotkeyAction::SwitchWorkspaceNext) => session
                                    .switch_workspace_direction(
                                        nickel_core::workspaces::WorkspaceDirection::Next,
                                    ),
                                Some(HotkeyAction::MoveWindowToPreviousWorkspace) => session
                                    .move_active_window_to_workspace(
                                        nickel_core::workspaces::WorkspaceDirection::Previous,
                                    ),
                                Some(HotkeyAction::MoveWindowToNextWorkspace) => session
                                    .move_active_window_to_workspace(
                                        nickel_core::workspaces::WorkspaceDirection::Next,
                                    ),
                                Some(HotkeyAction::CaptureActiveWindow) => {
                                    tracing::info!(
                                        action = "capture_active_window",
                                        "screenshot shortcut requested"
                                    );
                                }
                                Some(HotkeyAction::CaptureActiveWindowToFile) => {
                                    tracing::info!(
                                        action = "capture_active_window_to_file",
                                        "screenshot shortcut requested"
                                    );
                                }
                                Some(HotkeyAction::ShowRun | HotkeyAction::ShowScreenshotTool)
                                | None => {}
                            }
                            if outcome.suppress {
                                return FilterResult::Intercept(None);
                            }
                            FilterResult::Forward
                        },
                    )
                    .flatten();
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
                let proposed = (
                    (current.x + delta.x).clamp(0.0, f64::from(max_x.saturating_sub(1))),
                    (current.y + delta.y).clamp(0.0, f64::from(max_y.saturating_sub(1))),
                )
                    .into();
                let pointer = self.seat.get_pointer().unwrap();
                let current_focus = self.pointer_surface_under(current);
                pointer.relative_motion(
                    self,
                    current_focus.clone(),
                    &RelativeMotionEvent {
                        delta,
                        delta_unaccel: event.delta_unaccel(),
                        time: event.time(),
                    },
                );
                let position = current_focus
                    .as_ref()
                    .map_or(proposed, |(surface, origin)| {
                        surface.wl_surface().map_or(proposed, |surface| {
                            self.constrained_pointer_position(&surface, *origin, current, proposed)
                        })
                    });
                self.update_frame_cursor(position);
                pointer.motion(
                    self,
                    self.pointer_surface_under(position),
                    &MotionEvent {
                        location: position,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time(),
                    },
                );
                pointer.frame(self);
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();

                let output_geo = self.space.output_geometry(output).unwrap();

                let proposed =
                    event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                let pointer = self.seat.get_pointer().unwrap();
                let current = pointer.current_location();
                let current_focus = self.pointer_surface_under(current);
                let pos = current_focus
                    .as_ref()
                    .map_or(proposed, |(surface, origin)| {
                        surface.wl_surface().map_or(proposed, |surface| {
                            self.constrained_pointer_position(&surface, *origin, current, proposed)
                        })
                    });
                self.update_frame_cursor(pos);

                let serial = SERIAL_COUNTER.next_serial();

                let under = self.pointer_surface_under(pos);

                pointer.motion(
                    self,
                    under,
                    &MotionEvent {
                        location: pos,
                        serial,
                        time: event.time(),
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

                const DOUBLE_CLICK_MS: u32 = 500;
                const DOUBLE_CLICK_DISTANCE: f64 = 6.0;
                let mut suppress_pointer_event = false;
                let mut frame_handled = false;
                let mut launcher_focus_restored = false;
                let mouse_button = event.button();
                let super_pressed = keyboard.modifier_state().logo;

                if mouse_button == Some(MouseButton::Left)
                    && button_state == ButtonState::Pressed
                    && self.launcher_visibility.is_visible()
                {
                    let target = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(window, _)| window.clone());
                    let restore_window_focus = target
                        .as_ref()
                        .is_none_or(|window| self.shell_windows().any(|shell| shell == window));
                    let target = if target.as_ref() == self.launcher_window.as_ref() {
                        LauncherPointerTarget::Launcher
                    } else {
                        LauncherPointerTarget::Other
                    };
                    launcher_focus_restored =
                        self.launcher_pointer_press(target, restore_window_focus);
                }

                if mouse_button == Some(MouseButton::Left)
                    && button_state == ButtonState::Pressed
                    && !super_pressed
                {
                    let location = pointer.current_location();
                    let frame_target = self
                        .space
                        .elements()
                        .filter(|window| {
                            !self.shell_windows().any(|shell| shell == *window)
                                && !self.is_fullscreen_window(window)
                                && self.is_server_decorated(window)
                        })
                        .filter_map(|window| {
                            let bounds = self.space.element_bbox(window)?;
                            let geometry = crate::shell_layout::Geometry {
                                x: bounds.loc.x,
                                y: bounds.loc.y,
                                width: bounds.size.w,
                                height: bounds.size.h,
                            };
                            window_frame::hit_test(
                                geometry,
                                location.x.round() as i32,
                                location.y.round() as i32,
                            )
                            .map(|part| (window.clone(), part))
                        })
                        .next_back();

                    if let Some((window, part)) = frame_target {
                        let surface = window.wl_surface().map(std::borrow::Cow::into_owned)?;
                        let id = surface.id();
                        let registry_id = self.surface_windows.get(&id).copied();
                        frame_handled = true;
                        suppress_pointer_event = true;
                        self.space.raise_element(&window, true);
                        if let Some(surface) = window.x11_surface() {
                            self.raise_x11_surface(surface);
                        }
                        if let Some(id) = registry_id {
                            self.windows.raise(id);
                            self.workspaces.focused(&id);
                        }
                        keyboard.set_focus(
                            self,
                            crate::focus::KeyboardFocusTarget::for_window(&window),
                            serial,
                        );
                        self.space.elements().for_each(|window| {
                            if let Some(toplevel) = window.toplevel() {
                                toplevel.send_pending_configure();
                            }
                        });
                        self.notify_protocol_snapshot();
                        match part {
                            FramePart::Close => {
                                self.suppress_left_button_release = true;
                                if let Some(id) = registry_id {
                                    self.close_window(id);
                                }
                            }
                            FramePart::Minimize => {
                                self.suppress_left_button_release = true;
                                if let Some(id) = registry_id {
                                    self.minimize_window(id);
                                }
                            }
                            FramePart::Maximize => {
                                self.suppress_left_button_release = true;
                                if let Some(id) = registry_id {
                                    self.maximize_window(id);
                                }
                            }
                            FramePart::Titlebar => {
                                let is_double_click =
                                    self.last_titlebar_click.as_ref().is_some_and(
                                        |(previous_id, previous_time, previous_location)| {
                                            previous_id == &id
                                                && event
                                                    .time()
                                                    .millis()
                                                    .wrapping_sub(*previous_time)
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
                                    if let Some(id) = registry_id {
                                        self.maximize_window(id);
                                    }
                                } else {
                                    self.last_titlebar_click =
                                        Some((id, event.time().millis(), location));
                                    let initial_window_location =
                                        self.space.element_location(&window).unwrap_or_default();
                                    let start_data = GrabStartData {
                                        focus: None,
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
                                    pointer.button(
                                        self,
                                        &ButtonEvent {
                                            button,
                                            state: button_state,
                                            serial,
                                            time: event.time(),
                                        },
                                    );
                                }
                            }
                            edge => {
                                let edges = match edge {
                                    FramePart::ResizeNorth => ResizeEdge::TOP,
                                    FramePart::ResizeNorthEast => {
                                        ResizeEdge::TOP | ResizeEdge::RIGHT
                                    }
                                    FramePart::ResizeEast => ResizeEdge::RIGHT,
                                    FramePart::ResizeSouthEast => {
                                        ResizeEdge::BOTTOM | ResizeEdge::RIGHT
                                    }
                                    FramePart::ResizeSouth => ResizeEdge::BOTTOM,
                                    FramePart::ResizeSouthWest => {
                                        ResizeEdge::BOTTOM | ResizeEdge::LEFT
                                    }
                                    FramePart::ResizeWest => ResizeEdge::LEFT,
                                    FramePart::ResizeNorthWest => {
                                        ResizeEdge::TOP | ResizeEdge::LEFT
                                    }
                                    _ => unreachable!(),
                                };
                                let initial_window_location =
                                    self.space.element_location(&window).unwrap_or_default();
                                let initial_rect =
                                    Rectangle::new(initial_window_location, window.geometry().size);
                                let start_data = GrabStartData {
                                    focus: None,
                                    button,
                                    location,
                                };
                                pointer.set_grab(
                                    self,
                                    ResizeSurfaceGrab::start(
                                        start_data,
                                        window,
                                        edges,
                                        initial_rect,
                                    ),
                                    serial,
                                    Focus::Clear,
                                );
                                pointer.button(
                                    self,
                                    &ButtonEvent {
                                        button,
                                        state: button_state,
                                        serial,
                                        time: event.time(),
                                    },
                                );
                            }
                        }
                    } else {
                        self.last_titlebar_click = None;
                    }
                } else if mouse_button == Some(MouseButton::Left)
                    && button_state == ButtonState::Released
                    && self.suppress_left_button_release
                {
                    self.suppress_left_button_release = false;
                    suppress_pointer_event = true;
                }

                if ButtonState::Pressed == button_state
                    && !pointer.is_grabbed()
                    && !frame_handled
                    && !launcher_focus_restored
                {
                    let pointer_position = pointer.current_location();
                    let ordinary_windows = self
                        .space
                        .elements()
                        .filter(|window| !self.shell_windows().any(|shell| shell == *window))
                        .filter_map(|window| {
                            let id = self
                                .surface_windows
                                .get(&window.wl_surface()?.id())
                                .copied()?;
                            let bounds = self.space.element_bbox(window)?;
                            Some(WindowSurface {
                                id,
                                geometry: WindowGeometry {
                                    x: f64::from(bounds.loc.x),
                                    y: f64::from(bounds.loc.y),
                                    width: f64::from(bounds.size.w),
                                    height: f64::from(bounds.size.h),
                                },
                            })
                        })
                        .collect::<Vec<_>>();
                    let window_effects = reduce_pointer_press(hit_test(
                        &ordinary_windows,
                        PointerPosition {
                            x: pointer_position.x,
                            y: pointer_position.y,
                        },
                    ));
                    if let Some((window, _loc)) = self
                        .space
                        .element_under(pointer_position)
                        .map(|(w, l)| (w.clone(), l))
                    {
                        self.space.raise_element(&window, true);
                        if let Some(surface) = window.x11_surface() {
                            self.raise_x11_surface(surface);
                        }
                        let actual_window = self
                            .surface_windows
                            .get(&window.wl_surface()?.id())
                            .copied();
                        for effect in window_effects {
                            match effect {
                                WindowPointerEffect::ActivateWindow(id)
                                    if actual_window == Some(id) =>
                                {
                                    self.windows.raise(id);
                                    self.workspaces.focused(&id);
                                }
                                WindowPointerEffect::ActivateWindow(_) => {}
                            }
                        }
                        if !self.is_panel_window(&window) {
                            self.space.elements().for_each(|candidate| {
                                candidate.set_activated(candidate == &window);
                            });
                            keyboard.set_focus(
                                self,
                                crate::focus::KeyboardFocusTarget::for_window(&window),
                                serial,
                            );
                            self.space.elements().for_each(|window| {
                                if let Some(toplevel) = window.toplevel() {
                                    toplevel.send_pending_configure();
                                }
                            });
                        }
                    } else {
                        self.space.elements().for_each(|window| {
                            window.set_activated(false);
                            if let Some(toplevel) = window.toplevel() {
                                toplevel.send_pending_configure();
                            }
                        });
                        keyboard.set_focus(
                            self,
                            Option::<crate::focus::KeyboardFocusTarget>::None,
                            serial,
                        );
                    }
                    self.notify_protocol_snapshot();
                };

                if mouse_button == Some(MouseButton::Left)
                    && button_state == ButtonState::Pressed
                    && super_pressed
                    && !pointer.is_grabbed()
                    && let Some((window, _)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(window, location)| (window.clone(), location))
                        .filter(|(window, _)| {
                            !self.desktop_windows.contains(window)
                                && !self.is_panel_window(window)
                                && self.launcher_window.as_ref() != Some(window)
                                && self.context_menu_window.as_ref() != Some(window)
                        })
                {
                    self.hotkeys.begin_pointer_chord();
                    let location = pointer.current_location();
                    let initial_window_location = self.space.element_location(&window).unwrap();
                    let start_data = GrabStartData {
                        focus: self.pointer_surface_under(location),
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

                if mouse_button == Some(MouseButton::Right)
                    && button_state == ButtonState::Pressed
                    && super_pressed
                    && !pointer.is_grabbed()
                    && let Some((window, _)) = self
                        .space
                        .element_under(pointer.current_location())
                        .map(|(window, location)| (window.clone(), location))
                        .filter(|(window, _)| {
                            !self.desktop_windows.contains(window)
                                && !self.is_panel_window(window)
                                && self.launcher_window.as_ref() != Some(window)
                                && self.context_menu_window.as_ref() != Some(window)
                        })
                {
                    self.hotkeys.begin_pointer_chord();
                    let location = pointer.current_location();
                    let initial_window_location = self.space.element_location(&window).unwrap();
                    let initial_rect =
                        Rectangle::new(initial_window_location, window.geometry().size);
                    let edges = resize_edges_at(location, initial_rect);
                    let start_data = GrabStartData {
                        focus: self.pointer_surface_under(location),
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
                            time: event.time(),
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

                let mut frame = AxisFrame::new(event.time()).source(source);
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
                        time: event.time(),
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
                        time: event.time(),
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
                        time: event.time(),
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

fn hotkey_from_keysym(sym: Keysym) -> Hotkey {
    match sym {
        value
            if value == Keysym::new(keysyms::KEY_Super_L)
                || value == Keysym::new(keysyms::KEY_Super_R) =>
        {
            Hotkey::Super
        }
        value
            if value == Keysym::new(keysyms::KEY_Alt_L)
                || value == Keysym::new(keysyms::KEY_Alt_R) =>
        {
            Hotkey::Alt
        }
        value
            if value == Keysym::new(keysyms::KEY_Shift_L)
                || value == Keysym::new(keysyms::KEY_Shift_R) =>
        {
            Hotkey::Shift
        }
        value
            if value == Keysym::new(keysyms::KEY_Control_L)
                || value == Keysym::new(keysyms::KEY_Control_R) =>
        {
            Hotkey::Control
        }
        value if value == Keysym::new(keysyms::KEY_Left) => Hotkey::Left,
        value if value == Keysym::new(keysyms::KEY_Right) => Hotkey::Right,
        value if value == Keysym::new(keysyms::KEY_Tab) => Hotkey::Tab,
        value
            if value == Keysym::new(keysyms::KEY_Print)
                || value == Keysym::new(keysyms::KEY_Sys_Req) =>
        {
            Hotkey::PrintScreen
        }
        value
            if value == Keysym::new(keysyms::KEY_grave)
                || value == Keysym::new(keysyms::KEY_asciitilde) =>
        {
            Hotkey::Grave
        }
        _ => Hotkey::Other,
    }
}

fn vt_from_keysym(sym: Keysym) -> Option<i32> {
    match sym.raw() {
        keysyms::KEY_F1 => Some(1),
        keysyms::KEY_F2 => Some(2),
        keysyms::KEY_F3 => Some(3),
        keysyms::KEY_F4 => Some(4),
        keysyms::KEY_F5 => Some(5),
        keysyms::KEY_F6 => Some(6),
        keysyms::KEY_F7 => Some(7),
        keysyms::KEY_F8 => Some(8),
        keysyms::KEY_F9 => Some(9),
        keysyms::KEY_F10 => Some(10),
        keysyms::KEY_F11 => Some(11),
        keysyms::KEY_F12 => Some(12),
        keysyms::KEY_XF86Switch_VT_1 => Some(1),
        keysyms::KEY_XF86Switch_VT_2 => Some(2),
        keysyms::KEY_XF86Switch_VT_3 => Some(3),
        keysyms::KEY_XF86Switch_VT_4 => Some(4),
        keysyms::KEY_XF86Switch_VT_5 => Some(5),
        keysyms::KEY_XF86Switch_VT_6 => Some(6),
        keysyms::KEY_XF86Switch_VT_7 => Some(7),
        keysyms::KEY_XF86Switch_VT_8 => Some(8),
        keysyms::KEY_XF86Switch_VT_9 => Some(9),
        keysyms::KEY_XF86Switch_VT_10 => Some(10),
        keysyms::KEY_XF86Switch_VT_11 => Some(11),
        keysyms::KEY_XF86Switch_VT_12 => Some(12),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use smithay::utils::{Point, Rectangle};

    use smithay::input::keyboard::{Keysym, keysyms};

    use super::{ResizeEdge, resize_edges_at, vt_from_keysym};

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
        assert_eq!(vt_from_keysym(Keysym::new(keysyms::KEY_F1)), Some(1));
        assert_eq!(vt_from_keysym(Keysym::new(keysyms::KEY_F10)), Some(10));
        assert_eq!(vt_from_keysym(Keysym::new(keysyms::KEY_F11)), Some(11));
        assert_eq!(vt_from_keysym(Keysym::new(keysyms::KEY_F12)), Some(12));
        assert_eq!(vt_from_keysym(Keysym::new(keysyms::KEY_Escape)), None);
    }

    #[test]
    fn xkb_server_control_keysyms_map_to_linux_virtual_terminals() {
        assert_eq!(
            vt_from_keysym(Keysym::new(keysyms::KEY_XF86Switch_VT_1)),
            Some(1)
        );
        assert_eq!(
            vt_from_keysym(Keysym::new(keysyms::KEY_XF86Switch_VT_10)),
            Some(10)
        );
        assert_eq!(
            vt_from_keysym(Keysym::new(keysyms::KEY_XF86Switch_VT_12)),
            Some(12)
        );
    }

    #[test]
    fn xkb_print_keysyms_map_to_the_screenshot_hotkey() {
        assert_eq!(
            super::hotkey_from_keysym(Keysym::new(keysyms::KEY_Print)),
            nickel_core::hotkeys::Hotkey::PrintScreen
        );
        assert_eq!(
            super::hotkey_from_keysym(Keysym::new(keysyms::KEY_Sys_Req)),
            nickel_core::hotkeys::Hotkey::PrintScreen
        );
    }
}
