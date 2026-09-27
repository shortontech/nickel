//! Shared native component adapter for Settings JSX pages.

use nickel_plugin_runtime::JsxRuntime;
use nickel_ui::{
    AnyView, Button, ButtonPresentation, Column, Container, Grid, RadioGroup, RadioOption, Row,
    SemanticTheme, SettingsCard, SettingsRow, Switch, SwitchState, TextField, Track,
};
use serde_json::Value;

use crate::SettingsMessage;

#[derive(Clone, Debug)]
pub(super) enum Node {
    Stack(Vec<Node>),
    CompactList(Vec<Node>),
    Grid(Vec<Node>),
    Fragment(Vec<Node>),
    Card {
        label: String,
        value: String,
        children: Vec<Node>,
    },
    Row {
        label: String,
        value: String,
        compact: bool,
        trailing: Option<Box<Node>>,
    },
    Inline(Vec<Node>),
    Button {
        id: Option<String>,
        label: String,
        accessibility_label: Option<String>,
        state: Option<String>,
        style: String,
        action: Option<usize>,
    },
    Switch {
        id: String,
        label: String,
        state: SwitchState,
        action: Option<usize>,
    },
    RadioGroup {
        id: String,
        options: Vec<Radio>,
    },
    Input {
        id: String,
        value: String,
        action: usize,
    },
}

#[derive(Clone, Debug)]
pub(super) struct Radio {
    id: String,
    label: String,
    description: String,
    selected: bool,
    action: Option<usize>,
}

impl Node {
    /// Lower bound for this parsed component tree's owned Rust allocations.
    /// Boa's heap and the shared resolved UI frame are deliberately excluded.
    pub(super) fn retained_bytes(&self) -> usize {
        std::mem::size_of::<Self>() + self.heap_bytes()
    }

    fn heap_bytes(&self) -> usize {
        let children_bytes = |children: &Vec<Self>| {
            children.capacity() * std::mem::size_of::<Self>()
                + children.iter().map(Self::heap_bytes).sum::<usize>()
        };
        match self {
            Self::Stack(children)
            | Self::CompactList(children)
            | Self::Grid(children)
            | Self::Fragment(children)
            | Self::Inline(children) => children_bytes(children),
            Self::Card {
                label,
                value,
                children,
            } => label.capacity() + value.capacity() + children_bytes(children),
            Self::Row {
                label,
                value,
                compact: _,
                trailing,
            } => {
                label.capacity()
                    + value.capacity()
                    + trailing
                        .as_ref()
                        .map_or(0, |node| std::mem::size_of::<Self>() + node.heap_bytes())
            }
            Self::Button {
                id,
                label,
                accessibility_label,
                state,
                style,
                ..
            } => {
                id.as_ref().map_or(0, String::capacity)
                    + label.capacity()
                    + accessibility_label.as_ref().map_or(0, String::capacity)
                    + state.as_ref().map_or(0, String::capacity)
                    + style.capacity()
            }
            Self::Switch { id, label, .. } => id.capacity() + label.capacity(),
            Self::RadioGroup { id, options } => {
                id.capacity()
                    + options.capacity() * std::mem::size_of::<Radio>()
                    + options
                        .iter()
                        .map(|option| {
                            option.id.capacity()
                                + option.label.capacity()
                                + option.description.capacity()
                        })
                        .sum::<usize>()
            }
            Self::Input { id, value, .. } => id.capacity() + value.capacity(),
        }
    }

