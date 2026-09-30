//! Portable package-owned Settings registrations.
//!
//! The registry stores declarative metadata only. JavaScript function values and
//! runtime handlers remain owned by the package host and are addressed by the
//! stable provider/registration IDs preserved in each snapshot.

use std::collections::{BTreeMap, BTreeSet, HashSet};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::plugins::PluginCapability;

pub const MAX_SETTINGS_PER_PROVIDER: usize = 128;
pub const MAX_SETTINGS_PAGES_PER_PROVIDER: usize = 32;
pub const MAX_SETTINGS_PROVIDERS: usize = 256;
pub const MAX_PROVIDER_REGISTRATION_BYTES: usize = 1024 * 1024;
pub const MAX_SETTING_FIELDS: usize = 64;
pub const MAX_SELECT_OPTIONS: usize = 128;
pub const MAX_REPEATED_ITEMS: usize = 256;
pub const MAX_SETTING_NESTING: usize = 4;

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingRegistration {
    pub id: String,
    pub group: String,
    #[serde(default)]
    pub group_order: i32,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_capabilities: Vec<PluginCapability>,
    #[serde(flatten)]
    pub control: SettingControl,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum SettingControl {
    Switch {
        default_value: bool,
    },
    Slider {
        default_value: f64,
        min: f64,
        max: f64,
        #[serde(default = "default_step")]
        step: f64,
    },
    Select {
        default_value: String,
        options: Vec<SettingOption>,
    },
    Color {
        default_value: String,
        #[serde(default)]
        allow_alpha: bool,
    },
    Text {
        default_value: String,
        #[serde(default = "default_text_length")]
        max_length: u32,
        #[serde(default)]
        multiline: bool,
    },
    Number {
        default_value: f64,
        min: f64,
        max: f64,
        #[serde(default = "default_step")]
        step: f64,
    },
    Shortcut {
        #[serde(default)]
        default_value: Vec<String>,
    },
    Action,
    Group {
        fields: Vec<SettingField>,
    },
    Repeated {
        fields: Vec<SettingField>,
        #[serde(default)]
        default_value: Vec<BTreeMap<String, Value>>,
        #[serde(default)]
        min_items: u16,
        max_items: u16,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SettingField {
    pub id: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub order: i32,
    #[serde(flatten)]
    pub control: SettingControl,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingOption {
    pub value: String,
    pub label: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsPageRegistration {
    pub id: String,
    pub group: String,
    #[serde(default)]
    pub group_order: i32,
    pub label: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(default)]
    pub order: i32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub required_capabilities: Vec<PluginCapability>,
    pub component: SettingsComponentRef,
}

/// A serializable reference resolved by the owning package's module graph.
#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SettingsComponentRef {
    pub module: String,
    #[serde(default = "default_export")]
    pub export: String,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredSetting {
    pub provider_package: String,
    #[serde(flatten)]
    pub registration: SettingRegistration,
}

#[derive(Clone, Debug, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredSettingsPage {
    pub provider_package: String,
    #[serde(flatten)]
    pub registration: SettingsPageRegistration,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginSettingsSnapshot {
    pub generation: u64,
    pub settings: Vec<RegisteredSetting>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize, Eq, PartialEq)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PluginSettingsPagesSnapshot {
    pub generation: u64,
    pub pages: Vec<RegisteredSettingsPage>,
}

#[derive(Clone, Debug, Default)]
pub struct SettingsRegistry {
    generation: u64,
    providers: BTreeMap<String, ProviderRegistrations>,
}

#[derive(Clone, Debug, Default)]
struct ProviderRegistrations {
    enabled: bool,
    settings: Vec<SettingRegistration>,
    pages: Vec<SettingsPageRegistration>,
}

impl SettingsRegistry {
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Validates and atomically replaces every Settings registration owned by a package.
    pub fn replace_provider(
        &mut self,
        provider_package: impl Into<String>,
        enabled: bool,
        settings: Vec<SettingRegistration>,
        pages: Vec<SettingsPageRegistration>,
    ) -> Result<(), String> {
        let provider_package = provider_package.into();
        if !self.providers.contains_key(&provider_package)
            && self.providers.len() >= MAX_SETTINGS_PROVIDERS
        {
            return Err(format!(
                "registry contains the maximum of {MAX_SETTINGS_PROVIDERS} providers"
            ));
        }
        validate_provider(&provider_package, &settings, &pages)?;
        let replacement = ProviderRegistrations {
            enabled,
            settings,
            pages,
        };
        self.providers.insert(provider_package, replacement);
        self.bump_generation();
        Ok(())
    }

    /// Enables or disables an installed provider. Disabled entries vanish from both snapshots.
    pub fn set_provider_enabled(&mut self, provider_package: &str, enabled: bool) -> bool {
        let Some(provider) = self.providers.get_mut(provider_package) else {
            return false;
        };
        if provider.enabled == enabled {
            return true;
        }
        provider.enabled = enabled;
        self.bump_generation();
        true
    }

    /// Permanently retires the provider's definitions from this registry.
    pub fn retire_provider(&mut self, provider_package: &str) -> bool {
        let removed = self.providers.remove(provider_package).is_some();
        if removed {
            self.bump_generation();
        }
        removed
    }

    pub fn settings_snapshot(&self) -> PluginSettingsSnapshot {
        let mut settings = self
            .providers
            .iter()
            .filter(|(_, provider)| provider.enabled)
            .flat_map(|(provider_package, provider)| {
                provider
                    .settings
                    .iter()
                    .cloned()
                    .map(|registration| RegisteredSetting {
                        provider_package: provider_package.clone(),
                        registration,
                    })
            })
            .collect::<Vec<_>>();
        settings.sort_by(|left, right| setting_sort_key(left).cmp(&setting_sort_key(right)));
        PluginSettingsSnapshot {
            generation: self.generation,
            settings,
        }
    }

    /// Snapshot shaped for the package host's `readPluginSettings()` bridge.
    pub fn read_plugin_settings(&self) -> PluginSettingsSnapshot {
        self.settings_snapshot()
    }

    pub fn settings_pages_snapshot(&self) -> PluginSettingsPagesSnapshot {
        let mut pages = self
            .providers
            .iter()
            .filter(|(_, provider)| provider.enabled)
            .flat_map(|(provider_package, provider)| {
                provider
                    .pages
                    .iter()
                    .cloned()
                    .map(|registration| RegisteredSettingsPage {
                        provider_package: provider_package.clone(),
                        registration,
                    })
            })
            .collect::<Vec<_>>();
        pages.sort_by(|left, right| page_sort_key(left).cmp(&page_sort_key(right)));
        PluginSettingsPagesSnapshot {
            generation: self.generation,
            pages,
        }
    }

    /// Snapshot shaped for the package host's `readPluginSettingsPages()` bridge.
    pub fn read_plugin_settings_pages(&self) -> PluginSettingsPagesSnapshot {
        self.settings_pages_snapshot()
    }

    fn bump_generation(&mut self) {
        self.generation = self.generation.wrapping_add(1);
    }
}

fn setting_sort_key(setting: &RegisteredSetting) -> (i32, &str, i32, &str, &str) {
    (
        setting.registration.group_order,
        &setting.registration.group,
        setting.registration.order,
        &setting.provider_package,
        &setting.registration.id,
    )
}

fn page_sort_key(page: &RegisteredSettingsPage) -> (i32, &str, i32, &str, &str) {
    (
        page.registration.group_order,
        &page.registration.group,
        page.registration.order,
        &page.provider_package,
        &page.registration.id,
    )
}

fn validate_provider(
    provider: &str,
    settings: &[SettingRegistration],
    pages: &[SettingsPageRegistration],
) -> Result<(), String> {
    if !valid_package_id(provider) {
        return Err("provider package has an invalid ID".into());
    }
    if settings.len() > MAX_SETTINGS_PER_PROVIDER {
        return Err(format!(
            "provider declares more than {MAX_SETTINGS_PER_PROVIDER} settings"
        ));
    }
    if pages.len() > MAX_SETTINGS_PAGES_PER_PROVIDER {
        return Err(format!(
            "provider declares more than {MAX_SETTINGS_PAGES_PER_PROVIDER} settings pages"
        ));
    }
    let serialized_size = serde_json::to_vec(&(settings, pages))
        .map_err(|error| format!("Settings registrations are not serializable: {error}"))?
        .len();
    if serialized_size > MAX_PROVIDER_REGISTRATION_BYTES {
        return Err(format!(
            "provider Settings registrations exceed {MAX_PROVIDER_REGISTRATION_BYTES} bytes"
        ));
    }
    let mut setting_ids = BTreeSet::new();
    for setting in settings {
        if !setting_ids.insert(setting.id.as_str()) {
            return Err(format!(
                "duplicate Settings registration ID {:?}",
                setting.id
            ));
        }
        validate_common(
            &setting.id,
            &setting.group,
            &setting.label,
            &setting.description,
            &setting.required_capabilities,
        )?;
        validate_control(&setting.control, 0)?;
    }
    let mut page_ids = BTreeSet::new();
    for page in pages {
        if !page_ids.insert(page.id.as_str()) {
            return Err(format!("duplicate Settings registration ID {:?}", page.id));
        }
        validate_common(
            &page.id,
            &page.group,
            &page.label,
            &page.description,
            &page.required_capabilities,
        )?;
        if page.component.module.is_empty()
            || page.component.module.len() > 256
            || page.component.module.contains('\0')
            || !valid_reference_name(&page.component.export)
        {
            return Err(format!(
                "settings page {:?} has an invalid component reference",
                page.id
            ));
        }
    }
    Ok(())
}

fn validate_common(
    id: &str,
    group: &str,
    label: &str,
    description: &str,
    capabilities: &[PluginCapability],
) -> Result<(), String> {
    if !valid_registration_id(id) {
        return Err(format!("invalid Settings registration ID {id:?}"));
    }
    if group.trim().is_empty()
        || group.len() > 80
        || label.trim().is_empty()
        || label.len() > 120
        || description.len() > 512
    {
        return Err(format!(
            "Settings registration {id:?} has invalid display metadata"
        ));
    }
    let unique = capabilities.iter().collect::<HashSet<_>>();
    if unique.len() != capabilities.len() || capabilities.len() > 32 {
        return Err(format!(
            "Settings registration {id:?} has invalid capability requirements"
        ));
    }
    Ok(())
}

fn validate_control(control: &SettingControl, depth: usize) -> Result<(), String> {
    if depth > MAX_SETTING_NESTING {
        return Err("setting fields exceed the nesting limit".into());
    }
    match control {
        SettingControl::Switch { .. } | SettingControl::Action => Ok(()),
        SettingControl::Slider {
            default_value,
            min,
            max,
            step,
        }
        | SettingControl::Number {
            default_value,
            min,
            max,
            step,
        } => {
            if finite_range(*default_value, *min, *max, *step) {
                Ok(())
            } else {
                Err("numeric setting has invalid bounds, default, or step".into())
            }
        }
        SettingControl::Select {
            default_value,
            options,
        } => {
            if options.is_empty() || options.len() > MAX_SELECT_OPTIONS {
                return Err("select setting has an invalid option count".into());
            }
            let mut values = BTreeSet::new();
            for option in options {
                if option.value.is_empty()
                    || option.value.len() > 128
                    || option.label.trim().is_empty()
                    || option.label.len() > 120
                    || !values.insert(option.value.as_str())
                {
                    return Err("select setting has invalid or duplicate options".into());
                }
            }
            if values.contains(default_value.as_str()) {
                Ok(())
            } else {
                Err("select default is not one of its options".into())
            }
        }
        SettingControl::Color { default_value, .. } => {
            if !default_value.trim().is_empty() && default_value.len() <= 128 {
                Ok(())
            } else {
                Err("color setting has an invalid default".into())
            }
        }
        SettingControl::Text {
            default_value,
            max_length,
            ..
        } => {
            if (1..=65_536).contains(max_length)
                && default_value.chars().count() <= *max_length as usize
            {
                Ok(())
            } else {
                Err("text setting has an invalid default or maximum length".into())
            }
        }
        SettingControl::Shortcut { default_value } => {
            if default_value.len() <= 8
                && default_value
                    .iter()
                    .all(|part| !part.trim().is_empty() && part.len() <= 64)
            {
                Ok(())
            } else {
                Err("shortcut setting has an invalid chord".into())
            }
        }
        SettingControl::Group { fields } => validate_fields(fields, depth + 1),
        SettingControl::Repeated {
            fields,
            default_value,
            min_items,
            max_items,
        } => {
            validate_fields(fields, depth + 1)?;
            if *max_items == 0
                || usize::from(*max_items) > MAX_REPEATED_ITEMS
                || min_items > max_items
                || default_value.len() < usize::from(*min_items)
                || default_value.len() > usize::from(*max_items)
            {
                return Err("repeated setting has invalid item bounds or defaults".into());
            }
            if default_value.iter().any(|item| {
                item.iter().any(|(key, value)| {
                    fields
                        .iter()
                        .find(|field| field.id == *key)
                        .is_none_or(|field| !accepts_value(&field.control, value))
                })
            }) {
                return Err("repeated setting default contains an unknown field".into());
            }
            Ok(())
        }
    }
}

fn accepts_value(control: &SettingControl, value: &Value) -> bool {
    match control {
        SettingControl::Switch { .. } => value.is_boolean(),
        SettingControl::Slider { min, max, .. } | SettingControl::Number { min, max, .. } => value
            .as_f64()
            .is_some_and(|number| number.is_finite() && *min <= number && number <= *max),
        SettingControl::Select { options, .. } => value
            .as_str()
            .is_some_and(|selected| options.iter().any(|option| option.value == selected)),
        SettingControl::Color { .. } => value
            .as_str()
            .is_some_and(|color| !color.trim().is_empty() && color.len() <= 128),
        SettingControl::Text { max_length, .. } => value
            .as_str()
            .is_some_and(|text| text.chars().count() <= *max_length as usize),
        SettingControl::Shortcut { .. } => value.as_array().is_some_and(|parts| {
            parts.len() <= 8
                && parts.iter().all(|part| {
                    part.as_str()
                        .is_some_and(|part| !part.trim().is_empty() && part.len() <= 64)
                })
        }),
        SettingControl::Action => value.is_null(),
        SettingControl::Group { fields } => value
            .as_object()
            .is_some_and(|object| accepts_fields(fields, object)),
        SettingControl::Repeated {
            fields,
            min_items,
            max_items,
            ..
        } => value.as_array().is_some_and(|items| {
            (usize::from(*min_items)..=usize::from(*max_items)).contains(&items.len())
                && items.iter().all(|item| {
                    item.as_object()
                        .is_some_and(|object| accepts_fields(fields, object))
                })
        }),
    }
}

fn accepts_fields(fields: &[SettingField], object: &serde_json::Map<String, Value>) -> bool {
    object.iter().all(|(key, value)| {
        fields
            .iter()
            .find(|field| field.id == *key)
            .is_some_and(|field| accepts_value(&field.control, value))
    })
}

fn validate_fields(fields: &[SettingField], depth: usize) -> Result<(), String> {
    if fields.is_empty() || fields.len() > MAX_SETTING_FIELDS {
        return Err("grouped setting has an invalid field count".into());
    }
    let mut ids = BTreeSet::new();
    for field in fields {
        if !valid_registration_id(&field.id)
            || !ids.insert(field.id.as_str())
            || field.label.trim().is_empty()
            || field.label.len() > 120
            || field.description.len() > 512
        {
            return Err("grouped setting has invalid or duplicate fields".into());
        }
        validate_control(&field.control, depth)?;
    }
    Ok(())
}

fn finite_range(default: f64, min: f64, max: f64, step: f64) -> bool {
    default.is_finite()
        && min.is_finite()
        && max.is_finite()
        && step.is_finite()
        && min <= default
        && default <= max
        && min < max
        && step > 0.0
}

fn valid_package_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 96
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b".-".contains(&byte))
        && !value.starts_with(['.', '-'])
        && !value.ends_with(['.', '-'])
        && !value.contains("..")
}

fn valid_registration_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"._-".contains(&byte))
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && value
            .bytes()
            .last()
            .is_some_and(|byte| byte.is_ascii_alphanumeric())
        && !value.contains("..")
}

