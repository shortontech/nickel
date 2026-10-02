use super::NickelSession;
pub(super) use crate::appearance_service::{AppearanceState, PreparedChange, PreparedRead};
use crate::appearance_service::{UNAVAILABLE, preferences};
use nickel_remote_control::appearance::{Snapshot, Transaction};

impl NickelSession {
    pub(super) fn remote_read_appearance(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.current()?;
        permit.with_debug(self.locked || self.shell_recovery_visible(), || {
            self.remote_appearance.observe(
                prepared.revision,
                preferences(&prepared.settings),
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            )
        })
    }
    pub(super) fn remote_change_appearance(
        &mut self,
        permit: &nickel_remote_control::DesktopPermit,
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
        let authorization = permit.with_debug_input_deadline(protected, |deadline| {
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
            committed = Some(self.remote_appearance.commit(
                prepared,
                &transaction,
                deadline.deadline(),
                || permit.check_commit_boundary(deadline),
            )?);
            Ok(())
        });
        let (requested, revision) = match committed {
            Some(committed) => committed,
            None => {
                authorization?;
                return Err(UNAVAILABLE.into());
            }
        };
        // Even if emergency stop races an already accepted rename, reconcile
        // that committed configuration before returning the cancelled outcome.
        let configured = preferences(&requested);
        let mut changed = self
            .internal_shell
            .as_mut()
            .map(|shell| shell.apply_prepared_shell_settings(requested))
            .unwrap_or_default();
        let theme = self
            .internal_shell
            .as_ref()
            .map(crate::internal_shell::InternalShellCoordinator::semantic_theme);
        if let Some(theme) = theme {
            self.recovery_ui.set_theme(theme);
        }
        if let (Some(theme), Some(mut codex)) = (theme, self.internal_codex.take()) {
            changed.extend(codex.set_theme(&mut self.internal_ui, theme));
            self.internal_codex = Some(codex);
        }
        changed.sort_unstable();
        changed.dedup();
        if !changed.is_empty() {
            self.sync_internal_shell_changes(Some(&changed));
        }
        self.sync_internal_window_decorations();
        self.notify_shell_settings_changed();
        self.schedule_internal_shell_deadline();
        authorization?;
        self.remote_appearance.observe(
            revision?,
            configured,
            self.start_time
                .elapsed()
                .as_micros()
                .min(u128::from(u64::MAX)) as u64,
        )
    }
}