    pub(super) fn parse(value: &Value) -> Result<Self, String> {
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
            "settings-stack" | "settings-features" => Self::Stack(children()?),
            "settings-compact-list" => Self::CompactList(children()?),
            "settings-grid" => Self::Grid(children()?),
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
                    compact: value
                        .get("compact")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    trailing: children.pop().map(Box::new),
                }
            }
            "settings-inline" => Self::Inline(children()?),
            "settings-button" => Self::Button {
                id: value
                    .get("id")
                    .map(|_| text(value, "id", 256))
                    .transpose()?,
                label: text(value, "label", 256)?,
                accessibility_label: value
                    .get("accessibilityLabel")
                    .map(|_| text(value, "accessibilityLabel", 512))
                    .transpose()?,
                state: value
                    .get("state")
                    .map(|_| text(value, "state", 64))
                    .transpose()?,
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
                    "mixed-unavailable" => SwitchState::MixedUnavailable,
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
            "settings-radio-group" => {
                let options = value
                    .get("children")
                    .and_then(Value::as_array)
                    .ok_or("Settings radio group has no options")?;
                if options.is_empty() || options.len() > 8 {
                    return Err("Settings radio group size is invalid".into());
                }
                let options = options
                    .iter()
                    .map(|option| {
                        if option.get("kind").and_then(Value::as_str) != Some("settings-radio") {
                            return Err("Settings radio option is invalid".into());
                        }
                        Ok(Radio {
                            id: text(option, "id", 256)?,
                            label: text(option, "label", 256)?,
                            description: text(option, "value", 256)?,
                            selected: option
                                .get("selected")
                                .and_then(Value::as_bool)
                                .ok_or("Settings radio selection is invalid")?,
                            action: option
                                .get("action")
                                .filter(|value| !value.is_null())
                                .map(|value| {
                                    value
                                        .as_u64()
                                        .and_then(|number| usize::try_from(number).ok())
                                        .ok_or("Settings radio action is invalid".to_owned())
                                })
                                .transpose()?,
                        })
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                if options.iter().filter(|option| option.selected).count() != 1 {
                    return Err("Settings radio group needs one selected option".into());
                }
                Self::RadioGroup {
                    id: text(value, "id", 256)?,
                    options,
                }
            }
            _ => return Err(format!("Unknown Settings component {kind:?}")),
        })
    }

    pub(super) fn view(
        &self,
        theme: SemanticTheme,
        input_placeholder: &str,
        action_message: fn(usize) -> SettingsMessage,
    ) -> AnyView<SettingsMessage> {
        self.view_with_input(
            theme,
            input_placeholder,
            action_message,
            SettingsMessage::PluginJsxInput,
        )
    }

    pub(super) fn view_with_input(
        &self,
        theme: SemanticTheme,
        input_placeholder: &str,
        action_message: fn(usize) -> SettingsMessage,
        input_message: fn(usize, String) -> SettingsMessage,
    ) -> AnyView<SettingsMessage> {
        match self {
            Self::Stack(children) => AnyView::new(
                Column::new().fill_width().gap(16.0).children(
                    children
                        .iter()
                        .map(|child| {
                            child.view_with_input(
                                theme,
                                input_placeholder,
                                action_message,
                                input_message,
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
            Self::CompactList(children) => AnyView::new(
                Column::new().fill_width().gap(0.0).children(
                    children
                        .iter()
                        .map(|child| {
                            child.view_with_input(
                                theme,
                                input_placeholder,
                                action_message,
                                input_message,
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
            Self::Grid(children) => AnyView::new(
                Grid::auto_fit(Track::minmax(Track::px(110.0), Track::fr(1.0)))
                    .gap(4.0)
                    .children(
                        children
                            .iter()
                            .map(|child| {
                                child.view_with_input(
                                    theme,
                                    input_placeholder,
                                    action_message,
                                    input_message,
                                )
                            })
                            .collect::<Vec<_>>(),
                    ),
            ),
            Self::Fragment(children) => AnyView::new(
                Column::new().fill_width().gap(8.0).children(
                    children
                        .iter()
                        .map(|child| {
                            child.view_with_input(
                                theme,
                                input_placeholder,
                                action_message,
                                input_message,
                            )
                        })
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
                        .map(|child| {
                            child.view_with_input(
                                theme,
                                input_placeholder,
                                action_message,
                                input_message,
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
            Self::Row {
                label,
                value,
                compact,
                trailing,
            } => {
                let row = SettingsRow::new(theme, label, value);
                let row = if *compact { row.compact() } else { row };
                AnyView::new(if let Some(trailing) = trailing {
                    row.trailing(trailing.view_with_input(
                        theme,
                        input_placeholder,
                        action_message,
                        input_message,
                    ))
                } else {
                    row
                })
            }
            Self::Inline(children) => AnyView::new(
                Row::new().gap(8.0).children(
                    children
                        .iter()
                        .map(|child| {
                            child.view_with_input(
                                theme,
                                input_placeholder,
                                action_message,
                                input_message,
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
            ),
            Self::Button {
                id,
                label,
                accessibility_label,
                state,
                style,
                action,
            } => {
                let presentation = match style.as_str() {
                    "primary" => ButtonPresentation::Primary,
                    "secondary" => ButtonPresentation::Secondary,
                    "quiet" => ButtonPresentation::Quiet,
                    _ => ButtonPresentation::Disabled,
                };
                let button = Button::semantic(
                    theme,
                    action_message(action.unwrap_or(usize::MAX)),
                    label,
                    if action.is_some() {
                        presentation
                    } else {
                        ButtonPresentation::Disabled
                    },
                );
                let button = if let Some(id) = id {
                    button.id(id.as_str())
                } else {
                    button
                };
                let button = if let Some(label) = accessibility_label {
                    button.accessibility_label(label)
                } else {
                    button
                };
                AnyView::new(if let Some(state) = state {
                    button.accessibility_state(state)
                } else {
                    button
                })
            }
            Self::Switch {
                id,
                label,
                state,
                action,
            } => AnyView::new(
                Switch::with_state_action(*state, action.map(action_message), theme)
                    .id(id.as_str())
                    .accessibility_label(label),
            ),
            Self::RadioGroup { id, options } => AnyView::new(
                RadioGroup::new(
                    options
                        .iter()
                        .map(|option| {
                            RadioOption::new(
                                theme,
                                action_message(option.action.unwrap_or(usize::MAX)),
                                &option.label,
                                option.selected,
                            )
                            .id(option.id.as_str())
                            .description(&option.description)
                            .enabled(option.action.is_some())
                        })
                        .collect::<Vec<_>>(),
                )
                .id(id.as_str()),
            ),
            Self::Input { id, value, action } => {
                let action = *action;
                AnyView::new(
                    Container::new().width(260.0).child(
                        TextField::on_change_with_placeholder_mapped(
                            value,
                            input_placeholder,
                            move |value| input_message(action, value),
                        )
                        .id(id.as_str()),
                    ),
                )
            }
        }
    }

    pub(super) fn contains_input(&self) -> bool {
        match self {
            Self::Input { .. } => true,
            Self::Stack(children)
            | Self::CompactList(children)
            | Self::Grid(children)
            | Self::Fragment(children)
            | Self::Inline(children)
            | Self::Card { children, .. } => children.iter().any(Self::contains_input),
            Self::Row { trailing, .. } => trailing.as_deref().is_some_and(Self::contains_input),
            _ => false,
        }
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, target: &str) -> Option<usize> {
        match self {
            Self::Switch { id, action, .. } if id == target => *action,
            Self::Button { id, action, .. } if id.as_deref() == Some(target) => *action,
            Self::Input { id, action, .. } if id == target => Some(*action),
            Self::Stack(children)
            | Self::CompactList(children)
            | Self::Grid(children)
            | Self::Fragment(children)
            | Self::Inline(children) => children
                .iter()
                .find_map(|child| child.action_for_id(target)),
            Self::Card { children, .. } => children
                .iter()
                .find_map(|child| child.action_for_id(target)),
            Self::Row { trailing, .. } => trailing
                .as_deref()
                .and_then(|child| child.action_for_id(target)),
            Self::RadioGroup { options, .. } => options
                .iter()
                .find(|option| option.id == target)
                .and_then(|option| option.action),
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

/// One live Settings page context. Render and event transactions share the
/// same rollback and stale-projection checks across ordinary pages.
pub(super) struct SettingsJsxContext {
    runtime: JsxRuntime,
    node: Option<Node>,
    last_data: Option<String>,
    parse: fn(&Value) -> Result<Node, String>,
    stale_error: &'static str,
    action_error: &'static str,
}

impl SettingsJsxContext {
    pub(super) fn retained_bytes(&self) -> usize {
        self.last_data.as_ref().map_or(0, String::capacity)
            + self.node.as_ref().map_or(0, Node::retained_bytes)
    }

    pub(super) fn new(
        source: &str,
        parse: fn(&Value) -> Result<Node, String>,
        stale_error: &'static str,
        action_error: &'static str,
    ) -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(source, None)?,
            node: None,
            last_data: None,
            parse,
            stale_error,
            action_error,
        })
    }

    pub(super) fn render(&mut self, data: &Value) -> Result<&Node, String> {
        let serialized = serde_json::to_string(data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&serialized) {
            self.runtime.set_data(&serialized)?;
            self.node = Some(self.runtime.render("__nickelRender()", self.parse)?);
            self.last_data = Some(serialized);
        }
        self.node
            .as_ref()
            .ok_or("Settings page is unavailable".into())
    }

    pub(super) fn dispatch(
        &mut self,
        index: usize,
        value: &Value,
        current_data: &Value,
        validate: impl FnOnce(&Value) -> Result<SettingsMessage, String>,
    ) -> Result<SettingsMessage, String> {
        let serialized = serde_json::to_string(current_data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&serialized) {
            return Err(self.stale_error.into());
        }
        let rendered = self
            .runtime
            .render(&format!("__nickelDispatch({index},{value})"), self.parse);
        let effects = if rendered.is_ok() {
            self.runtime.take_effects()
        } else {
            Ok(Vec::new())
        };
        let result: Result<(Node, SettingsMessage), String> = (|| {
            let node = rendered?;
            let mut effects = effects?;
            if effects.len() != 1 {
                return Err(self.action_error.into());
            }
            Ok((node, validate(&effects.remove(0))?))
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn rejected_page_action_restores_jsx_hook_state() {
        let source = "function App() { const [count, setCount] = useState(0); return h('settings-stack', {}, h('settings-button', {id: 'count', label: String(count), value: 'primary', onClick: () => { setCount(count + 1); nickel.request({type: 'increment'}); }})); }";
        let mut context =
            SettingsJsxContext::new(source, Node::parse, "stale", "missing effect").unwrap();
        let initial = json!({"generation":0});
        context.render(&initial).unwrap();
        let action = context.action_for_id("count").unwrap();
        assert_eq!(
            context
                .dispatch(action, &Value::Null, &initial, |_| Err("denied".into()))
                .unwrap_err(),
            "denied"
        );
        let refreshed = context.render(&json!({"generation":1})).unwrap();
        let Node::Stack(children) = refreshed else {
            panic!("Settings page root changed");
        };
        assert!(matches!(&children[0], Node::Button { label, .. } if label == "0"));
    }
}
