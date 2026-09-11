use super::native_key_worker::NativeKeyObservation;
use super::{NickelSession, WindowId};
use nickel_remote_control::{
    DesktopPermit, HeldInput,
    keyboard::KeyboardAction,
    leases::{ResourceEvidence, ResourceId},
};
use smithay::xwayland::xwm::X11IsolatedModifiers;
use smithay::{
    backend::input::{InputTime, KeyState},
    input::keyboard::{FilterResult, KeyboardSource, Keysym},
    utils::SERIAL_COUNTER,
};
use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

pub(crate) struct RemoteHeldKeyboard {
    owner: HeldInput,
    window: WindowId,
    source: KeyboardSource,
    idle_deadline: Instant,
    native_x11: Option<NativeHeldKeys>,
}

struct NativeHeldKeys {
    surface: smithay::xwayland::X11Surface,
    codes: Vec<u8>,
    sent: usize,
    pending: Option<NativeStart>,
    modifiers: X11IsolatedModifiers,
    restore_modifiers: Option<X11IsolatedModifiers>,
}

struct NativeStart {
    permit: DesktopPermit,
    reply: Option<std::sync::mpsc::SyncSender<Result<(), String>>>,
    index: usize,
    attempts: u8,
    sequence: u64,
    release_after_confirmation: bool,
    prepare_keyboard: bool,
    remaining_text: VecDeque<(u8, X11IsolatedModifiers)>,
    completed_text_keys: Option<usize>,
    text_batch_len: usize,
}

impl Drop for NativeStart {
    fn drop(&mut self) {
        if let Some(reply) = self.reply.take() {
            let error = self.completed_text_keys.map_or_else(
                || "native keyboard start cancelled".to_owned(),
                |count| format!("native text cancelled after {count} key presses in completed batches; additional input may have been delivered"),
            );
            let _ = reply.try_send(Err(error));
        }
    }
}

impl NativeHeldKeys {
    fn release(&self) {
        let codes: Vec<_> = self.codes[..self.sent].iter().rev().copied().collect();
        let _ = self
            .surface
            .send_isolated_key_inputs(&codes, KeyState::Released);
        if let Some(modifiers) = self.restore_modifiers {
            let _ = self.surface.set_isolated_modifiers(modifiers);
        }
    }
}

fn press_native_key(
    surface: &smithay::xwayland::X11Surface,
    code: u8,
    modifiers: X11IsolatedModifiers,
) -> Result<(), String> {
    surface
        .focus_isolated_keyboard()
        .map_err(|_| "isolated keyboard focus failed")?;
    surface
        .set_isolated_modifiers(modifiers)
        .map_err(|_| "isolated text modifiers failed")?;
    surface
        .send_isolated_key_inputs(&[code], KeyState::Pressed)
        .map_err(|_| "isolated text delivery failed".to_owned())
}

impl NickelSession {
    pub(crate) fn remote_keyboard_native_focus_lost(
        &mut self,
        window: &smithay::xwayland::X11Surface,
    ) {
        if self
            .remote_held_keyboard
            .as_ref()
            .and_then(|held| held.native_x11.as_ref())
            .is_some_and(|native| native.surface == *window)
        {
            self.cancel_remote_keyboard();
        }
    }

    fn submit_native_keyboard_query(&self) -> Result<(), String> {
        let held = self
            .remote_held_keyboard
            .as_ref()
            .ok_or("keyboard owner unavailable")?;
        let native = held
            .native_x11
            .as_ref()
            .ok_or("native keyboard unavailable")?;
        let pending = native
            .pending
            .as_ref()
            .ok_or("keyboard start is not pending")?;
        self.remote_native_key_worker
            .as_ref()
            .ok_or("native keyboard verifier unavailable")?
            .submit(super::native_key_worker::NativeKeyQuery {
                surface: native.surface.clone(),
                prepare_keyboard: pending.prepare_keyboard,
                source: held.source,
                sequence: pending.sequence,
                code: native.codes[pending.index],
            })
    }

