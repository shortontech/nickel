use super::NickelSession;
pub(super) use crate::wallpaper_service::{PreparedChange, PreparedRead, WallpaperState};
use crate::wallpaper_service::{STALE, UNAVAILABLE};
use nickel_remote_control::{
    DesktopPermit,
    wallpaper::{Change, Snapshot, Transaction},
};
use std::io;

impl NickelSession {
    pub(super) fn remote_read_wallpaper(
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
            self.remote_wallpaper.observe(&prepared, observed_at_us)
        })
    }

    pub(super) fn remote_change_wallpaper(
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
            committed = Some(self.remote_wallpaper.commit(prepared, &transaction, || {
                permit
                    .check_commit_boundary(boundary)
                    .map_err(io::Error::other)
            })?);
            Ok(())
        });
        let requested = match committed {
            Some(requested) => requested,
            None => {
                authorized?;
                return Err(UNAVAILABLE.into());
            }
        };
        // Reconcile an accepted rename even if response authorization raced it.
        self.notify_shell_settings_changed();
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
        let mut snapshot = self.remote_wallpaper.observe(&read, observed_at_us)?;
        snapshot.selected_image_decoded =
            matches!(transaction.change, Change::SelectApprovedImage { .. });
        snapshot.runtime_reload_requested = true;
        Ok(snapshot)
    }
}
