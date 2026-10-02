//! Reusable optional feature settings; observation and checked native persistence only.
use nickel_core::{
    on_screen_keyboard::KeyboardPreference,
    optional_features::{
        self, ApplyRequirement, FeatureCapability, FeaturePolicy, FeatureState,
        OptionalFeatureRuntime, OptionalFeatureSettings,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq)]
pub struct FeatureEffect {
    operation: String,
    revision: String,
    mode: Option<KeyboardPreference>,
    enabled: Option<bool>,
    confirmed: bool,
}
impl FeatureEffect {
    pub(crate) fn parse(value: &Value) -> Result<Self, String> {
        let operation = value["type"].as_str().ok_or("invalid feature operation")?;
        if !matches!(
            operation,
            "features.setKeyboardMode" | "features.setCodexEnabled" | "features.retryCodex"
        ) {
            return Err("unsupported feature operation".into());
        }
        let revision = value["revision"]
            .as_str()
            .filter(|revision| revision.len() == 64)
            .ok_or("feature snapshot is unavailable")?;
        let mode = if operation == "features.setKeyboardMode" {
            Some(
                value["mode"]
                    .as_str()
                    .and_then(KeyboardPreference::parse)
                    .ok_or("invalid keyboard mode")?,
            )
        } else {
            None
        };
        let enabled = if operation == "features.setCodexEnabled" {
            Some(
                value["enabled"]
                    .as_bool()
                    .ok_or("invalid Codex enablement")?,
            )
        } else {
            None
        };
        Ok(Self {
            operation: operation.into(),
            revision: revision.into(),
            mode,
            enabled,
            confirmed: value["confirmed"].as_bool().unwrap_or(false),
        })
    }
    pub(crate) fn commits_preference(&self) -> bool {
        self.operation != "features.retryCodex"
    }
    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot["revision"].as_str() != Some(self.revision.as_str()) {
            return Err("feature snapshot is stale".into());
        }
        let operation = self.operation.split_once('.').unwrap().1;
        if snapshot["operations"][operation] != true {
            return Err("feature operation is unavailable or policy controlled".into());
        }
        if self.enabled == Some(false)
            && snapshot["codex"]["disableConfirmationRequired"] == true
            && !self.confirmed
        {
            return Err("disabling Codex with active windows requires confirmation".into());
        }
        Ok(())
    }
}

pub(crate) fn snapshot(
    settings: &OptionalFeatureSettings,
    runtime: &OptionalFeatureRuntime,
    keyboard: Option<&nickel_session_protocol::OnScreenKeyboardSnapshot>,
    override_active: bool,
    policy: FeaturePolicy,
    native: bool,
) -> Value {
    let state = FeatureState::resolve(
        settings.codex_enabled,
        settings.codex_generation,
        runtime.codex_generation,
        FeatureCapability {
            support: runtime.codex_support,
            installation: runtime.codex_installation,
            health: runtime.codex_health,
            policy,
            policy_source: optional_features::codex_policy().1,
            required_permissions: Vec::new(),
            configuration_destination: None,
            apply_requirement: ApplyRequirement::Live,
            source_label: runtime.source_label.clone(),
            diagnostic: runtime.diagnostic.clone(),
        },
    );
    let mut value = json!({"available":true,"operations":{"setKeyboardMode":native&&!override_active,"setCodexEnabled":native&&policy==FeaturePolicy::Editable,"retryCodex":native&&cfg!(target_os="linux")},
        "keyboard":{"mode":settings.on_screen_keyboard.as_str(),"generation":settings.on_screen_keyboard_generation,"environmentOverride":override_active,
            "runtimeAvailable":keyboard.is_some(),"enabled":keyboard.map(|snapshot|snapshot.enabled),"touchscreenPresent":keyboard.map(|snapshot|snapshot.touchscreen_present)},
        "codex":{"configuredEnabled":settings.codex_enabled,"requestedEnabled":state.requested_enabled,"generation":settings.codex_generation,"acknowledgedGeneration":runtime.codex_generation,
            "state":format!("{:?}",state.effective).to_lowercase(),"policy":format!("{policy:?}").to_lowercase(),"support":format!("{:?}",runtime.codex_support).to_lowercase(),
            "installation":format!("{:?}",runtime.codex_installation).to_lowercase(),"health":format!("{:?}",runtime.codex_health).to_lowercase(),"source":runtime.source_label.chars().take(256).collect::<String>(),
            "disableConfirmationRequired":runtime.active_windows>0||cfg!(target_os="linux"),"activeWindows":if cfg!(target_os="linux"){None}else{Some(runtime.active_windows)},"runtimeCountersAvailable":!cfg!(target_os="linux"),"backgroundWorkers":if cfg!(target_os="linux"){None}else{Some(runtime.background_workers)},"subscriptions":if cfg!(target_os="linux"){None}else{Some(runtime.subscriptions)},"warmSurfaces":if cfg!(target_os="linux"){None}else{Some(runtime.warm_surfaces)},"cacheEntries":if cfg!(target_os="linux"){None}else{Some(runtime.cache_entries)},
            "diagnostic":runtime.diagnostic.as_ref().map(|diagnostic|diagnostic.chars().take(512).collect::<String>())}});
    // Include the selected source in the opaque revision without exposing its filesystem path.
    value["revision"] = format!(
        "{:x}",
        Sha256::digest(format!("{}{:?}", value, settings.codex_source).as_bytes())
    )
    .into();
    value
}

