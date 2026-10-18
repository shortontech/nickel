//! Portable shell preferences with revision-checked native storage transactions.
use nickel_core::{
    plugins::PluginCapability,
    shell_settings::{FileIconPreference, MAX_CONFIGURED_WORKSPACES, ShellSettings},
};
use nickel_storage::{RegularFileRevision, TransactionLock, regular_file_revision};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    path::PathBuf,
};

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Preferences {
    pub bar_on_all_displays: bool,
    pub all_windows_on_every_bar: bool,
    pub desktop_count: u8,
    pub preferred_terminal: Option<String>,
    pub preferred_file_manager: Option<String>,
    pub file_icon_provider: IconProvider,
    pub file_icon_theme: Option<String>,
    pub idle_dim_seconds: Option<u32>,
    pub idle_lock_seconds: Option<u32>,
    pub idle_suspend_seconds: Option<u32>,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub enum IconProvider {
    Nickel,
    System,
}

fn valid_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 512 && !value.chars().any(char::is_control)
}
fn valid_theme(theme: &str) -> bool {
    valid_id(theme)
        && theme.len() <= 128
        && !matches!(theme, "system" | "." | "..")
        && !theme.contains(['/', '\\'])
}

impl Preferences {
    fn valid_fields(&self, fields: &[String]) -> bool {
        fields.iter().all(|field| match field.as_str() {
            "barOnAllDisplays" | "allWindowsOnEveryBar" | "fileIconProvider" => true,
            "desktopCount" => (1..=MAX_CONFIGURED_WORKSPACES).contains(&self.desktop_count),
            "preferredTerminal" => self.preferred_terminal.as_deref().is_none_or(valid_id),
            "preferredFileManager" => self.preferred_file_manager.as_deref().is_none_or(valid_id),
            "fileIconTheme" => self.file_icon_theme.as_deref().is_none_or(valid_theme),
            "idleDimSeconds" | "idleLockSeconds" | "idleSuspendSeconds" => {
                let timeout = match field.as_str() {
                    "idleDimSeconds" => self.idle_dim_seconds,
                    "idleLockSeconds" => self.idle_lock_seconds,
                    _ => self.idle_suspend_seconds,
                };
                timeout.is_none_or(|seconds| (30..=604_800).contains(&seconds))
            }
            _ => false,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct Catalog {
    applications: Vec<(String, String)>,
    themes: Vec<String>,
}
impl Catalog {
    pub(crate) fn new(
        applications: impl IntoIterator<Item = (String, String)>,
        themes: Vec<String>,
    ) -> Result<Self, String> {
        let mut applications = applications
            .into_iter()
            .filter(|(id, command)| {
                valid_id(id)
                    && !command.is_empty()
                    && command.len() <= 4096
                    && !command.chars().any(char::is_control)
            })
            .collect::<Vec<_>>();
        if applications.len() > 512 || themes.len() > 256 {
            return Err("preferences catalog exceeds its supported bound".into());
        }
        applications.sort();
        if applications
            .windows(2)
            .any(|items| items[0].0 == items[1].0)
        {
            return Err("preferences application identity is ambiguous".into());
        }
        let mut themes = themes
            .into_iter()
            .filter(|theme| valid_theme(theme))
            .collect::<Vec<_>>();
        themes.sort();
        themes.dedup();
        Ok(Self {
            applications,
            themes,
        })
    }
    fn selected_id(&self, command: Option<&str>) -> Option<String> {
        let mut candidates = self
            .applications
            .iter()
            .filter(|(_, candidate)| Some(candidate.as_str()) == command);
        let first = candidates.next();
        if candidates.next().is_none() {
            first.map(|(id, _)| id.clone())
        } else {
            None
        }
    }
    fn command(&self, id: &Option<String>) -> Result<Option<String>, String> {
        id.as_ref()
            .map(|id| {
                self.applications
                    .iter()
                    .find(|(candidate, _)| candidate == id)
                    .map(|(_, command)| command.clone())
                    .ok_or_else(|| "preferred application is unavailable".into())
            })
            .transpose()
    }
    fn configured(&self, settings: &ShellSettings) -> Preferences {
        Preferences {
            bar_on_all_displays: settings.bar_on_all_displays,
            all_windows_on_every_bar: settings.all_windows_on_every_bar,
            desktop_count: settings.desktop_count,
            preferred_terminal: self.selected_id(settings.preferred_terminal.as_deref()),
            preferred_file_manager: self.selected_id(settings.preferred_file_manager.as_deref()),
            file_icon_provider: match settings.file_icon_provider {
                FileIconPreference::Nickel => IconProvider::Nickel,
                FileIconPreference::System => IconProvider::System,
            },
            file_icon_theme: settings.file_icon_theme.clone(),
            idle_dim_seconds: settings.idle_dim_seconds,
            idle_lock_seconds: settings.idle_lock_seconds,
            idle_suspend_seconds: settings.idle_suspend_seconds,
        }
    }
    fn apply(
        &self,
        settings: &mut ShellSettings,
        prior: &Preferences,
        requested: &Preferences,
        changed_fields: &[String],
    ) -> Result<(), String> {
        if !requested.valid_fields(changed_fields) {
            return Err("preferences value is outside its supported range".into());
        }
        settings.bar_on_all_displays = requested.bar_on_all_displays;
        settings.all_windows_on_every_bar = requested.all_windows_on_every_bar;
        settings.desktop_count = requested.desktop_count;
        // Preserve a temporarily unavailable native selection when another field changes.
        if requested.preferred_terminal != prior.preferred_terminal {
            settings.preferred_terminal = self.command(&requested.preferred_terminal)?;
        }
        if requested.preferred_file_manager != prior.preferred_file_manager {
            settings.preferred_file_manager = self.command(&requested.preferred_file_manager)?;
        }
        if requested.file_icon_theme != prior.file_icon_theme
            && requested
                .file_icon_theme
                .as_ref()
                .is_some_and(|theme| !self.themes.contains(theme))
        {
            return Err("file icon theme is unavailable".into());
        }
        settings.file_icon_provider = match requested.file_icon_provider {
            IconProvider::Nickel => FileIconPreference::Nickel,
            IconProvider::System => FileIconPreference::System,
        };
        settings.file_icon_theme = requested.file_icon_theme.clone();
        settings.idle_dim_seconds = requested.idle_dim_seconds;
        settings.idle_lock_seconds = requested.idle_lock_seconds;
        settings.idle_suspend_seconds = requested.idle_suspend_seconds;
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct PreferencesEffect {
    revision: String,
    prior: Preferences,
    requested: Preferences,
    changed_fields: Vec<String>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Transaction {
    revision: String,
    prior: Preferences,
    requested: Preferences,
    changed_fields: Vec<String>,
}
impl PreferencesEffect {
    pub(crate) fn parse(effect: &Value) -> Result<Self, String> {
        let object = effect.as_object().ok_or("invalid preferences operation")?;
        if object.len() != 2 || effect["type"] != "preferences.set" {
            return Err("unknown preferences operation fields".into());
        }
        let transaction: Transaction = serde_json::from_value(effect["transaction"].clone())
            .map_err(|_| "invalid preferences transaction")?;
        if transaction.revision.len() != 16
            || !transaction
                .revision
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || !transaction
                .requested
                .valid_fields(&transaction.changed_fields)
        {
            return Err("invalid preferences revision or values".into());
        }
        let prior = serde_json::to_value(&transaction.prior).unwrap();
        let requested = serde_json::to_value(&transaction.requested).unwrap();
        if transaction.changed_fields.is_empty()
            || transaction.changed_fields.len() > 10
            || transaction
                .changed_fields
                .iter()
                .enumerate()
                .any(|(index, field)| {
                    !prior.as_object().unwrap().contains_key(field)
                        || transaction.changed_fields[..index].contains(field)
                })
            || prior.as_object().unwrap().iter().any(|(field, value)| {
                !transaction.changed_fields.contains(field) && requested[field] != *value
            })
        {
            return Err("unknown or unrequested preference fields".into());
        }
        Ok(Self {
            revision: transaction.revision,
            prior: transaction.prior,
            requested: transaction.requested,
            changed_fields: transaction.changed_fields,
        })
    }
    pub(crate) fn capability(&self) -> PluginCapability {
        PluginCapability::PreferencesControl
    }
    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot["available"] != true
            || snapshot["revision"].as_str() != Some(&self.revision)
            || snapshot["configured"] != serde_json::to_value(&self.prior).unwrap()
        {
            return Err("preferences changed; read current preferences before retrying".into());
        }
        Ok(())
    }
}

struct Observation {
    revision: Option<RegularFileRevision>,
    settings: ShellSettings,
    token: String,
}
fn observe(path: &PathBuf, catalog: &Catalog) -> Result<Observation, String> {
    let revision = regular_file_revision(path).map_err(|error| error.to_string())?;
    let settings = ShellSettings::load_for_update(path).map_err(|error| error.to_string())?;
    if regular_file_revision(path).map_err(|error| error.to_string())? != revision {
        return Err("preferences changed during observation".into());
    }
    let mut hash = DefaultHasher::new();
    format!("{revision:?}").hash(&mut hash);
    catalog.hash(&mut hash);
    serde_json::to_string(&catalog.configured(&settings))
        .unwrap()
        .hash(&mut hash);
    Ok(Observation {
        revision,
        settings,
        token: format!("{:016x}", hash.finish()),
    })
}

#[derive(Default)]
pub(crate) struct PreferencesCapabilities {
    snapshot: Option<Value>,
}
impl PreferencesCapabilities {
    pub(crate) fn refresh(&mut self, catalog: &Catalog) -> Value {
        let value = nickel_core::shell_settings::settings_path()
            .map_err(|error| error.to_string())
            .and_then(|path| self.read_at(&path, catalog))
            .unwrap_or_else(|reason| json!({"available":false,"reason":reason}));
        self.snapshot = Some(value.clone());
        value
    }
    pub(crate) fn snapshot(&mut self, catalog: &Catalog) -> Value {
        self.snapshot
            .clone()
            .unwrap_or_else(|| self.refresh(catalog))
    }
    fn read_at(&self, path: &PathBuf, catalog: &Catalog) -> Result<Value, String> {
        let observed = observe(path, catalog)?;
        let configured = catalog.configured(&observed.settings);
        Ok(
            json!({"available":true,"revision":observed.token,"configured":configured,
            "applications":catalog.applications.iter().map(|(id, _)| json!({"id":id})).collect::<Vec<_>>(), "iconThemes":catalog.themes,
            "unavailableSelections":{"preferredTerminal":observed.settings.preferred_terminal.is_some() && configured.preferred_terminal.is_none(), "preferredFileManager":observed.settings.preferred_file_manager.is_some() && configured.preferred_file_manager.is_none(), "fileIconTheme":configured.file_icon_theme.as_ref().is_some_and(|theme| !catalog.themes.contains(theme))}}),
        )
    }
    pub(crate) fn execute(
        &mut self,
        effect: &PreferencesEffect,
        catalog: &Catalog,
        check: impl FnMut() -> Result<(), String>,
    ) -> Result<ShellSettings, String> {
        let path =
            nickel_core::shell_settings::settings_path().map_err(|error| error.to_string())?;
        let result = self.execute_at(path, effect, catalog, check);
        self.refresh(catalog);
        result
    }
    fn execute_at(
        &mut self,
        path: PathBuf,
        effect: &PreferencesEffect,
        catalog: &Catalog,
        mut check: impl FnMut() -> Result<(), String>,
    ) -> Result<ShellSettings, String> {
        check()?;
        let _lock = TransactionLock::try_acquire(&path).map_err(|error| error.to_string())?;
        let observed = observe(&path, catalog)?;
        if effect.revision != observed.token
            || effect.prior != catalog.configured(&observed.settings)
        {
            return Err("preferences changed; read current preferences before retrying".into());
        }
        let mut requested = observed.settings;
        catalog.apply(
            &mut requested,
            &effect.prior,
            &effect.requested,
            &effect.changed_fields,
        )?;
        if effect
            .changed_fields
            .iter()
            .any(|field| field == "preferredTerminal")
        {
            requested.preferred_terminal = catalog.command(&effect.requested.preferred_terminal)?;
        }
        if effect
            .changed_fields
            .iter()
            .any(|field| field == "preferredFileManager")
        {
            requested.preferred_file_manager =
                catalog.command(&effect.requested.preferred_file_manager)?;
        }
        let staged = requested.stage(&path).map_err(|error| error.to_string())?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        staged
            .commit(|| {
                if std::time::Instant::now() >= deadline {
                    return Err(std::io::Error::other("preferences commit expired"));
                }
                if regular_file_revision(&path)? != observed.revision {
                    return Err(std::io::Error::other("preferences changed before commit"));
                }
                check().map_err(std::io::Error::other)
            })
            .map_err(|error| error.to_string())?;
        // The successful rename is authoritative even if a later observation fails.
        Ok(requested)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_core::shell_settings::ThemePreference;

    fn catalog() -> Catalog {
        Catalog::new(
            [
                ("terminal.desktop".into(), "foot".into()),
                ("files.desktop".into(), "nautilus".into()),
            ],
            vec!["Papirus".into()],
        )
        .unwrap()
    }
    fn effect(snapshot: &Value, patch: Value) -> PreferencesEffect {
        let mut requested = snapshot["configured"].clone();
        requested
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        PreferencesEffect::parse(&json!({"type":"preferences.set","transaction":{"revision":snapshot["revision"],"prior":snapshot["configured"],"requested":requested,"changedFields":patch.as_object().unwrap().keys().collect::<Vec<_>>()}})).unwrap()
    }

    #[test]
    fn preferences_atomic_patch_preserves_appearance_and_resolves_native_application_ids() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings");
        let prior = ShellSettings {
            theme: ThemePreference::Light,
            accent_hue: Some(37),
            preferred_terminal: Some("temporarily-missing-terminal".into()),
            ..Default::default()
        };
        prior.save(&path).unwrap();
        let mut service = PreferencesCapabilities::default();
        let catalog = catalog();
        let snapshot = service.read_at(&path, &catalog).unwrap();
        assert_eq!(snapshot["unavailableSelections"]["preferredTerminal"], true);
        assert!(snapshot["configured"].get("theme").is_none());
        let change = effect(
            &snapshot,
            json!({"barOnAllDisplays":false,"desktopCount":8,"preferredFileManager":"files.desktop","fileIconProvider":"system","fileIconTheme":"Papirus","idleLockSeconds":60}),
        );
        let accepted = service
            .execute_at(path.clone(), &change, &catalog, || Ok(()))
            .unwrap();
        assert_eq!(accepted, ShellSettings::load(&path).unwrap());
        assert_eq!(accepted.theme, prior.theme);
        assert_eq!(accepted.accent_hue, prior.accent_hue);
        assert_eq!(accepted.preferred_terminal, prior.preferred_terminal);
        assert_eq!(accepted.preferred_file_manager.as_deref(), Some("nautilus"));
        assert_eq!(accepted.idle_lock_seconds, Some(60));
        assert!(!accepted.bar_on_all_displays);
        assert!(
            service
                .execute_at(path.clone(), &change, &catalog, || Ok(()))
                .is_err()
        );
        let snapshot = service.read_at(&path, &catalog).unwrap();
        let reset = effect(&snapshot, json!({"preferredTerminal":null}));
        assert_eq!(
            service
                .execute_at(path, &reset, &catalog, || Ok(()))
                .unwrap()
                .preferred_terminal,
            None
        );
    }

    #[test]
    fn preferences_reject_external_revision_changes_and_retired_authority_at_commit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings");
        ShellSettings::default().save(&path).unwrap();
        let catalog = catalog();
        let mut service = PreferencesCapabilities::default();
        let snapshot = service.read_at(&path, &catalog).unwrap();
        let request = effect(&snapshot, json!({"allWindowsOnEveryBar":false}));
        let mut calls = 0;
        assert!(
            service
                .execute_at(path.clone(), &request, &catalog, || {
                    calls += 1;
                    if calls == 1 {
                        Ok(())
                    } else {
                        Err("grant retired".into())
                    }
                })
                .is_err()
        );
        assert!(ShellSettings::load(&path).unwrap().all_windows_on_every_bar);
        let external = ShellSettings {
            accent_hue: Some(222),
            ..Default::default()
        };
        external.save(&path).unwrap();
        assert!(
            service
                .execute_at(path.clone(), &request, &catalog, || Ok(()))
                .is_err()
        );
        assert_eq!(ShellSettings::load(&path).unwrap(), external);
        let fresh = service.read_at(&path, &catalog).unwrap();
        let request = effect(&fresh, json!({"preferredTerminal":"unknown.desktop"}));
        assert!(
            service
                .execute_at(path.clone(), &request, &catalog, || Ok(()))
                .is_err()
        );
        let request = effect(&fresh, json!({"fileIconTheme":"missing-theme"}));
        assert!(
            service
                .execute_at(path, &request, &catalog, || Ok(()))
                .is_err()
        );
    }

    #[test]
    fn preferences_validate_types_ranges_and_reject_unrequested_fields() {
        let service = PreferencesCapabilities::default();
        let catalog = catalog();
        let dir = tempfile::tempdir().unwrap();
        let snapshot = service
            .read_at(&dir.path().join("missing"), &catalog)
            .unwrap();
        for patch in [
            json!({"desktopCount":0}),
            json!({"desktopCount":11}),
            json!({"idleDimSeconds":0}),
            json!({"idleLockSeconds":604801}),
            json!({"fileIconTheme":"/tmp/theme"}),
            json!({"barOnAllDisplays":"yes"}),
        ] {
            let mut requested = snapshot["configured"].clone();
            requested
                .as_object_mut()
                .unwrap()
                .extend(patch.as_object().unwrap().clone());
            let wire = json!({"type":"preferences.set","transaction":{"revision":snapshot["revision"],"prior":snapshot["configured"],"requested":requested,"changedFields":patch.as_object().unwrap().keys().collect::<Vec<_>>()}});
            assert!(PreferencesEffect::parse(&wire).is_err());
        }
        let valid = effect(
            &snapshot,
            json!({"desktopCount":10,"idleSuspendSeconds":null}),
        );
        assert_eq!(valid.capability(), PluginCapability::PreferencesControl);
        assert!(valid.validate(&snapshot).is_ok());
        let mut stale = snapshot.clone();
        stale["revision"] = "0000000000000000".into();
        assert!(valid.validate(&stale).is_err());
        let mut raw = json!({"type":"preferences.set","transaction":{"revision":snapshot["revision"],"prior":snapshot["configured"],"requested":snapshot["configured"],"changedFields":["desktopCount"]}});
        raw["transaction"]["requested"]["idleLockSeconds"] = json!(null);
        assert!(PreferencesEffect::parse(&raw).is_err());
        raw["transaction"]["requested"] = snapshot["configured"].clone();
        raw["transaction"]["changedFields"] = json!(["theme"]);
        assert!(PreferencesEffect::parse(&raw).is_err());
    }
}
