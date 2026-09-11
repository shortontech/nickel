//! Terminal presentation settings are staged off-thread and committed by the desktop owner.
use super::NickelSession;
use nickel_core::terminal_settings::{
    PreparedTerminalSettings, TerminalCursorStyle, TerminalSettings, settings_path,
};
use nickel_remote_control::{
    DesktopPermit,
    terminal_presentation::{CursorStyle, Preferences, Snapshot, Transaction},
};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{io, path::PathBuf};

const STALE: &str = "terminal presentation changed; read current state before retrying";
const UNAVAILABLE: &str = "terminal presentation unavailable; read current state before retrying";

fn cursor(value: TerminalCursorStyle) -> CursorStyle {
    match value {
        TerminalCursorStyle::Block => CursorStyle::Block,
        TerminalCursorStyle::Beam => CursorStyle::Beam,
        TerminalCursorStyle::Underline => CursorStyle::Underline,
    }
}

fn core_cursor(value: CursorStyle) -> TerminalCursorStyle {
    match value {
        CursorStyle::Block => TerminalCursorStyle::Block,
        CursorStyle::Beam => TerminalCursorStyle::Beam,
        CursorStyle::Underline => TerminalCursorStyle::Underline,
    }
}

fn preferences(settings: &TerminalSettings) -> Preferences {
    Preferences {
        font_family: settings.font_family.clone(),
        font_size_tenths: settings.font_size_tenths,
        scrollback_lines: settings.scrollback_lines,
        cursor_style: cursor(settings.cursor_style),
        foreground: settings.foreground,
        background: settings.background,
        close_on_successful_exit: settings.close_on_successful_exit,
    }
}

fn apply(settings: &mut TerminalSettings, requested: &Preferences) -> Result<(), String> {
    if !requested.valid() {
        return Err("terminal presentation value is outside its supported range".into());
    }
    settings.font_family.clone_from(&requested.font_family);
    settings.font_size_tenths = requested.font_size_tenths;
    settings.scrollback_lines = requested.scrollback_lines;
    settings.cursor_style = core_cursor(requested.cursor_style);
    settings.foreground = requested.foreground;
    settings.background = requested.background;
    settings.close_on_successful_exit = requested.close_on_successful_exit;
    Ok(())
}

pub(super) struct PreparedRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: TerminalSettings,
}

impl PreparedRead {
    pub(super) fn prepare() -> Result<Self, String> {
        Self::at(settings_path().map_err(|_| UNAVAILABLE)?)
    }

    fn at(path: PathBuf) -> Result<Self, String> {
        let revision = regular_file_revision(&path).map_err(|_| UNAVAILABLE)?;
        let settings = match TerminalSettings::load(&path) {
            Ok(settings) => settings,
            Err(error) if error.kind() == io::ErrorKind::NotFound && revision.is_none() => {
                TerminalSettings::default()
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
    staged: PreparedTerminalSettings,
}

impl PreparedChange {
    pub(super) fn prepare(transaction: &Transaction) -> Result<Self, String> {
        Self::from_read(PreparedRead::prepare()?, transaction)
    }

    fn from_read(prior: PreparedRead, transaction: &Transaction) -> Result<Self, String> {
        if transaction.generation == 0
            || !transaction.prior.valid()
            || preferences(&prior.settings) != transaction.prior
        {
            return Err(STALE.into());
        }
        let mut requested = prior.settings.clone();
        apply(&mut requested, &transaction.requested)?;
        let staged =
            PreparedTerminalSettings::prepare(prior.path.clone(), &prior.settings, requested)
                .map_err(|_| STALE)?;
        Ok(Self { prior, staged })
    }
}

#[derive(Default)]
pub(super) struct TerminalPresentationState {
    generation: u64,
    observed: Option<(Option<RegularFileRevision>, Preferences)>,
}

impl TerminalPresentationState {
    fn observe(&mut self, read: &PreparedRead, observed_at_us: u64) -> Result<Snapshot, String> {
        let configured = preferences(&read.settings);
        if self
            .observed
            .as_ref()
            .is_none_or(|value| value != &(read.revision.clone(), configured.clone()))
        {
            self.generation = self
                .generation
                .checked_add(1)
                .ok_or("terminal presentation generation exhausted")?;
            self.observed = Some((read.revision.clone(), configured.clone()));
        }
        Ok(Snapshot {
            generation: self.generation,
            observed_at_us,
            configured,
            custom_shell_configured: read.settings.default_shell.is_some(),
            initial_directory_configured: read.settings.initial_working_directory.is_some(),
            applies_to_new_terminals: true,
        })
    }

    fn validate(&self, prepared: &PreparedChange, transaction: &Transaction) -> Result<(), String> {
        if self.generation == u64::MAX
            || self.generation != transaction.generation
            || self.observed.as_ref().is_none_or(|(revision, configured)| {
                revision != &prepared.prior.revision || configured != &transaction.prior
            })
        {
            return Err(STALE.into());
        }
        Ok(())
    }
}

impl NickelSession {
    pub(super) fn remote_read_terminal_presentation(
        &mut self,
        permit: &DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.current()?;
        permit.with_debug(self.locked || self.shell_recovery_visible(), || {
            let observed_at_us = self
                .start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64;
            self.remote_terminal_presentation
                .observe(&prepared, observed_at_us)
        })
    }

    pub(super) fn remote_change_terminal_presentation(
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
        let path = prepared.prior.path.clone();
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
            self.remote_terminal_presentation
                .validate(&prepared, &transaction)?;
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
            self.remote_terminal_presentation.observed = None;
            Ok(())
        });
        let requested = match committed {
            Some(requested) => requested,
            None => {
                authorized?;
                return Err(UNAVAILABLE.into());
            }
        };
        authorized?;
        let read = PreparedRead::at(path)?;
        if read.settings != requested {
            return Err(STALE.into());
        }
        let observed_at_us = self
            .start_time
            .elapsed()
            .as_micros()
            .min(u128::from(u64::MAX)) as u64;
        self.remote_terminal_presentation
            .observe(&read, observed_at_us)
    }
}

impl NickelSession {
    pub(super) fn remote_read_terminal_launch_policy(
        &mut self,
        permit: &DesktopPermit,
        prepared: crate::remote_terminal_launch_policy::PreparedRead,
    ) -> Result<nickel_remote_control::terminal_launch_policy::Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.ensure_current(std::time::Instant::now())?;
        let protected = self.locked
            || self.shell_recovery_visible()
            || self
                .internal_ui
                .focused()
                .is_some_and(|surface| self.internal_ui.remote_access_protected(surface));
        permit.with_debug(protected, || {
            self.remote_terminal_launch_policy.observe(
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            )
        })
    }

