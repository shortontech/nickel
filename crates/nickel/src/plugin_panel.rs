//! Experimental JavaScript panel host. The bundled example uses the same small
//! component vocabulary as an external plugin; native surfaces remain shell-owned.

use std::{
    collections::{BTreeMap, HashSet},
    sync::{Arc, OnceLock},
};

use boa_engine::{Context, Source};
use nickel_core::plugins::{
    PluginCapability, PluginManifest, PluginPackage, PluginSurface, PluginSurfaceKind,
};
use nickel_ui::{
    AnyView, Column, ComponentBuilderExt, Container, FrameOverlay, Image, Insets, OverlayAnchor,
    OverlayId, OverlayMenu, OverlayMenuItem, OverlayStyle, Row, SemanticRole, Shortcut, Size,
    Spacer, Text, TextField as UiTextField, TransientSurface, UiId, VerticalScroll, ViewContext,
};
use serde_json::Value;

pub use crate::launcher::LauncherView;
use crate::launcher::{Application, DashboardSection, Launcher, LauncherMode, TaskbarApplication};
use crate::notification::DesktopNotification;

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

pub fn launcher_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!("../../../assets/plugins/launcher/plugin.json"))
            .expect("bundled launcher plugin manifest must be valid")
    })
}

pub fn taskbar_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!("../../../assets/plugins/taskbar/plugin.json"))
            .expect("bundled taskbar plugin manifest must be valid")
    })
}

pub fn notification_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!(
            "../../../assets/plugins/notification/plugin.json"
        ))
        .expect("bundled notification plugin manifest must be valid")
    })
}

pub fn run_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!("../../../assets/plugins/run/plugin.json"))
            .expect("bundled run plugin manifest must be valid")
    })
}

pub fn run_enabled() -> bool {
    true
}

pub fn notification_enabled() -> bool {
    std::env::var_os("NICKEL_DEV_PLUGIN_NOTIFICATION").is_some()
}

pub fn taskbar_enabled() -> bool {
    std::env::var_os("NICKEL_DEV_PLUGIN_TASKBAR").is_some()
}