fn valid_reference_name(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_$".contains(&byte))
        && value
            .bytes()
            .next()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || b"_$".contains(&byte))
}

fn default_step() -> f64 {
    1.0
}
fn default_text_length() -> u32 {
    1024
}
fn default_export() -> String {
    "default".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn switch(id: &str, group: &str, group_order: i32, order: i32) -> SettingRegistration {
        SettingRegistration {
            id: id.into(),
            group: group.into(),
            group_order,
            label: id.into(),
            description: String::new(),
            order,
            required_capabilities: vec![PluginCapability::SettingsRead],
            control: SettingControl::Switch {
                default_value: false,
            },
        }
    }

    fn page(id: &str) -> SettingsPageRegistration {
        SettingsPageRegistration {
            id: id.into(),
            group: "Network".into(),
            group_order: 10,
            label: "Wi-Fi".into(),
            description: String::new(),
            order: 5,
            required_capabilities: vec![PluginCapability::NetworkRead],
            component: SettingsComponentRef {
                module: "./settings/Wifi.jsx".into(),
                export: "WifiSettings".into(),
            },
        }
    }

    #[test]
    fn snapshots_preserve_provenance_and_have_stable_order() {
        let mut registry = SettingsRegistry::default();
        registry
            .replace_provider(
                "org.nickel.zed",
                true,
                vec![switch("later", "VPN", 20, 1)],
                vec![],
            )
            .unwrap();
        registry
            .replace_provider(
                "org.nickel.alpha",
                true,
                vec![
                    switch("second", "VPN", 10, 20),
                    switch("first", "VPN", 10, 10),
                ],
                vec![page("wifi.networks")],
            )
            .unwrap();

        let snapshot = registry.settings_snapshot();
        assert_eq!(snapshot.generation, 2);
        assert_eq!(
            snapshot
                .settings
                .iter()
                .map(|entry| entry.registration.id.as_str())
                .collect::<Vec<_>>(),
            ["first", "second", "later"]
        );
        assert!(
            snapshot
                .settings
                .iter()
                .all(|entry| !entry.provider_package.is_empty())
        );
        assert_eq!(
            registry.settings_pages_snapshot().pages[0].provider_package,
            "org.nickel.alpha"
        );
        serde_json::to_string(&snapshot).expect("snapshot is serializable");
    }

    #[test]
    fn disable_and_retire_atomically_remove_provider_entries() {
        let mut registry = SettingsRegistry::default();
        registry
            .replace_provider(
                "org.nickel.vpn",
                true,
                vec![switch("vpn.autoConnect", "VPN", 0, 0)],
                vec![page("vpn.details")],
            )
            .unwrap();
        assert!(registry.set_provider_enabled("org.nickel.vpn", false));
        assert!(registry.settings_snapshot().settings.is_empty());
        assert!(registry.settings_pages_snapshot().pages.is_empty());
        assert!(registry.set_provider_enabled("org.nickel.vpn", true));
        assert_eq!(registry.settings_snapshot().settings.len(), 1);
        assert!(registry.retire_provider("org.nickel.vpn"));
        assert!(registry.settings_snapshot().settings.is_empty());
        assert!(!registry.set_provider_enabled("org.nickel.vpn", true));
    }

    #[test]
    fn failed_replacement_leaves_previous_provider_intact() {
        let mut registry = SettingsRegistry::default();
        registry
            .replace_provider(
                "org.nickel.vpn",
                true,
                vec![switch("valid", "VPN", 0, 0)],
                vec![],
            )
            .unwrap();
        let generation = registry.generation();
        let duplicate = switch("same", "VPN", 0, 0);
        assert!(
            registry
                .replace_provider(
                    "org.nickel.vpn",
                    true,
                    vec![duplicate.clone(), duplicate],
                    vec![]
                )
                .is_err()
        );
        assert_eq!(registry.generation(), generation);
        assert_eq!(
            registry.settings_snapshot().settings[0].registration.id,
            "valid"
        );
    }

    #[test]
    fn validates_every_control_family_and_nested_bounds() {
        let fields = vec![SettingField {
            id: "name".into(),
            label: "Name".into(),
            description: String::new(),
            order: 0,
            control: SettingControl::Text {
                default_value: String::new(),
                max_length: 40,
                multiline: false,
            },
        }];
        let controls = vec![
            SettingControl::Slider {
                default_value: 0.5,
                min: 0.0,
                max: 1.0,
                step: 0.1,
            },
            SettingControl::Select {
                default_value: "a".into(),
                options: vec![SettingOption {
                    value: "a".into(),
                    label: "A".into(),
                }],
            },
            SettingControl::Color {
                default_value: "#123456".into(),
                allow_alpha: false,
            },
            SettingControl::Number {
                default_value: 3.0,
                min: 1.0,
                max: 5.0,
                step: 1.0,
            },
            SettingControl::Shortcut {
                default_value: vec!["Ctrl".into(), "K".into()],
            },
            SettingControl::Action,
            SettingControl::Group {
                fields: fields.clone(),
            },
            SettingControl::Repeated {
                fields,
                default_value: vec![BTreeMap::from([(
                    "name".into(),
                    Value::String("one".into()),
                )])],
                min_items: 0,
                max_items: 5,
            },
        ];
        for (index, control) in controls.into_iter().enumerate() {
            let mut registration = switch(&format!("control{index}"), "Test", 0, index as i32);
            registration.control = control;
            validate_provider("org.nickel.test", &[registration], &[]).unwrap();
        }
    }

    #[test]
    fn rejects_bad_numeric_select_and_repeated_definitions() {
        assert!(
            validate_control(
                &SettingControl::Number {
                    default_value: 2.0,
                    min: 0.0,
                    max: 1.0,
                    step: 0.0
                },
                0
            )
            .is_err()
        );
        assert!(
            validate_control(
                &SettingControl::Select {
                    default_value: "missing".into(),
                    options: vec![SettingOption {
                        value: "present".into(),
                        label: "Present".into()
                    }]
                },
                0
            )
            .is_err()
        );
        assert!(
            validate_control(
                &SettingControl::Repeated {
                    fields: vec![SettingField {
                        id: "known".into(),
                        label: "Known".into(),
                        description: String::new(),
                        order: 0,
                        control: SettingControl::Switch {
                            default_value: false
                        }
                    }],
                    default_value: vec![BTreeMap::from([("unknown".into(), Value::Bool(true))])],
                    min_items: 0,
                    max_items: 1
                },
                0
            )
            .is_err()
        );
    }

    #[test]
    fn json_schema_uses_javascript_friendly_names() {
        let registration = switch("vpn.autoConnect", "VPN", 3, 4);
        let json = serde_json::to_value(&registration).unwrap();
        assert_eq!(json["id"], "vpn.autoConnect");
        assert_eq!(json["groupOrder"], 3);
        assert_eq!(json["requiredCapabilities"][0], "settings-read");
        assert_eq!(json["type"], "switch");
        assert_eq!(json["defaultValue"], false);
        assert_eq!(
            serde_json::from_value::<SettingRegistration>(json).unwrap(),
            registration
        );
    }
}
