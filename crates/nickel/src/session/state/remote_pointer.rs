use super::{NickelSession, WindowId};
use nickel_remote_control::{
    DesktopPermit, HeldInput,
    leases::{ResourceEvidence, ResourceId},
    pointer::{PointerAction, PointerButton, PointerTarget},
};
use smithay::reexports::wayland_server::Resource;
use smithay::wayland::seat::WaylandFocus;
use std::time::{Duration, Instant};

pub(crate) struct RemoteHeldPointer {
    owner: HeldInput,
    target: PointerTarget,
    button: PointerButton,
    requested_x: i32,
    requested_y: i32,
    x: i32,
    y: i32,
    idle_deadline: Instant,
}

pub(super) struct ResolvedPointerTarget {
    pub(super) global_x: i32,
    pub(super) global_y: i32,
    pub(super) window: Option<ResourceId>,
    pub(super) surface: Option<ResourceId>,
    pub(super) output: Option<ResourceId>,
    pub(super) application: Option<String>,
    pub(super) surface_ancestors: Vec<ResourceId>,
    pub(super) native_window: Option<WindowId>,
}

impl ResolvedPointerTarget {
    fn evidence(&self) -> ResourceEvidence<'_> {
        ResourceEvidence {
            window: self.window.as_ref(),
            surface: self.surface.as_ref(),
            verified_application: self.application.as_deref(),
            output: self.output.as_ref(),
            authorized_surface_ancestors: &self.surface_ancestors,
            protected: false,
        }
    }
}

impl NickelSession {
    fn schedule_remote_pointer_check(&mut self) {
        if self.remote_pointer_timer_armed {
            return;
        }
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        self.event_loop_handle
            .insert_source(
                Timer::from_duration(self.remote_pointer_check_delay()),
                |_, _, data| {
                    data.revalidate_remote_pointer();
                    if data.remote_held_pointer.is_some() {
                        TimeoutAction::ToDuration(data.remote_pointer_check_delay())
                    } else {
                        data.remote_pointer_timer_armed = false;
                        TimeoutAction::Drop
                    }
                },
            )
            .expect("failed to register remote pointer cancellation timer");
        self.remote_pointer_timer_armed = true;
    }

    fn remote_pointer_check_delay(&self) -> Duration {
        let now = Instant::now();
        let Some(held) = self.remote_held_pointer.as_ref() else {
            return Duration::ZERO;
        };
        let deadline = held
            .owner
            .expires_at()
            .ok()
            .flatten()
            .map_or(held.idle_deadline, |expiry| expiry.min(held.idle_deadline));
        deadline
            .saturating_duration_since(now)
            .min(Duration::from_millis(100))
    }
    pub(crate) fn cancel_remote_pointer(&mut self) {
        self.invalidate_remote_shell_actions();
        if let Some(held) = self.remote_held_pointer.take() {
            self.release_controlled_pointer(held.button);
            drop(held);
            self.record_remote_input_ownership();
        }
    }

    pub(crate) fn revalidate_remote_pointer(&mut self) {
        if self.remote_input_dispatching {
            return;
        }
        if self.remote_held_pointer.is_some() && self.poll_remote_controller_ownership() {
            return;
        }
        let Some(held) = self.remote_held_pointer.as_ref() else {
            return;
        };
        let resolved =
            self.resolve_remote_pointer_target(&held.target, held.requested_x, held.requested_y);
        let valid = Instant::now() < held.idle_deadline
            && self.remote_pointer_has_click_grab()
            && resolved.is_ok_and(|resolved| {
                (matches!(held.target, PointerTarget::Window { .. })
                    || (resolved.global_x == held.x && resolved.global_y == held.y))
                    && held.owner.check_resource(&resolved.evidence()).is_ok()
            });
        if !valid {
            self.cancel_remote_pointer();
        }
    }

    fn remote_pointer_has_click_grab(&self) -> bool {
        self.seat.get_pointer().is_some_and(|pointer| {
            pointer
                .with_grab(|_, grab| grab.is::<smithay::input::pointer::ClickGrab<Self>>())
                .unwrap_or(false)
        })
    }

