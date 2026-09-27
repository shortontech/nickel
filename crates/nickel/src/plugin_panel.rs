//! Experimental JavaScript panel host. The bundled example uses the same small
//! component vocabulary as an external plugin; native surfaces remain shell-owned.

use boa_engine::{Context, Source};
use nickel_ui::{AnyView, Container, Insets, Row, SemanticRole, Spacer, Text, ViewContext};
use serde_json::Value;

pub const WIDTH: u32 = 360;
pub const HEIGHT: u32 = 64;
pub const BOTTOM_OFFSET: u32 = 24;

pub fn bottom_offset() -> u32 {
    std::env::var("NICKEL_DEV_PLUGIN_PANEL_BOTTOM")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(BOTTOM_OFFSET)
}

pub fn enabled() -> bool {
    std::env::var_os("NICKEL_DEV_PLUGIN_PANEL").is_some()
}

const BOOTSTRAP: &str = r#"
const Panel = 'panel';
const Row = 'row';
const Text = 'text';
const Button = 'button';
let __hooks = [];
let __hookIndex = 0;
let __handlers = [];

function useState(initial) {
    const slot = __hookIndex++;
    if (!(slot in __hooks)) __hooks[slot] = initial;
    return [__hooks[slot], next => {
        __hooks[slot] = typeof next === 'function' ? next(__hooks[slot]) : next;
    }];
}

function useRef(initial) {
    const slot = __hookIndex++;
    if (!(slot in __hooks)) __hooks[slot] = {current: initial};
    return __hooks[slot];
}

function h(kind, props, ...children) {
    if (typeof kind === 'function') return kind({...props, children});
    const action = typeof props?.onClick === 'function' ? __handlers.push(props.onClick) - 1 : null;
    return {kind, action, background: props?.background, children: children.flat(Infinity).filter(child => child !== null && child !== false)};
}

function __nickelRender() {
    __handlers = [];
    __hookIndex = 0;
    return JSON.stringify(App());
}

function __nickelDispatch(action) {
    const handler = __handlers[action];
    if (handler) handler();
    return __nickelRender();
}
"#;

#[derive(Clone, Debug, PartialEq)]
enum PanelNode {
    Panel {
        children: Vec<Self>,
        background: u32,
    },
    Row(Vec<Self>),
    Text(String),
    Button {
        label: String,
        action: usize,
    },
}

impl PanelNode {
    fn parse(value: &Value) -> Result<Self, String> {
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("component needs a kind")?;
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("component needs children")?;
        match kind {
            "panel" | "row" => {
                let children = children
                    .iter()
                    .filter(|value| !value.is_null())
                    .map(Self::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                if kind == "panel" {
                    let background = value
                        .get("background")
                        .and_then(Value::as_u64)
                        .map_or(0xc926_2b36, |value| value as u32);
                    Ok(Self::Panel {
                        children,
                        background,
                    })
                } else {
                    Ok(Self::Row(children))
                }
            }
            "text" => Ok(Self::Text(child_text(children)?)),
            "button" => Ok(Self::Button {
                label: child_text(children)?,
                action: value
                    .get("action")
                    .and_then(Value::as_u64)
                    .ok_or("button needs an onClick handler")? as usize,
            }),
            _ => Err(format!("unknown component {kind:?}")),
        }
    }

    fn view(&self) -> AnyView<usize> {
        match self {
            Self::Panel {
                children,
                background,
            } => {
                let mut row = Row::new().fill_width().height(48.0);
                for child in children {
                    row = row.child(child.view());
                }
                AnyView::new(
                    Container::new()
                        .height(64.0)
                        .background(*background)
                        .radius(16.0)
                        .padding(Insets::all(8.0))
                        .child(row),
                )
            }
            Self::Row(children) => {
                let mut row = Row::new().fill_width().height(48.0);
                for child in children {
                    row = row.child(child.view());
                }
                AnyView::new(row)
            }
            Self::Text(text) => AnyView::new(
                Container::new()
                    .height(48.0)
                    .padding(Insets::all(10.0))
                    .child(Text::new(text).color(0xf4f6fa).scale(1.0)),
            ),
            Self::Button { label, action } => AnyView::new(
                Container::new()
                    .id(format!("plugin-button-{action}"))
                    .accessibility_label(label)
                    .semantic_role(SemanticRole::Button)
                    .message(*action)
                    .height(42.0)
                    .padding(Insets::all(10.0))
                    .background(0x6645_5675)
                    .radius(10.0)
                    .child(Text::new(label).color(0xffffff)),
            ),
        }
    }
}

fn child_text(children: &[Value]) -> Result<String, String> {
    let mut result = String::new();
    for child in children {
        match child {
            Value::String(text) => result.push_str(text),
            Value::Number(number) => result.push_str(&number.to_string()),
            _ => return Err("text and button children must be strings or numbers".into()),
        }
    }
    Ok(result)
}

pub struct PluginPanelApplication {
    context: Context,
    node: PanelNode,
    last_error: Option<String>,
}

impl PluginPanelApplication {
    pub fn bundled() -> Result<Self, String> {
        if let Some(path) = std::env::var_os("NICKEL_DEV_PLUGIN_PANEL_SOURCE") {
            let source = std::fs::read_to_string(&path).map_err(|error| {
                format!(
                    "could not read {}: {error}",
                    std::path::Path::new(&path).display()
                )
            })?;
            Self::new(&source)
        } else {
            Self::new(include_str!("../../../assets/plugins/hello-panel/main.js"))
        }
    }

    pub fn new(source: &str) -> Result<Self, String> {
        let mut context = Context::default();
        context
            .eval(Source::from_bytes(BOOTSTRAP))
            .map_err(|error| error.to_string())?;
        context
            .eval(Source::from_bytes(source))
            .map_err(|error| error.to_string())?;
        let node = evaluate_tree(&mut context, "__nickelRender()")?;
        Ok(Self {
            context,
            node,
            last_error: None,
        })
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

fn evaluate_tree(context: &mut Context, expression: &str) -> Result<PanelNode, String> {
    let value = context
        .eval(Source::from_bytes(expression))
        .map_err(|error| error.to_string())?;
    let text = value
        .to_string(context)
        .map_err(|error| error.to_string())?
        .to_std_string_escaped();
    let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
    PanelNode::parse(&value)
}

impl nickel_ui::Application for PluginPanelApplication {
    type Message = usize;

    fn update(&mut self, message: Self::Message) {
        match evaluate_tree(&mut self.context, &format!("__nickelDispatch({message})")) {
            Ok(node) => {
                self.node = node;
                self.last_error = None;
            }
            Err(error) => self.last_error = Some(error),
        }
    }

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        Row::new()
            .fill_width()
            .height(64.0)
            .child(Spacer::flex())
            .child(self.node.view())
            .child(Spacer::flex())
    }

    fn title(&self) -> &str {
        "Plugin Panel"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_ui::Application;

    #[test]
    fn bundled_panel_updates_from_javascript_click() {
        let mut panel = PluginPanelApplication::bundled().expect("bundled plugin loads");
        assert!(format!("{:?}", panel.node).contains("Count: 0"));
        panel.update(0);
        assert!(format!("{:?}", panel.node).contains("Count: 1"));
        assert!(panel.last_error().is_none());
    }
}