pub fn launcher_enabled() -> bool {
    std::env::var_os("NICKEL_DEV_PLUGIN_LAUNCHER").is_some()
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
const Column = 'column';
const ScrollView = 'scroll-view';
const Text = 'text';
const TextField = 'text-field';
const Button = 'button';
const Spacer = 'spacer';
const Dialog = 'dialog';
const Menu = 'menu';
const MenuItem = 'menu-item';
const __componentIds = new WeakMap();
let __nextComponentId = 0;
let __componentHooks = new Map();
let __visitedComponents = new Set();
let __componentChildren = new Map();
let __currentComponent = null;
let __hookIndex = 0;
let __handlers = [];
let __effects = [];
let __pendingRender = null;
let __nickelData = Object.freeze({query: '', results: []});

const nickel = Object.freeze({
    request(effect) { __effects.push(effect); },
    openDialog(id) { __effects.push(`open-dialog:${id}`); },
    openMenu(id) { __effects.push(`open-menu:${id}`); },
    get data() { return __nickelData; }
});

function __nickelSetData(data) { __nickelData = Object.freeze(data); }

function __nickelTakeEffects() {
    return JSON.stringify(__effects.splice(0));
}

function useState(initial) {
    if (__currentComponent === null) throw Error('useState requires a component');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    if (!hooks[slot]) hooks[slot] = {kind: 'state', value: typeof initial === 'function' ? initial() : initial};
    if (hooks[slot].kind !== 'state') throw Error('hook order changed');
    const entry = hooks[slot];
    const owner = __currentComponent;
    return [entry.value, next => {
        if (__componentHooks.get(owner)?.[slot] !== entry) return;
        entry.value = typeof next === 'function' ? next(entry.value) : next;
    }];
}

function useRef(initial) {
    if (__currentComponent === null) throw Error('useRef requires a component');
    const slot = __hookIndex++;
    const hooks = __componentHooks.get(__currentComponent);
    if (!hooks[slot]) hooks[slot] = {kind: 'ref', value: {current: initial}};
    if (hooks[slot].kind !== 'ref') throw Error('hook order changed');
    return hooks[slot].value;
}

function h(kind, props, ...children) {
    if (typeof kind === 'function') {
        let type = __componentIds.get(kind);
        if (type === undefined) {
            type = ++__nextComponentId;
            __componentIds.set(kind, type);
        }
        const parent = __currentComponent ?? 'root';
        const ordinalKey = `${parent}/${type}`;
        const ordinal = __componentChildren.get(ordinalKey) ?? 0;
        __componentChildren.set(ordinalKey, ordinal + 1);
        const identity = props?.key === undefined ? `#${ordinal}` : `@${encodeURIComponent(String(props.key))}`;
        const path = `${ordinalKey}/${identity}`;
        if (__visitedComponents.has(path)) throw Error(`duplicate component key ${identity}`);
        __visitedComponents.add(path);
        if (!__componentHooks.has(path)) __componentHooks.set(path, []);
        const previous = __currentComponent;
        const previousIndex = __hookIndex;
        __currentComponent = path;
        __hookIndex = 0;
        try {
            const node = kind({...props, children});
            if (__hookIndex !== __componentHooks.get(path).length) throw Error('hook order changed');
            return node;
        } finally {
            __currentComponent = previous;
            __hookIndex = previousIndex;
        }
    }
    const handler = typeof props?.onClick === 'function' ? props.onClick : props?.onChange;
    const action = typeof handler === 'function' ? __handlers.push(handler) - 1 : null;
    const contextAction = typeof props?.onContextMenu === 'function'
        ? __handlers.push(props.onContextMenu) - 1 : null;
    return {kind, action, id: props?.id, open: props?.open, anchor: props?.anchor,
        width: props?.width, height: props?.height, background: props?.background,
        accessibilityLabel: props?.accessibilityLabel, icon: props?.icon,
        showLabel: props?.showLabel, contextAction,
        value: props?.value, placeholder: props?.placeholder,
        children: children.flat(Infinity).filter(child => child !== null && child !== false)};
}

function __nickelRollbackRender() {
    if (__pendingRender === null) return;
    const {handlers, hooks, values, effectsLength} = __pendingRender;
    __handlers = handlers;
    let componentIndex = 0;
    for (const slots of hooks.values()) {
        const oldValues = values[componentIndex++];
        slots.forEach((entry, index) => {
            if (entry.kind === 'ref') entry.value.current = oldValues[index];
            else entry.value = oldValues[index];
        });
    }
    __componentHooks = hooks;
    __effects.length = effectsLength;
    __pendingRender = null;
}

function __nickelCommitRender() { __pendingRender = null; }

function __nickelRender() {
    if (__pendingRender !== null) throw Error('previous render was not finalized');
    const previousHandlers = __handlers;
    const previousHooks = new Map(Array.from(__componentHooks, ([path, hooks]) => [path, hooks.slice()]));
    const previousValues = Array.from(__componentHooks.values(), hooks => hooks.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    __pendingRender = {handlers: previousHandlers, hooks: previousHooks,
        values: previousValues, effectsLength: __effects.length};
    __handlers = [];
    __visitedComponents = new Set();
    __componentChildren = new Map();
    __currentComponent = null;
    __hookIndex = 0;
    try {
        const node = h(App, {});
        for (const path of __componentHooks.keys()) {
            if (!__visitedComponents.has(path)) __componentHooks.delete(path);
        }
        return JSON.stringify(node);
    } catch (error) {
        __nickelRollbackRender();
        throw error;
    }
}

function __nickelDispatch(action, value) {
    const handler = __handlers[action];
    if (handler) handler(value);
    return __nickelRender();
}
"#;

#[derive(Clone, Debug, PartialEq)]
enum PanelNode {
    Panel {
        children: Vec<Self>,
        background: u32,
        height: u32,
    },
    Row(Vec<Self>),
    Column(Vec<Self>),
    ScrollView {
        id: String,
        height: u32,
        children: Vec<Self>,
    },
    Text(String),
    Spacer,
    TextField {
        id: String,
        value: String,
        placeholder: String,
        action: usize,
    },
    Button {
        id: String,
        label: String,
        accessibility_label: String,
        icon: Option<String>,
        show_label: bool,
        action: usize,
        context_action: Option<usize>,
    },
    Dialog {
        id: String,
        anchor: String,
        open: bool,
        width: u32,
        height: u32,
        children: Vec<Self>,
    },
    Menu {
        id: String,
        anchor: String,
        open: bool,
        items: Vec<Self>,
    },
    MenuItem {
        id: String,
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
            "panel" | "row" | "column" | "scroll-view" => {
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
                    let height = value
                        .get("height")
                        .and_then(Value::as_u64)
                        .filter(|height| (1..=8192).contains(height))
                        .unwrap_or(64) as u32;
                    Ok(Self::Panel {
                        children,
                        background,
                        height,
                    })
                } else if kind == "row" {
                    Ok(Self::Row(children))
                } else if kind == "scroll-view" {
                    let id = value
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("scroll view needs an id")?;
                    if id.is_empty() || id.len() > 128 {
                        return Err("scroll view ID must contain 1 to 128 characters".into());
                    }
                    let height = value.get("height").and_then(Value::as_u64).unwrap_or(480);
                    if !(1..=8192).contains(&height) {
                        return Err("scroll view height must be 1 to 8192".into());
                    }
                    Ok(Self::ScrollView {
                        id: id.to_owned(),
                        height: height as u32,
                        children,
                    })
                } else {
                    Ok(Self::Column(children))
                }
            }
            "text" => Ok(Self::Text(child_text(children)?)),
            "spacer" => Ok(Self::Spacer),
            "text-field" => Ok(Self::TextField {
                id: value
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("text field needs an id")?
                    .to_owned(),
                value: value
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                placeholder: value
                    .get("placeholder")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned(),
                action: value
                    .get("action")
                    .and_then(Value::as_u64)
                    .ok_or("text field needs an onChange handler")?
                    as usize,
            }),
            "button" => {
                let label = child_text(children)?;
                Ok(Self::Button {
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
                    accessibility_label: value
                        .get("accessibilityLabel")
                        .and_then(Value::as_str)
                        .unwrap_or(&label)
                        .to_owned(),
                    icon: value
                        .get("icon")
                        .and_then(Value::as_str)
                        .filter(|asset| asset.len() <= 128)
                        .map(str::to_owned),
                    show_label: value
                        .get("showLabel")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    label,
                    action: value
                        .get("action")
                        .and_then(Value::as_u64)
                        .ok_or("button needs an onClick handler")?
                        as usize,
                    context_action: value
                        .get("contextAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
                })
            }
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
            "menu" => {
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("menu needs an id")?;
                let anchor = value
                    .get("anchor")
                    .and_then(Value::as_str)
                    .ok_or("menu needs an anchor")?;
                if id.is_empty() || id.len() > 128 || anchor.is_empty() || anchor.len() > 128 {
                    return Err("menu ID and anchor must contain 1 to 128 characters".into());
                }
                if children.is_empty() || children.len() > 16 {
                    return Err("menu needs 1 to 16 items".into());
                }
                let items = children
                    .iter()
                    .map(Self::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                let mut seen = HashSet::new();
                for item in &items {
                    let Self::MenuItem { id, .. } = item else {
                        return Err("menu children must be MenuItem components".into());
                    };
                    if !seen.insert(id) {
                        return Err("menu item IDs must be unique".into());
                    }
                }
                Ok(Self::Menu {
                    id: id.to_owned(),
                    anchor: anchor.to_owned(),
                    open: value.get("open").and_then(Value::as_bool).unwrap_or(false),
                    items,
                })
            }
            "menu-item" => {
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("menu item needs an id")?;
                let label = child_text(children)?;
                if id.is_empty()
                    || id.len() > 128
                    || label.is_empty()
                    || label.chars().count() > 120
                {
                    return Err("menu item ID or label is invalid".into());
                }
                let action = value
                    .get("action")
                    .and_then(Value::as_u64)
                    .and_then(|action| usize::try_from(action).ok())
                    .ok_or("menu item needs an onClick handler")?;
                Ok(Self::MenuItem {
                    id: id.to_owned(),
                    label,
                    action,
                })
            }
            _ => Err(format!("unknown component {kind:?}")),
        }
    }

    fn view(&self, images: &PluginImages) -> AnyView<PluginMessage> {
        match self {
            Self::Panel {
                children,
                background,
                height,
            } => {
                let mut row = Row::new()
                    .fill_width()
                    .height((*height).saturating_sub(16) as f32);
                for child in children {
                    if !matches!(child, Self::Dialog { .. }) {
                        row = row.child(child.view(images));
                    }
                }
                AnyView::new(
                    Container::new()
                        .height(*height as f32)
                        .background(*background)
                        .radius(16.0)
                        .padding(Insets::all(8.0))
                        .child(row),
                )
            }
            Self::Row(children) => {
                let mut row = Row::new().fill_width().height(48.0);
                for child in children {
                    row = row.child(child.view(images));
                }
                AnyView::new(row)
            }
            Self::Column(children) => {
                let mut column = Column::new().fill_width();
                for child in children {
                    column = column.child(child.view(images));
                }
                AnyView::new(column)
            }
            Self::ScrollView {
                id,
                height,
                children,
            } => {
                let mut column = Column::new().fill_width();
                for child in children {
                    column = column.child(child.view(images));
                }
                AnyView::new(
                    VerticalScroll::new(PluginMessage::Scroll, 0.0)
                        .id(id.clone())
                        .height(*height as f32)
                        .child(column),
                )
            }
            Self::Text(text) => AnyView::new(
                Container::new()
                    .height(48.0)
                    .padding(Insets::all(10.0))
                    .child(Text::new(text).color(0xf4f6fa).scale(1.0)),
            ),
            Self::Spacer => AnyView::new(Spacer::flex()),
            Self::TextField {
                id,
                value,
                placeholder,
                action,
            } => AnyView::new(
                UiTextField::on_change_with_placeholder_mapped(value, placeholder, {
                    let action = *action;
                    move |value| PluginMessage::Text(action, value)
                })
                .id(id.clone())
                .accessibility_label(placeholder)
                .height(44.0),
            ),
            Self::Button {
                id,
                label,
                accessibility_label,
                icon,
                show_label,
                action,
                context_action,
            } => {
                let visual = icon
                    .as_ref()
                    .and_then(|asset| images.get(asset))
                    .map_or_else(
                        || AnyView::new(Text::new(label).color(0xffffff)),
                        |(id, image)| {
                            let icon = Image::new(*id, Arc::clone(image)).width(32.0).height(32.0);
                            if *show_label {
                                AnyView::new(
                                    Row::new()
                                        .gap(8.0)
                                        .child(icon)
                                        .child(Text::new(label).color(0xffffff)),
                                )
                            } else {
                                AnyView::new(icon)
                            }
                        },
                    );
                let mut container = Container::new()
                    .id(id.clone())
                    .accessibility_label(accessibility_label)
                    .semantic_role(SemanticRole::Button)
                    .message(PluginMessage::Click(*action))
                    .height(42.0)
                    .padding(Insets::all(10.0))
                    .background(0x6645_5675)
                    .radius(10.0);
                if let Some(action) = context_action {
                    container = container.context_message(PluginMessage::Context(*action));
                }
                AnyView::new(container.child(visual))
            }
            Self::Dialog { .. } | Self::Menu { .. } | Self::MenuItem { .. } => {
                AnyView::new(Spacer::fixed(0.0))
            }
        }
    }

    fn dialog(&self, requested_id: &str) -> Option<&Self> {
        match self {
            Self::Dialog { id, .. } if id == requested_id => Some(self),
            Self::Panel { children, .. }
            | Self::Row(children)
            | Self::Column(children)
            | Self::ScrollView { children, .. } => {
                children.iter().find_map(|child| child.dialog(requested_id))
            }
            _ => None,
        }
    }

    fn menu(&self, requested_id: &str) -> Option<&Self> {
        match self {
            Self::Menu { id, .. } if id == requested_id => Some(self),
            Self::Panel { children, .. }
            | Self::Row(children)
            | Self::Column(children)
            | Self::ScrollView { children, .. } => {
                children.iter().find_map(|child| child.menu(requested_id))
            }
            _ => None,
        }
    }

    fn transients<'a>(&'a self, output: &mut Vec<&'a Self>) {
        match self {
            Self::Dialog { .. } | Self::Menu { .. } => output.push(self),
            Self::Panel { children, .. }
            | Self::Row(children)
            | Self::Column(children)
            | Self::ScrollView { children, .. } => {
                for child in children {
                    child.transients(output);
                }
            }
            _ => {}
        }
    }

    fn button_action(&self, requested_id: &str) -> Option<usize> {
        match self {
            Self::Button { id, action, .. } if id == requested_id => Some(*action),
            Self::Panel { children, .. }
            | Self::Row(children)
            | Self::Column(children)
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.button_action(requested_id)),
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
    manifest: PluginManifest,
    projection_data: Option<String>,
    launcher_shortcuts: Option<LauncherShortcutState>,
    notification_shortcuts: Option<(Option<u32>, bool)>,
    overlay_open: bool,
    images: PluginImages,
}

struct LauncherShortcutState {
    query: String,
    dashboard_visible: bool,
    first_result: Option<(usize, String)>,
}

impl From<&LauncherPluginProjection> for LauncherShortcutState {
    fn from(projection: &LauncherPluginProjection) -> Self {
        Self {
            query: projection.query.clone(),
            dashboard_visible: projection.dashboard_visible,
            first_result: projection
                .results
                .first()
                .map(|result| (result.index, result.id.clone())),
        }
    }
}

pub type PluginImages = BTreeMap<String, (u16, Arc<image::RgbaImage>)>;

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginEffect {
    ShowLauncher,
    RunSubmit(String),
    RunDismiss,
    ToggleLauncher,
    ToggleOnScreenKeyboard,
    ToggleCodexProjects,
    SetLauncherQuery(String),
    SetLauncherPage { dashboard: bool, page: usize },
    ActivateLauncherResult { index: usize, id: String },
    LaunchDashboardApplication { id: String },
    SetLauncherView(LauncherView),
    ToggleLauncherPin { id: String },
    DismissLauncher,
    LauncherOpenSettings,
    LauncherOpenAccount,
    LauncherOpenProject { id: String },
    LauncherSeeAllProjects,
    LauncherRequestLogout,
    ActivateTaskbarItem { index: usize, id: String },
    ContextTaskbarItem { index: usize, id: String },
    ActivateTrayItem { id: String },
    ContextTrayItem { id: String },
    ToggleControlCenter,
    InvokeNotification { id: u32, key: String },
    DismissNotification { id: u32 },
    CloseNotificationHistory,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PluginMessage {
    Click(usize),
    Context(usize),
    Text(usize, String),
    Scroll,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LauncherPluginResult {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub pinned: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LauncherPluginProject {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LauncherPluginProjection {
    pub query: String,
    pub dashboard_visible: bool,
    pub view: LauncherView,
    pub result_page: usize,
    pub result_page_count: usize,
    pub dashboard_page: usize,
    pub dashboard_page_count: usize,
    pub results: Vec<LauncherPluginResult>,
    pub dashboard: Vec<LauncherPluginResult>,
    pub places: Vec<LauncherPluginResult>,
    pub projects: Vec<LauncherPluginProject>,
    pub codex_available: bool,
    pub account_name: String,
    pub logout_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarPluginItem {
    pub index: usize,
    pub id: String,
    pub name: String,
    pub active: bool,
    pub pinned: bool,
    pub icon: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarPluginTrayItem {
    pub id: String,
    pub title: String,
    pub icon: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarPluginProjection {
    pub items: Vec<TaskbarPluginItem>,
    pub tray: Vec<TaskbarPluginTrayItem>,
    pub clock: String,
    pub keyboard_enabled: bool,
    pub codex_available: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationPluginAction {
    pub key: String,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationPluginItem {
    pub id: u32,
    pub app_name: String,
    pub summary: String,
    pub body: String,
    pub actions: Vec<NotificationPluginAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NotificationPluginProjection {
    pub notification: Option<NotificationPluginItem>,
    pub history: Vec<NotificationPluginItem>,
    pub history_visible: bool,
}

impl NotificationPluginProjection {
    pub fn from_feed(
        notification: Option<&DesktopNotification>,
        history: &[DesktopNotification],
        history_visible: bool,
    ) -> Self {
        let item = |notification: &DesktopNotification| NotificationPluginItem {
            id: notification.id,
            app_name: notification.app_name.clone(),
            summary: notification.summary.clone(),
            body: notification.body.clone(),
            actions: notification
                .actions
                .iter()
                .take(crate::notification::MAX_NOTIFICATION_ACTIONS)
                .map(|action| NotificationPluginAction {
                    key: action.key.clone(),
                    label: action.label.clone(),
                })
                .collect(),
        };
        Self {
            notification: notification.map(item),
            history: if history_visible {
                history.iter().take(12).map(item).collect()
            } else {
                Vec::new()
            },
            history_visible,
        }
    }

    fn to_json(&self) -> String {
        let item = |item: &NotificationPluginItem| {
            serde_json::json!({"id": item.id, "appName": item.app_name,
                "summary": item.summary, "body": item.body,
                "actions": item.actions.iter().map(|action| serde_json::json!({
                    "key": action.key, "label": action.label
                })).collect::<Vec<_>>()})
        };
        serde_json::json!({
            "notification": self.notification.as_ref().map(item),
            "history": self.history.iter().map(item).collect::<Vec<_>>(),
            "historyVisible": self.history_visible,
        })
        .to_string()
    }
}

impl TaskbarPluginProjection {
    pub fn from_groups(groups: &[TaskbarApplication], clock: &str) -> Self {
        let mut seen = HashSet::new();
        Self {
            items: groups
                .iter()
                .take(12)
                .enumerate()
                .filter_map(|(index, group)| {
                    let id = taskbar_item_id(group);
                    if id.is_empty() || id.len() > 256 || !seen.insert(id.clone()) {
                        return None;
                    }
                    Some(TaskbarPluginItem {
                        index,
                        id,
                        name: group.application_name.chars().take(120).collect(),
                        active: group.active(),
                        pinned: group.pinned,
                        icon: false,
                    })
                })
                .collect(),
            tray: Vec::new(),
            clock: clock.to_owned(),
            keyboard_enabled: false,
            codex_available: false,
        }
    }

    fn to_json(&self) -> String {
        serde_json::json!({"items": self.items.iter().map(|item| serde_json::json!({
            "index": item.index, "id": item.id, "name": item.name,
            "active": item.active, "pinned": item.pinned, "icon": item.icon,
        })).collect::<Vec<_>>(),
        "tray": self.tray.iter().map(|item| serde_json::json!({
            "id": item.id, "title": item.title, "icon": item.icon,
        })).collect::<Vec<_>>(),
        "clock": self.clock, "keyboardEnabled": self.keyboard_enabled,
        "codexAvailable": self.codex_available})
        .to_string()
    }
}

pub fn taskbar_item_id(group: &TaskbarApplication) -> String {
    group.application_id.as_ref().map_or_else(
        || {
            group.windows.last().map_or_else(
                || format!("unidentified:{}", group.application_name),
                |window| format!("window:{}", window.id.0),
            )
        },
        |id| id.as_str().to_owned(),
    )
}

pub fn taskbar_item_matches(groups: &[TaskbarApplication], index: usize, id: &str) -> bool {
    groups
        .get(index)
        .is_some_and(|group| taskbar_item_id(group) == id)
}

impl LauncherPluginProjection {
    pub(crate) fn from_launcher(launcher: &Launcher) -> Self {
        Self::from_launcher_pages(launcher, 0, 0)
    }

    pub(crate) fn from_launcher_pages(
        launcher: &Launcher,
        result_page: usize,
        dashboard_page: usize,
    ) -> Self {
        const SEARCH_PAGE_SIZE: usize = 12;
        const DASHBOARD_PAGE_SIZE: usize = 48;
        let count = launcher.result_count();
        let result_page_count = count.div_ceil(SEARCH_PAGE_SIZE).max(1);
        let dashboard_page_count = count.div_ceil(DASHBOARD_PAGE_SIZE).max(1);
        let result_page = result_page.min(result_page_count - 1);
        let dashboard_page = dashboard_page.min(dashboard_page_count - 1);
        let result_start = result_page * SEARCH_PAGE_SIZE;
        let dashboard_start = dashboard_page * DASHBOARD_PAGE_SIZE;
        let results = (result_start..count.min(result_start + SEARCH_PAGE_SIZE))
            .filter_map(|index| {
                launcher
                    .result_at(index)
                    .and_then(|application| launcher_plugin_result(launcher, index, application))
            })
            .collect();
        Self {
            query: launcher.query().to_owned(),
            dashboard_visible: launcher.mode() == LauncherMode::Dashboard,
            view: launcher.view(),
            result_page,
            result_page_count,
            dashboard_page,
            dashboard_page_count,
            results,
            dashboard: (dashboard_start..count.min(dashboard_start + DASHBOARD_PAGE_SIZE))
                .filter_map(|index| {
                    launcher.result_at(index).and_then(|application| {
                        launcher_plugin_result(launcher, index, application)
                    })
                })
                .collect(),
            places: launcher
                .place_applications()
                .take(12)
                .enumerate()
                .filter_map(|(index, application)| {
                    launcher_plugin_result(launcher, index, application)
                })
                .collect(),
            projects: match launcher.dashboard_projects() {
                DashboardSection::Ready(projects) if launcher.codex_available() => {
                    let mut recent = projects
                        .iter()
                        .filter(|project| {
                            project.last_used_at.is_some()
                                && !project.id.is_empty()
                                && project.id.len() <= 256
                        })
                        .collect::<Vec<_>>();
                    recent.sort_by_key(|project| std::cmp::Reverse(project.last_used_at));
                    recent
                        .into_iter()
                        .take(3)
                        .map(|project| LauncherPluginProject {
                            id: project.id.clone(),
                            name: project.name.chars().take(120).collect(),
                        })
                        .collect()
                }
                _ => Vec::new(),
            },
            codex_available: launcher.codex_available(),
            account_name: match launcher.dashboard_account() {
                DashboardSection::Ready(account) => {
                    account.display_name.chars().take(120).collect()
                }
                _ => "Local session".into(),
            },
            logout_available: launcher.logout_available(),
        }
    }

    fn to_json(&self) -> String {
        let results = self.results.iter().map(|result| {
            serde_json::json!({"index": result.index, "id": result.id, "name": result.name, "pinned": result.pinned})
        }).collect::<Vec<_>>();
        let items = |items: &[LauncherPluginResult]| {
            items.iter().map(|item| {
            serde_json::json!({"index": item.index, "id": item.id, "name": item.name, "pinned": item.pinned})
        }).collect::<Vec<_>>()
        };
        let view = match self.view {
            LauncherView::Favorites => "favorites",
            LauncherView::Applications => "applications",
            LauncherView::Places => "places",
        };
        serde_json::json!({"query": self.query, "dashboardVisible": self.dashboard_visible, "view": view,
            "resultPage": self.result_page, "resultPageCount": self.result_page_count,
            "dashboardPage": self.dashboard_page, "dashboardPageCount": self.dashboard_page_count,
            "results": results,
            "dashboard": items(&self.dashboard), "places": items(&self.places),
            "projects": self.projects.iter().map(|project| serde_json::json!({"id": project.id, "name": project.name})).collect::<Vec<_>>(),
            "codexAvailable": self.codex_available, "accountName": self.account_name,
            "logoutAvailable": self.logout_available})
        .to_string()
    }
}

fn launcher_plugin_result(
    launcher: &Launcher,
    index: usize,
    application: &Application,
) -> Option<LauncherPluginResult> {
    let id = application.id();
    (!id.is_empty() && id.len() <= 256).then(|| LauncherPluginResult {
        index,
        id: id.to_owned(),
        name: application.name().chars().take(120).collect(),
        pinned: launcher.is_pinned(id),
    })
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
        Self::new_with_manifest(source, manifest(), None)
    }

    pub fn from_package(package: &PluginPackage) -> Result<Self, String> {
        Self::new_with_manifest(&package.source, &package.manifest, None)
    }

    pub fn launcher(launcher: &Launcher) -> Result<Self, String> {
        Self::launcher_with_projection(&LauncherPluginProjection::from_launcher(launcher))
    }

    pub fn launcher_with_projection(projection: &LauncherPluginProjection) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/launcher/main.js");
        let data = projection.to_json();
        let mut application = Self::new_with_manifest(source, launcher_manifest(), Some(data))?;
        application.launcher_shortcuts = Some(projection.into());
        Ok(application)
    }

    pub fn taskbar_with_projection(projection: &TaskbarPluginProjection) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/taskbar/main.js");
        Self::new_with_manifest(source, taskbar_manifest(), Some(projection.to_json()))
    }

    pub fn notification_with_projection(
        projection: &NotificationPluginProjection,
    ) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/notification/main.js");
        let mut application =
            Self::new_with_manifest(source, notification_manifest(), Some(projection.to_json()))?;
        application.notification_shortcuts = Some((
            projection.notification.as_ref().map(|item| item.id),
            projection.history_visible,
        ));
        Ok(application)
    }

    pub fn run_with_status(status: Option<&str>) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/run/main.js");
        let data = serde_json::json!({ "status": status }).to_string();
        Self::new_with_manifest(source, run_manifest(), Some(data))
    }

    pub fn sync_run_status(&mut self, status: Option<&str>) -> Result<bool, String> {
        if self.manifest.id != run_manifest().id {
            return Err("this plugin is not the Run dialog".into());
        }
        let data = serde_json::json!({ "status": status }).to_string();
        if self.projection_data.as_deref() == Some(data.as_str()) {
            return Ok(false);
        }
        self.context
            .eval(Source::from_bytes(&format!("__nickelSetData({data})")))
            .map_err(|error| error.to_string())?;
        self.node = evaluate_tree(&mut self.context, "__nickelRender()")?;
        self.projection_data = Some(data);
        Ok(true)
    }

    fn new_with_manifest(
        source: &str,
        manifest: &PluginManifest,
        data: Option<String>,
    ) -> Result<Self, String> {
        let mut context = Context::default();
        context
            .eval(Source::from_bytes(BOOTSTRAP))
            .map_err(|error| error.to_string())?;
        if let Some(data) = &data {
            context
                .eval(Source::from_bytes(&format!("__nickelSetData({data})")))
                .map_err(|error| error.to_string())?;
        }
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
            manifest: manifest.clone(),
            projection_data: data,
            launcher_shortcuts: None,
            notification_shortcuts: None,
            overlay_open: false,
            images: PluginImages::new(),
        })
    }

    pub fn sync_images(&mut self, images: PluginImages) -> bool {
        let changed = self.images.len() != images.len()
            || self.images.iter().any(|(key, (id, image))| {
                images
                    .get(key)
                    .is_none_or(|(next_id, next)| id != next_id || !Arc::ptr_eq(image, next))
            });
        if changed {
            self.images = images;
        }
        changed
    }

    pub fn sync_launcher(&mut self, launcher: &Launcher) -> Result<bool, String> {
        self.sync_launcher_projection(&LauncherPluginProjection::from_launcher(launcher))
    }

    pub fn set_overlay_open(&mut self, open: bool) {
        self.overlay_open = open;
    }

    pub fn sync_launcher_projection(
        &mut self,
        projection: &LauncherPluginProjection,
    ) -> Result<bool, String> {
        if self.manifest.id != launcher_manifest().id {
            return Err("this plugin is not the launcher".into());
        }
        let data = projection.to_json();
        if self.projection_data.as_deref() == Some(data.as_str()) {
            self.launcher_shortcuts = Some(projection.into());
            return Ok(false);
        }
        self.context
            .eval(Source::from_bytes(&format!("__nickelSetData({data})")))
            .map_err(|error| error.to_string())?;
        self.node = evaluate_tree(&mut self.context, "__nickelRender()")?;
        self.projection_data = Some(data);
        self.launcher_shortcuts = Some(projection.into());
        Ok(true)
    }

    pub fn sync_taskbar_projection(
        &mut self,
        projection: &TaskbarPluginProjection,
    ) -> Result<bool, String> {
        if self.manifest.id != taskbar_manifest().id {
            return Err("this plugin is not the taskbar".into());
        }
        let data = projection.to_json();
        if self.projection_data.as_deref() == Some(data.as_str()) {
            return Ok(false);
        }
        self.context
            .eval(Source::from_bytes(&format!("__nickelSetData({data})")))
            .map_err(|error| error.to_string())?;
        self.node = evaluate_tree(&mut self.context, "__nickelRender()")?;
        self.projection_data = Some(data);
        Ok(true)
    }

    pub fn sync_notification_projection(
        &mut self,
        projection: &NotificationPluginProjection,
    ) -> Result<bool, String> {
        if self.manifest.id != notification_manifest().id {
            return Err("this plugin is not notifications".into());
        }
        let data = projection.to_json();
        if self.projection_data.as_deref() == Some(data.as_str()) {
            self.notification_shortcuts = Some((
                projection.notification.as_ref().map(|item| item.id),
                projection.history_visible,
            ));
            return Ok(false);
        }
        self.context
            .eval(Source::from_bytes(&format!("__nickelSetData({data})")))
            .map_err(|error| error.to_string())?;
        self.node = evaluate_tree(&mut self.context, "__nickelRender()")?;
        self.projection_data = Some(data);
        self.notification_shortcuts = Some((
            projection.notification.as_ref().map(|item| item.id),
            projection.history_visible,
        ));
        Ok(true)
    }

    pub fn take_effects(&mut self) -> Vec<PluginEffect> {
        std::mem::take(&mut self.effects)
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }
}

fn evaluate_tree(context: &mut Context, expression: &str) -> Result<PanelNode, String> {
    let parsed = (|| {
        let value = context
            .eval(Source::from_bytes(expression))
            .map_err(|error| error.to_string())?;
        let text = value
            .to_string(context)
            .map_err(|error| error.to_string())?
            .to_std_string_escaped();
        let value: Value = serde_json::from_str(&text).map_err(|error| error.to_string())?;
        PanelNode::parse(&value)
    })();
    let finalizer = if parsed.is_ok() {
        "__nickelCommitRender()"
    } else {
        "__nickelRollbackRender()"
    };
    context
        .eval(Source::from_bytes(finalizer))
        .map_err(|error| format!("could not finalize plugin render: {error}"))?;
    parsed
}

impl nickel_ui::Application for PluginPanelApplication {
    type Message = PluginMessage;

    fn shortcut_outcome(&mut self, shortcut: Shortcut) -> nickel_ui::ShortcutOutcome {
        if self.manifest.id == run_manifest().id {
            return match shortcut {
                Shortcut::Escape => {
                    self.effects.push(PluginEffect::RunDismiss);
                    nickel_ui::ShortcutOutcome::handled(true)
                }
                Shortcut::Submit => {
                    if let Some(action) = self.node.button_action("run-submit") {
                        self.update(PluginMessage::Click(action));
                        nickel_ui::ShortcutOutcome::handled(true)
                    } else {
                        nickel_ui::ShortcutOutcome::from_changed(false)
                    }
                }
                _ => nickel_ui::ShortcutOutcome::from_changed(false),
            };
        }
        if shortcut == Shortcut::Escape
            && let Some((id, history_visible)) = self.notification_shortcuts
        {
            if history_visible {
                self.effects.push(PluginEffect::CloseNotificationHistory);
                return nickel_ui::ShortcutOutcome::handled(true);
            }
            if let Some(id) = id {
                self.effects.push(PluginEffect::DismissNotification { id });
                return nickel_ui::ShortcutOutcome::handled(true);
            }
        }
        if self.overlay_open && shortcut == Shortcut::Escape {
            return nickel_ui::ShortcutOutcome::from_changed(false);
        }
        if self.overlay_open && shortcut == Shortcut::Submit {
            return nickel_ui::ShortcutOutcome::from_changed(false);
        }
        let Some(shortcuts) = &self.launcher_shortcuts else {
            return nickel_ui::ShortcutOutcome::from_changed(false);
        };
        match shortcut {
            Shortcut::Escape => {
                if shortcuts.query.is_empty() {
                    self.effects.push(PluginEffect::DismissLauncher);
                } else {
                    self.effects
                        .push(PluginEffect::SetLauncherQuery(String::new()));
                }
                nickel_ui::ShortcutOutcome::handled(true)
            }
            Shortcut::Submit if !shortcuts.dashboard_visible => {
                let Some((index, id)) = &shortcuts.first_result else {
                    return nickel_ui::ShortcutOutcome::handled(false);
                };
                self.effects.push(PluginEffect::ActivateLauncherResult {
                    index: *index,
                    id: id.clone(),
                });
                nickel_ui::ShortcutOutcome::handled(true)
            }
            _ => nickel_ui::ShortcutOutcome::from_changed(false),
        }
    }

    fn update(&mut self, message: Self::Message) {
        if message == PluginMessage::Scroll {
            return;
        }
        let expression = match message {
            PluginMessage::Click(action) | PluginMessage::Context(action) => {
                format!("__nickelDispatch({action})")
            }
            PluginMessage::Text(action, value) => {
                let encoded = serde_json::to_string(&value).expect("string serialization");
                format!("__nickelDispatch({action}, {encoded})")
            }
            PluginMessage::Scroll => unreachable!(),
        };
        let rendered = evaluate_tree(&mut self.context, &expression);
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
                serde_json::from_str::<Vec<Value>>(&value.to_std_string_escaped())
                    .map_err(|error| error.to_string())
            });
        match (rendered, effects) {
            (Ok(node), Ok(effects)) => {
                let mut approved = Vec::new();
                let mut requested_dialog = None;
                for effect in effects {
                    match effect.as_str() {
                        Some("show-launcher")
                            if self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::LauncherShow) =>
                        {
                            approved.push(PluginEffect::ShowLauncher);
                        }
                        _ if effect.get("type").and_then(Value::as_str) == Some("run-submit")
                            && self.manifest.id == run_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::RunCommand) =>
                        {
                            let Some(command) = effect.get("command").and_then(Value::as_str)
                            else {
                                self.last_error = Some("Run command is missing".into());
                                return;
                            };
                            let command = command.trim();
                            if command.is_empty() || command.chars().count() > 4096 {
                                self.last_error = Some("Run command is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::RunSubmit(command.to_owned()));
                        }
                        _ if effect.get("type").and_then(Value::as_str) == Some("run-dismiss")
                            && self.manifest.id == run_manifest().id =>
                        {
                            approved.push(PluginEffect::RunDismiss);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-toggle-launcher")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::LauncherShow) =>
                        {
                            approved.push(PluginEffect::ToggleLauncher);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-toggle-control")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ControlCenterShow) =>
                        {
                            approved.push(PluginEffect::ToggleControlCenter);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-toggle-keyboard")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::OnScreenKeyboardShow) =>
                        {
                            approved.push(PluginEffect::ToggleOnScreenKeyboard);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-toggle-codex")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ProjectsMenuShow) =>
                        {
                            approved.push(PluginEffect::ToggleCodexProjects);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-activate-item")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::WindowsFocus)
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsLaunch) =>
                        {
                            let Some(index) = effect
                                .get("index")
                                .and_then(Value::as_u64)
                                .and_then(|index| usize::try_from(index).ok())
                            else {
                                self.last_error = Some("taskbar item index is invalid".into());
                                return;
                            };
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("taskbar item ID is missing".into());
                                return;
                            };
                            if id.len() > 256 || index >= 12 {
                                self.last_error = Some("taskbar item reference is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::ActivateTaskbarItem {
                                index,
                                id: id.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-context-item")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::WindowsContext) =>
                        {
                            let Some(index) = effect
                                .get("index")
                                .and_then(Value::as_u64)
                                .and_then(|index| usize::try_from(index).ok())
                            else {
                                self.last_error =
                                    Some("taskbar context item index is invalid".into());
                                return;
                            };
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("taskbar context item ID is missing".into());
                                return;
                            };
                            if id.is_empty() || id.len() > 256 || index >= 12 {
                                self.last_error =
                                    Some("taskbar context item reference is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::ContextTaskbarItem {
                                index,
                                id: id.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-activate-tray")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::TrayActivate) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("tray item ID is missing".into());
                                return;
                            };
                            if id.is_empty() || id.len() > 256 {
                                self.last_error = Some("tray item ID is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::ActivateTrayItem { id: id.to_owned() });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-context-tray")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::TrayContext) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("tray item ID is missing".into());
                                return;
                            };
                            if id.is_empty() || id.len() > 256 {
                                self.last_error = Some("tray item ID is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::ContextTrayItem { id: id.to_owned() });
                        }
                        Some(effect) if effect.starts_with("open-dialog:") => {
                            let id = &effect["open-dialog:".len()..];
                            match node.dialog(id) {
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
                        Some(effect) if effect.starts_with("open-menu:") => {
                            let id = &effect["open-menu:".len()..];
                            match node.menu(id) {
                                Some(PanelNode::Menu {
                                    id: declared,
                                    anchor,
                                    open: true,
                                    ..
                                }) if declared == id => {
                                    requested_dialog = Some((
                                        OverlayId::new(format!("plugin-menu-{id}")),
                                        UiId::from(anchor.clone()),
                                    ));
                                }
                                _ => {
                                    self.last_error =
                                        Some(format!("menu {id:?} is not declared and open"));
                                    return;
                                }
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-set-query")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsRead) =>
                        {
                            let Some(query) = effect.get("query").and_then(Value::as_str) else {
                                self.last_error = Some("launcher query must be text".into());
                                return;
                            };
                            if query.chars().count() > 512 {
                                self.last_error =
                                    Some("launcher query exceeds 512 characters".into());
                                return;
                            }
                            approved.push(PluginEffect::SetLauncherQuery(query.to_owned()));
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-set-page")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsRead) =>
                        {
                            let Some(page) = effect
                                .get("page")
                                .and_then(Value::as_u64)
                                .and_then(|page| usize::try_from(page).ok())
                            else {
                                self.last_error = Some("launcher page is invalid".into());
                                return;
                            };
                            if page > 10_000 {
                                self.last_error = Some("launcher page exceeds limit".into());
                                return;
                            }
                            let dashboard = match effect.get("view").and_then(Value::as_str) {
                                Some("dashboard") => true,
                                Some("search") => false,
                                _ => {
                                    self.last_error = Some("launcher page view is invalid".into());
                                    return;
                                }
                            };
                            approved.push(PluginEffect::SetLauncherPage { dashboard, page });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-activate-result")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsLaunch) =>
                        {
                            let Some(index) = effect
                                .get("index")
                                .and_then(Value::as_u64)
                                .and_then(|index| usize::try_from(index).ok())
                            else {
                                self.last_error = Some("launcher result index is invalid".into());
                                return;
                            };
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("launcher result ID is missing".into());
                                return;
                            };
                            approved.push(PluginEffect::ActivateLauncherResult {
                                index,
                                id: id.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-launch-dashboard")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsLaunch) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error =
                                    Some("dashboard application ID is missing".into());
                                return;
                            };
                            if id.is_empty() || id.len() > 256 {
                                self.last_error =
                                    Some("dashboard application ID is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::LaunchDashboardApplication {
                                id: id.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-set-view")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsRead) =>
                        {
                            let view = match effect.get("view").and_then(Value::as_str) {
                                Some("favorites") => LauncherView::Favorites,
                                Some("applications") => LauncherView::Applications,
                                Some("places") => LauncherView::Places,
                                _ => {
                                    self.last_error = Some("launcher view is invalid".into());
                                    return;
                                }
                            };
                            approved.push(PluginEffect::SetLauncherView(view));
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-toggle-pin")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsPin) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("pinned application ID is missing".into());
                                return;
                            };
                            if id.is_empty() || id.len() > 256 {
                                self.last_error = Some("pinned application ID is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::ToggleLauncherPin { id: id.to_owned() });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-open-settings")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::SettingsShow) =>
                        {
                            approved.push(PluginEffect::LauncherOpenSettings);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-open-account")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ControlCenterShow) =>
                        {
                            approved.push(PluginEffect::LauncherOpenAccount);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-open-project")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ProjectsOpen) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("project ID is missing".into());
                                return;
                            };
                            if id.is_empty() || id.len() > 256 {
                                self.last_error = Some("project ID is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::LauncherOpenProject { id: id.to_owned() });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-see-all-projects")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ProjectsRead) =>
                        {
                            approved.push(PluginEffect::LauncherSeeAllProjects);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-request-logout")
                            && self.manifest.id == launcher_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::SessionLogoutRequest) =>
                        {
                            approved.push(PluginEffect::LauncherRequestLogout);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("notification-invoke")
                            && self.manifest.id == notification_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::NotificationsAct) =>
                        {
                            let Some(id) = effect
                                .get("id")
                                .and_then(Value::as_u64)
                                .and_then(|id| u32::try_from(id).ok())
                                .filter(|id| *id != 0)
                            else {
                                self.last_error = Some("notification ID is invalid".into());
                                return;
                            };
                            let Some(key) = effect
                                .get("key")
                                .and_then(Value::as_str)
                                .filter(|key| !key.is_empty() && key.len() <= 128)
                            else {
                                self.last_error = Some("notification action key is invalid".into());
                                return;
                            };
                            approved.push(PluginEffect::InvokeNotification {
                                id,
                                key: key.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("notification-dismiss")
                            && self.manifest.id == notification_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::NotificationsAct) =>
                        {
                            let Some(id) = effect
                                .get("id")
                                .and_then(Value::as_u64)
                                .and_then(|id| u32::try_from(id).ok())
                                .filter(|id| *id != 0)
                            else {
                                self.last_error = Some("notification ID is invalid".into());
                                return;
                            };
                            approved.push(PluginEffect::DismissNotification { id });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("notification-close-history")
                            && self.manifest.id == notification_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::NotificationsRead) =>
                        {
                            approved.push(PluginEffect::CloseNotificationHistory);
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
        if self.manifest.id == launcher_manifest().id {
            AnyView::new(
                Container::new()
                    .fill_width()
                    .height(680.0)
                    .padding(Insets::all(20.0))
                    .background(0xf12b_303c)
                    .child(self.node.view(&self.images)),
            )
        } else if self.manifest.id == taskbar_manifest().id
            || self.manifest.id == notification_manifest().id
            || self.manifest.id == run_manifest().id
        {
            AnyView::new(self.node.view(&self.images))
        } else {
            AnyView::new(
                Column::new()
                    .fill_width()
                    .height(surface().height as f32)
                    .child(Spacer::flex())
                    .child(
                        Row::new()
                            .fill_width()
                            .child(Spacer::flex())
                            .child(self.node.view(&self.images))
                            .child(Spacer::flex()),
                    ),
            )
        }
    }

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let mut overlays = Vec::new();
        let mut transients = Vec::new();
        self.node.transients(&mut transients);
        for transient in transients {
            match transient {
                PanelNode::Dialog {
                    id,
                    anchor,
                    open: true,
                    width,
                    height,
                    children,
                } => {
                    let mut content = Column::new().fill_width();
                    for child in children {
                        content = content.child(child.view(&self.images));
                    }
                    overlays.push(FrameOverlay::surface(
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
                    ));
                }
                PanelNode::Menu {
                    id,
                    anchor,
                    open: true,
                    items,
                } => {
                    let mut menu = OverlayMenu::new(
                        format!("plugin-menu-{id}"),
                        OverlayAnchor::Node(UiId::from(anchor.clone())),
                    );
                    for item in items {
                        if let PanelNode::MenuItem { id, label, action } = item {
                            menu = menu.item(OverlayMenuItem::action(
                                id.clone(),
                                label.clone(),
                                PluginMessage::Click(*action),
                            ));
                        }
                    }
                    overlays.push(FrameOverlay::Menu(menu));
                }
                _ => {}
            }
        }
        overlays
    }

    fn take_transient_request(&mut self) -> Option<(OverlayId, UiId)> {
        self.pending_transient.take()
    }

    fn title(&self) -> &str {
        if self.manifest.id == launcher_manifest().id {
            "Plugin Launcher"
        } else if self.manifest.id == run_manifest().id {
            "Plugin Run Dialog"
        } else if self.manifest.id == taskbar_manifest().id {
            "Plugin Taskbar"
        } else {
            "Plugin Panel"
        }
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
        panel.update(PluginMessage::Click(0));
        assert!(format!("{:?}", panel.node).contains("Count: 1"));
        assert!(panel.last_error().is_none());
    }

    #[test]
    fn bundled_panel_dialog_opens_and_cancels() {
        let mut panel = PluginPanelApplication::bundled().expect("bundled plugin loads");
        panel.update(PluginMessage::Click(1));
        assert!(panel.pending_transient.is_some());
        assert!(matches!(
            panel.node.dialog("launcher-dialog"),
            Some(PanelNode::Dialog { open: true, .. })
        ));
        panel.pending_transient.take();
        panel.update(PluginMessage::Click(3));
        assert!(matches!(
            panel.node.dialog("launcher-dialog"),
            Some(PanelNode::Dialog { open: false, .. })
        ));
        assert!(panel.take_effects().is_empty());
        assert!(panel.last_error().is_none());
    }

    #[test]
    fn later_declared_dialog_can_open_by_id() {
        let source = r#"
            function App() {
                return h(Panel, null,
                    h(Button, {id: 'first-button', onClick: () => nickel.openDialog('first')}, 'First'),
                    h(Button, {id: 'second-button', onClick: () => nickel.openDialog('second')}, 'Second'),
                    h(Dialog, {id: 'first', anchor: 'first-button', open: true}, h(Text, null, 'First dialog')),
                    h(Dialog, {id: 'second', anchor: 'second-button', open: true}, h(Text, null, 'Second dialog')));
            }
        "#;
        let mut panel = PluginPanelApplication::new(source).unwrap();
        panel.update(PluginMessage::Click(1));
        assert_eq!(
            panel.pending_transient,
            Some((OverlayId::new("plugin-second"), UiId::from("second-button")))
        );
        assert!(panel.last_error().is_none());
    }

    #[test]
    fn run_plugin_submits_bounded_command_and_shows_host_error() {
        let mut panel = PluginPanelApplication::run_with_status(None).unwrap();
        panel.update(PluginMessage::Text(0, "  nickel-test  ".into()));
        let submit = panel.node.button_action("run-submit").unwrap();
        panel.update(PluginMessage::Click(submit));
        assert_eq!(
            panel.take_effects(),
            vec![PluginEffect::RunSubmit("nickel-test".into())]
        );
        assert!(
            panel
                .sync_run_status(Some("Could not run command: missing"))
                .unwrap()
        );
        assert!(format!("{:?}", panel.node).contains("Could not run command: missing"));
        panel.update(PluginMessage::Text(0, "x".repeat(5000)));
        let submit = panel.node.button_action("run-submit").unwrap();
        panel.update(PluginMessage::Click(submit));
        assert_eq!(
            panel.take_effects(),
            vec![PluginEffect::RunSubmit("x".repeat(4096))]
        );
        assert_eq!(panel.manifest.id, run_manifest().id);
        assert!(panel.last_error().is_none());
    }

    #[test]
    fn run_plugin_escape_requests_dismissal() {
        let mut panel = PluginPanelApplication::run_with_status(None).unwrap();
        assert_eq!(
            panel.shortcut_outcome(Shortcut::Escape).disposition,
            nickel_ui::EventDisposition::Handled
        );
        assert_eq!(panel.take_effects(), vec![PluginEffect::RunDismiss]);
    }

    #[test]
    fn run_plugin_host_accepts_text_and_submit_from_focused_field() {
        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::run_with_status(None).unwrap(),
            620,
            180,
        );
        host.step(nickel_ui::HostBatch {
            window_focused: Some(true),
            ..nickel_ui::HostBatch::default()
        });
        let field = host
            .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
            .unwrap();
        let field_id = field.id;
        host.request_focus(field_id.clone());
        assert_eq!(host.inspect().keyboard_focus, Some(field_id));
        host.handle_event(nickel_ui::UiEvent::TextInput("nickel-test".into()));
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Shortcut(Shortcut::Submit)],
            ..nickel_ui::HostBatch::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::RunSubmit("nickel-test".into())]
        );
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Shortcut(Shortcut::Escape)],
            ..nickel_ui::HostBatch::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::RunDismiss]
        );
    }

    #[test]
    fn launcher_plugin_submit_does_not_bypass_open_menu() {
        let mut launcher = Launcher::new(vec![crate::launcher::Application::new(
            "org.nickel.demo".into(),
            "Demo".into(),
            None,
            None,
            None,
        )]);
        launcher.set_query("demo");
        let mut panel = PluginPanelApplication::launcher(&launcher).unwrap();
        panel.set_overlay_open(true);
        assert_eq!(
            panel.shortcut_outcome(Shortcut::Submit).disposition,
            nickel_ui::EventDisposition::Unhandled
        );
        assert!(panel.take_effects().is_empty());
    }

    #[test]
    fn keyed_components_keep_state_when_a_sibling_unmounts() {
        let source = r#"
            function Counter(props) {
                const [count, setCount] = useState(0);
                return h(Button, {id: props.id, onClick: () => setCount(count + 1)}, props.id + ':' + count);
            }
            function App() {
                const [showFirst, setShowFirst] = useState(true);
                return h(Panel, null,
                    h(Button, {id: 'toggle', onClick: () => setShowFirst(!showFirst)}, 'Toggle'),
                    showFirst ? h(Counter, {key: 'first', id: 'first'}) : null,
                    h(Counter, {key: 'second', id: 'second'}));
            }
        "#;
        let mut panel = PluginPanelApplication::new(source).unwrap();
        panel.update(PluginMessage::Click(2));
        assert!(format!("{:?}", panel.node).contains("second:1"));
        panel.update(PluginMessage::Click(0));
        let without_first = format!("{:?}", panel.node);
        assert!(!without_first.contains("first:0"));
        assert!(without_first.contains("second:1"));
        panel.update(PluginMessage::Click(0));
        let restored = format!("{:?}", panel.node);
        assert!(restored.contains("first:0"));
        assert!(restored.contains("second:1"));
    }

    #[test]
    fn duplicate_component_keys_are_rejected() {
        let source = r#"
            function Child() { return h(Text, null, 'child'); }
            function App() {
                return h(Panel, null, h(Child, {key: 'same'}), h(Child, {key: 'same'}));
            }
        "#;
        assert!(
            PluginPanelApplication::new(source)
                .err()
                .is_some_and(|error| error.contains("duplicate component key"))
        );
    }
}