    pub(super) fn remote_change_terminal_launch_policy(
        &mut self,
        permit: &DesktopPermit,
        transaction: nickel_remote_control::terminal_launch_policy::Transaction,
        prepared: crate::remote_terminal_launch_policy::PreparedChange,
    ) -> Result<nickel_remote_control::terminal_launch_policy::TransactionOutcome, String> {
        let controller_busy = self.poll_remote_controller_ownership();
        let protected = self.locked
            || self.shell_recovery_visible()
            || self
                .internal_ui
                .focused()
                .is_some_and(|surface| self.internal_ui.remote_access_protected(surface));
        let mut committed = None;
        let authorization = permit.with_debug_input_deadline(protected, |boundary| {
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
            self.remote_terminal_launch_policy.validate(
                &prepared,
                &transaction,
                std::time::Instant::now(),
            )?;
            committed = Some(prepared.commit(boundary.deadline(), || {
                if self.remote_held_keyboard.is_some()
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
                permit.check_commit_boundary(boundary)
            })?);
            Ok(())
        });
        let committed = match committed {
            Some(committed) => committed,
            None => {
                authorization?;
                return Err(
                    "terminal launch policy unavailable; read current state before retrying".into(),
                );
            }
        };
        let snapshot = self.remote_terminal_launch_policy.observe_committed(
            &committed,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        )?;
        authorization?;
        Ok(crate::remote_terminal_launch_policy::outcome(snapshot))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_change_preserves_launch_policy_and_rejects_aba() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("terminal-settings");
        let settings = TerminalSettings {
            default_shell: Some("/private/shell".into()),
            initial_working_directory: Some("/private/directory".into()),
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let read = PreparedRead::at(path.clone()).unwrap();
        let prior = preferences(&settings);
        let mut requested = prior.clone();
        requested.font_family = "Iosevka".into();
        requested.cursor_style = CursorStyle::Beam;
        let transaction = Transaction {
            generation: 1,
            prior,
            requested,
        };
        let mut state = TerminalPresentationState::default();
        assert_eq!(state.observe(&read, 1).unwrap().generation, 1);
        let prepared = PreparedChange::from_read(read, &transaction).unwrap();
        state.validate(&prepared, &transaction).unwrap();
        std::fs::write(&path, "font_family=external\nfont_size_tenths=140\n").unwrap();
        assert!(prepared.staged.commit(|| Ok(())).is_err());
        assert_eq!(
            TerminalSettings::load(&path).unwrap().font_family,
            "external"
        );
    }
}
