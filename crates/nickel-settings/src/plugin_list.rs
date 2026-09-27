//! JSX-owned ordinary plugin list. Permission approval remains a native overlay.

use nickel_plugin_runtime::JsxRuntime;
use nickel_session_protocol::{
    PluginMemorySnapshot, PluginRuntimeHealth, PluginSettingKind, PluginStatusSnapshot,
};
use nickel_ui::{
    AnyView, Button, ButtonPresentation, Column, Container, Row, SemanticTheme, SettingsCard,
    SettingsRow, Switch, SwitchState, TextField,
};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage};

#[derive(Clone, Debug)]
enum Node {
    Stack(Vec<Node>),
    Fragment(Vec<Node>),
    Card {
        label: String,
        value: String,
        children: Vec<Node>,
    },
    Row {
        label: String,
        value: String,
        trailing: Option<Box<Node>>,
    },
    Inline(Vec<Node>),
    Button {
        label: String,
        style: String,
        action: Option<usize>,
    },
    Switch {
        id: String,
        label: String,
        state: SwitchState,
        action: Option<usize>,
    },
    Input {
        id: String,
        value: String,
        action: usize,
    },
}

impl Node {
    fn parse(value: &Value) -> Result<Self, String> {
        Self::parse_bounded(value, 0)
    }

    fn parse_bounded(value: &Value, depth: usize) -> Result<Self, String> {
        if depth > 8 {
            return Err("Settings plugin tree is too deep".into());
        }
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("Settings node has no kind")?;
        let children = || -> Result<Vec<Self>, String> {
            let values = value
                .get("children")
                .and_then(Value::as_array)
                .ok_or("Settings node has no children")?;
            if values.len() > 512 {
                return Err("Settings node has too many children".into());
            }
            values
                .iter()
                .map(|child| Self::parse_bounded(child, depth + 1))
                .collect()
        };
        let action = || -> Result<Option<usize>, String> {
            value
                .get("action")
                .filter(|value| !value.is_null())
                .map(|value| {
                    value
                        .as_u64()
                        .and_then(|number| usize::try_from(number).ok())
                        .ok_or("Settings action is invalid".into())
                })
                .transpose()
        };
        Ok(match kind {
            "settings-stack" => Self::Stack(children()?),
            "settings-fragment" => Self::Fragment(children()?),
            "settings-card" => Self::Card {
                label: text(value, "label", 256)?,
                value: text(value, "value", 65_535)?,
                children: children()?,
            },
            "settings-row" => {
                let mut children = children()?;
                if children.len() > 1 {
                    return Err("Settings row may have one control".into());
                }
                Self::Row {
                    label: text(value, "label", 256)?,
                    value: text(value, "value", 65_535)?,
                    trailing: children.pop().map(Box::new),
                }
            }
            "settings-inline" => Self::Inline(children()?),
            "settings-button" => Self::Button {
                label: text(value, "label", 256)?,
                style: text(value, "value", 24)?,
                action: action()?,
            },
            "settings-switch" => {
                let state = match text(value, "value", 24)?.as_str() {
                    "on" => SwitchState::On,
                    "off" => SwitchState::Off,
                    "disabled-on" => SwitchState::DisabledOn,
                    "disabled-off" => SwitchState::DisabledOff,
                    "mixed" => SwitchState::Mixed,
                    _ => return Err("Settings switch state is invalid".into()),
                };
                Self::Switch {
                    id: text(value, "id", 256)?,
                    label: text(value, "label", 256)?,
                    state,
                    action: action()?,
                }
            }
            "settings-input" => Self::Input {
                id: text(value, "id", 256)?,
                value: text(value, "value", 65_535)?,
                action: action()?.ok_or("Settings input requires onChange")?,
            },
            _ => return Err(format!("Unknown Settings component {kind:?}")),
        })
    }

    fn view(&self, theme: SemanticTheme) -> AnyView<SettingsMessage> {
        match self {
            Self::Stack(children) => AnyView::new(
                Column::new().fill_width().gap(16.0).children(
                    children
                        .iter()
                        .map(|child| child.view(theme))
                        .collect::<Vec<_>>(),
                ),
            ),
            Self::Fragment(children) => AnyView::new(
                Column::new().fill_width().gap(8.0).children(
                    children
                        .iter()
                        .map(|child| child.view(theme))
                        .collect::<Vec<_>>(),
                ),
            ),
            Self::Card {
                label,
                value,
                children,
            } => AnyView::new(
                SettingsCard::titled(theme, label, value).children(
                    children
                        .iter()
                        .map(|child| child.view(theme))
                        .collect::<Vec<_>>(),
                ),
            ),
            Self::Row {
                label,
                value,
                trailing,
            } => {
                let row = SettingsRow::new(theme, label, value);
                AnyView::new(if let Some(trailing) = trailing {
                    row.trailing(trailing.view(theme))
                } else {
                    row
                })
            }
            Self::Inline(children) => AnyView::new(
                Row::new().gap(8.0).children(
                    children
                        .iter()
                        .map(|child| child.view(theme))
                        .collect::<Vec<_>>(),
                ),
            ),
            Self::Button {
                label,
                style,
                action,
            } => {
                let presentation = match style.as_str() {
                    "primary" => ButtonPresentation::Primary,
                    "secondary" => ButtonPresentation::Secondary,
                    "quiet" => ButtonPresentation::Quiet,
                    _ => ButtonPresentation::Disabled,
                };
                AnyView::new(Button::semantic(
                    theme,
                    SettingsMessage::PluginJsxAction(action.unwrap_or(usize::MAX)),
                    label,
                    if action.is_some() {
                        presentation
                    } else {
                        ButtonPresentation::Disabled
                    },
                ))
            }
            Self::Switch {
                id,
                label,
                state,
                action,
            } => AnyView::new(
                Switch::with_state_action(
                    *state,
                    action.map(SettingsMessage::PluginJsxAction),
                    theme,
                )
                .id(id.as_str())
                .accessibility_label(label),
            ),
            Self::Input { id, value, action } => {
                let action = *action;
                AnyView::new(
                    Container::new().width(260.0).child(
                        TextField::on_change_with_placeholder_mapped(
                            value,
                            "Value",
                            move |value| SettingsMessage::PluginJsxInput(action, value),
                        )
                        .id(id.as_str()),
                    ),
                )
            }
        }
    }

