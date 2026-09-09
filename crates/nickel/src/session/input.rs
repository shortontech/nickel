use nickel_core::hotkeys::{HotkeyAction, KeyCode, KeyEdge};
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
    desktop::WindowSurfaceType,
    input::{
        keyboard::{FilterResult, Keysym, keysyms},
        pointer::{AxisFrame, ButtonEvent, Focus, GrabStartData, MotionEvent, RelativeMotionEvent},
        touch::{DownEvent, MotionEvent as TouchMotion, UpEvent},
    },
    reexports::wayland_server::Resource,
    utils::{Logical, Rectangle, SERIAL_COUNTER},
    wayland::seat::WaylandFocus,
};

use crate::session::{
    grabs::{MoveInternalSurfaceGrab, MoveSurfaceGrab, ResizeEdge, ResizeSurfaceGrab},
    state::NickelSession,
    window_frame::{self, FramePart},
};

fn desktop_modifiers(
    modifiers: &smithay::input::keyboard::ModifiersState,
) -> nickel_input::ModifierState {
    use nickel_input::AggregateModifier;
    nickel_input::ModifierState::from_sides_and_unsided(
        [],
        [
            (modifiers.ctrl, AggregateModifier::Control),
            (modifiers.shift, AggregateModifier::Shift),
            (modifiers.alt, AggregateModifier::Alt),
            (modifiers.logo, AggregateModifier::Super),
        ]
        .into_iter()
        .filter_map(|(held, modifier)| held.then_some(modifier)),
    )
}

pub(super) fn internal_keyboard_event(sym: Keysym, state: KeyState) -> Option<nickel_ui::UiEvent> {
    if state != KeyState::Pressed {
        return None;
    }
    Some(match sym.raw() {
        keysyms::KEY_Tab => nickel_ui::UiEvent::FocusNext,
        keysyms::KEY_ISO_Left_Tab => nickel_ui::UiEvent::FocusPrevious,
        keysyms::KEY_Up => nickel_ui::UiEvent::KeyboardNavigateUp,
        keysyms::KEY_Down => nickel_ui::UiEvent::KeyboardNavigateDown,
        keysyms::KEY_Left => nickel_ui::UiEvent::KeyboardNavigateLeft,
        keysyms::KEY_Right => nickel_ui::UiEvent::KeyboardNavigateRight,
        keysyms::KEY_Return | keysyms::KEY_KP_Enter | keysyms::KEY_space => {
            nickel_ui::UiEvent::KeyboardActivate
        }
        keysyms::KEY_Escape => nickel_ui::UiEvent::KeyboardNavigateBack,
        keysyms::KEY_BackSpace => nickel_ui::UiEvent::TextBackspace,
        keysyms::KEY_Delete => nickel_ui::UiEvent::TextDelete,
        keysyms::KEY_Home => nickel_ui::UiEvent::KeyboardNavigateStart,
        keysyms::KEY_End => nickel_ui::UiEvent::KeyboardNavigateEnd,
        keysyms::KEY_Page_Up => nickel_ui::UiEvent::KeyboardNavigatePageUp,
        keysyms::KEY_Page_Down => nickel_ui::UiEvent::KeyboardNavigatePageDown,
        _ => {
            let character = sym.key_char()?;
            if character.is_control() {
                return None;
            }
            nickel_ui::UiEvent::TextInput(character.to_string())
        }
    })
}

/// Normalize physical identity independently of the active XKB layout. Unknown
/// keys keep their native identity; characters retain XKB's modified symbol.
fn desktop_key_event(
    native: (u32, Keysym, KeyState),
    modifiers: nickel_input::ModifierState,
    device: nickel_input::DeviceId,
    order: nickel_input::EventOrder,
    repeat: bool,
) -> nickel_input::KeyEvent {
    use nickel_input::{KeyLocation, LogicalKey, NamedKey, NativeCode, NativeKey, PhysicalKey};
    use winit::platform::scancode::PhysicalKeyExtScancode;
    let (raw, sym, state) = native;
    let physical = raw
        .checked_sub(8)
        .map(|scan| {
            nickel_input::winit::physical_key(winit::keyboard::PhysicalKey::from_scancode(scan))
        })
        .unwrap_or_else(|| {
            PhysicalKey::Native(NativeKey {
                namespace: "xkb-keycode".into(),
                code: NativeCode::Numeric(u64::from(raw)),
            })
        });
    let named = match sym.raw() {
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Some(NamedKey::Enter),
        keysyms::KEY_Escape => Some(NamedKey::Escape),
        keysyms::KEY_Tab | keysyms::KEY_ISO_Left_Tab => Some(NamedKey::Tab),
        keysyms::KEY_Up => Some(NamedKey::ArrowUp),
        keysyms::KEY_Down => Some(NamedKey::ArrowDown),
        keysyms::KEY_Left => Some(NamedKey::ArrowLeft),
        keysyms::KEY_Right => Some(NamedKey::ArrowRight),
        keysyms::KEY_Home => Some(NamedKey::Home),
        keysyms::KEY_End => Some(NamedKey::End),
        keysyms::KEY_Page_Up => Some(NamedKey::PageUp),
        keysyms::KEY_Page_Down => Some(NamedKey::PageDown),
        keysyms::KEY_BackSpace => Some(NamedKey::Backspace),
        keysyms::KEY_Delete => Some(NamedKey::Delete),
        keysyms::KEY_Menu => Some(NamedKey::ContextMenu),
        _ => None,
    };
    let logical = named
        .map(LogicalKey::Named)
        .or_else(|| {
            sym.key_char()
                .filter(|ch| !ch.is_control())
                .map(|ch| LogicalKey::Character(ch.to_string()))
        })
        .unwrap_or_else(|| {
            LogicalKey::Native(NativeKey {
                namespace: "xkb-keysym".into(),
                code: NativeCode::Numeric(u64::from(sym.raw())),
            })
        });
    let location = match &physical {
        PhysicalKey::Code(
            KeyCode::ShiftLeft | KeyCode::ControlLeft | KeyCode::AltLeft | KeyCode::SuperLeft,
        ) => KeyLocation::Left,
        PhysicalKey::Code(
            KeyCode::ShiftRight | KeyCode::ControlRight | KeyCode::AltRight | KeyCode::SuperRight,
        ) => KeyLocation::Right,
        PhysicalKey::Code(
            KeyCode::NumpadEnter
            | KeyCode::Numpad0
            | KeyCode::Numpad1
            | KeyCode::Numpad2
            | KeyCode::Numpad3
            | KeyCode::Numpad4
            | KeyCode::Numpad5
            | KeyCode::Numpad6
            | KeyCode::Numpad7
            | KeyCode::Numpad8
            | KeyCode::Numpad9
            | KeyCode::NumpadAdd
            | KeyCode::NumpadSubtract
            | KeyCode::NumpadMultiply
            | KeyCode::NumpadDivide
            | KeyCode::NumpadDecimal,
        ) => KeyLocation::Numpad,
        PhysicalKey::Code(_) => KeyLocation::Standard,
        PhysicalKey::Native(_) => KeyLocation::Unknown,
    };
    nickel_input::KeyEvent {
        device,
        order,
        physical,
        logical,
        location,
        edge: if state == KeyState::Pressed {
            nickel_input::KeyEdge::Pressed
        } else {
            nickel_input::KeyEdge::Released
        },
        repeat,
        modifiers,
    }
}