    pub(super) fn complete_native_keyboard_query(
        &mut self,
        source: KeyboardSource,
        sequence: u64,
        result: Result<NativeKeyObservation, String>,
    ) {
        self.revalidate_remote_keyboard();
        if !self.remote_held_keyboard.as_ref().is_some_and(|held| {
            held.source == source
                && held
                    .native_x11
                    .as_ref()
                    .and_then(|native| native.pending.as_ref())
                    .is_some_and(|pending| pending.sequence == sequence)
        }) {
            return;
        }
        let mut held = self.remote_held_keyboard.take().unwrap();
        let identity = ResourceId {
            id: held.window.0.to_string(),
            generation: held.window.0,
        };
        let output = self.remote_window_output(held.window);
        let application = self.remote_verified_application(held.window);
        let evidence = ResourceEvidence {
            window: Some(&identity),
            surface: None,
            verified_application: application.as_deref(),
            output: output.as_ref(),
            authorized_surface_ancestors: &[],
            protected: self.remote_window_is_protected(held.window),
        };
        let native = held.native_x11.as_mut().unwrap();
        let pending = native.pending.as_mut().unwrap();
        let outcome = (|| {
            let observation = result?;
            pending.permit.continue_input(&held.owner, &evidence, || {
                let pressed = match observation {
                    NativeKeyObservation::Modifiers(modifiers, keymap)
                        if pending.prepare_keyboard =>
                    {
                        native.restore_modifiers = Some(modifiers);
                        native
                            .surface
                            .set_isolated_keymap(*keymap)
                            .map_err(|_| "isolated keyboard map setup failed".to_owned())?;
                        pending.prepare_keyboard = false;
                        pending.sequence += 1;
                        pending.attempts = 1;
                        native.sent = 1;
                        press_native_key(&native.surface, native.codes[0], native.modifiers)?;
                        return Ok(false);
                    }
                    NativeKeyObservation::Pressed(pressed) if !pending.prepare_keyboard => pressed,
                    _ => return Err("unexpected native keyboard observation".into()),
                };
                if pressed {
                    pending.index += 1;
                    pending.attempts = 0;
                    if pending.index == native.codes.len() {
                        if let Some(count) = &mut pending.completed_text_keys {
                            *count += pending.text_batch_len;
                        }
                        if pending.release_after_confirmation {
                            let codes: Vec<_> =
                                native.codes[..native.sent].iter().rev().copied().collect();
                            let released = if pending.remaining_text.is_empty() {
                                native
                                    .surface
                                    .send_isolated_key_inputs(&codes, KeyState::Released)
                            } else {
                                native.surface.release_isolated_text_key(codes[0])
                            };
                            released.map_err(|_| "native X11 key release failed".to_owned())?;
                            native.sent = 0;
                        }
                        if !pending.remaining_text.is_empty() {
                            // Bound queued native work and return to the owner
                            // loop between batches for focus/lease revalidation.
                            // The final key remains down for native confirmation;
                            // preceding characters are balanced on the same X11
                            // connection before that confirmation request.
                            pending.text_batch_len = pending.remaining_text.len().min(8);
                            pending.index = 0;
                            pending.attempts = 1;
                            pending.sequence += 1;
                            for index in 0..pending.text_batch_len {
                                let (code, modifiers) = pending.remaining_text.pop_front().unwrap();
                                native.codes = vec![code];
                                native.modifiers = modifiers;
                                native.sent = 1;
                                press_native_key(&native.surface, code, modifiers)?;
                                if index + 1 < pending.text_batch_len {
                                    native
                                        .surface
                                        .release_isolated_text_key(code)
                                        .map_err(|_| "native text release failed".to_owned())?;
                                    native.sent = 0;
                                }
                            }
                            return Ok(false);
                        }
                        if pending.release_after_confirmation
                            && let Some(modifiers) = native.restore_modifiers
                        {
                            native
                                .surface
                                .set_isolated_modifiers(modifiers)
                                .map_err(|_| "native modifier restoration failed".to_owned())?;
                            native.restore_modifiers = None;
                        }
                        return Ok(true);
                    }
                } else if pending.attempts >= 3 {
                    return Err("native keyboard did not confirm the requested key".into());
                }
                pending.attempts += 1;
                pending.sequence += 1;
                native.sent = pending.index + 1;
                native
                    .surface
                    .send_isolated_key_inputs(
                        &native.codes[pending.index..=pending.index],
                        KeyState::Pressed,
                    )
                    .map_err(|_| "native X11 key delivery failed".to_owned())?;
                Ok(false)
            })
        })();
        match outcome {
            Ok(true) => {
                let mut pending = native.pending.take().unwrap();
                if pending.release_after_confirmation {
                    self.release_remote_keyboard_source(held.source);
                    drop(held);
                } else {
                    self.remote_held_keyboard = Some(held);
                }
                if let Some(reply) = pending.reply.take() {
                    let _ = reply.try_send(Ok(()));
                }
            }
            Ok(false) => {
                self.remote_held_keyboard = Some(held);
                if self.submit_native_keyboard_query().is_err() {
                    self.cancel_remote_keyboard();
                }
            }
            Err(_) => {
                native.release();
                self.release_remote_keyboard_source(held.source);
                drop(held);
            }
        }
        self.record_remote_input_ownership();
    }

