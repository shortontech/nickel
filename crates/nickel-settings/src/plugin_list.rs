//! JSX-owned ordinary plugin list. Permission approval remains a native overlay.

use nickel_i18n::Localizer;
use nickel_plugin_presentation::{components::PluginImages, page::JsxPage};
use nickel_session_protocol::{
    PluginMemorySnapshot, PluginRuntimeHealth, PluginSettingKind, PluginStatusSnapshot,
};
use nickel_ui::{AnyView, SemanticTheme};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage, settings_plugin::StyledSettingsPage};

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum PluginRequest {
    Refresh,
    ReviewEnable {
        id: String,
    },
    Disable {
        id: String,
    },
    SetSetting {
        id: String,
        key: String,
        value: Value,
    },
    StepSetting {
        id: String,
        key: String,
        direction: String,
    },
    EditText {
        id: String,
        key: String,
    },
    TextChanged {
        value: String,
    },
    SaveText,
    CancelText,
}

pub(super) struct PluginList {
    page: StyledSettingsPage,
}

impl PluginList {
    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes()
    }

    #[cfg(test)]
    pub(super) fn new() -> Result<Self, String> {
        Self::new_with_page(JsxPage::new(
            crate::settings_package::source(crate::settings_package::Script::Plugins)?,
            crate::settings_package::manifest()?.clone(),
            None,
        )?)
    }

    pub(super) fn new_with_page(page: JsxPage) -> Result<Self, String> {
        Ok(Self {
            page: StyledSettingsPage::new(page),
        })
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let (node, stylesheet) = self.page.render(
            data,
            theme,
            include_str!("../../../assets/plugins/settings/settings-plugins.css"),
        )?;
        Ok(node.view_as::<SettingsMessage>(&PluginImages::new(), stylesheet))
    }

    pub(super) fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        current_data: &Value,
        validate: impl FnOnce(&PluginRequest) -> Result<SettingsMessage, String>,
    ) -> Result<SettingsMessage, String> {
        self.page.dispatch(index, &value, current_data, |effect| {
            let request: PluginRequest =
                serde_json::from_value(effect).map_err(|error| error.to_string())?;
            validate(&request)
        })
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.page.node()?.button_action(id)
    }
}

fn memory_label(bytes: Option<u64>) -> String {
    match bytes {
        None => "Unavailable".into(),
        Some(bytes) if bytes < 1024 => format!("{bytes} B"),
        Some(bytes) => format!("{} KiB", bytes.div_ceil(1024)),
    }
}

fn memory_projection(memory: &PluginMemorySnapshot, overlap: bool) -> Value {
    let measured = [
        memory.js_heap_bytes,
        memory.native_ui_bytes,
        memory.texture_bytes,
    ];
    let tracked = measured.iter().any(Option::is_some).then(|| {
        measured
            .into_iter()
            .flatten()
            .fold(0_u64, u64::saturating_add)
    });
    json!({
        "tracked": memory_label(tracked), "peak": memory_label(memory.tracked_peak_bytes),
        "js": memory_label(memory.js_heap_bytes), "native": memory_label(memory.native_ui_bytes),
        "textures": memory_label(memory.texture_bytes),
        "timers": format!("{} / {}", memory.timers, memory.subscriptions), "overlap": overlap,
    })
}