pub(crate) fn read_settings(path: &Path) -> Result<OptionalFeatureSettings, String> {
    match OptionalFeatureSettings::load(path) {
        Ok(settings) => Ok(settings),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Default::default()),
        Err(error) => Err(error.to_string()),
    }
}

#[derive(Default)]
pub(crate) struct FeatureClient {
    last_result: Option<Value>,
}
impl FeatureClient {
    pub(crate) fn read(
        &self,
        keyboard: Option<&nickel_session_protocol::OnScreenKeyboardSnapshot>,
        override_active: bool,
        native: bool,
        runtime_override: Option<&OptionalFeatureRuntime>,
    ) -> Value {
        let result = optional_features::settings_path()
            .map_err(|error| error.to_string())
            .and_then(|path| read_settings(&path));
        let mut value = match result {
            Ok(settings) => snapshot(
                &settings,
                &runtime_override
                    .cloned()
                    .unwrap_or_else(OptionalFeatureRuntime::load_default),
                keyboard,
                override_active,
                optional_features::codex_policy().0,
                native,
            ),
            Err(reason) => {
                json!({"available":false,"reason":reason,"operations":{},"keyboard":{},"codex":{}})
            }
        };
        value["lastResult"] = self.last_result.clone().unwrap_or(Value::Null);
        value
    }
    pub(crate) fn record(&mut self, result: Result<(), String>) {
        self.last_result = Some(match result {
            Ok(()) => {
                json!({"status":"requested","detail":"The native runtime is observing the preference."})
            }
            Err(detail) => {
                json!({"status":if detail.starts_with("Saved;"){"pending"}else{"rejected"},"detail":detail.chars().take(512).collect::<String>()})
            }
        });
    }
    pub(crate) fn apply(
        effect: &FeatureEffect,
        path: PathBuf,
        current: &Value,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<OptionalFeatureSettings, String> {
        effect.validate(current)?;
        check()?;
        let prior = read_settings(&path)?;
        let mut revision_data = current.clone();
        revision_data
            .as_object_mut()
            .ok_or("invalid feature snapshot")?
            .remove("revision");
        revision_data.as_object_mut().unwrap().remove("lastResult");
        let expected_revision = format!(
            "{:x}",
            Sha256::digest(format!("{}{:?}", revision_data, prior.codex_source).as_bytes())
        );
        if prior.codex_generation
            != current["codex"]["generation"]
                .as_u64()
                .ok_or("invalid Codex generation")?
            || Some(prior.codex_enabled) != current["codex"]["configuredEnabled"].as_bool()
            || prior.on_screen_keyboard_generation
                != current["keyboard"]["generation"]
                    .as_u64()
                    .ok_or("invalid keyboard generation")?
            || Some(prior.on_screen_keyboard.as_str()) != current["keyboard"]["mode"].as_str()
            || expected_revision != effect.revision
        {
            return Err("feature preference changed".into());
        }

        if effect.operation == "features.retryCodex" {
            return Ok(prior);
        }
        let checked = || check().map_err(std::io::Error::other);
        if let Some(mode) = effect.mode {
            optional_features::PreparedKeyboardPreference::prepare(path, &prior, mode)
                .map_err(|error| error.to_string())?
                .commit(checked)
                .map_err(|error| error.to_string())
        } else {
            optional_features::PreparedCodexPreference::prepare(
                path,
                &prior,
                effect.enabled.unwrap(),
            )
            .map_err(|error| error.to_string())?
            .commit(checked)
            .map_err(|error| error.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn feature_transactions_reject_stale_policy_and_unconfirmed_active_windows_and_preserve_other_fields()
     {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("features");
        let settings = OptionalFeatureSettings {
            codex_enabled: true,
            on_screen_keyboard: KeyboardPreference::Enabled,
            ..Default::default()
        };
        settings.save(&path).unwrap();
        let runtime = OptionalFeatureRuntime {
            active_windows: 1,
            ..Default::default()
        };
        let current = snapshot(
            &settings,
            &runtime,
            None,
            false,
            FeaturePolicy::Editable,
            true,
        );
        let request = json!({"type":"features.setCodexEnabled","enabled":false,"revision":current["revision"]});
        let effect = FeatureEffect::parse(&request).unwrap();
        assert!(effect.validate(&current).is_err());
        let mut confirmed = request;
        confirmed["confirmed"] = true.into();
        let effect = FeatureEffect::parse(&confirmed).unwrap();
        let accepted = FeatureClient::apply(&effect, path.clone(), &current, || Ok(())).unwrap();
        assert!(!accepted.codex_enabled);
        assert_eq!(accepted.on_screen_keyboard, KeyboardPreference::Enabled);
        let changed = snapshot(
            &accepted,
            &runtime,
            None,
            false,
            FeaturePolicy::Editable,
            true,
        );
        assert!(effect.validate(&changed).is_err());
        let controlled = snapshot(
            &accepted,
            &runtime,
            None,
            true,
            FeaturePolicy::ForceDisabled,
            true,
        );
        let keyboard=FeatureEffect::parse(&json!({"type":"features.setKeyboardMode","mode":"automatic","revision":controlled["revision"]})).unwrap();
        assert!(keyboard.validate(&controlled).is_err());
        let disabled_by_policy = FeatureEffect::parse(&json!({"type":"features.setCodexEnabled","enabled":true,"revision":controlled["revision"]})).unwrap();
        assert!(disabled_by_policy.validate(&controlled).is_err());
        let mut disk = accepted.clone();
        disk.codex_generation += 1;
        disk.save(&path).unwrap();
        assert!(FeatureClient::apply(&effect, path, &current, || Ok(())).is_err());
    }
    #[test]
    fn native_feature_effect_is_cancelled_before_persistence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("features");
        let settings = OptionalFeatureSettings::default();
        settings.save(&path).unwrap();
        let current = snapshot(
            &settings,
            &OptionalFeatureRuntime::default(),
            None,
            false,
            FeaturePolicy::Editable,
            true,
        );
        let effect=FeatureEffect::parse(&json!({"type":"features.setKeyboardMode","mode":"disabled","revision":current["revision"]})).unwrap();
        assert!(
            FeatureClient::apply(&effect, path.clone(), &current, || Err(
                "staged authority".into()
            ))
            .is_err()
        );
        assert_eq!(OptionalFeatureSettings::load(&path).unwrap(), settings);
        let mut checks = 0;
        assert!(
            FeatureClient::apply(&effect, path.clone(), &current, || {
                checks += 1;
                if checks > 1 {
                    Err("authority revoked during staging".into())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        assert_eq!(checks, 2);
        assert_eq!(OptionalFeatureSettings::load(path).unwrap(), settings);
    }
}
