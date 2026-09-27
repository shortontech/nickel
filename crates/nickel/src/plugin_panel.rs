//! Experimental JavaScript panel host. The bundled example uses the same small
//! component vocabulary as an external plugin; native surfaces remain shell-owned.

use std::sync::OnceLock;

use boa_engine::{Context, Source};
use nickel_core::plugins::{PluginCapability, PluginManifest, PluginSurface, PluginSurfaceKind};
use nickel_ui::{
    AnyView, Column, Container, FrameOverlay, Insets, OverlayAnchor, OverlayId, OverlayStyle, Row,
    SemanticRole, Size, Spacer, Text, TransientSurface, UiId, ViewContext,
};
use serde_json::Value;

pub fn manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!(
            "../../../assets/plugins/hello-panel/plugin.json"
        ))
        .expect("bundled plugin manifest must be valid")
    })
}

pub fn surface() -> &'static PluginSurface {
    let surface = manifest()
        .surfaces
        .first()
        .expect("bundled panel needs a surface");
    assert_eq!(surface.kind, PluginSurfaceKind::Panel);
    surface
}

pub fn bottom_offset() -> u32 {
    std::env::var("NICKEL_DEV_PLUGIN_PANEL_BOTTOM")
        .ok()
        .and_then(|value| value.parse::<u32>().ok())
        .unwrap_or(surface().bottom_offset)
}

pub fn enabled() -> bool {
    std::env::var_os("NICKEL_DEV_PLUGIN_PANEL").is_some()
}

const BOOTSTRAP: &str = r#"
const Panel = 'panel';
const Row = 'row';
const Text = 'text';
const Button = 'button';
const Dialog = 'dialog';
let __hooks = [];
let __hookIndex = 0;
let __handlers = [];
let __effects = [];

const nickel = Object.freeze({
    request(effect) { __effects.push(effect); },
    openDialog(id) { __effects.push(`open-dialog:${id}`); }
});

