//! Trusted shell input, delivered to a leased seat recipient without taking focus.

use nickel_session_protocol::{OnScreenKeyboardInput, OnScreenKeyboardSnapshot, ShellRole};
use smithay::{
    backend::input::{InputTime, KeyState},
    desktop::Window,
    input::keyboard::{
        FilterResult, KeyboardSource, KeyboardTarget, Keycode, Keysym, ModifiersState, xkb,
    },
    reexports::wayland_server::Resource,
    utils::SERIAL_COUNTER,
    wayland::{seat::WaylandFocus, text_input::TextInputSeat},
};

use crate::state::NickelSession;

pub(crate) struct OnScreenKeyboardState {
    pub(crate) height: u32,
    pub(crate) dock_top: bool,
    pub(crate) output_name: Option<String>,
    pub(crate) displaced: Vec<(Window, crate::shell_layout::Geometry)>,
    pub(crate) auto_show_requested: bool,
    pub(crate) touchscreens: std::collections::HashSet<String>,
    controller_barrier_unix_ms: u64,
    generation: u64,
    environment_override: bool,
    epoch: u64,
    enabled: bool,
    pub(crate) visible: bool,
    source: KeyboardSource,
}

impl Default for OnScreenKeyboardState {
    fn default() -> Self {
        Self {
            height: nickel_core::on_screen_keyboard::KEYBOARD_HEIGHT,
            dock_top: false,
            output_name: None,
            displaced: Vec::new(),
            auto_show_requested: false,
            controller_barrier_unix_ms: controller_barrier_now(),
            generation: 0,
            environment_override: false,
            touchscreens: Default::default(),
            epoch: 1,
            enabled: false,
            visible: false,
            source: KeyboardSource::new_auxiliary(),
        }
    }
}

impl NickelSession {
    pub(crate) fn on_screen_keyboard_focus_changed(&mut self) {
        self.on_screen_keyboard.epoch = self.on_screen_keyboard.epoch.wrapping_add(1);
        self.on_screen_keyboard.auto_show_requested = false;
    }

    pub(crate) fn request_on_screen_keyboard(&mut self) {
        self.on_screen_keyboard.auto_show_requested =
            self.on_screen_keyboard.enabled && !self.locked;
    }

    pub(crate) fn is_on_screen_keyboard_window(&self, window: &Window) -> bool {
        window.wl_surface().is_some_and(|surface| {
            self.surface_windows.get(&surface.id()).is_some_and(|id| {
                self.is_shell_owned_window(window)
                    && self.windows.snapshot().iter().any(|entry| {
                        entry.id == *id
                            && ShellRole::from_application_id(&entry.app_id)
                                == Some(ShellRole::OnScreenKeyboard)
                    })
            })
        })
    }

    pub(crate) fn on_screen_keyboard_snapshot(&self) -> OnScreenKeyboardSnapshot {
        let focus = self
            .seat
            .get_keyboard()
            .and_then(|keyboard| keyboard.current_focus());
        let recipient = focus
            .as_ref()
            .and_then(|focus| focus.wl_surface())
            .filter(|surface| {
                self.space.elements().any(|window| {
                    window
                        .wl_surface()
                        .is_some_and(|mapped| mapped.id() == surface.id())
                })
            })
            .and_then(|surface| self.surface_windows.get(&surface.id()).copied())
            .map(|id| nickel_session_protocol::WindowId(id.0));
        let mut text_input_active = false;
        self.seat
            .text_input()
            .with_active_text_input(|_, _| text_input_active = true);
        OnScreenKeyboardSnapshot {
            height: self.on_screen_keyboard.height,
            controller_barrier_unix_ms: self.on_screen_keyboard.controller_barrier_unix_ms,
            dock_top: self.on_screen_keyboard.dock_top,
            auto_show_requested: self.on_screen_keyboard.auto_show_requested
                && text_input_active
                && !self.locked,
            touchscreen_present: !self.on_screen_keyboard.touchscreens.is_empty(),
            generation: self.on_screen_keyboard.generation,
            environment_override: self.on_screen_keyboard.environment_override,
            epoch: self.on_screen_keyboard.epoch,
            recipient: if self.locked { None } else { recipient },
            text_input_active: text_input_active && !self.locked,
            enabled: self.on_screen_keyboard.enabled,
            visible: self.on_screen_keyboard.visible && !self.locked,
        }
    }

