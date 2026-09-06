//! Trusted shell input, delivered to a leased seat recipient without taking focus.

use nickel_session_protocol::{OnScreenKeyboardInput, OnScreenKeyboardSnapshot, ShellRole};
use smithay::{
    backend::input::{InputTime, KeyState},
    desktop::Window,
    input::keyboard::{FilterResult, KeyboardSource, Keysym},
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
                } else {
                    let symbols: Vec<_> = text.chars().map(Keysym::from_char).collect();
                    keyboard.inject_text_keysyms(self, &symbols);
                }
            }
            OnScreenKeyboardInput::Key { keysym, modifiers } => {
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

fn controller_barrier_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}