    pub(crate) fn remote_keyboard_source_focus_cancelled(&mut self, source: KeyboardSource) {
        if self
            .remote_held_keyboard
            .as_ref()
            .is_some_and(|held| held.source == source)
        {
            let held = self.remote_held_keyboard.take().unwrap();
            if let Some(native) = &held.native_x11 {
                native.release();
            }
            // Smithay already cleared this source while holding its keyboard
            // lock. Do not call release_source again from this callback.
            drop(held);
            self.record_remote_input_ownership();
        }
    }

    pub(crate) fn cancel_remote_keyboard_before_focus(&mut self, next: WindowId) {
        if self
            .remote_held_keyboard
            .as_ref()
            .is_some_and(|held| held.window != next)
        {
            self.cancel_remote_keyboard();
        }
    }
    pub(crate) fn cancel_remote_keyboard(&mut self) {
        self.invalidate_remote_shell_actions();
        if let Some(held) = self.remote_held_keyboard.take() {
            if let Some(native) = &held.native_x11 {
                native.release();
            }
            self.release_remote_keyboard_source(held.source);
            drop(held);
            self.record_remote_input_ownership();
        }
    }

    fn release_remote_keyboard_source(&mut self, source: KeyboardSource) {
        if let Some(keyboard) = self.seat.get_keyboard() {
            keyboard.release_source(self, source);
        }
        let _ = self.display_handle.flush_clients();
    }

    pub(crate) fn revalidate_remote_keyboard(&mut self) {
        if self.remote_input_dispatching {
            return;
        }
        if self.remote_held_keyboard.is_some() && self.poll_remote_controller_ownership() {
            return;
        }
        let Some(held) = self.remote_held_keyboard.as_ref() else {
            return;
        };
        let identity = ResourceId {
            id: held.window.0.to_string(),
            generation: held.window.0,
        };
        let output = self.remote_window_output(held.window);
        let application = self.remote_verified_application(held.window);
        let evidence = ResourceEvidence {
            window: Some(&identity),
            surface: None,
            verified_application: application.as_deref(),
            output: output.as_ref(),
            authorized_surface_ancestors: &[],
            protected: self.remote_window_is_protected(held.window),
        };
        let valid = held
            .native_x11
            .as_ref()
            .and_then(|native| native.pending.as_ref())
            .is_none_or(|pending| pending.permit.check_live().is_ok())
            && Instant::now() < held.idle_deadline
            && self.remote_keyboard_focus_matches(held.window)
            && self
                .seat
                .get_keyboard()
                .is_some_and(|keyboard| keyboard.source_has_input(held.source))
            && held.owner.check_resource(&evidence).is_ok();
        if !valid {
            self.cancel_remote_keyboard();
        }
    }