    pub(crate) fn configure_on_screen_keyboard(
        &mut self,
        enabled: bool,
        visible: bool,
        generation: u64,
        environment_override: bool,
        dock_top: bool,
        height: u32,
    ) {
        let height = height.clamp(248, 640);
        let layout_changed = self.on_screen_keyboard.height != height
            || self.on_screen_keyboard.dock_top != dock_top
            || self.on_screen_keyboard.visible != (enabled && visible && !self.locked);
        if enabled && visible && !self.locked && !self.on_screen_keyboard.visible {
            self.on_screen_keyboard.output_name = self.preferred_interaction_output_name();
        }
        self.on_screen_keyboard.height = height;
        self.on_screen_keyboard.dock_top = dock_top;
        self.on_screen_keyboard.generation = generation;
        self.on_screen_keyboard.environment_override = environment_override;
        let visible = enabled && visible && !self.locked;
        if visible || self.on_screen_keyboard.visible || !enabled {
            self.on_screen_keyboard.auto_show_requested = false;
        }
        if self.on_screen_keyboard.enabled != enabled || self.on_screen_keyboard.visible != visible
        {
            self.on_screen_keyboard.controller_barrier_unix_ms =
                controller_barrier_now().max(self.on_screen_keyboard.controller_barrier_unix_ms);
            self.on_screen_keyboard_focus_changed();
        }
        self.on_screen_keyboard.enabled = enabled;
        self.on_screen_keyboard.visible = visible;
        self.seat.text_input().set_compositor_input_method(enabled);
        self.set_shell_role_visible(ShellRole::OnScreenKeyboard, visible);
        if layout_changed {
            self.relayout_shell_surfaces();
            self.notify_protocol_snapshot();
            self.request_output_redraw();
        }
    }

    pub(crate) fn deliver_on_screen_keyboard_input(
        &mut self,
        epoch: u64,
        input: OnScreenKeyboardInput,
    ) -> Result<(), &'static str> {
        let snapshot = self.on_screen_keyboard_snapshot();
        if !snapshot.enabled
            || !snapshot.visible
            || snapshot.recipient.is_none()
            || snapshot.epoch != epoch
        {
            return Err("on-screen keyboard recipient is no longer available");
        }
        let keyboard = self
            .seat
            .get_keyboard()
            .ok_or("seat keyboard unavailable")?;
        match input {
            OnScreenKeyboardInput::Text { text } => {
                if text.is_empty()
                    || text.chars().count() > 16
                    || text.chars().any(char::is_control)
                {
                    return Err("invalid on-screen keyboard text");
                }
                let text_input = self.seat.text_input();
                let mut committed = false;
                text_input.with_active_text_input(|input, _| {
                    input.commit_string(Some(text.clone()));
                    committed = true;
                });
                if committed {
                    text_input.done(false);
                } else if matches!(
                    keyboard.current_focus(),
                    Some(crate::focus::KeyboardFocusTarget::X11(_))
                ) {
                    // Xwayland does not apply the throwaway keymap used by
                    // inject_text_keysyms: its spare code 9 arrives as Escape.
                    // Resolve real codes and modifier levels in the existing map.
                    let plan = keyboard.with_xkb_state(self, |context| {
                        let xkb = context.xkb().lock().unwrap();
                        // SAFETY: the borrowed keymap and the temporary state made
                        // by text_key_plan are dropped before this Xkb guard. Only
                        // owned keycodes and modifier values leave the closure.
                        text_key_plan(unsafe { xkb.keymap() }, xkb.active_layout().0, &text)
                    })?;
                    let focus = keyboard
                        .current_focus()
                        .ok_or("seat recipient unavailable")?;
                    let seat = self.seat.clone();
                    let original_modifiers = keyboard.modifier_state();
                    for (code, modifiers) in plan {
                        // Override the recipient's interpretation, not the shared
                        // seat state: physically held modifiers remain held.
                        focus.modifiers(&seat, self, modifiers, SERIAL_COUNTER.next_serial());
                        for state in [KeyState::Pressed, KeyState::Released] {
                            keyboard.input_from_source::<(), _>(
                                self.on_screen_keyboard.source,
                                self,
                                code,
                                state,
                                SERIAL_COUNTER.next_serial(),
                                InputTime::now(),
                                |_, _, _| FilterResult::Forward,
                            );
                        }
                    }
                    focus.modifiers(
                        &seat,
                        self,
                        original_modifiers,
                        SERIAL_COUNTER.next_serial(),
                    );
                } else {
                    let symbols: Vec<_> = text.chars().map(Keysym::from_char).collect();
                    keyboard.inject_text_keysyms(self, &symbols);
                }
            }
            OnScreenKeyboardInput::Key { keysym, modifiers } => {
                if keysym == 0xff1b {
                    tracing::warn!(
                        epoch,
                        "diagnostic: delivering Escape from the on-screen keyboard"
                    );
                }
                // Only keyboard modifiers are accepted as a chord prefix.
                if modifiers.len() > 5
                    || modifiers
                        .iter()
                        .any(|key| !matches!(*key, 0xffe1 | 0xffe3 | 0xffe9 | 0xffeb | 0xfe03))
                {
                    return Err("invalid on-screen keyboard modifiers");
                }
                let mut keys = Vec::with_capacity(modifiers.len() + 1);
                for symbol in modifiers.into_iter().chain(std::iter::once(keysym)) {
                    let code = keyboard
                        .keycode_for_keysym(Keysym::new(symbol))
                        .ok_or("key unavailable in the current keyboard layout")?;
                    if !keys.contains(&code) {
                        keys.push(code);
                    }
                }
                let source = self.on_screen_keyboard.source;
                for code in &keys {
                    keyboard.input_from_source::<(), _>(
                        source,
                        self,
                        *code,
                        KeyState::Pressed,
                        SERIAL_COUNTER.next_serial(),
                        InputTime::now(),
                        |_, _, _| FilterResult::Forward,
                    );
                }
                for code in keys.into_iter().rev() {
                    keyboard.input_from_source::<(), _>(
                        source,
                        self,
                        code,
                        KeyState::Released,
                        SERIAL_COUNTER.next_serial(),
                        InputTime::now(),
                        |_, _, _| FilterResult::Forward,
                    );
                }
            }
        }
        self.note_input_activity();
        Ok(())
    }
}