pub(super) fn projection(
    localizer: &Localizer,
    snapshot: Option<&PluginStatusSnapshot>,
    notice: Option<&str>,
    pending: Option<&(String, bool)>,
    setting_pending: Option<&(String, String, Value)>,
    edit: Option<&(String, String, String)>,
) -> Value {
    let plugins = snapshot.map(|snapshot| snapshot.plugins.iter().map(|plugin| {
        let pending = pending.is_some_and(|(id, _)| id == &plugin.id);
        let health = match &plugin.health {
            PluginRuntimeHealth::Disabled => "Disabled".to_owned(),
            PluginRuntimeHealth::Idle => "Enabled, closed".to_owned(),
            PluginRuntimeHealth::Starting => "Starting".to_owned(),
            PluginRuntimeHealth::Running => "Running".to_owned(),
            PluginRuntimeHealth::Failed(reason) => format!("Failed: {reason}"),
        };
        let switch_state = if pending {
            if plugin.desired_enabled { "disabled-on" } else { "disabled-off" }
        } else if plugin.desired_enabled {
            match &plugin.health { PluginRuntimeHealth::Running | PluginRuntimeHealth::Idle => "on", PluginRuntimeHealth::Starting => "disabled-on", _ => "mixed" }
        } else { "off" };
        let settings = plugin.settings.iter().map(|setting| {
            let display_value = match &setting.value {
                Value::Bool(value) => if *value { "On".into() } else { "Off".into() },
                Value::Number(value) => value.to_string(),
                Value::String(value) => value.clone(),
                _ => "Unavailable".into(),
            };
            let editing = edit.is_some_and(|(id,key,_)| id == &plugin.id && key == &setting.id);
            json!({
                "id":setting.id,"label":setting.label,
                "displayLabel":localizer.value("settings-plugin-setting-name", "name", &setting.label),
                "description":setting.description,
                "kind":setting.kind,"value":setting.value,"displayValue":display_value,
                "pending":setting_pending.is_some_and(|(id,key,_)| id == &plugin.id && key == &setting.id),
                "editing":editing,"draft":edit.filter(|_| editing).map(|(_,_,draft)| draft),
            })
        }).collect::<Vec<_>>();
        let overlap = plugin.composition.iter().any(|entry| entry.starts_with("add ") || entry.starts_with("replace "));
        json!({
            "id":plugin.id,"name":plugin.name,"author":plugin.author.as_deref().unwrap_or("Unknown"),
            "version":plugin.version.as_deref().unwrap_or("Unspecified"),
            "toggleLabel":localizer.value(
                if plugin.desired_enabled { "settings-plugin-disable-name" } else { "settings-plugin-enable-name" },
                "name", &plugin.name,
            ),
            "desiredEnabled":plugin.desired_enabled,"health":health,"switchState":switch_state,
            "pending":pending,
            "access":if plugin.capabilities.is_empty() { "None".into() } else { plugin.capabilities.join(", ") },
            "surfaces":if plugin.surfaces.is_empty() { "None".into() } else { plugin.surfaces.join(", ") },
            "composition":if plugin.composition.is_empty() { "None".into() } else { plugin.composition.join(", ") },
            "memory":memory_projection(&plugin.memory,overlap), "settings":settings,
        })
    }).collect::<Vec<_>>()).unwrap_or_default();
    json!({
        "available":snapshot.is_some(),
        "generation":snapshot.map(|snapshot| snapshot.activation_generation),
        "notice":notice,
        "plugins":plugins,
        "inputPlaceholder":localizer.text("settings-plugin-input-placeholder"),
        "labels":{
            "change":localizer.text("settings-plugin-change"),
            "edit":localizer.text("settings-plugin-edit"),
            "save":localizer.text("settings-plugin-save"),
            "cancel":localizer.text("settings-plugin-cancel"),
            "publisher":localizer.text("settings-plugin-publisher"),
            "version":localizer.text("settings-plugin-version"),
            "enabled":localizer.text("settings-plugin-enabled"),
            "access":localizer.text("settings-plugin-access"),
            "surfaces":localizer.text("settings-plugin-surfaces"),
            "composition":localizer.text("settings-plugin-composition"),
            "trackedMemory":localizer.text("settings-plugin-tracked-memory"),
            "peakMemory":localizer.text("settings-plugin-peak-memory"),
            "jsHeap":localizer.text("settings-plugin-js-heap"),
            "nativeUi":localizer.text("settings-plugin-native-ui"),
            "textures":localizer.text("settings-plugin-textures"),
            "timers":localizer.text("settings-plugin-timers"),
            "memoryAttribution":localizer.text("settings-plugin-memory-attribution"),
            "extensionOverlap":localizer.text("settings-plugin-extension-overlap"),
            "pluginStatus":localizer.text("settings-plugin-status"),
            "waiting":localizer.text("settings-plugin-waiting"),
            "statusUnavailable":localizer.text("settings-plugin-status-unavailable"),
            "refresh":localizer.text("settings-plugin-refresh"),
        }
    })
}