    fn remote_keyboard_check_delay(&self) -> Duration {
        let Some(held) = self.remote_held_keyboard.as_ref() else {
            return Duration::ZERO;
        };
        held.owner
            .expires_at()
            .ok()
            .flatten()
            .map_or(held.idle_deadline, |expiry| expiry.min(held.idle_deadline))
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(100))
    }

    fn schedule_remote_keyboard_check(&mut self) {
        if self.remote_keyboard_timer_armed {
            return;
        }
        use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
        self.event_loop_handle
            .insert_source(
                Timer::from_duration(self.remote_keyboard_check_delay()),
                |_, _, data| {
                    data.revalidate_remote_keyboard();
                    if data.remote_held_keyboard.is_some() {
                        TimeoutAction::ToDuration(data.remote_keyboard_check_delay())
                    } else {
                        data.remote_keyboard_timer_armed = false;
                        TimeoutAction::Drop
                    }
                },
            )
            .expect("failed to register remote keyboard cancellation timer");
        self.remote_keyboard_timer_armed = true;
    }

    pub(crate) fn dispatch_remote_keyboard(
        &mut self,
        permit: DesktopPermit,
        id: String,
        generation: u64,
        action: KeyboardAction,
        reply: std::sync::mpsc::SyncSender<Result<(), String>>,
    ) -> Result<bool, String> {
        let mut deferred = false;
        self.revalidate_remote_keyboard();
        let owns = self
            .remote_held_keyboard
            .as_ref()
            .is_some_and(|held| held.owner.owned_by(&permit));
        if matches!(action, KeyboardAction::HoldCancel) {
            if !owns {
                return Err("request does not own held keyboard input".into());
            }
            permit.check_live()?;
            self.cancel_remote_keyboard();
            return Ok(false);
        }
        let continuing = matches!(
            action,
            KeyboardAction::HoldKeepAlive | KeyboardAction::HoldEnd
        );
        if self.remote_held_keyboard.is_some() && (!owns || !continuing) {
            return Err("shared input is owned by held keys".into());
        }
        if continuing && !owns {
            return Err("no matching held keyboard input".into());
        }
        if continuing
            && self
                .remote_held_keyboard
                .as_ref()
                .and_then(|held| held.native_x11.as_ref())
                .is_some_and(|native| native.pending.is_some())
        {
            return Err("native keyboard start is still pending".into());
        }
        let result = (|| {
            action.validate()?;
            if self.poll_remote_controller_ownership() {
                return Err("local controller input is held or pending".into());
            }
            if self
                .seat
                .get_pointer()
                .is_some_and(|pointer| pointer.is_grabbed())
            {
                return Err("pointer input is held or grabbed".into());
            }
            let numeric = id.parse::<u64>().map_err(|_| "invalid window identity")?;
            if numeric != generation {
                return Err("stale window generation".into());
            }
            let id = WindowId(numeric);
            if self
                .remote_held_keyboard
                .as_ref()
                .is_some_and(|held| held.window != id)
            {
                return Err("held keyboard input cannot change recipient".into());
            }
            if !(if continuing {
                self.remote_keyboard_focus_matches(id)
            } else {
                self.remote_keyboard_target_matches(id)
            }) {
                return Err("keyboard recipient changed or input is busy".into());
            }
            let identity = ResourceId {
                id: numeric.to_string(),
                generation,
            };
            let output = self.remote_window_output(id);
            let application = self.remote_verified_application(id);
            let evidence = ResourceEvidence {
                window: Some(&identity),
                surface: None,
                verified_application: application.as_deref(),
                output: output.as_ref(),
                authorized_surface_ancestors: &[],
                protected: false,
            };
            let mut text_keys = VecDeque::new();
            let is_x11 = self
                .window_for_registry_id(id)
                .is_some_and(|window| window.x11_surface().is_some());
            let (action, release_after_confirmation) = match action {
                KeyboardAction::Text { text } if is_x11 => {
                    let keyboard = self.seat.get_keyboard().ok_or("keyboard unavailable")?;
                    let plan = keyboard.with_xkb_state(self, |context| {
                        let xkb = context.xkb().lock().unwrap();
                        // SAFETY: the borrowed keymap does not leave this guard.
                        super::super::on_screen_keyboard::text_key_plan(
                            unsafe { xkb.keymap() },
                            xkb.active_layout().0,
                            &text,
                        )
                    })?;
                    for (code, modifiers) in plan {
                        text_keys.push_back((
                            u8::try_from(code.raw())
                                .map_err(|_| "text keycode outside X11 range")?,
                            X11IsolatedModifiers {
                                locked: u8::try_from(modifiers.serialized.depressed)
                                    .map_err(|_| "text modifier mask outside X11 range")?,
                                latched: 0,
                                group: u8::try_from(modifiers.serialized.layout_effective)
                                    .map_err(|_| "text layout outside X11 range")?,
                                latched_group: 0,
                            },
                        ));
                    }
                    (
                        KeyboardAction::HoldStart {
                            keysym: 0,
                            modifiers: Vec::new(),
                        },
                        true,
                    )
                }
                KeyboardAction::Key { keysym, modifiers }
                    if self
                        .window_for_registry_id(id)
                        .is_some_and(|window| window.x11_surface().is_some()) =>
                {
                    (KeyboardAction::HoldStart { keysym, modifiers }, true)
                }
                action => (action, false),
            };
            if let KeyboardAction::HoldStart { keysym, modifiers } = action {
                let keyboard = self.seat.get_keyboard().ok_or("keyboard unavailable")?;
                let current_text = text_keys.pop_front();
                let mut keys = Vec::with_capacity(modifiers.len() + 1);
                if let Some((code, _)) = current_text {
                    keys.push(smithay::input::keyboard::Keycode::new(u32::from(code)));
                }
                for symbol in modifiers
                    .into_iter()
                    .chain((current_text.is_none()).then_some(keysym))
                {
                    let code = keyboard
                        .keycode_for_keysym(Keysym::new(symbol))
                        .ok_or("key unavailable in the current layout")?;
                    if !keys.contains(&code) {
                        keys.push(code);
                    }
                }
                let native_group =
                    u8::try_from(keyboard.modifier_state().serialized.layout_effective)
                        .map_err(|_| "keyboard layout outside X11 range")?;
                let native_x11 = self
                    .window_for_registry_id(id)
                    .and_then(|window| window.x11_surface().cloned())
                    .map(|surface| NativeHeldKeys {
                        surface,
                        codes: keys.iter().map(|code| code.raw() as u8).collect(),
                        sent: 0,
                        modifiers: current_text.map(|(_, modifiers)| modifiers).unwrap_or(
                            X11IsolatedModifiers {
                                locked: 0,
                                latched: 0,
                                group: native_group,
                                latched_group: 0,
                            },
                        ),
                        restore_modifiers: None,
                        pending: Some(NativeStart {
                            permit: permit.clone(),
                            // Arm the deferred reply only after begin_input accepts
                            // the resource boundary. Otherwise dropping this
                            // uncommitted plan can mask the synchronous denial
                            // with "native keyboard start cancelled".
                            reply: None,
                            index: 0,
                            attempts: 1,
                            sequence: 1,
                            release_after_confirmation,
                            prepare_keyboard: true,
                            remaining_text: text_keys,
                            completed_text_keys: current_text.map(|_| 0),
                            text_batch_len: 1,
                        }),
                    });
                if native_x11.is_some() && keys.iter().any(|code| u8::try_from(code.raw()).is_err())
                {
                    return Err("key is outside the X11 keycode range".into());
                }
                let source = KeyboardSource::new_focus_bound_auxiliary();
                let result = permit.begin_input(&evidence, || {
                    if native_x11.is_some()
                        && !keyboard.register_external_focus_bound_source(source)
                    {
                        return Err("could not register native keyboard ownership".into());
                    }
                    if native_x11.is_some() {
                        // Capture private state and the native map before any
                        // effect, for text, one-shot keys, and held chords alike.
                        return Ok(());
                    } else {
                        for code in keys {
                            keyboard.input_from_source::<(), _>(
                                source,
                                self,
                                code,
                                KeyState::Pressed,
                                SERIAL_COUNTER.next_serial(),
                                InputTime::now(),
                                |_, _, _| FilterResult::Forward,
                            );
                        }
                    }
                    self.display_handle
                        .flush_clients()
                        .map_err(|_| "could not flush keyboard input".into())
                });
                match result {
                    Ok(owner) => {
                        let mut native_x11 = native_x11;
                        if let Some(pending) = native_x11
                            .as_mut()
                            .and_then(|native| native.pending.as_mut())
                        {
                            pending.reply = Some(reply.clone());
                        }
                        self.remote_held_keyboard = Some(RemoteHeldKeyboard {
                            owner,
                            window: id,
                            source,
                            idle_deadline: Instant::now() + Duration::from_secs(30),
                            native_x11,
                        });
                        self.record_remote_input_ownership();
                        if self
                            .remote_held_keyboard
                            .as_ref()
                            .unwrap()
                            .native_x11
                            .is_some()
                        {
                            if let Err(error) = self.submit_native_keyboard_query() {
                                self.cancel_remote_keyboard();
                                return Err(error);
                            }
                            deferred = true;
                        }
                        self.schedule_remote_keyboard_check();
                        Ok(())
                    }
                    Err(error) => {
                        if let Some(native) = &native_x11 {
                            native.release();
                        }
                        self.release_remote_keyboard_source(source);
                        Err(error)
                    }
                }
            } else if continuing {
                let mut held = self.remote_held_keyboard.take().unwrap();
                let result = permit.continue_input(&held.owner, &evidence, || Ok(()));
                if result.is_err() || matches!(action, KeyboardAction::HoldEnd) {
                    if let Some(native) = &held.native_x11 {
                        native.release();
                    }
                    self.release_remote_keyboard_source(held.source);
                    drop(held);
                } else {
                    held.idle_deadline = Instant::now() + Duration::from_secs(30);
                    self.remote_held_keyboard = Some(held);
                }
                self.record_remote_input_ownership();
                result
            } else {
                permit.with_input(&evidence, || self.inject_controlled_keyboard(id, action))
            }
        })();
        if result.is_err() && owns {
            self.cancel_remote_keyboard();
        }
        result.map(|()| deferred)
    }
}