/// Virtual modifiers describe one normalized chord; they never update the seat's
/// physical modifier state or masquerade as a hardware scan code.
pub(super) fn internal_virtual_key(
    keysym: u32,
    modifiers: &[u32],
    order: nickel_input::EventOrder,
) -> Result<nickel_input::KeyEvent, &'static str> {
    use nickel_input::{Modifier, ModifierState, NativeCode, NativeKey, PhysicalKey};
    if modifiers.len() > 5 {
        return Err("invalid on-screen keyboard modifiers");
    }
    let sides = modifiers
        .iter()
        .map(|modifier| match modifier {
            0xffe1 => Ok(Modifier::ShiftLeft),
            0xffe3 => Ok(Modifier::ControlLeft),
            0xffe9 => Ok(Modifier::AltLeft),
            0xffeb => Ok(Modifier::SuperLeft),
            0xfe03 => Ok(Modifier::AltRight),
            _ => Err("invalid on-screen keyboard modifiers"),
        })
        .collect::<Result<Vec<_>, _>>()?;
    let mut event = desktop_key_event(
        (0, Keysym::new(keysym), KeyState::Pressed),
        ModifierState::from_sides(sides),
        nickel_input::DeviceId(u64::MAX),
        order,
        false,
    );
    event.physical = PhysicalKey::Native(NativeKey {
        namespace: "nickel-virtual-keysym".into(),
        code: NativeCode::Numeric(u64::from(keysym)),
    });
    Ok(event)
}

impl NickelSession {
    fn route_internal_pointer_motion(
        &mut self,
        position: smithay::utils::Point<f64, Logical>,
        device: &str,
    ) -> bool {
        let client_present =
            self.client_scene_under(position) && !self.internal_applications_are_foremost();
        let modifiers = desktop_modifiers(&self.seat.get_keyboard().unwrap().modifier_state());
        let handled = self.internal_ui.desktop_pointer_input(
            device,
            (position.x, position.y),
            super::internal_ui::DesktopPointerAction::Motion,
            modifiers,
            client_present,
        ) || self
            .internal_ui
            .pointer_motion_with_client((position.x, position.y), client_present);
        // Leaving an internal surface can queue cancellation even when the new
        // target is a client and this motion itself is not internally handled.
        self.flush_internal_shell_input();
        if handled {
            self.request_output_redraw();
        }
        handled
    }

    fn recovery_pointer_action(
        &mut self,
        position: smithay::utils::Point<f64, Logical>,
        pressed: bool,
    ) -> Option<crate::session::recovery_ui::RecoveryAction> {
        let output = self.space.outputs().find_map(|output| {
            let geometry = self.space.output_geometry(output)?;
            let output = crate::session::shell_layout::Geometry {
                x: geometry.loc.x,
                y: geometry.loc.y,
                width: geometry.size.w,
                height: geometry.size.h,
            };
            (position.x >= f64::from(output.x)
                && position.x < f64::from(output.x + output.width)
                && position.y >= f64::from(output.y)
                && position.y < f64::from(output.y + output.height))
            .then_some(output)
        })?;
        self.recovery_ui
            .pointer(output, position.x, position.y, pressed)
    }

    pub(crate) fn apply_recovery_action(
        &mut self,
        action: crate::session::recovery_ui::RecoveryAction,
    ) {
        match action {
            crate::session::recovery_ui::RecoveryAction::Retry => {
                self.retry_shell_from_recovery();
            }
            crate::session::recovery_ui::RecoveryAction::Exit => {
                self.exit_from_recovery();
            }
        }
    }

    pub fn release_pressed_keys_on_host_focus_loss(&mut self) {
        let keyboard = self.seat.get_keyboard().unwrap();
        let pressed = keyboard.pressed_keys();
        for keycode in pressed {
            if keycode.raw() == 9 {
                tracing::warn!("diagnostic: releasing a retained Escape after host focus loss");
            }
            keyboard.input::<Option<i32>, _>(
                self,
                keycode,
                KeyState::Released,
                SERIAL_COUNTER.next_serial(),
                InputTime::now(),
                |_session, _modifiers, _handle| FilterResult::Forward,
            );
        }
        self.hotkeys.reset_pressed_state();
        self.cancel_consumer_control_repeats();
    }

    pub(super) fn consumer_control_key(
        &mut self,
        control: nickel_session_protocol::ConsumerControl,
        state: KeyState,
    ) {
        use smithay::reexports::calloop::{timer::TimeoutAction, timer::Timer};

        if state == KeyState::Released {
            if let Some((_, Some(token))) = self.held_consumer_controls.remove(&control) {
                self.event_loop_handle.remove(token);
            }
            return;
        }
        if self.held_consumer_controls.contains_key(&control) {
            return;
        }
        // Each physical hold owns one timer and a distinct lease. A queued
        // callback from a released hold cannot repeat a newly pressed key.
        self.consumer_repeat_epoch = self.consumer_repeat_epoch.wrapping_add(1);
        let epoch = self.consumer_repeat_epoch;
        self.held_consumer_controls.insert(control, (epoch, None));
        self.notify_consumer_control(control);
        if !matches!(
            control,
            nickel_session_protocol::ConsumerControl::VolumeUp
                | nickel_session_protocol::ConsumerControl::VolumeDown
        ) {
            return;
        }
        let timer = Timer::from_duration(std::time::Duration::from_millis(600));
        match self
            .event_loop_handle
            .insert_source(timer, move |_, _, session| {
                if !session.consumer_repeat_is_current(control, epoch) {
                    return TimeoutAction::Drop;
                }
                session.notify_consumer_control(control);
                TimeoutAction::ToDuration(std::time::Duration::from_millis(40))
            }) {
            Ok(token) => self.held_consumer_controls.get_mut(&control).unwrap().1 = Some(token),
            Err(error) => tracing::warn!(?error, "failed to schedule consumer-control repeat"),
        }
    }

    pub(super) fn consumer_repeat_is_current(
        &self,
        control: nickel_session_protocol::ConsumerControl,
        epoch: u64,
    ) -> bool {
        self.held_consumer_controls
            .get(&control)
            .is_some_and(|(current, _)| *current == epoch)
    }

    pub(crate) fn cancel_consumer_control_repeats(&mut self) {
        for (_, (_, token)) in self.held_consumer_controls.drain() {
            if let Some(token) = token {
                self.event_loop_handle.remove(token);
            }
        }
        self.consumer_repeat_epoch = self.consumer_repeat_epoch.wrapping_add(1);
    }

    fn update_frame_cursor(&mut self, position: smithay::utils::Point<f64, Logical>) {
        if self.locked {
            self.frame_cursor = Default::default();
            return;
        }
        self.frame_cursor =
            window_frame::topmost_frame_target(self.space.elements().rev().filter_map(|window| {
                let surface_accepts_input =
                    self.space.element_location(window).is_some_and(|loc| {
                        window
                            .surface_under(position - loc.to_f64(), WindowSurfaceType::ALL)
                            .is_some()
                    });
                let bounds = self.space.element_geometry(window)?;
                let geometry = crate::session::shell_layout::Geometry {
                    x: bounds.loc.x,
                    y: bounds.loc.y,
                    width: bounds.size.w,
                    height: bounds.size.h,
                };
                let frame_part = (!self.shell_windows().any(|shell| shell == window)
                    && !self.is_fullscreen_window(window)
                    && self.is_server_decorated(window))
                .then(|| {
                    window_frame::hit_test(
                        geometry,
                        position.x.round() as i32,
                        position.y.round() as i32,
                    )
                })
                .flatten();
                Some(((), surface_accepts_input, frame_part))
            }))
            .map(|(_, part)| part.cursor())
            .unwrap_or_default();
    }

