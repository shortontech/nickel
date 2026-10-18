//! Bounded Codex preference staging for the Windows winit owner.

use nickel_core::optional_features::{
    CodexSource, FeaturePolicy, OptionalFeatureSettings, PreparedCodexPreference, codex_policy,
    settings_path,
};
use nickel_remote_control::codex_preference::{Policy, Snapshot, Transaction};
use nickel_storage::{RegularFileRevision, regular_file_revision};
use std::{io, path::PathBuf, time::Instant};

const STALE: &str = "Codex preference changed; read current state before retrying";
const UNAVAILABLE: &str = "Codex preference unavailable; read current state before retrying";

fn policy(value: FeaturePolicy) -> Policy {
    match value {
        FeaturePolicy::Editable => Policy::Editable,
        FeaturePolicy::ForceEnabled => Policy::ForceEnabled,
        FeaturePolicy::ForceDisabled => Policy::ForceDisabled,
    }
}

pub(crate) struct RuntimeState {
    pub(crate) generation: u64,
    pub(crate) enabled: bool,
    pub(crate) active_chat_windows: u32,
}

pub(crate) struct PreparedRead {
    path: PathBuf,
    revision: Option<RegularFileRevision>,
    settings: OptionalFeatureSettings,
    policy: FeaturePolicy,
}

impl PreparedRead {
    pub(crate) fn prepare() -> Result<Self, String> {
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

    pub(crate) fn ensure_current(&self) -> Result<(), String> {
        if regular_file_revision(&self.path).map_err(|_| UNAVAILABLE)? != self.revision
            || codex_policy().0 != self.policy
        {
            return Err(STALE.into());
        }
        Ok(())
    }

    pub(crate) fn snapshot(&self, runtime: RuntimeState, observed_at_us: u64) -> Snapshot {
        let effective_enabled = self.settings.effective_codex_enabled();
        Snapshot {
            generation: self.settings.codex_generation,
            observed_at_us,
            configured_enabled: self.settings.codex_enabled,
            effective_enabled,
            policy: policy(self.policy),
            custom_executable_configured: matches!(
                self.settings.codex_source,
                CodexSource::Executable(_)
            ),
            runtime_generation: runtime.generation,
            runtime_enabled: runtime.enabled,
            active_chat_windows: runtime.active_chat_windows,
            pending: runtime.generation != self.settings.codex_generation
                || runtime.enabled != effective_enabled,
        }
    }
}

pub(crate) struct PreparedChange {
    prior: PreparedRead,
    staged: PreparedCodexPreference,
    requested_enabled: bool,
}

impl PreparedChange {
    pub(crate) fn prepare(transaction: &Transaction) -> Result<Self, String> {
        let prior = PreparedRead::prepare()?;
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

    pub(crate) fn requested_enabled(&self) -> bool {
        self.requested_enabled
    }

    pub(crate) fn ensure_current(&self, transaction: &Transaction) -> Result<(), String> {
        self.prior.ensure_current()?;
        if self.prior.policy != FeaturePolicy::Editable
            || transaction.generation != self.prior.settings.codex_generation
            || transaction.prior_enabled != self.prior.settings.codex_enabled
            || transaction.requested_enabled != self.requested_enabled
        {
            return Err(STALE.into());
        }
        Ok(())
    }

    pub(crate) fn commit(
        self,
        deadline: Instant,
        check_boundary: impl FnOnce() -> Result<(), String>,
    ) -> Result<OptionalFeatureSettings, String> {
        self.staged
            .commit(|| {
                if codex_policy().0 != FeaturePolicy::Editable {
                    return Err(io::Error::new(io::ErrorKind::PermissionDenied, STALE));
                }
                if Instant::now() >= deadline {
                    return Err(io::Error::new(
                        io::ErrorKind::TimedOut,
                        "Codex preference commit expired",
                    ));
                }
                check_boundary().map_err(io::Error::other)
            })
            .map_err(|error| {
                if matches!(
                    error.kind(),
                    io::ErrorKind::InvalidData | io::ErrorKind::PermissionDenied
                ) {
                    STALE
                } else {
                    UNAVAILABLE
                }
                .to_owned()
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_preserves_source_keyboard_fields_and_rejects_aba() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("optional-features");
        let settings = OptionalFeatureSettings {
            codex_enabled: true,
            codex_generation: 7,
            codex_source: CodexSource::Executable("C:/private/codex.exe".into()),
            on_screen_keyboard_generation: 11,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let prior = PreparedRead::at(path.clone()).unwrap();
        let staged =
            PreparedCodexPreference::prepare(path.clone(), &prior.settings, false).unwrap();
        let prepared = PreparedChange {
            prior,
            staged,
            requested_enabled: false,
        };
        std::fs::write(
            &path,
            "version=1\ncodex.enabled=true\ncodex.generation=7\ncodex.source=executable:C:/private/codex.exe\non_screen_keyboard.preference=automatic\non_screen_keyboard.generation=12\n",
        )
        .unwrap();
        assert!(
            prepared
                .commit(
                    Instant::now() + std::time::Duration::from_secs(1),
                    || Ok(())
                )
                .is_err()
        );
        settings.save(&path).unwrap();

        let prior = PreparedRead::at(path.clone()).unwrap();
        let staged = PreparedCodexPreference::prepare(path, &prior.settings, false).unwrap();
        let accepted = PreparedChange {
            prior,
            staged,
            requested_enabled: false,
        }
        .commit(
            Instant::now() + std::time::Duration::from_secs(1),
            || Ok(()),
        )
        .unwrap();
        assert!(!accepted.codex_enabled);
        assert_eq!(accepted.codex_source, settings.codex_source);
        assert_eq!(accepted.on_screen_keyboard_generation, 11);
    }

    #[test]
    fn snapshot_discloses_only_custom_source_presence_and_truthful_pending_state() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("optional-features");
        OptionalFeatureSettings {
            codex_enabled: true,
            codex_generation: 9,
            codex_source: CodexSource::Executable("C:/secret/account/codex.exe".into()),
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        let read = PreparedRead::at(path).unwrap();
        let snapshot = read.snapshot(
            RuntimeState {
                generation: 8,
                enabled: false,
                active_chat_windows: 2,
            },
            77,
        );
        assert!(snapshot.custom_executable_configured);
        assert!(snapshot.pending);
        let diagnostic = format!("{snapshot:?}");
        assert!(!diagnostic.contains("secret"));
        assert!(!diagnostic.contains("account"));
        assert!(!diagnostic.contains("codex.exe"));
    }
}