/// Resolve the complete string before delivering anything, so unsupported text
/// fails without partially typing or changing the recipient's modifiers.
fn text_key_plan(
    keymap: &xkb::Keymap,
    layout: u32,
    text: &str,
) -> Result<Vec<(Keycode, ModifiersState)>, &'static str> {
    let mut state = xkb::State::new(keymap);
    text.chars()
        .map(|character| {
            let symbol = Keysym::from_char(character);
            for code in keymap.min_keycode().raw()..=keymap.max_keycode().raw() {
                let code = Keycode::new(code);
                for level in 0..keymap.num_levels_for_key(code, layout) {
                    if !keymap
                        .key_get_syms_by_level(code, layout, level)
                        .contains(&symbol)
                    {
                        continue;
                    }
                    let mut masks = [0; 32];
                    let count = keymap.key_get_mods_for_level(code, layout, level, &mut masks);
                    masks[..count].sort_by_key(|mask| (mask.count_ones(), *mask));
                    for &mask in &masks[..count] {
                        state.update_mask(mask, 0, 0, 0, 0, layout);
                        if state.key_get_one_sym(code) == symbol {
                            let mut modifiers = ModifiersState::default();
                            modifiers.update_with(&state);
                            return Ok((code, modifiers));
                        }
                    }
                }
            }
            Err("text unavailable in the current X11 keyboard layout")
        })
        .collect()
}

fn controller_barrier_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn verify_layout(layout: &str, text: &str) {
        let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
        let keymap = xkb::Keymap::new_from_names(
            &context,
            "",
            "",
            layout,
            "",
            None,
            xkb::KEYMAP_COMPILE_NO_FLAGS,
        )
        .unwrap();
        let plan = text_key_plan(&keymap, 0, text).unwrap();
        let mut state = xkb::State::new(&keymap);
        for ((code, modifiers), character) in plan.into_iter().zip(text.chars()) {
            let mods = modifiers.serialized;
            state.update_mask(
                mods.depressed,
                mods.latched,
                mods.locked,
                0,
                0,
                mods.layout_effective,
            );
            assert_eq!(state.key_get_one_sym(code), Keysym::from_char(character));
            assert_ne!(code.raw(), 9, "text must not reuse the Escape keycode");
        }
        assert!(text_key_plan(&keymap, 0, "h🦀").is_err());
    }

    #[test]
    fn x11_text_uses_real_us_letter_and_punctuation_levels() {
        verify_layout(
            "us",
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789`~!@#$%^&*()-_=+[]{}\\|;:'\",.<>/? ",
        );
    }

    #[test]
    fn x11_text_resolves_altgr_and_non_us_letter_positions() {
        verify_layout("de", "hHzZyY@€äÄöÖß");
    }
}