    pub(super) fn resolve_remote_pointer_target(
        &self,
        target: &PointerTarget,
        x: i32,
        y: i32,
    ) -> Result<ResolvedPointerTarget, String> {
        target.validate()?;
        if self.locked || self.shell_recovery_visible() {
            return Err("pointer target is protected".into());
        }
        let mut resolved = match target {
            PointerTarget::Window {
                window_id,
                generation,
            } => {
                let numeric = window_id
                    .parse::<u64>()
                    .map_err(|_| "invalid window identity")?;
                if numeric != *generation {
                    return Err("stale window generation".into());
                }
                let id = WindowId(numeric);
                let window = self
                    .window_for_registry_id(id)
                    .ok_or("window is not mapped")?;
                let geometry = self
                    .space
                    .element_geometry(&window)
                    .ok_or("window geometry unavailable")?;
                if x < 0 || y < 0 || x >= geometry.size.w || y >= geometry.size.h {
                    return Err("pointer coordinates are outside the client area".into());
                }
                ResolvedPointerTarget {
                    global_x: geometry.loc.x.checked_add(x).ok_or("coordinate overflow")?,
                    global_y: geometry.loc.y.checked_add(y).ok_or("coordinate overflow")?,
                    window: Some(ResourceId {
                        id: window_id.clone(),
                        generation: *generation,
                    }),
                    surface: None,
                    output: self.remote_window_output(id),
                    application: self.remote_verified_application(id),
                    surface_ancestors: Vec::new(),
                    native_window: Some(id),
                }
            }
            PointerTarget::Surface {
                surface_id,
                generation,
            } => {
                let identity = ResourceId {
                    id: surface_id.clone(),
                    generation: *generation,
                };
                let (runtime, output) = self.surface_capture_evidence(&identity)?;
                let placement = self
                    .internal_ui
                    .placement(runtime)
                    .ok_or("shell surface is unavailable")?;
                if x < 0
                    || y < 0
                    || i64::from(x) >= i64::from(placement.geometry.2)
                    || i64::from(y) >= i64::from(placement.geometry.3)
                {
                    return Err("pointer coordinates are outside the surface".into());
                }
                let surface_ancestors = self.remote_surface_ancestors(&identity);
                ResolvedPointerTarget {
                    global_x: placement
                        .geometry
                        .0
                        .checked_add(x)
                        .ok_or("coordinate overflow")?,
                    global_y: placement
                        .geometry
                        .1
                        .checked_add(y)
                        .ok_or("coordinate overflow")?,
                    window: None,
                    surface: Some(identity),
                    output,
                    application: None,
                    surface_ancestors,
                    native_window: None,
                }
            }
            PointerTarget::Output {
                output_id,
                generation,
            } => {
                let (output, current_generation) = self
                    .remote_output_generations
                    .get(output_id)
                    .ok_or("output generation has retired")?;
                if current_generation != generation
                    || !self.space.outputs().any(|current| current == output)
                {
                    return Err("output generation has retired".into());
                }
                let geometry = self
                    .space
                    .output_geometry(output)
                    .ok_or("output geometry is unavailable")?;
                if !geometry.contains((x, y)) {
                    return Err("global pointer coordinates are outside the output".into());
                }
                ResolvedPointerTarget {
                    global_x: x,
                    global_y: y,
                    window: None,
                    surface: None,
                    output: Some(ResourceId {
                        id: output_id.clone(),
                        generation: *generation,
                    }),
                    application: None,
                    surface_ancestors: Vec::new(),
                    native_window: None,
                }
            }
            PointerTarget::Desktop => {
                if self
                    .output_name_at((f64::from(x), f64::from(y)).into())
                    .is_none()
                {
                    return Err("global pointer coordinates are outside the desktop".into());
                }
                ResolvedPointerTarget {
                    global_x: x,
                    global_y: y,
                    window: None,
                    surface: None,
                    output: None,
                    application: None,
                    surface_ancestors: Vec::new(),
                    native_window: None,
                }
            }
        };
        if !self.remote_pointer_target_matches(target, resolved.global_x, resolved.global_y) {
            return Err("pointer target is occluded, protected, or outside its boundary".into());
        }
        if resolved.native_window.is_none() {
            resolved.native_window = self
                .space
                .element_under((f64::from(resolved.global_x), f64::from(resolved.global_y)))
                .and_then(|(window, _)| window.wl_surface())
                .and_then(|surface| self.surface_windows.get(&surface.id()))
                .copied();
        }
        Ok(resolved)
    }

