use super::{NickelSession, WindowId};
use nickel_remote_control::{
    DesktopPermit, HeldInput,
    leases::{ResourceEvidence, ResourceId},
    pointer::{PointerAction, PointerButton},
};
use std::time::{Duration, Instant};

pub(crate) struct RemoteHeldPointer {
    owner: HeldInput,
    window: WindowId,
    button: PointerButton,
    x: i32,
    y: i32,
    idle_deadline: Instant,
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
        let valid = Instant::now() < held.idle_deadline
            && self.remote_pointer_target_matches(held.window, held.x, held.y)
            && self.remote_pointer_has_click_grab()
            && self
                .with_remote_pointer_evidence(held.window, held.x, held.y, |evidence| {
                    held.owner.check_resource(evidence)
                })
                .is_ok();
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

    fn with_remote_pointer_evidence<T>(
        &self,
        id: WindowId,
        x: i32,
        y: i32,
        effect: impl FnOnce(&ResourceEvidence<'_>) -> Result<T, String>,
    ) -> Result<T, String> {
        let identity = ResourceId {
            id: id.0.to_string(),
            generation: id.0,
        };
        let output = self
            .output_name_at((f64::from(x), f64::from(y)).into())
            .and_then(|name| self.remote_output_identity(name));
        let application = self.remote_verified_application(id);
        effect(&ResourceEvidence {
            window: Some(&identity),
            surface: None,
            verified_application: application.as_deref(),
            output: output.as_ref(),
            authorized_surface_ancestors: &[],
            protected: self.remote_window_is_protected(id),
        })
    }

    pub(crate) fn dispatch_remote_pointer(
        &mut self,
        permit: DesktopPermit,
        id: String,
        generation: u64,
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
            let numeric = id.parse::<u64>().map_err(|_| "invalid window identity")?;
            if numeric != generation {
                return Err("stale window generation".into());
            }
            let id = WindowId(numeric);
            if self
                .remote_held_pointer
                .as_ref()
                .is_some_and(|held| held.window != id)
            {
                return Err("pointer gesture cannot change its recipient".into());
            }
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
            let x = geometry.loc.x.checked_add(x).ok_or("coordinate overflow")?;
            let y = geometry.loc.y.checked_add(y).ok_or("coordinate overflow")?;
            let pointer = self.seat.get_pointer().ok_or("pointer unavailable")?;
            if (!continuing && pointer.is_grabbed())
                || !self.remote_pointer_target_matches(id, x, y)
            {
                return Err("pointer target is occluded, protected, or grabbed".into());
            }
            // Own these evidence values before mutating the compositor in the
            // authorization closure. Every continuation resolves them anew.
            let identity = ResourceId {
                id: numeric.to_string(),
                generation,
            };
            let output = self
                .output_name_at((f64::from(x), f64::from(y)).into())
                .and_then(|name| self.remote_output_identity(name));
            let application = self.remote_verified_application(id);
            let evidence = ResourceEvidence {
                window: Some(&identity),
                surface: None,
                verified_application: application.as_deref(),
                output: output.as_ref(),
                authorized_surface_ancestors: &[],
                protected: false,
            };
            self.remote_native_press = Some((permit.clone(), id));
            self.remote_input_dispatching = true;
            let delivered = if let PointerAction::DragStart { button } = action {
                let mut pressed = false;
                match permit.begin_input(&evidence, || {
                    self.inject_controlled_pointer(id, x, y, PointerAction::Move)?;
                    pressed = true;
                    self.press_controlled_pointer(button)
                }) {
                    Ok(owner) => {
                        self.remote_held_pointer = Some(RemoteHeldPointer {
                            owner,
                            window: id,
                            button,
                            x,
                            y,
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
                    self.inject_controlled_pointer(id, x, y, PointerAction::Move)
                });
                if delivered.is_err() || matches!(action, PointerAction::DragEnd) {
                    self.release_controlled_pointer(held.button);
                    drop(held);
                } else {
                    held.x = x;
                    held.y = y;
                    held.idle_deadline = Instant::now() + Duration::from_secs(30);
                    self.remote_held_pointer = Some(held);
                }
                self.record_remote_input_ownership();
                delivered
            } else {
                permit.with_input(&evidence, || {
                    self.inject_controlled_pointer(id, x, y, action)
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