pub(super) fn validate_request(
    request: &PluginRequest,
    snapshot: Option<&PluginStatusSnapshot>,
    pending: Option<&(String, bool)>,
    setting_pending: Option<&(String, String, Value)>,
    edit: Option<&(String, String, String)>,
) -> Result<SettingsMessage, String> {
    if matches!(request, PluginRequest::Refresh) {
        return Ok(SettingsMessage::RefreshPlugins);
    }
    let snapshot = snapshot.ok_or("Plugin status is unavailable")?;
    match request {
        PluginRequest::ReviewEnable { id } => {
            if pending.is_some()
                || !snapshot
                    .plugins
                    .iter()
                    .any(|plugin| plugin.id == *id && !plugin.desired_enabled)
            {
                return Err("Plugin cannot be enabled from this status".into());
            }
            Ok(SettingsMessage::ReviewPluginEnable(id.clone()))
        }
        PluginRequest::Disable { id } => {
            if pending.is_some()
                || !snapshot
                    .plugins
                    .iter()
                    .any(|plugin| plugin.id == *id && plugin.desired_enabled)
            {
                return Err("Plugin cannot be disabled from this status".into());
            }
            Ok(SettingsMessage::SetPluginEnabled {
                id: id.clone(),
                enabled: false,
            })
        }
        PluginRequest::SetSetting { id, key, value } => {
            if setting_pending.is_some() || pending.is_some() {
                return Err("Plugin setting is busy".into());
            }
            let setting = snapshot
                .plugins
                .iter()
                .find(|plugin| plugin.id == *id)
                .and_then(|plugin| plugin.settings.iter().find(|setting| setting.id == *key))
                .ok_or("Plugin setting is unavailable")?;
            if !setting.kind.accepts(value) {
                return Err("Plugin setting value is invalid".into());
            }
            if !valid_next_setting_value(&setting.kind, &setting.value, value) {
                return Err("Plugin setting value is stale".into());
            }
            Ok(SettingsMessage::SetPluginSetting {
                id: id.clone(),
                key: key.clone(),
                value: value.clone(),
            })
        }
        PluginRequest::StepSetting { id, key, direction } => {
            if setting_pending.is_some() || pending.is_some() {
                return Err("Plugin setting is busy".into());
            }
            let setting = snapshot
                .plugins
                .iter()
                .find(|plugin| plugin.id == *id)
                .and_then(|plugin| plugin.settings.iter().find(|setting| setting.id == *key))
                .ok_or("Plugin setting is unavailable")?;
            let PluginSettingKind::Integer { min, max } = &setting.kind else {
                return Err("Plugin setting is not an integer".into());
            };
            let current = setting
                .value
                .as_i64()
                .ok_or("Plugin integer value is unavailable")?;
            let value = match direction.as_str() {
                "decrement" => current.saturating_sub(1).max(*min),
                "increment" => current.saturating_add(1).min(*max),
                _ => return Err("Plugin setting direction is invalid".into()),
            };
            Ok(SettingsMessage::SetPluginSetting {
                id: id.clone(),
                key: key.clone(),
                value: Value::from(value),
            })
        }
        PluginRequest::EditText { id, key } => {
            let text = snapshot
                .plugins
                .iter()
                .find(|plugin| plugin.id == *id)
                .and_then(|plugin| plugin.settings.iter().find(|setting| setting.id == *key))
                .ok_or("Plugin setting is unavailable")?;
            if !matches!(text.kind, PluginSettingKind::Text { .. }) {
                return Err("Plugin setting is not text".into());
            }
            Ok(SettingsMessage::EditPluginTextSetting {
                id: id.clone(),
                key: key.clone(),
            })
        }
        PluginRequest::TextChanged { value } => {
            if edit.is_none() || value.chars().count() > 65_535 {
                return Err("Plugin text edit is unavailable".into());
            }
            Ok(SettingsMessage::PluginTextSettingChanged(value.clone()))
        }
        PluginRequest::SaveText => {
            if edit.is_none() {
                return Err("Plugin text edit is unavailable".into());
            }
            Ok(SettingsMessage::SavePluginTextSetting)
        }
        PluginRequest::CancelText => {
            if edit.is_none() {
                return Err("Plugin text edit is unavailable".into());
            }
            Ok(SettingsMessage::CancelPluginTextSetting)
        }
        PluginRequest::Refresh => unreachable!(),
    }
}

fn valid_next_setting_value(kind: &PluginSettingKind, current: &Value, next: &Value) -> bool {
    match kind {
        PluginSettingKind::Boolean => current
            .as_bool()
            .is_some_and(|current| next.as_bool() == Some(!current)),
        PluginSettingKind::Integer { min, max } => current.as_i64().is_some_and(|current| {
            let next = next.as_i64();
            next == Some(current.saturating_sub(1).max(*min))
                || next == Some(current.saturating_add(1).min(*max))
        }),
        PluginSettingKind::Choice { options } => {
            if options.is_empty() {
                return false;
            }
            let index = options
                .iter()
                .position(|option| current.as_str() == Some(option));
            options
                .get(index.map_or(0, |index| (index + 1) % options.len()))
                .is_some_and(|expected| next.as_str() == Some(expected))
        }
        PluginSettingKind::Text { .. } => next.is_string(),
    }
}