function __nickelTakeEffects() {
    return JSON.stringify(__effects.splice(0));
}

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
    return {kind, action, id: props?.id, open: props?.open, anchor: props?.anchor,
        width: props?.width, height: props?.height, background: props?.background,
        children: children.flat(Infinity).filter(child => child !== null && child !== false)};
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
        id: String,
        label: String,
        action: usize,
    },
    Dialog {
        id: String,
        anchor: String,
        open: bool,
        width: u32,
        height: u32,
        children: Vec<Self>,
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
                id: value
                    .get("id")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .unwrap_or_else(|| {
                        format!(
                            "plugin-button-{}",
                            value.get("action").and_then(Value::as_u64).unwrap_or(0)
                        )
                    }),
                label: child_text(children)?,
                action: value
                    .get("action")
                    .and_then(Value::as_u64)
                    .ok_or("button needs an onClick handler")? as usize,
            }),
            "dialog" => Ok(Self::Dialog {
                id: value
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("dialog")
                    .to_owned(),
                anchor: value
                    .get("anchor")
                    .and_then(Value::as_str)
                    .ok_or("dialog needs an anchor")?
                    .to_owned(),
                open: value.get("open").and_then(Value::as_bool).unwrap_or(false),
                width: value.get("width").and_then(Value::as_u64).unwrap_or(320) as u32,
                height: value.get("height").and_then(Value::as_u64).unwrap_or(120) as u32,
                children: children
                    .iter()
                    .map(Self::parse)
                    .collect::<Result<Vec<_>, _>>()?,
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
                    if !matches!(child, Self::Dialog { .. }) {
                        row = row.child(child.view());
                    }
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
            Self::Button { id, label, action } => AnyView::new(
                Container::new()
                    .id(id.clone())
                    .accessibility_label(label)
                    .semantic_role(SemanticRole::Button)
                    .message(*action)
                    .height(42.0)
                    .padding(Insets::all(10.0))
                    .background(0x6645_5675)
                    .radius(10.0)
                    .child(Text::new(label).color(0xffffff)),
            ),
            Self::Dialog { .. } => AnyView::new(Spacer::fixed(0.0)),
        }
    }

    fn dialog(&self) -> Option<&Self> {
        match self {
            Self::Dialog { .. } => Some(self),
            Self::Panel { children, .. } | Self::Row(children) => {
                children.iter().find_map(Self::dialog)
            }
            _ => None,
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
    effects: Vec<PluginEffect>,
    pending_transient: Option<(OverlayId, UiId)>,
    last_error: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PluginEffect {
    ShowLauncher,
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
            let source = match manifest().entry.as_str() {
                "main.js" => include_str!("../../../assets/plugins/hello-panel/main.js"),
                entry => return Err(format!("bundled plugin entry {entry:?} is unavailable")),
            };
            Self::new(source)
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
            effects: Vec::new(),
            pending_transient: None,
            last_error: None,
        })
    }

    pub fn take_effects(&mut self) -> Vec<PluginEffect> {
        std::mem::take(&mut self.effects)
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
        let rendered = evaluate_tree(&mut self.context, &format!("__nickelDispatch({message})"));
        let effects = self
            .context
            .eval(Source::from_bytes("__nickelTakeEffects()"))
            .map_err(|error| error.to_string())
            .and_then(|value| {
                value
                    .to_string(&mut self.context)
                    .map_err(|error| error.to_string())
            })
            .and_then(|value| {
                serde_json::from_str::<Vec<String>>(&value.to_std_string_escaped())
                    .map_err(|error| error.to_string())
            });
        match (rendered, effects) {
            (Ok(node), Ok(effects)) => {
                let mut approved = Vec::new();
                let mut requested_dialog = None;
                for effect in effects {
                    match effect.as_str() {
                        "show-launcher"
                            if manifest()
                                .capabilities
                                .contains(&PluginCapability::LauncherShow) =>
                        {
                            approved.push(PluginEffect::ShowLauncher);
                        }
                        effect if effect.starts_with("open-dialog:") => {
                            let id = &effect["open-dialog:".len()..];
                            match node.dialog() {
                                Some(PanelNode::Dialog {
                                    id: declared,
                                    anchor,
                                    open: true,
                                    ..
                                }) if declared == id => {
                                    requested_dialog = Some((
                                        OverlayId::new(format!("plugin-{id}")),
                                        UiId::from(anchor.clone()),
                                    ));
                                }
                                _ => {
                                    self.last_error =
                                        Some(format!("dialog {id:?} is not declared and open"));
                                    return;
                                }
                            }
                        }
                        _ => {
                            self.last_error =
                                Some(format!("plugin effect {effect:?} is not granted"));
                            return;
                        }
                    }
                }
                self.effects.extend(approved);
                self.pending_transient = requested_dialog;
                self.node = node;
                self.last_error = None;
            }
            (Err(error), _) | (_, Err(error)) => self.last_error = Some(error),
        }
    }

    fn view(&self, _context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        Column::new()
            .fill_width()
            .height(surface().height as f32)
            .child(Spacer::flex())
            .child(
                Row::new()
                    .fill_width()
                    .child(Spacer::flex())
                    .child(self.node.view())
                    .child(Spacer::flex()),
            )
    }

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let Some(PanelNode::Dialog {
            id,
            anchor,
            open: true,
            width,
            height,
            children,
        }) = self.node.dialog()
        else {
            return Vec::new();
        };
        let mut content = Column::new().fill_width();
        for child in children {
            content = content.child(child.view());
        }
        vec![FrameOverlay::surface(
            TransientSurface::dialog(
                format!("plugin-{id}"),
                OverlayAnchor::Node(UiId::from(anchor.clone())),
                Size::new(*width as f32, *height as f32),
                OverlayStyle {
                    background: 0xf12b_303c,
                    foreground: 0xffffff,
                    border: 0x657188,
                    selected: 0x405a82,
                    radius: 12,
                },
            ),
            content,
        )]
    }

    fn take_transient_request(&mut self) -> Option<(OverlayId, UiId)> {
        self.pending_transient.take()
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
