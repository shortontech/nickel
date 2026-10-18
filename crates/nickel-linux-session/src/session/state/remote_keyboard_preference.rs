//! OSK preference persistence is separate from controller-owned runtime layout and input.
use super::NickelSession;
use nickel_core::{
    on_screen_keyboard::KeyboardPreference,
    optional_features::{OptionalFeatureSettings, PreparedKeyboardPreference, settings_path},
};
use nickel_remote_control::{
    DesktopPermit,
    keyboard_preference::{Preference, Snapshot, Transaction},
};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{io, path::PathBuf};

const STALE: &str = "keyboard preference changed; read current state before retrying";
const UNAVAILABLE: &str = "keyboard preference unavailable; read current state before retrying";

fn preference(value: KeyboardPreference) -> Preference {
    match value {
        KeyboardPreference::Automatic => Preference::Automatic,
        KeyboardPreference::Enabled => Preference::Enabled,
        KeyboardPreference::Disabled => Preference::Disabled,
    }
}

fn core_preference(value: Preference) -> KeyboardPreference {
    match value {
        Preference::Automatic => KeyboardPreference::Automatic,
        Preference::Enabled => KeyboardPreference::Enabled,
        Preference::Disabled => KeyboardPreference::Disabled,
    }
}

pub(super) struct PreparedRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: OptionalFeatureSettings,
}

impl PreparedRead {
    pub(super) fn prepare() -> Result<Self, String> {
        Self::at(settings_path().map_err(|_| UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let revision = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = match OptionalFeatureSettings::load(&path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                OptionalFeatureSettings::default()
            }
            Err(_) => return Err(UNAVAILABLE.into()),
        };
        if regular_file_revision(&path).map_err(|_| UNAVAILABLE)? != revision {
            return Err(STALE.into());
        }
        Ok(Self {
            path,
            revision,
            settings,
        })
    }

    fn current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision {
            return Err(STALE.into());
        }
        Ok(())
    }
}

pub(super) struct PreparedChange {
    prior: PreparedRead,
    staged: PreparedKeyboardPreference,
}

impl PreparedChange {
    pub(super) fn prepare(transaction: &Transaction) -> Result<Self, String> {
        let prior = PreparedRead::prepare()?;
        if transaction.generation != prior.settings.on_screen_keyboard_generation
            || transaction.prior != preference(prior.settings.on_screen_keyboard)
            || prior.settings.on_screen_keyboard_generation == u64::MAX
        {
            return Err(STALE.into());
        }
        let staged = PreparedKeyboardPreference::prepare(
            prior.path.clone(),
            &prior.settings,
            core_preference(transaction.requested),
        )
        .map_err(|_| STALE)?;
        Ok(Self { prior, staged })
    }
}

fn snapshot(
    settings: &OptionalFeatureSettings,
    runtime: nickel_session_protocol::OnScreenKeyboardSnapshot,
    observed_at_us: u64,
) -> Snapshot {
    Snapshot {
        generation: settings.on_screen_keyboard_generation,
        observed_at_us,
        configured: preference(settings.on_screen_keyboard),
        runtime_generation: runtime.generation,
        runtime_enabled: runtime.enabled,
        touchscreen_present: runtime.touchscreen_present,
        environment_override: runtime.environment_override,
        pending: runtime.generation != settings.on_screen_keyboard_generation,
    }
}

impl NickelSession {
    pub(super) fn remote_read_keyboard_preference(
        &mut self,
        permit: &DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.current()?;
        permit.with_debug(self.locked || self.shell_recovery_visible(), || {
            Ok(snapshot(
                &prepared.settings,
                self.on_screen_keyboard_snapshot(),
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            ))
        })
    }

    pub(super) fn remote_change_keyboard_preference(
        &mut self,
        permit: &DesktopPermit,
        transaction: Transaction,
        prepared: PreparedChange,
    ) -> Result<Snapshot, String> {
        let controller_busy = self.poll_remote_controller_ownership();
        let protected = self.locked
            || self.shell_recovery_visible()
            || self
                .internal_ui
                .focused()
                .is_some_and(|surface| self.internal_ui.remote_access_protected(surface));
        let mut committed = None;
        let authorized = permit.with_debug_input_deadline(protected, |boundary| {
            if controller_busy
                || self.remote_held_keyboard.is_some()
                || self.remote_held_pointer.is_some()
                || !self.active_touch_slots.is_empty()
                || self.internal_ui.pointer_interaction_active()
                || self.internal_ui.desktop_keyboard_interaction_active()
                || self.seat.get_keyboard().is_some_and(|keyboard| {
                    !keyboard.pressed_keys().is_empty() || keyboard.is_grabbed()
                })
                || self
                    .seat
                    .get_pointer()
                    .is_some_and(|pointer| pointer.is_grabbed())
                || self
                    .internal_shell
                    .as_ref()
                    .is_some_and(|shell| shell.pointer_interaction_active())
            {
                return Err("shared input is busy".into());
            }
            if transaction.generation != prepared.prior.settings.on_screen_keyboard_generation
                || transaction.prior != preference(prepared.prior.settings.on_screen_keyboard)
            {
                return Err(STALE.into());
            }
            committed = Some(
                prepared
                    .staged
                    .commit(|| {
                        permit
                            .check_commit_boundary(boundary)
                            .map_err(io::Error::other)
                    })
                    .map_err(|_| UNAVAILABLE)?,
            );
            Ok(())
        });
        let settings = match committed {
            Some(settings) => settings,
            None => {
                authorized?;
                return Err(UNAVAILABLE.into());
            }
        };
        // An accepted preference is reconciled even if reply authority raced it.
        self.notify_shell_settings_changed();
        authorized?;
        Ok(snapshot(
            &settings,
            self.on_screen_keyboard_snapshot(),
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn projection_reports_pending_without_recipient_or_layout_state() {
        let settings = OptionalFeatureSettings {
            on_screen_keyboard: KeyboardPreference::Enabled,
            on_screen_keyboard_generation: 4,
            ..Default::default()
        };
        let runtime = nickel_session_protocol::OnScreenKeyboardSnapshot {
            generation: 3,
            enabled: false,
            touchscreen_present: true,
            environment_override: true,
            visible: true,
            recipient: Some(nickel_session_protocol::WindowId(99)),
            text_input_active: true,
            ..Default::default()
        };
        let result = snapshot(&settings, runtime, 7);
        assert_eq!(result.configured, Preference::Enabled);
        assert_eq!(result.runtime_generation, 3);
        assert!(result.pending && result.touchscreen_present && result.environment_override);
    }
}
