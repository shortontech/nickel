//! Codex enablement is persisted off-thread and applied by the compositor owner.
use super::NickelSession;
use nickel_core::optional_features::{
    CodexAvailabilityProjection, CodexSource, FeatureHealth, FeatureInstallation, FeaturePolicy,
    FeatureSupport, OptionalFeatureSettings, PreparedCodexPreference, codex_policy, settings_path,
};
use nickel_remote_control::{
    DesktopPermit,
    codex_preference::{Policy, Snapshot, Transaction},
};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{io, path::PathBuf};

const STALE: &str = "Codex preference changed; read current state before retrying";
const UNAVAILABLE: &str = "Codex preference unavailable; read current state before retrying";

fn policy(value: FeaturePolicy) -> Policy {
    match value {
        FeaturePolicy::Editable => Policy::Editable,
        FeaturePolicy::ForceEnabled => Policy::ForceEnabled,
        FeaturePolicy::ForceDisabled => Policy::ForceDisabled,
    }
}

pub(super) struct PreparedRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: OptionalFeatureSettings,
    policy: FeaturePolicy,
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
            policy: codex_policy().0,
        })
    }

    fn current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision
            || codex_policy().0 != self.policy
        {
            return Err(STALE.into());
        }
        Ok(())
    }
}

pub(super) struct PreparedChange {
    prior: PreparedRead,
    staged: PreparedCodexPreference,
    requested_enabled: bool,
}

impl PreparedChange {
    pub(super) fn prepare(transaction: &Transaction) -> Result<Self, String> {
        Self::prepare_at(settings_path().map_err(|_| UNAVAILABLE)?, transaction)
    }

    fn prepare_at(path: PathBuf, transaction: &Transaction) -> Result<Self, String> {
        let prior = PreparedRead::at(path)?;
        if prior.policy != FeaturePolicy::Editable
            || transaction.generation != prior.settings.codex_generation
            || transaction.prior_enabled != prior.settings.codex_enabled
            || transaction.prior_enabled == transaction.requested_enabled
            || prior.settings.codex_generation == u64::MAX
        {
            return Err(STALE.into());
        }
        let staged = PreparedCodexPreference::prepare(
            prior.path.clone(),
            &prior.settings,
            transaction.requested_enabled,
        )
        .map_err(|_| STALE)?;
        Ok(Self {
            prior,
            staged,
            requested_enabled: transaction.requested_enabled,
        })
    }
}

fn snapshot(session: &NickelSession, read: &PreparedRead, observed_at_us: u64) -> Snapshot {
    let runtime_enabled = session.internal_codex.is_some();
    let runtime_generation = session.remote_codex_runtime_generation;
    Snapshot {
        generation: read.settings.codex_generation,
        observed_at_us,
        configured_enabled: read.settings.codex_enabled,
        effective_enabled: read.settings.effective_codex_enabled(),
        policy: policy(read.policy),
        custom_executable_configured: matches!(
            read.settings.codex_source,
            CodexSource::Executable(_)
        ),
        runtime_generation,
        runtime_enabled,
        active_chat_windows: session.internal_codex.as_ref().map_or(
            0,
            crate::internal_codex::InternalCodexHost::active_chat_count,
        ),
        pending: runtime_generation != read.settings.codex_generation
            || runtime_enabled != read.settings.effective_codex_enabled(),
    }
}