    /// Reconcile retained pointer focus with current compositor stacking when no
    /// physical motion has occurred. Active constraints and grabs continue to own
    /// focus until their protocol lifetimes end.
    fn refresh_stationary_pointer_focus(&mut self, time: InputTime) {
        let pointer = self.seat.get_pointer().unwrap();
        if pointer.is_grabbed() {
            return;
        }

        let location = pointer.current_location();
        let hit = self.pointer_surface_under(location);
        let constraint_focus = pointer.current_focus().and_then(|focus| {
            let surface = focus.wl_surface()?.into_owned();
            let origin = self
                .active_pointer_constraint_origins
                .get(&surface.id())
                .copied()?;
            Some((focus, origin))
        });
        let target = constraint_focus.or(hit);
        if !pointer_focus_needs_refresh(pointer.current_focus().as_ref(), target.as_ref()) {
            return;
        }
        pointer.motion(
            self,
            target,
            &MotionEvent {
                location,
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    pub fn process_input_event<I: InputBackend>(&mut self, event: InputEvent<I>) -> Option<i32> {
        self.process_input_event_on_output(event, None)
    }

    pub(crate) fn touch_output_geometry(
        &self,
        output_name: Option<&str>,
    ) -> Option<Rectangle<i32, Logical>> {
        // A device's explicit mapping is authoritative. Losing that output must
        // not redirect touches to an unrelated client on the fallback monitor.
        let output = match output_name {
            Some(name) => self.space.outputs().find(|output| output.name() == name)?,
            None => self.space.outputs().next()?,
        };
        self.space.output_geometry(output)
    }

    pub(crate) fn process_input_event_on_output<I: InputBackend>(
        &mut self,
        event: InputEvent<I>,
        output_name: Option<&str>,
    ) -> Option<i32> {
        use smithay::backend::input::{Device, DeviceCapability};
        match &event {
            InputEvent::DeviceAdded { device }
                if device.has_capability(DeviceCapability::Touch) && device.syspath().is_some() =>
            {
                self.on_screen_keyboard.touchscreens.insert(device.id());
            }
            InputEvent::DeviceRemoved { device } => {
                self.on_screen_keyboard.touchscreens.remove(&device.id());
                self.internal_ui.remove_desktop_pointer_device(&device.id());
                self.flush_internal_shell_input();
            }
            _ => {}
        }
        self.note_input_activity();
        if self.shell_recovery_visible() {
            match &event {
                InputEvent::PointerMotionAbsolute { event, .. } => {
                    let output = self.space.outputs().next()?;
                    let geometry = self.space.output_geometry(output)?;
                    let position =
                        event.position_transformed(geometry.size) + geometry.loc.to_f64();
                    let pointer = self.seat.get_pointer().unwrap();
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: position,
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time(),
                        },
                    );
                    pointer.frame(self);
                    return None;
                }
                InputEvent::PointerMotion { event, .. } => {
                    let pointer = self.seat.get_pointer().unwrap();
                    let current = pointer.current_location();
                    let (max_x, max_y) = self
                        .space
                        .outputs()
                        .filter_map(|output| self.space.output_geometry(output))
                        .fold((1, 1), |(max_x, max_y), geometry| {
                            (
                                max_x.max(geometry.loc.x + geometry.size.w),
                                max_y.max(geometry.loc.y + geometry.size.h),
                            )
                        });
                    let delta = event.delta();
                    let position = (
                        (current.x + delta.x).clamp(0.0, f64::from(max_x.saturating_sub(1))),
                        (current.y + delta.y).clamp(0.0, f64::from(max_y.saturating_sub(1))),
                    )
                        .into();
                    pointer.relative_motion(
                        self,
                        None,
                        &RelativeMotionEvent {
                            delta,
                            delta_unaccel: event.delta_unaccel(),
                            time: event.time(),
                        },
                    );
                    pointer.motion(
                        self,
                        None,
                        &MotionEvent {
                            location: position,
                            serial: SERIAL_COUNTER.next_serial(),
                            time: event.time(),
                        },
                    );
                    pointer.frame(self);
                    return None;
                }
                InputEvent::PointerButton { event, .. }
                    if event.button() == Some(MouseButton::Left) =>
                {
                    let position = self.seat.get_pointer().unwrap().current_location();
                    if let Some(action) = self
                        .recovery_pointer_action(position, event.state() == ButtonState::Pressed)
                    {
                        self.apply_recovery_action(action);
                    }
                    return None;
                }
                InputEvent::TouchDown { event, .. } => {
                    let geometry = self.touch_output_geometry(output_name)?;
                    let position =
                        event.position_transformed(geometry.size) + geometry.loc.to_f64();
                    let output = crate::session::shell_layout::Geometry {
                        x: geometry.loc.x,
                        y: geometry.loc.y,
                        width: geometry.size.w,
                        height: geometry.size.h,
                    };
                    if let Some(action) = self.recovery_ui.touch(output, position.x, position.y) {
                        self.apply_recovery_action(action);
                    }
                    return None;
                }
                InputEvent::PointerButton { .. }
                | InputEvent::PointerAxis { .. }
                | InputEvent::TouchMotion { .. }
                | InputEvent::TouchUp { .. }
                | InputEvent::TouchFrame { .. }
                | InputEvent::TouchCancel { .. } => return None,
                _ => {}
            }
        }
        match event {
            InputEvent::Keyboard { event, .. } => {
                let serial = SERIAL_COUNTER.next_serial();
                let time = event.time();
                let state = event.state();
                // XKB keycodes are evdev codes plus eight, so Escape is keycode nine. Keep this
                // diagnostic deliberately limited to Escape: logging ordinary keys would expose
                // typed text, while this provenance is essential for distinguishing a physical
                // HID report from the compositor's auxiliary and test input paths.
                if event.key_code().raw() == 9 {
                    let device = event.device();
                    tracing::warn!(
                        device_id = %device.id(),
                        device_name = %device.name(),
                        device_path = ?device.syspath(),
                        ?state,
                        "diagnostic: received Escape from an input backend"
                    );
                }
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
                            if let Some(control) = consumer_control_from_keysym(sym) {
                                session.consumer_control_key(control, state);
                                return FilterResult::Intercept(None);
                            }
                            if modifiers.ctrl
                                && modifiers.alt
                                && let Some(vt) = vt_from_keysym(sym)
                            {
                                if state == KeyState::Pressed {
                                    session.hotkeys.reset_pressed_state();
                                }
                                return FilterResult::Intercept(
                                    (state == KeyState::Pressed).then_some(vt),
                                );
                            }
                            if session.shell_recovery_visible() {
                                if state == KeyState::Pressed
                                    && let Some(shortcut) = recovery_shortcut_from_keysym(sym)
                                    && let Some(action) = session.recovery_ui.shortcut(shortcut)
                                {
                                    session.apply_recovery_action(action);
                                }
                                return FilterResult::Intercept(None);
                            }
                            let edge = if state == KeyState::Pressed {
                                KeyEdge::Pressed
                            } else {
                                KeyEdge::Released
                            };
                            let outcome = match key_code_from_keysym(sym) {
                                Some(key) => {
                                    if session.remote_emergency_chord.handle_physical(
                                        key,
                                        edge,
                                        session.remote_control.status().effective
                                            == nickel_remote_control::EffectiveState::Enabled,
                                    ) {
                                        session.emergency_stop_remote_control();
                                        return FilterResult::Intercept(None);
                                    }
                                    session.hotkeys.handle(key, edge)
                                }
                                None => session.hotkeys.handle_unmapped(edge),
                            };
                            if outcome.action == Some(HotkeyAction::LockSession) {
                                session.lock_session();
                                return FilterResult::Intercept(None);
                            }
                            if session.locked {
                                if outcome.suppress {
                                    return FilterResult::Intercept(None);
                                }
                                // Compositor-owned lock surfaces intentionally clear the native
                                // seat focus when they take internal focus. Forwarding through
                                // Smithay therefore has no recipient; deliver physical keys to
                                // the focused internal lock UI just as we do for other internal
                                // applications. External session-lock clients still use the
                                // native forwarding path below.
                                if session.internal_ui.focused().is_some() {
                                    let desktop_handled = session.internal_ui.desktop_keyboard_input(
                                        &event.device().id(),
                                        event.key_code().raw(),
                                        state == KeyState::Pressed,
                                        |device, order, repeat| {
                                            desktop_key_event(
                                                (event.key_code().raw(), sym, state),
                                                desktop_modifiers(modifiers),
                                                device,
                                                order,
                                                repeat,
                                            )
                                        },
                                    );
                                    if !desktop_handled {
                                        if state == KeyState::Pressed
                                            && matches!(
                                                sym.raw(),
                                                keysyms::KEY_Return | keysyms::KEY_KP_Enter
                                            )
                                        {
                                            session.internal_ui.submit_or_activate();
                                        } else if let Some(event) =
                                            internal_keyboard_event(sym, state)
                                        {
                                            session.internal_ui.keyboard(event);
                                        }
                                    }
                                    session.flush_internal_shell_input();
                                    session.request_output_redraw();
                                    return FilterResult::Intercept(None);
                                }
                                return if session.keyboard_focus_is_lock_surface() {
                                    FilterResult::Forward
                                } else {
                                    // Locked sessions fail closed. A stale ordinary-client focus
                                    // must never receive text intended for the lock screen.
                                    FilterResult::Intercept(None)
                                };
                            }
                            match outcome.action {
                                Some(HotkeyAction::LockSession) => unreachable!(),
                                Some(HotkeyAction::ToggleLauncher) => {
                                    session.toggle_launcher_visibility()
                                }
                                Some(
                                    action @ (HotkeyAction::SwitchNext
                                    | HotkeyAction::SwitchPrevious
                                    | HotkeyAction::SwitchGroupNext
                                    | HotkeyAction::SwitchGroupPrevious
                                    | HotkeyAction::CommitSwitch
                                    | HotkeyAction::CancelSwitch),
                                ) => session.apply_task_switch_action(action),
                                Some(HotkeyAction::SwitchWorkspacePrevious) => session
                                    .switch_workspace_direction(
                                        nickel_core::workspaces::WorkspaceDirection::Previous,
                                    ),
                                Some(HotkeyAction::SwitchWorkspaceNext) => session
                                    .switch_workspace_direction(
                                        nickel_core::workspaces::WorkspaceDirection::Next,
                                    ),
                                Some(HotkeyAction::SwitchWorkspace(number)) => {
                                    session.switch_workspace_number(number)
                                }
                                Some(HotkeyAction::MoveWindowToPreviousWorkspace) => session
                                    .move_active_window_to_workspace(
                                        nickel_core::workspaces::WorkspaceDirection::Previous,
                                    ),
                                Some(HotkeyAction::MoveWindowToNextWorkspace) => session
                                    .move_active_window_to_workspace(
                                        nickel_core::workspaces::WorkspaceDirection::Next,
                                    ),
                                Some(HotkeyAction::CreateWorkspace) => session.create_workspace_and_switch(),
                                Some(HotkeyAction::RemoveActiveWorkspace) => session.remove_active_workspace(),
                                Some(HotkeyAction::CloseActiveWindow) => session.close_active_window(),
                                Some(HotkeyAction::MaximizeActiveWindow) => session.maximize_active_window(),
                                Some(HotkeyAction::RestoreOrMinimizeActiveWindow) => session.restore_or_minimize_active_window(),
                                Some(HotkeyAction::MoveWindowToPreviousOutput) => session.move_active_window_to_output_direction(true),
                                Some(HotkeyAction::MoveWindowToNextOutput) => session.move_active_window_to_output_direction(false),
                                Some(HotkeyAction::OpenFiles) => session.notify_global_shortcut(nickel_session_protocol::ShortcutAction::OpenFiles),
                                Some(HotkeyAction::OpenSettings) => session.notify_global_shortcut(nickel_session_protocol::ShortcutAction::OpenSettings),
                                Some(HotkeyAction::ShowControlCenter) => session.notify_global_shortcut(nickel_session_protocol::ShortcutAction::ShowControlCenter),
                                Some(HotkeyAction::ShowNotifications) => session.notify_global_shortcut(nickel_session_protocol::ShortcutAction::ShowNotifications),
                                Some(HotkeyAction::ShowDesktop) => session.toggle_show_desktop(),
                                Some(HotkeyAction::ProjectDisplays) => session.notify_global_shortcut(nickel_session_protocol::ShortcutAction::ProjectDisplays),
                                Some(HotkeyAction::ShowWindowMenu) => session.notify_global_shortcut(nickel_session_protocol::ShortcutAction::ShowWindowMenu),
                                Some(HotkeyAction::SnapLeading) => session.snap_active_window(true),
                                Some(HotkeyAction::SnapTrailing) => session.snap_active_window(false),
                                Some(HotkeyAction::CaptureActiveWindow) => {
                                    session.notify_global_shortcut(
                                        nickel_session_protocol::ShortcutAction::CaptureActiveWindow,
                                    );
                                }
                                Some(HotkeyAction::CaptureActiveWindowToFile) => {
                                    session.notify_global_shortcut(
                                        nickel_session_protocol::ShortcutAction::CaptureActiveWindowToFile,
                                    );
                                }
                                Some(HotkeyAction::ShowRun) => session.notify_global_shortcut(
                                    nickel_session_protocol::ShortcutAction::ShowRun,
                                ),
                                Some(HotkeyAction::ShowScreenshotTool) => {
                                    session.notify_global_shortcut(
                                        nickel_session_protocol::ShortcutAction::ShowScreenshotTool,
                                    );
                                }
                                None => {}
                            }
                            if outcome.suppress {
                                return FilterResult::Intercept(None);
                            }
                            if session.internal_ui.focused().is_some() {
                                if state == KeyState::Pressed
                                    && (modifiers.ctrl || modifiers.logo)
                                    && matches!(sym.raw(), keysyms::KEY_v | keysyms::KEY_V)
                                    && let Some(recipient) = session.internal_ui.focused()
                                {
                                    let paste_event = desktop_key_event(
                                        (event.key_code().raw(), sym, state),
                                        desktop_modifiers(modifiers),
                                        nickel_input::DeviceId(0),
                                        nickel_input::EventOrder(u64::from(time.millis())),
                                        false,
                                    );
                                    match session.request_native_image_paste(recipient) {
                                        Ok(true) => return FilterResult::Intercept(None),
                                        Ok(false) => {
                                            if let Err(error) = session
                                                .request_native_direct_text_paste(
                                                    recipient,
                                                    paste_event,
                                                )
                                            {
                                                tracing::warn!(
                                                    error,
                                                    "native clipboard text paste rejected"
                                                );
                                                session.native_clipboard.last_failure =
                                                    Some(error.into());
                                            }
                                            return FilterResult::Intercept(None);
                                        }
                                        Err(error) => {
                                            tracing::warn!(error, "native clipboard image paste rejected");
                                            session.native_clipboard.last_failure =
                                                Some(error.into());
                                            return FilterResult::Intercept(None);
                                        }
                                    }
                                }
                                let desktop_handled = session.internal_ui.desktop_keyboard_input(
                                    &event.device().id(), event.key_code().raw(), state == KeyState::Pressed,
                                    |device, order, repeat| desktop_key_event((event.key_code().raw(), sym, state), desktop_modifiers(modifiers), device, order, repeat),
                                );
                                if !desktop_handled {
                                    if state == KeyState::Pressed
                                        && matches!(
                                            sym.raw(),
                                            keysyms::KEY_Return | keysyms::KEY_KP_Enter
                                        )
                                    {
                                        session.internal_ui.submit_or_activate();
                                    } else if let Some(event) = internal_keyboard_event(sym, state) {
                                        session.internal_ui.keyboard(event);
                                    }
                                }
                                session.flush_internal_shell_input();
                                session.request_output_redraw();
                                return FilterResult::Intercept(None);
                            }
                            FilterResult::Forward
                        },
                    )
                    .flatten();
            }
            InputEvent::PointerMotion { event, .. } => {
                let current = self.seat.get_pointer().unwrap().current_location();
                let current = self.restore_released_pointer_lock_hint(current);
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
                // An active constraint owns pointer focus until the protocol
                // releases it. Re-hit-testing here would let an overlapping
                // window steal motion before the pointer reaches the
                // constrained surface boundary.
                let protocol_focus = pointer.current_focus().and_then(|focus| {
                    let surface = focus.wl_surface()?.into_owned();
                    let origin = self
                        .active_pointer_constraint_origins
                        .get(&surface.id())
                        .copied()
                        .or_else(|| {
                            current_focus.as_ref().and_then(|(candidate, origin)| {
                                (candidate.wl_surface().as_deref() == Some(&surface))
                                    .then_some(*origin)
                            })
                        })?;
                    Some((focus, surface, origin))
                });
                let (position, constraint_focus) =
                    protocol_focus.map_or((proposed, None), |(focus, surface, origin)| {
                        let (position, active) =
                            self.constrained_pointer_position(&surface, origin, current, proposed);
                        (position, active.then_some((focus, origin)))
                    });
                let internal = constraint_focus.is_none()
                    && self.route_internal_pointer_motion(position, &event.device().id());
                self.update_frame_cursor(position);
                let motion_focus = constraint_focus.or_else(|| {
                    (!internal)
                        .then(|| self.pointer_surface_under(position))
                        .flatten()
                });
                pointer.motion(
                    self,
                    motion_focus,
                    &MotionEvent {
                        location: position,
                        serial: SERIAL_COUNTER.next_serial(),
                        time: event.time(),
                    },
                );
                pointer.frame(self);
                self.record_interaction_output(position);
            }
            InputEvent::PointerMotionAbsolute { event, .. } => {
                let output = self.space.outputs().next().unwrap();

                let output_geo = self.space.output_geometry(output).unwrap();

                let proposed =
                    event.position_transformed(output_geo.size) + output_geo.loc.to_f64();
                let pointer = self.seat.get_pointer().unwrap();
                let current = pointer.current_location();
                let current_focus = self.pointer_surface_under(current);
                let protocol_focus = pointer.current_focus().and_then(|focus| {
                    let surface = focus.wl_surface()?.into_owned();
                    let origin = self
                        .active_pointer_constraint_origins
                        .get(&surface.id())
                        .copied()
                        .or_else(|| {
                            current_focus.as_ref().and_then(|(candidate, origin)| {
                                (candidate.wl_surface().as_deref() == Some(&surface))
                                    .then_some(*origin)
                            })
                        })?;
                    Some((focus, surface, origin))
                });
                let (pos, constraint_focus) =
                    protocol_focus.map_or((proposed, None), |(focus, surface, origin)| {
                        let (position, active) =
                            self.constrained_pointer_position(&surface, origin, current, proposed);
                        (position, active.then_some((focus, origin)))
                    });
                self.update_frame_cursor(pos);

                let serial = SERIAL_COUNTER.next_serial();

                let internal = constraint_focus.is_none()
                    && self.route_internal_pointer_motion(pos, &event.device().id());
                let under = constraint_focus.or_else(|| self.pointer_surface_under(pos));
                let under = (!internal).then_some(under).flatten();

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
                self.record_interaction_output(pos);
            }
            InputEvent::PointerButton { event, .. } => {
                let pointer = self.seat.get_pointer().unwrap();
                let keyboard = self.seat.get_keyboard().unwrap();

                let serial = SERIAL_COUNTER.next_serial();

                let button = event.button_code();

                let button_state = event.state();

                let location = pointer.current_location();
                let client_present =
                    self.client_scene_under(location) && !self.internal_applications_are_foremost();
                if event.button() == Some(MouseButton::Left)
                    && button_state == ButtonState::Pressed
                    && !self.locked
                    && !pointer.is_grabbed()
                    && !client_present
                    && self
                        .internal_ui
                        .surface_at((location.x, location.y), true)
                        .is_none()
                    && let Some((surface, part)) = self
                        .internal_ui
                        .internal_frame_target((location.x, location.y))
                    && let Some(id) = self.internal_window_for_surface(surface)
                {
                    self.activate_window(id);
                    match part {
                        FramePart::Close => {
                            self.suppress_left_button_release = true;
                            self.close_window(id);
                        }
                        FramePart::Minimize => {
                            self.suppress_left_button_release = true;
                            self.minimize_window(id);
                        }
                        FramePart::Maximize => {
                            self.suppress_left_button_release = true;
                            self.maximize_window(id);
                        }
                        FramePart::Titlebar => {
                            let placement = self.internal_ui.placement(surface).cloned()?;
                            let start_data = GrabStartData {
                                focus: None,
                                button,
                                location,
                            };
                            pointer.set_grab(
                                self,
                                MoveInternalSurfaceGrab {
                                    start_data,
                                    surface,
                                    initial_location: (placement.geometry.0, placement.geometry.1)
                                        .into(),
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
                        // Internal application resizing is not exposed until the host can
                        // negotiate live content sizes. Consume the frame border instead of
                        // leaking the click into the hosted app or a client below it.
                        FramePart::ResizeNorth
                        | FramePart::ResizeNorthEast
                        | FramePart::ResizeEast
                        | FramePart::ResizeSouthEast
                        | FramePart::ResizeSouth
                        | FramePart::ResizeSouthWest
                        | FramePart::ResizeWest
                        | FramePart::ResizeNorthWest => {
                            self.suppress_left_button_release = true;
                        }
                    }
                    self.request_output_redraw();
                    return None;
                }
                if event.button() == Some(MouseButton::Left)
                    && button_state == ButtonState::Pressed
                    && keyboard.modifier_state().logo
                    && !pointer.is_grabbed()
                    && !client_present
                    && let Some((surface, _)) = self
                        .internal_ui
                        .application_surface_at((location.x, location.y))
                    && let Some(placement) = self.internal_ui.placement(surface).cloned()
                {
                    self.hotkeys.begin_pointer_chord();
                    self.internal_ui.focus_surface(surface);
                    self.reconcile_internal_application_focus();
                    let start_data = GrabStartData {
                        focus: None,
                        button,
                        location,
                    };
                    pointer.set_grab(
                        self,
                        MoveInternalSurfaceGrab {
                            start_data,
                            surface,
                            initial_location: (placement.geometry.0, placement.geometry.1).into(),
                        },
                        serial,
                        Focus::Clear,
                    );
                    return None;
                }
                // Preserve native button identity before the generic widget adapter
                // reduces all buttons to a boolean pressed/released action.
                let desktop_button = match event.button() {
                    Some(MouseButton::Left) => nickel_input::PointerButton::Primary,
                    Some(MouseButton::Right) => nickel_input::PointerButton::Secondary,
                    Some(MouseButton::Middle) => nickel_input::PointerButton::Middle,
                    Some(MouseButton::Back) => nickel_input::PointerButton::Back,
                    Some(MouseButton::Forward) => nickel_input::PointerButton::Forward,
                    _ => nickel_input::PointerButton::Native(button as u16),
                };
                // Once Smithay owns a pointer grab, every following button edge must reach that
                // grab. Letting an internal surface consume the release here strands Super+drag
                // in its move grab and leaves the surface attached to the cursor indefinitely.
                let internally_handled = !pointer.is_grabbed()
                    && (self.internal_ui.desktop_pointer_input(
                        &event.device().id(),
                        (location.x, location.y),
                        super::internal_ui::DesktopPointerAction::Button {
                            button: desktop_button,
                            edge: if button_state == ButtonState::Pressed {
                                nickel_input::KeyEdge::Pressed
                            } else {
                                nickel_input::KeyEdge::Released
                            },
                        },
                        desktop_modifiers(&keyboard.modifier_state()),
                        client_present,
                    ) || self.internal_ui.pointer_button_with_client(
                        (location.x, location.y),
                        button_state == ButtonState::Pressed,
                        client_present,
                    ));
                // A client press can blur the old internal owner without being
                // consumed by it. Deliver that lifecycle batch before forwarding.
                self.flush_internal_shell_input();
                if internally_handled {
                    if button_state == ButtonState::Pressed {
                        self.reconcile_internal_application_focus();
                    }
                    self.request_output_redraw();
                    return None;
                }

                // Foreground shell surfaces and captured gestures get first refusal.
                // A client underneath Launcher does not own a click inside Launcher.
                if event.button() == Some(MouseButton::Left)
                    && button_state == ButtonState::Pressed
                    && client_present
                {
                    self.dismiss_internal_launcher_for_client_press();
                }
                if button_state == ButtonState::Pressed {
                    self.record_interaction_output(pointer.current_location());
                }

                if button_state == ButtonState::Pressed {
                    self.refresh_stationary_pointer_focus(event.time());
                }

                const DOUBLE_CLICK_MS: u32 = 500;
                const DOUBLE_CLICK_DISTANCE: f64 = 6.0;
                let mut suppress_pointer_event = false;
                let mut frame_handled = false;
                let mut launcher_focus_restored = false;
                let mouse_button = event.button();
                let super_pressed = keyboard.modifier_state().logo;

                if mouse_button == Some(MouseButton::Left)
                    && button_state == ButtonState::Pressed
                    && !self.locked
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
                    } else if target
                        .as_ref()
                        .is_some_and(|window| self.is_on_screen_keyboard_window(window))
                    {
                        LauncherPointerTarget::OnScreenKeyboard
                    } else {
                        LauncherPointerTarget::Other
                    };
                    launcher_focus_restored =
                        self.launcher_pointer_press(target, restore_window_focus);
                }

                if mouse_button == Some(MouseButton::Left)
                    && button_state == ButtonState::Pressed
                    && !self.locked
                    && !super_pressed
                {
                    let location = pointer.current_location();
                    let frame_target = window_frame::topmost_frame_target(
                        self.space.elements().rev().filter_map(|window| {
                            let surface_accepts_input =
                                self.space.element_location(window).is_some_and(|loc| {
                                    window
                                        .surface_under(
                                            location - loc.to_f64(),
                                            WindowSurfaceType::ALL,
                                        )
                                        .is_some()
                                });
                            let bounds = self.space.element_geometry(window)?;
                            let geometry = crate::session::shell_layout::Geometry {
                                x: bounds.loc.x,
                                y: bounds.loc.y,
                                width: bounds.size.w,
                                height: bounds.size.h,
                            };
                            let frame_part = (!self.shell_windows().any(|shell| shell == window)
                                && !self.is_fullscreen_window(window)
                                && self.is_server_decorated(window))
                            .then(|| {
                                window_frame::hit_test(
                                    geometry,
                                    location.x.round() as i32,
                                    location.y.round() as i32,
                                )
                            })
                            .flatten();
                            Some((window.clone(), surface_accepts_input, frame_part))
                        }),
                    );

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
                        self.surrender_internal_focus();
                        keyboard.set_focus(
                            self,
                            crate::session::focus::KeyboardFocusTarget::for_window(&window),
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
                                            restored_from_maximized: false,
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
                        let unmanaged_x11_popup = window
                            .x11_surface()
                            .is_some_and(|surface| surface.is_override_redirect());
                        if !unmanaged_x11_popup {
                            let activate = !self.is_on_screen_keyboard_window(&window);
                            self.space.raise_element(&window, activate);
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
                                        if activate && actual_window == Some(id) =>
                                    {
                                        self.windows.raise(id);
                                        self.workspaces.focused(&id);
                                    }
                                    WindowPointerEffect::ActivateWindow(_) => {}
                                }
                            }
                            if !self.is_panel_window(&window)
                                && !self.is_on_screen_keyboard_window(&window)
                            {
                                self.space.elements().for_each(|candidate| {
                                    candidate.set_activated(candidate == &window);
                                });
                                self.surrender_internal_focus();
                                keyboard.set_focus(
                                    self,
                                    crate::session::focus::KeyboardFocusTarget::for_window(&window),
                                    serial,
                                );
                                self.space.elements().for_each(|window| {
                                    if let Some(toplevel) = window.toplevel() {
                                        toplevel.send_pending_configure();
                                    }
                                });
                            }
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
                            Option::<crate::session::focus::KeyboardFocusTarget>::None,
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
                            !self.is_shell_owned_window(window)
                                && !self.desktop_windows.contains(window)
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
                            restored_from_maximized: false,
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
                            !self.is_shell_owned_window(window)
                                && !self.desktop_windows.contains(window)
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
                let pointer = self.seat.get_pointer().unwrap();
                self.refresh_stationary_pointer_focus(event.time());

                let source = event.source();

                let horizontal_amount_discrete = event.amount_v120(Axis::Horizontal);
                let vertical_amount_discrete = event.amount_v120(Axis::Vertical);
                let horizontal_amount =
                    axis_amount(event.amount(Axis::Horizontal), horizontal_amount_discrete);
                let vertical_amount =
                    axis_amount(event.amount(Axis::Vertical), vertical_amount_discrete);

                let location = pointer.current_location();
                let client_present =
                    self.client_scene_under(location) && !self.internal_applications_are_foremost();
                let modifiers =
                    desktop_modifiers(&self.seat.get_keyboard().unwrap().modifier_state());
                if self.internal_ui.desktop_pointer_input(
                    &event.device().id(),
                    (location.x, location.y),
                    desktop_axis(
                        (horizontal_amount, vertical_amount),
                        (horizontal_amount_discrete, vertical_amount_discrete),
                    ),
                    modifiers,
                    client_present,
                ) || self.internal_ui.scroll_with_client(
                    (location.x, location.y),
                    horizontal_amount as f32,
                    vertical_amount as f32,
                    client_present,
                ) {
                    self.flush_internal_shell_input();
                    self.request_output_redraw();
                    return None;
                }

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

                pointer.axis(self, frame);
                pointer.frame(self);
            }
            InputEvent::TouchDown { event, .. } => {
                let geometry = self.touch_output_geometry(output_name)?;
                let location = event.position_transformed(geometry.size) + geometry.loc.to_f64();
                let client_present =
                    self.client_scene_under(location) && !self.internal_applications_are_foremost();
                let normalized = self.internal_ui.normalized_touch_input(
                    &event.device().id(),
                    i32::from(event.slot()) as u64,
                    (location.x, location.y),
                    crate::session::TouchPhase::Started,
                    client_present,
                );
                if normalized
                    || self.internal_ui.touch_with_client(
                        i32::from(event.slot()) as u64,
                        (location.x, location.y),
                        crate::session::TouchPhase::Started,
                        client_present,
                    )
                {
                    self.flush_internal_shell_input();
                    // Generic hosted apps can acquire focus on touch too; the
                    // seat and OSK lease must follow that runtime transition.
                    self.reconcile_internal_application_focus();
                    self.request_output_redraw();
                    return None;
                }
                if let Some(window) = self
                    .space
                    .element_under(location)
                    .map(|(window, _)| window.clone())
                    && !self.is_on_screen_keyboard_window(&window)
                    && !self.is_panel_window(&window)
                {
                    if let Some(id) = window
                        .wl_surface()
                        .and_then(|surface| self.surface_windows.get(&surface.id()).copied())
                    {
                        self.activate_window(id);
                    }
                    self.request_on_screen_keyboard();
                }
                self.record_interaction_output(location);
                self.active_touch_slots.insert(event.slot());
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
                let geometry = self.touch_output_geometry(output_name)?;
                let location = event.position_transformed(geometry.size) + geometry.loc.to_f64();
                if self.internal_ui.normalized_touch_input(
                    &event.device().id(),
                    i32::from(event.slot()) as u64,
                    (location.x, location.y),
                    crate::session::TouchPhase::Moved,
                    false,
                ) || self.internal_ui.touch(
                    i32::from(event.slot()) as u64,
                    (location.x, location.y),
                    crate::session::TouchPhase::Moved,
                ) {
                    self.flush_internal_shell_input();
                    self.request_output_redraw();
                    return None;
                }
                self.record_interaction_output(location);
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
                if self.internal_ui.normalized_touch_input(
                    &event.device().id(),
                    i32::from(event.slot()) as u64,
                    (0.0, 0.0),
                    crate::session::TouchPhase::Ended,
                    false,
                ) || self.internal_ui.touch(
                    i32::from(event.slot()) as u64,
                    (0.0, 0.0),
                    crate::session::TouchPhase::Ended,
                ) {
                    self.flush_internal_shell_input();
                    self.request_output_redraw();
                    return None;
                }
                self.active_touch_slots.remove(&event.slot());
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
            InputEvent::TouchCancel { .. } => {
                // Match the seat-wide Smithay cancellation below, and deliver
                // normalized cancellations before another input batch can run.
                let normalized = self.internal_ui.cancel_normalized_touches(None);
                if self.internal_ui.cancel_touches() | normalized {
                    self.flush_internal_shell_input();
                    self.request_output_redraw();
                }
                self.active_touch_slots.clear();
                self.seat.get_touch().unwrap().cancel(self);
            }
            _ => {}
        }
        None
    }
}

fn axis_amount(continuous: Option<f64>, v120: Option<f64>) -> f64 {
    let continuous = continuous.unwrap_or(0.0);
    if continuous != 0.0 {
        continuous
    } else {
        v120.unwrap_or(0.0) * 15.0 / 120.0
    }
}

fn desktop_axis(
    continuous: (f64, f64),
    v120: (Option<f64>, Option<f64>),
) -> super::internal_ui::DesktopPointerAction {
    // The normalized contract uses wheel lines when discrete is present, logical
    // pixels otherwise, with the opposite sign to Smithay's axis values. Preserve
    // fractional lines in delta; discrete is only the integer compatibility hint.
    let wheel = v120.0.is_some() || v120.1.is_some();
    let (x, y) = if wheel {
        (
            -v120.0.map_or(continuous.0 / 15.0, |value| value / 120.0),
            -v120.1.map_or(continuous.1 / 15.0, |value| value / 120.0),
        )
    } else {
        (-continuous.0, -continuous.1)
    };
    super::internal_ui::DesktopPointerAction::Axis {
        delta: nickel_input::Vector { x, y },
        discrete: wheel.then_some((x as i32, y as i32)),
    }
}

fn pointer_focus_needs_refresh<T: PartialEq, P>(
    current: Option<&T>,
    resolved: Option<&(T, P)>,
) -> bool {
    current != resolved.map(|(target, _)| target)
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

fn key_code_from_keysym(sym: Keysym) -> Option<KeyCode> {
    match sym {
        value if value == Keysym::new(keysyms::KEY_Super_L) => Some(KeyCode::SuperLeft),
        value if value == Keysym::new(keysyms::KEY_Super_R) => Some(KeyCode::SuperRight),
        value if value == Keysym::new(keysyms::KEY_l) || value == Keysym::new(keysyms::KEY_L) => {
            Some(KeyCode::KeyL)
        }
        value if value == Keysym::new(keysyms::KEY_Alt_L) => Some(KeyCode::AltLeft),
        value if value == Keysym::new(keysyms::KEY_Alt_R) => Some(KeyCode::AltRight),
        value if value == Keysym::new(keysyms::KEY_Shift_L) => Some(KeyCode::ShiftLeft),
        value if value == Keysym::new(keysyms::KEY_Shift_R) => Some(KeyCode::ShiftRight),
        value if value == Keysym::new(keysyms::KEY_Control_L) => Some(KeyCode::ControlLeft),
        value if value == Keysym::new(keysyms::KEY_Control_R) => Some(KeyCode::ControlRight),
        value if value == Keysym::new(keysyms::KEY_Left) => Some(KeyCode::ArrowLeft),
        value if value == Keysym::new(keysyms::KEY_Right) => Some(KeyCode::ArrowRight),
        value if value == Keysym::new(keysyms::KEY_Up) => Some(KeyCode::ArrowUp),
        value if value == Keysym::new(keysyms::KEY_Down) => Some(KeyCode::ArrowDown),
        value if value == Keysym::new(keysyms::KEY_space) => Some(KeyCode::Space),
        value if value == Keysym::new(keysyms::KEY_Escape) => Some(KeyCode::Escape),
        value if value == Keysym::new(keysyms::KEY_F4) => Some(KeyCode::F4),
        value if value == Keysym::new(keysyms::KEY_0) => Some(KeyCode::Digit0),
        value if value == Keysym::new(keysyms::KEY_1) => Some(KeyCode::Digit1),
        value if value == Keysym::new(keysyms::KEY_2) => Some(KeyCode::Digit2),
        value if value == Keysym::new(keysyms::KEY_3) => Some(KeyCode::Digit3),
        value if value == Keysym::new(keysyms::KEY_4) => Some(KeyCode::Digit4),
        value if value == Keysym::new(keysyms::KEY_5) => Some(KeyCode::Digit5),
        value if value == Keysym::new(keysyms::KEY_6) => Some(KeyCode::Digit6),
        value if value == Keysym::new(keysyms::KEY_7) => Some(KeyCode::Digit7),
        value if value == Keysym::new(keysyms::KEY_8) => Some(KeyCode::Digit8),
        value if value == Keysym::new(keysyms::KEY_9) => Some(KeyCode::Digit9),
        value if value == Keysym::new(keysyms::KEY_KP_0) => Some(KeyCode::Numpad0),
        value if value == Keysym::new(keysyms::KEY_KP_1) => Some(KeyCode::Numpad1),
        value if value == Keysym::new(keysyms::KEY_KP_2) => Some(KeyCode::Numpad2),
        value if value == Keysym::new(keysyms::KEY_KP_3) => Some(KeyCode::Numpad3),
        value if value == Keysym::new(keysyms::KEY_KP_4) => Some(KeyCode::Numpad4),
        value if value == Keysym::new(keysyms::KEY_KP_5) => Some(KeyCode::Numpad5),
        value if value == Keysym::new(keysyms::KEY_KP_6) => Some(KeyCode::Numpad6),
        value if value == Keysym::new(keysyms::KEY_KP_7) => Some(KeyCode::Numpad7),
        value if value == Keysym::new(keysyms::KEY_KP_8) => Some(KeyCode::Numpad8),
        value if value == Keysym::new(keysyms::KEY_KP_9) => Some(KeyCode::Numpad9),
        value
            if value == Keysym::new(keysyms::KEY_Tab)
                || value == Keysym::new(keysyms::KEY_ISO_Left_Tab) =>
        {
            Some(KeyCode::Tab)
        }
        value
            if value == Keysym::new(keysyms::KEY_Print)
                || value == Keysym::new(keysyms::KEY_Sys_Req) =>
        {
            Some(KeyCode::PrintScreen)
        }
        value
            if value == Keysym::new(keysyms::KEY_grave)
                || value == Keysym::new(keysyms::KEY_asciitilde) =>
        {
            Some(KeyCode::Backquote)
        }
        value if value == Keysym::new(keysyms::KEY_a) || value == Keysym::new(keysyms::KEY_A) => {
            Some(KeyCode::KeyA)
        }
        value if value == Keysym::new(keysyms::KEY_d) || value == Keysym::new(keysyms::KEY_D) => {
            Some(KeyCode::KeyD)
        }
        value if value == Keysym::new(keysyms::KEY_e) || value == Keysym::new(keysyms::KEY_E) => {
            Some(KeyCode::KeyE)
        }
        value if value == Keysym::new(keysyms::KEY_i) || value == Keysym::new(keysyms::KEY_I) => {
            Some(KeyCode::KeyI)
        }
        value if value == Keysym::new(keysyms::KEY_n) || value == Keysym::new(keysyms::KEY_N) => {
            Some(KeyCode::KeyN)
        }
        value if value == Keysym::new(keysyms::KEY_p) || value == Keysym::new(keysyms::KEY_P) => {
            Some(KeyCode::KeyP)
        }
        value if value == Keysym::new(keysyms::KEY_r) || value == Keysym::new(keysyms::KEY_R) => {
            Some(KeyCode::KeyR)
        }
        _ => None,
    }
}

fn consumer_control_from_keysym(sym: Keysym) -> Option<nickel_session_protocol::ConsumerControl> {
    use nickel_session_protocol::ConsumerControl;
    Some(match sym.raw() {
        keysyms::KEY_XF86AudioRaiseVolume => ConsumerControl::VolumeUp,
        keysyms::KEY_XF86AudioLowerVolume => ConsumerControl::VolumeDown,
        keysyms::KEY_XF86AudioMute => ConsumerControl::VolumeMute,
        keysyms::KEY_XF86AudioPlay => ConsumerControl::PlayPause,
        keysyms::KEY_XF86AudioPause => ConsumerControl::Pause,
        keysyms::KEY_XF86AudioStop => ConsumerControl::Stop,
        keysyms::KEY_XF86AudioNext => ConsumerControl::Next,
        keysyms::KEY_XF86AudioPrev => ConsumerControl::Previous,
        keysyms::KEY_XF86AudioForward => ConsumerControl::FastForward,
        keysyms::KEY_XF86AudioRewind => ConsumerControl::Rewind,
        _ => return None,
    })
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

fn recovery_shortcut_from_keysym(sym: Keysym) -> Option<nickel_ui::Shortcut> {
    match sym.raw() {
        keysyms::KEY_Return | keysyms::KEY_KP_Enter => Some(nickel_ui::Shortcut::Submit),
        keysyms::KEY_Escape => Some(nickel_ui::Shortcut::Escape),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn desktop_scroll_keeps_fractional_wheel_lines_and_pixel_distances() {
        use super::super::internal_ui::DesktopPointerAction;
        let DesktopPointerAction::Axis { delta, discrete } =
            super::desktop_axis((0.0, 7.5), (None, Some(60.0)))
        else {
            panic!("axis event")
        };
        assert_eq!(delta.y, -0.5);
        assert_eq!(discrete, Some((0, 0)));
        let DesktopPointerAction::Axis { delta, discrete } =
            super::desktop_axis((0.25, 1.5), (None, None))
        else {
            panic!("axis event")
        };
        assert_eq!(delta, nickel_input::Vector { x: -0.25, y: -1.5 });
        assert_eq!(discrete, None);
    }

    #[test]
    fn desktop_keys_keep_physical_identity_separate_from_layout_and_edges() {
        use nickel_input::{DeviceId, EventOrder, KeyEdge, LogicalKey, PhysicalKey};
        use smithay::input::keyboard::{Keysym, keysyms};
        let event = super::desktop_key_event(
            (38, Keysym::new(keysyms::KEY_q), super::KeyState::Released),
            Default::default(),
            DeviceId(7),
            EventOrder(11),
            false,
        );
        // evdev 30 / XKB 38 is physical A, regardless of the layout's q symbol.
        assert_eq!(
            event.physical,
            PhysicalKey::Code(nickel_input::KeyCode::KeyA)
        );
        assert_eq!(event.logical, LogicalKey::Character("q".into()));
        assert_eq!(event.edge, KeyEdge::Released);
        assert_eq!(event.device, DeviceId(7));
        assert_eq!(event.order, EventOrder(11));
        let enter = super::desktop_key_event(
            (
                104,
                Keysym::new(keysyms::KEY_KP_Enter),
                super::KeyState::Pressed,
            ),
            Default::default(),
            DeviceId(7),
            EventOrder(12),
            true,
        );
        assert_eq!(
            enter.physical,
            PhysicalKey::Code(nickel_input::KeyCode::NumpadEnter)
        );
        assert_eq!(enter.location, nickel_input::KeyLocation::Numpad);
        assert_eq!(
            enter.logical,
            LogicalKey::Named(nickel_input::NamedKey::Enter)
        );
        assert!(enter.repeat);
    }

    use smithay::utils::{Point, Rectangle};

    use smithay::input::keyboard::{Keysym, keysyms};

    use super::{
        ResizeEdge, axis_amount, consumer_control_from_keysym, pointer_focus_needs_refresh,
        recovery_shortcut_from_keysym, resize_edges_at, vt_from_keysym,
    };

    #[test]
    fn stationary_focus_refreshes_only_when_the_resolved_target_changes() {
        assert!(!pointer_focus_needs_refresh(Some(&7_u8), Some(&(7_u8, ()))));
        assert!(pointer_focus_needs_refresh(Some(&7_u8), Some(&(9_u8, ()))));
        assert!(pointer_focus_needs_refresh(Some(&7_u8), None::<&(u8, ())>));
        assert!(pointer_focus_needs_refresh(None, Some(&(9_u8, ()))));
        assert!(!pointer_focus_needs_refresh(None, None::<&(u8, ())>));
    }

    #[test]
    fn wheel_v120_survives_a_spurious_zero_continuous_amount() {
        assert_eq!(axis_amount(Some(0.0), Some(120.0)), 15.0);
        assert_eq!(axis_amount(Some(0.0), Some(-120.0)), -15.0);
        assert_eq!(axis_amount(Some(7.5), Some(120.0)), 7.5);
        assert_eq!(axis_amount(None, None), 0.0);
    }

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
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_Print)),
            Some(nickel_core::hotkeys::KeyCode::PrintScreen)
        );
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_Sys_Req)),
            Some(nickel_core::hotkeys::KeyCode::PrintScreen)
        );
    }

    #[test]
    fn xkb_forward_and_reverse_tab_keysyms_share_the_tab_hotkey() {
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_Tab)),
            Some(nickel_core::hotkeys::KeyCode::Tab)
        );
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_ISO_Left_Tab)),
            Some(nickel_core::hotkeys::KeyCode::Tab)
        );
    }

    #[test]
    fn xkb_top_row_and_keypad_digits_keep_distinct_physical_workspace_keys() {
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_0)),
            Some(nickel_core::hotkeys::KeyCode::Digit0)
        );
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_7)),
            Some(nickel_core::hotkeys::KeyCode::Digit7)
        );
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_KP_0)),
            Some(nickel_core::hotkeys::KeyCode::Numpad0)
        );
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_KP_7)),
            Some(nickel_core::hotkeys::KeyCode::Numpad7)
        );
    }

    #[test]
    fn xkb_l_keysyms_map_to_the_lock_hotkey() {
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_l)),
            Some(nickel_core::hotkeys::KeyCode::KeyL)
        );
        assert_eq!(
            super::key_code_from_keysym(Keysym::new(keysyms::KEY_L)),
            Some(nickel_core::hotkeys::KeyCode::KeyL)
        );
    }

    #[test]
    fn xkb_consumer_keysyms_map_without_collapsing_unrelated_keys() {
        use nickel_session_protocol::ConsumerControl;
        assert_eq!(
            consumer_control_from_keysym(Keysym::new(keysyms::KEY_XF86AudioRaiseVolume)),
            Some(ConsumerControl::VolumeUp)
        );
        assert_eq!(
            consumer_control_from_keysym(Keysym::new(keysyms::KEY_XF86AudioMute)),
            Some(ConsumerControl::VolumeMute)
        );
        assert_eq!(
            consumer_control_from_keysym(Keysym::new(keysyms::KEY_XF86AudioNext)),
            Some(ConsumerControl::Next)
        );
        assert_eq!(
            consumer_control_from_keysym(Keysym::new(keysyms::KEY_XF86MonBrightnessUp)),
            None
        );
    }

    #[test]
    fn recovery_keys_offer_retry_and_safe_exit_without_forwarding_text() {
        assert_eq!(
            recovery_shortcut_from_keysym(Keysym::new(keysyms::KEY_Return)),
            Some(nickel_ui::Shortcut::Submit)
        );
        assert_eq!(
            recovery_shortcut_from_keysym(Keysym::new(keysyms::KEY_KP_Enter)),
            Some(nickel_ui::Shortcut::Submit)
        );
        assert_eq!(
            recovery_shortcut_from_keysym(Keysym::new(keysyms::KEY_Escape)),
            Some(nickel_ui::Shortcut::Escape)
        );
        assert_eq!(
            recovery_shortcut_from_keysym(Keysym::new(keysyms::KEY_a)),
            None
        );
    }
}