impl SettingsApp {
    pub(super) fn handle_plugin_jsx_action(&mut self, index: usize, value: Value) {
        let mut current_status = self.plugin_status.clone();
        if let (Some(snapshot), Some(memory)) = (
            current_status.as_mut(),
            self.settings_jsx_displayed_memory.borrow().as_ref(),
        ) && let Some(settings) = snapshot
            .plugins
            .iter_mut()
            .find(|plugin| plugin.id == crate::settings_package::ID)
        {
            settings.memory = memory.clone();
        }
        let data = projection(
            &self.localizer,
            current_status.as_ref(),
            self.plugin_notice.as_deref(),
            self.plugin_pending.as_ref(),
            self.plugin_setting_pending.as_ref(),
            self.plugin_setting_edit.as_ref(),
        );
        let result = self
            .plugin_list
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "Plugin list is not loaded".to_owned())
            .and_then(|list| list.as_mut().map_err(|error| error.clone()))
            .and_then(|list| {
                list.dispatch(index, value, &data, |request| {
                    validate_request(
                        request,
                        self.plugin_status.as_ref(),
                        self.plugin_pending.as_ref(),
                        self.plugin_setting_pending.as_ref(),
                        self.plugin_setting_edit.as_ref(),
                    )
                })
            });
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => self.plugin_notice = Some(error),
        }
        self.request_redraw();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_session_protocol::{PluginSettingStatus, PluginStatus};

    fn snapshot() -> PluginStatusSnapshot {
        PluginStatusSnapshot {
            activation_generation: 7,
            plugins: vec![PluginStatus {
                id: "example.plugin".into(),
                name: "Example".into(),
                author: None,
                version: None,
                desired_enabled: false,
                health: PluginRuntimeHealth::Disabled,
                capabilities: Vec::new(),
                surfaces: Vec::new(),
                composition: Vec::new(),
                settings: vec![PluginSettingStatus {
                    id: "enabled".into(),
                    label: "Enabled".into(),
                    description: String::new(),
                    kind: PluginSettingKind::Boolean,
                    value: Value::Bool(true),
                }],
                memory: PluginMemorySnapshot::default(),
            }],
        }
    }

    #[test]
    fn idle_settings_plugin_remains_enabled_in_the_list() {
        let mut snapshot = snapshot();
        snapshot.plugins[0].id = "org.nickel.settings".into();
        snapshot.plugins[0].desired_enabled = true;
        snapshot.plugins[0].health = PluginRuntimeHealth::Idle;
        let data = projection(
            &Localizer::system(),
            Some(&snapshot),
            None,
            None,
            None,
            None,
        );
        let plugin = &data["plugins"][0];
        assert_eq!(plugin["health"], "Enabled, closed");
        assert_eq!(plugin["switchState"], "on");
    }

    #[test]
    fn jsx_enable_action_routes_through_trusted_review() {
        let snapshot = snapshot();
        let localizer = Localizer::system();
        let data = projection(&localizer, Some(&snapshot), None, None, None, None);
        let mut list = PluginList::new().unwrap();
        let theme = crate::semantic_theme(nickel_core::theme::ThemePalette::from_appearance(
            nickel_core::theme::Appearance::default(),
        ));
        list.render(&data, theme).unwrap();
        let action = list.action_for_id("plugin-enable-example.plugin").unwrap();
        let message = list
            .dispatch(action, Value::Null, &data, |request| {
                validate_request(request, Some(&snapshot), None, None, None)
            })
            .unwrap();
        assert_eq!(
            message,
            SettingsMessage::ReviewPluginEnable("example.plugin".into())
        );
        let mut stale = snapshot.clone();
        stale.activation_generation += 1;
        let stale_data = projection(&localizer, Some(&stale), None, None, None, None);
        assert!(
            list.dispatch(action, Value::Null, &stale_data, |_| {
                Ok(SettingsMessage::RefreshPlugins)
            })
            .is_err()
        );
    }

    #[test]
    fn setting_requests_are_checked_against_the_current_value() {
        let snapshot = snapshot();
        let current = Some(&snapshot);
        assert!(
            validate_request(
                &PluginRequest::SetSetting {
                    id: "example.plugin".into(),
                    key: "enabled".into(),
                    value: Value::Bool(true),
                },
                current,
                None,
                None,
                None
            )
            .is_err()
        );
        assert_eq!(
            validate_request(
                &PluginRequest::SetSetting {
                    id: "example.plugin".into(),
                    key: "enabled".into(),
                    value: Value::Bool(false),
                },
                current,
                None,
                None,
                None
            )
            .unwrap(),
            SettingsMessage::SetPluginSetting {
                id: "example.plugin".into(),
                key: "enabled".into(),
                value: Value::Bool(false),
            }
        );
    }
}