    #[cfg(test)]
    fn action_for_id(&self, target: &str) -> Option<usize> {
        match self {
            Self::Switch { id, action, .. } if id == target => *action,
            Self::Input { id, action, .. } if id == target => Some(*action),
            Self::Stack(children) | Self::Fragment(children) | Self::Inline(children) => children
                .iter()
                .find_map(|child| child.action_for_id(target)),
            Self::Card { children, .. } => children
                .iter()
                .find_map(|child| child.action_for_id(target)),
            Self::Row { trailing, .. } => trailing
                .as_deref()
                .and_then(|child| child.action_for_id(target)),
            _ => None,
        }
    }
}

fn text(value: &Value, key: &str, limit: usize) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Settings node is missing {key}"))?;
    if text.chars().count() > limit {
        return Err(format!("Settings node {key} is too long"));
    }
    Ok(text.to_owned())
}

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
    runtime: JsxRuntime,
    node: Option<Node>,
    last_data: Option<String>,
}

impl PluginList {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(
                include_str!("../../../assets/plugin-runtime/settings-plugins.js"),
                None,
            )?,
            node: None,
            last_data: None,
        })
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let data = serde_json::to_string(data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&data) {
            self.runtime.set_data(&data)?;
            self.node = Some(self.runtime.render("__nickelRender()", Node::parse)?);
            self.last_data = Some(data);
        }
        Ok(self
            .node
            .as_ref()
            .ok_or("Settings plugin list is unavailable")?
            .view(theme))
    }

    pub(super) fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        current_data: &Value,
        validate: impl FnOnce(&PluginRequest) -> Result<SettingsMessage, String>,
    ) -> Result<SettingsMessage, String> {
        let data = serde_json::to_string(current_data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&data) {
            return Err("Plugin status changed; refresh the page".into());
        }
        let expression = format!("__nickelDispatch({index},{value})");
        let rendered = self.runtime.render(&expression, Node::parse);
        let requested = if rendered.is_ok() {
            self.runtime.take_effects()
        } else {
            Ok(Vec::new())
        };
        let result: Result<(Node, SettingsMessage), String> = (|| {
            let node = rendered?;
            let mut requested = requested?;
            if requested.len() != 1 {
                return Err("Plugin action must request exactly one operation".into());
            }
            let request: PluginRequest =
                serde_json::from_value(requested.remove(0)).map_err(|error| error.to_string())?;
            let message = validate(&request)?;
            Ok((node, message))
        })();
        self.runtime.finish_event(result.is_ok())?;
        let (node, message) = result?;
        self.node = Some(node);
        Ok(message)
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.node.as_ref()?.action_for_id(id)
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
            PluginRuntimeHealth::Starting => "Starting".to_owned(),
            PluginRuntimeHealth::Running => "Running".to_owned(),
            PluginRuntimeHealth::Failed(reason) => format!("Failed: {reason}"),
        };
        let switch_state = if pending {
            if plugin.desired_enabled { "disabled-on" } else { "disabled-off" }
        } else if plugin.desired_enabled {
            match &plugin.health { PluginRuntimeHealth::Running => "on", PluginRuntimeHealth::Starting => "disabled-on", _ => "mixed" }
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
                "id":setting.id,"label":setting.label,"description":setting.description,
                "kind":setting.kind,"value":setting.value,"displayValue":display_value,
                "pending":setting_pending.is_some_and(|(id,key,_)| id == &plugin.id && key == &setting.id),
                "editing":editing,"draft":edit.filter(|_| editing).map(|(_,_,draft)| draft),
            })
        }).collect::<Vec<_>>();
        let overlap = plugin.composition.iter().any(|entry| entry.starts_with("add ") || entry.starts_with("replace "));
        json!({
            "id":plugin.id,"name":plugin.name,"author":plugin.author.as_deref().unwrap_or("Unknown"),
            "version":plugin.version.as_deref().unwrap_or("Unspecified"),
            "desiredEnabled":plugin.desired_enabled,"health":health,"switchState":switch_state,
            "pending":pending,
            "access":if plugin.capabilities.is_empty() { "None".into() } else { plugin.capabilities.join(", ") },
            "surfaces":if plugin.surfaces.is_empty() { "None".into() } else { plugin.surfaces.join(", ") },
            "composition":if plugin.composition.is_empty() { "None".into() } else { plugin.composition.join(", ") },
            "memory":memory_projection(&plugin.memory,overlap), "settings":settings,
        })
    }).collect::<Vec<_>>()).unwrap_or_default();
    json!({"available":snapshot.is_some(),"generation":snapshot.map(|snapshot| snapshot.activation_generation),"notice":notice,"plugins":plugins})
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
        let data = projection(
            self.plugin_status.as_ref(),
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
    fn jsx_enable_action_routes_through_trusted_review() {
        let snapshot = snapshot();
        let data = projection(Some(&snapshot), None, None, None, None);
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
        let stale_data = projection(Some(&stale), None, None, None, None);
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
        assert!(Node::parse(&json!({"kind":"host-command","children":[]})).is_err());
    }
}