    pub(crate) fn dispatch_remote_pointer(
        &mut self,
        permit: DesktopPermit,
        target: PointerTarget,
        x: i32,
        y: i32,
        action: PointerAction,
    ) -> Result<(), String> {
        self.revalidate_remote_pointer();
        let owns = self
            .remote_held_pointer
            .as_ref()
            .is_some_and(|held| held.owner.owned_by(&permit));
        if matches!(action, PointerAction::DragCancel) {
            if !owns {
                return Err("request does not own a pointer gesture".into());
            }
            permit.check_live()?;
            self.cancel_remote_pointer();
            return Ok(());
        }
        let continuing = matches!(action, PointerAction::DragMove | PointerAction::DragEnd);
        if self.remote_held_pointer.is_some() && (!continuing || !owns) {
            return Err("shared input is owned by an active gesture".into());
        }
        if continuing && !owns {
            return Err("no matching pointer gesture".into());
        }
        let result = (|| {
            action.validate()?;
            if self.poll_remote_controller_ownership() {
                return Err("local controller input is held or pending".into());
            }
            if self
                .seat
                .get_keyboard()
                .is_some_and(|keyboard| !keyboard.pressed_keys().is_empty())
            {
                return Err("local keyboard input is held".into());
            }
            if self
                .remote_held_pointer
                .as_ref()
                .is_some_and(|held| held.target != target)
            {
                return Err("pointer gesture cannot change its recipient".into());
            }
            let resolved = self.resolve_remote_pointer_target(&target, x, y)?;
            let global_x = resolved.global_x;
            let global_y = resolved.global_y;
            let pointer = self.seat.get_pointer().ok_or("pointer unavailable")?;
            if !continuing && pointer.is_grabbed() {
                return Err("pointer target is occluded, protected, or grabbed".into());
            }
            let evidence = resolved.evidence();
            self.remote_native_press = resolved
                .native_window
                .map(|window| (permit.clone(), window));
            self.remote_input_dispatching = true;
            let delivered = if let PointerAction::DragStart { button } = action {
                let mut pressed = false;
                match permit.begin_input(&evidence, || {
                    self.inject_controlled_pointer(
                        &target,
                        global_x,
                        global_y,
                        PointerAction::Move,
                    )?;
                    pressed = true;
                    self.press_controlled_pointer(button)
                }) {
                    Ok(owner) => {
                        self.remote_held_pointer = Some(RemoteHeldPointer {
                            owner,
                            target: target.clone(),
                            button,
                            requested_x: x,
                            requested_y: y,
                            x: global_x,
                            y: global_y,
                            idle_deadline: Instant::now() + Duration::from_secs(30),
                        });
                        self.record_remote_input_ownership();
                        Ok(())
                    }
                    Err(error) => {
                        if pressed {
                            self.release_controlled_pointer(button);
                        }
                        Err(error)
                    }
                }
            } else if continuing {
                let mut held = self.remote_held_pointer.take().unwrap();
                let delivered = permit.continue_input(&held.owner, &evidence, || {
                    self.inject_controlled_pointer(&target, global_x, global_y, PointerAction::Move)
                });
                if delivered.is_err() || matches!(action, PointerAction::DragEnd) {
                    self.release_controlled_pointer(held.button);
                    drop(held);
                } else {
                    held.requested_x = x;
                    held.requested_y = y;
                    held.x = global_x;
                    held.y = global_y;
                    held.idle_deadline = Instant::now() + Duration::from_secs(30);
                    self.remote_held_pointer = Some(held);
                }
                self.record_remote_input_ownership();
                delivered
            } else {
                permit.with_input(&evidence, || {
                    self.inject_controlled_pointer(&target, global_x, global_y, action)
                })
            };
            self.remote_input_dispatching = false;
            self.remote_native_press = None;
            if self.remote_held_pointer.is_some() {
                self.schedule_remote_pointer_check();
            }
            delivered
        })();
        if result.is_err() {
            self.remote_gtk_epoch = self.remote_gtk_epoch.saturating_add(1);
        }
        if result.is_err() && owns {
            self.cancel_remote_pointer();
        }
        result
    }
}