impl NickelSession {
    fn apply_remote_codex_preference(
        &mut self,
        settings: &OptionalFeatureSettings,
    ) -> Result<(), String> {
        let enabled = settings.effective_codex_enabled();
        if !enabled {
            if let Some(mut host) = self.internal_codex.take() {
                host.shutdown(&mut self.internal_ui);
            }
            if let Some(shell) = self.internal_shell.as_mut() {
                shell.apply_codex_projection(CodexAvailabilityProjection::new(
                    FeatureSupport::Supported,
                    FeatureInstallation::Installed,
                    false,
                    FeatureHealth::Unknown,
                    settings.codex_generation,
                    Some("Codex integration is disabled".into()),
                ));
            }
            self.remote_codex_runtime_generation = settings.codex_generation;
            self.sync_internal_shell_changes(None);
            self.request_output_redraw();
            return Ok(());
        }
        if self.internal_codex.is_none() {
            let shell = self
                .internal_shell
                .as_ref()
                .ok_or("internal shell unavailable")?;
            let mut host = crate::internal_codex::InternalCodexHost::new(
                settings.clone(),
                shell.semantic_theme(),
                std::env::current_dir().unwrap_or_else(|_| PathBuf::from("/")),
            );
            let outputs = self.internal_outputs();
            let fallback =
                self.resolve_interaction_output(super::InvocationSource::RecentInteraction);
            let placement =
                super::internal_codex_project_menu_placement(None, &outputs, fallback.as_deref());
            let menu = host.ensure_project_menu(&mut self.internal_ui, placement)?;
            host.set_project_menu_visible(&mut self.internal_ui, false);
            if let Some(shell) = self.internal_shell.as_mut() {
                shell.apply_codex_projection(CodexAvailabilityProjection::new(
                    FeatureSupport::Supported,
                    FeatureInstallation::Installed,
                    true,
                    FeatureHealth::Loading,
                    settings.codex_generation,
                    Some("Checking the selected Codex backend…".into()),
                ));
                host.sync_shell_projection(&self.internal_ui, shell);
            }
            self.internal_codex = Some(host);
            self.internal_ui.mark_dirty(menu);
        }
        self.remote_codex_runtime_generation = settings.codex_generation;
        self.sync_internal_shell_changes(None);
        self.schedule_internal_shell_deadline();
        self.request_output_redraw();
        Ok(())
    }

    pub(super) fn remote_read_codex_preference(
        &mut self,
        permit: &DesktopPermit,
        prepared: PreparedRead,
    ) -> Result<Snapshot, String> {
        permit.with_debug(false, || Ok(()))?;
        prepared.current()?;
        permit.with_debug(self.locked || self.shell_recovery_visible(), || {
            Ok(snapshot(
                self,
                &prepared,
                self.start_time
                    .elapsed()
                    .as_micros()
                    .min(u128::from(u64::MAX)) as u64,
            ))
        })
    }

    pub(super) fn remote_change_codex_preference(
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
            if !prepared.requested_enabled
                && self
                    .internal_codex
                    .as_ref()
                    .is_some_and(|host| host.active_chat_count() > 0)
            {
                return Err("close active Codex chat windows before disabling Codex".into());
            }
            if codex_policy().0 != FeaturePolicy::Editable
                || transaction.generation != prepared.prior.settings.codex_generation
                || transaction.prior_enabled != prepared.prior.settings.codex_enabled
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
                authorization?;
                return Err(UNAVAILABLE.into());
            }
        };
        let applied = self.apply_remote_codex_preference(&settings);
        self.notify_shell_settings_changed();
        authorization?;
        applied?;
        let read = PreparedRead::at(prepared.prior.path)?;
        Ok(snapshot(
            self,
            &read,
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
    fn preference_transaction_preserves_source_keyboard_and_rejects_aba() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("optional-features");
        let settings = OptionalFeatureSettings {
            codex_enabled: true,
            codex_generation: 7,
            codex_source: CodexSource::Executable("/private/codex".into()),
            on_screen_keyboard_generation: 11,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let transaction = Transaction {
            generation: 7,
            prior_enabled: true,
            requested_enabled: false,
        };
        let prepared = PreparedChange::prepare_at(path.clone(), &transaction).unwrap();
        std::fs::write(
            &path,
            "version=1\ncodex.enabled=true\ncodex.generation=7\ncodex.source=executable:/private/codex\non_screen_keyboard.preference=automatic\non_screen_keyboard.generation=12\n",
        )
        .unwrap();
        assert!(prepared.staged.commit(|| Ok(())).is_err());
        settings.save(&path).unwrap();

        let prepared = PreparedChange::prepare_at(path.clone(), &transaction).unwrap();
        let accepted = prepared.staged.commit(|| Ok(())).unwrap();
        assert!(!accepted.codex_enabled);
        assert_eq!(accepted.codex_source, settings.codex_source);
        assert_eq!(accepted.on_screen_keyboard_generation, 11);
    }
}
