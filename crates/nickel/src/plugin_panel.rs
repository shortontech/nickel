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
    AnyView, Column, ComponentBuilderExt, Container, DragGesture, DragPhase, FilePlaneItem,
    FrameOverlay, Image, ImageFit, Insets, Layer, OverlayAnchor, OverlayId, OverlayMenu,
    OverlayMenuItem, OverlayStyle, Point, Row, SemanticRole, Shortcut, Size, Spacer, Text,
    TextField as UiTextField, TransientSurface, UiId, VerticalScroll, ViewContext,
};
use serde_json::Value;

use crate::control_view::ControlAction;
use crate::platform::SessionAction;
use nickel_core::display_projection::ProjectionMode;

pub use crate::launcher::LauncherView;
use crate::launcher::{Application, DashboardSection, Launcher, LauncherMode, TaskbarApplication};
use crate::notification::DesktopNotification;
use crate::window_preview::PreviewAction;

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

pub fn volume_osd_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!(
            "../../../assets/plugins/volume-osd/plugin.json"
        ))
        .expect("bundled volume OSD plugin manifest must be valid")
    })
}

pub fn control_center_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!(
            "../../../assets/plugins/control-center/plugin.json"
        ))
        .expect("bundled control center plugin manifest must be valid")
    })
}

pub fn window_preview_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!(
            "../../../assets/plugins/window-preview/plugin.json"
        ))
        .expect("bundled window preview plugin manifest must be valid")
    })
}

pub fn desktop_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!("../../../assets/plugins/desktop/plugin.json"))
            .expect("bundled desktop plugin manifest must be valid")
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
    true
}

pub fn taskbar_enabled() -> bool {
    true
}

pub fn launcher_enabled() -> bool {
    true
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
const Surface = 'surface';
const Box = 'box';
const FileTile = 'file-tile';
const Badge = 'badge';
const Row = 'row';
const Column = 'column';
const ScrollView = 'scroll-view';
const Text = 'text';
const Image = 'image';
const ImageButton = 'image-button';
const Progress = 'progress';
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
let __pendingEvent = null;
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
    const dragAction = typeof props?.onDrag === 'function'
        ? __handlers.push(props.onDrag) - 1 : null;
    const closeAction = typeof props?.onClose === 'function'
        ? __handlers.push(props.onClose) - 1 : null;
    return {kind, action, id: props?.id, open: props?.open, anchor: props?.anchor,
        x: props?.x, y: props?.y, width: props?.width, height: props?.height,
        background: props?.background, radius: props?.radius, color: props?.color,
        label: props?.label, selected: props?.selected, hovered: props?.hovered,
        dragging: props?.dragging, outline: props?.outline,
        hoverBackground: props?.hoverBackground,
        selectedBackground: props?.selectedBackground, accent: props?.accent,
        complement: props?.complement,
        item: props?.item, count: props?.count,
        asset: props?.asset, fit: props?.fit,
        accessibilityLabel: props?.accessibilityLabel, icon: props?.icon,
        showLabel: props?.showLabel, contextAction, dragAction, closeAction,
        value: props?.value, placeholder: props?.placeholder,
        percent: props?.percent,
        children: children.flat(Infinity).filter(child => child !== null && child !== false)};
}

function __nickelRollbackRender() {
    if (__pendingRender !== null) {
        const {handlers, hooks, values, effectsLength} = __pendingRender;
        __handlers = handlers;
        __nickelRestoreHooks(hooks, values, effectsLength);
        __pendingRender = null;
    }
    __nickelRollbackEvent();
}

function __nickelRestoreHooks(hooks, values, effectsLength) {
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
}

function __nickelRollbackEvent() {
    if (__pendingEvent === null) return;
    const {handlers, hooks, values, effectsLength, effects} = __pendingEvent;
    __handlers = handlers;
    __nickelRestoreHooks(hooks, values, effectsLength);
    __effects = effects;
    __pendingEvent = null;
}

function __nickelCommitRender() {
    __pendingRender = null;
}

function __nickelAcceptEvent() {
    __pendingEvent = null;
}

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
    if (!handler) return __nickelRender();
    const hooks = new Map(Array.from(__componentHooks, ([path, slots]) => [path, slots.slice()]));
    const values = Array.from(__componentHooks.values(), slots => slots.map(entry =>
        entry.kind === 'ref' ? entry.value.current : entry.value));
    const effectsLength = __effects.length;
    __pendingEvent = {handlers: __handlers, hooks, values,
        effectsLength, effects: __effects.slice()};
    try {
        handler(value);
        return __nickelRender();
    } catch (error) {
        __nickelRollbackEvent();
        throw error;
    }
}
"#;

#[derive(Clone, Debug, PartialEq)]
enum PanelNode {
    Badge {
        item: Option<String>,
        label: String,
        count: u16,
        color: u32,
    },
    FileTile {
        asset: String,
        label: String,
        x: f32,
        y: f32,
        width: f32,
        height: f32,
        selected: bool,
        hovered: bool,
        dragging: bool,
        foreground: u32,
        outline: u32,
        hover_background: u32,
        selected_background: u32,
        accent: u32,
        complement: u32,
    },
    Box {
        children: Vec<Self>,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        background: u32,
        radius: u32,
    },
    Surface {
        children: Vec<Self>,
        background: u32,
        width: u32,
        height: u32,
    },
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
    Text {
        value: String,
        color: u32,
    },
    Image {
        id: Option<String>,
        asset: String,
        accessibility_label: Option<String>,
        width: u32,
        height: u32,
        fit: ImageFit,
        action: Option<usize>,
        context_action: Option<usize>,
    },
    Progress {
        percent: u8,
        width: u32,
        height: u32,
    },
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
        drag_action: Option<usize>,
    },
    Dialog {
        id: String,
        anchor: String,
        open: bool,
        close_action: Option<usize>,
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
            "badge" => {
                if !children.is_empty() {
                    return Err("badge cannot have children".into());
                }
                let count = value
                    .get("count")
                    .and_then(Value::as_u64)
                    .filter(|count| *count <= 999)
                    .ok_or("badge count must be 0 to 999")? as u16;
                let item = value
                    .get("item")
                    .and_then(Value::as_str)
                    .filter(|item| !item.is_empty() && item.len() <= 256)
                    .map(str::to_owned);
                let label = value
                    .get("label")
                    .and_then(Value::as_str)
                    .filter(|label| !label.is_empty() && label.len() <= 64)
                    .unwrap_or("Badge")
                    .to_owned();
                Ok(Self::Badge {
                    item,
                    label,
                    count,
                    color: value
                        .get("color")
                        .and_then(Value::as_u64)
                        .filter(|color| *color <= u32::MAX as u64)
                        .map_or(0xffc9354c, |color| color as u32),
                })
            }
            "file-tile" => {
                if !children.is_empty() {
                    return Err("file tile cannot have children".into());
                }
                let number = |name: &str, min: f64, max: f64| {
                    value
                        .get(name)
                        .and_then(Value::as_f64)
                        .filter(|number| number.is_finite() && (min..=max).contains(number))
                        .map(|number| number as f32)
                        .ok_or_else(|| format!("file tile {name} must be {min} to {max}"))
                };
                let color = |name: &str| {
                    value
                        .get(name)
                        .and_then(Value::as_u64)
                        .filter(|color| *color <= u32::MAX as u64)
                        .map(|color| color as u32)
                        .ok_or_else(|| format!("file tile {name} needs a color"))
                };
                Ok(Self::FileTile {
                    asset: value
                        .get("asset")
                        .and_then(Value::as_str)
                        .filter(|asset| !asset.is_empty() && asset.len() <= 128)
                        .ok_or("file tile needs an asset")?
                        .to_owned(),
                    label: value
                        .get("label")
                        .and_then(Value::as_str)
                        .filter(|label| !label.is_empty() && label.len() <= 1024)
                        .ok_or("file tile needs a label")?
                        .to_owned(),
                    x: number("x", -8192.0, 8192.0)?,
                    y: number("y", -8192.0, 8192.0)?,
                    width: number("width", 1.0, 8192.0)?,
                    height: number("height", 1.0, 8192.0)?,
                    selected: value
                        .get("selected")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    hovered: value
                        .get("hovered")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    dragging: value
                        .get("dragging")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    foreground: color("color")?,
                    outline: color("outline")?,
                    hover_background: color("hoverBackground")?,
                    selected_background: color("selectedBackground")?,
                    accent: color("accent")?,
                    complement: color("complement")?,
                })
            }
            "box" => {
                let coordinate = |name| {
                    value
                        .get(name)
                        .and_then(Value::as_i64)
                        .filter(|position| (-8192..=8192).contains(position))
                        .map(|position| position as i32)
                        .ok_or_else(|| format!("box {name} must be -8192 to 8192"))
                };
                let dimension = |name| {
                    value
                        .get(name)
                        .and_then(Value::as_u64)
                        .filter(|size| (1..=8192).contains(size))
                        .map(|size| size as u32)
                        .ok_or_else(|| format!("box {name} must be 1 to 8192"))
                };
                Ok(Self::Box {
                    children: children
                        .iter()
                        .filter(|child| !child.is_null())
                        .map(Self::parse)
                        .collect::<Result<Vec<_>, _>>()?,
                    x: coordinate("x")?,
                    y: coordinate("y")?,
                    width: dimension("width")?,
                    height: dimension("height")?,
                    background: value
                        .get("background")
                        .and_then(Value::as_u64)
                        .map_or(0, |color| color as u32),
                    radius: value
                        .get("radius")
                        .and_then(Value::as_u64)
                        .filter(|radius| *radius <= 256)
                        .unwrap_or(0) as u32,
                })
            }
            "surface" => {
                let dimension = |name| {
                    value
                        .get(name)
                        .and_then(Value::as_u64)
                        .filter(|size| (1..=8192).contains(size))
                        .map(|size| size as u32)
                        .ok_or_else(|| format!("surface {name} must be 1 to 8192"))
                };
                Ok(Self::Surface {
                    children: children
                        .iter()
                        .filter(|value| !value.is_null())
                        .map(Self::parse)
                        .collect::<Result<Vec<_>, _>>()?,
                    background: value
                        .get("background")
                        .and_then(Value::as_u64)
                        .map_or(0xff202124, |color| color as u32),
                    width: dimension("width")?,
                    height: dimension("height")?,
                })
            }
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
            "text" => Ok(Self::Text {
                value: child_text(children)?,
                color: value
                    .get("color")
                    .and_then(Value::as_u64)
                    .map_or(0xfff4f6fa, |color| color as u32),
            }),
            "image" | "image-button" => {
                if !children.is_empty() {
                    return Err("image cannot have children".into());
                }
                let asset = value
                    .get("asset")
                    .and_then(Value::as_str)
                    .filter(|asset| !asset.is_empty() && asset.len() <= 128)
                    .ok_or("image asset must contain 1 to 128 characters")?
                    .to_owned();
                let dimension = |name| {
                    value
                        .get(name)
                        .and_then(Value::as_u64)
                        .filter(|size| (1..=8192).contains(size))
                        .map(|size| size as u32)
                        .ok_or_else(|| format!("image {name} must be 1 to 8192"))
                };
                let fit = match value
                    .get("fit")
                    .and_then(Value::as_str)
                    .unwrap_or("contain")
                {
                    "contain" => ImageFit::Contain,
                    "cover" => ImageFit::Cover,
                    "stretch" => ImageFit::Stretch,
                    _ => return Err("image fit must be contain, cover, or stretch".into()),
                };
                let accessibility_label = value
                    .get("accessibilityLabel")
                    .and_then(Value::as_str)
                    .filter(|label| !label.is_empty() && label.len() <= 256)
                    .map(str::to_owned);
                let (id, action, context_action) = if kind == "image-button" {
                    let id = value
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty() && id.len() <= 128)
                        .ok_or("image button ID must contain 1 to 128 characters")?;
                    if accessibility_label.is_none() {
                        return Err("image button needs an accessibility label".into());
                    }
                    let action = value
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok())
                        .ok_or("image button needs an onClick handler")?;
                    let context = value
                        .get("contextAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok());
                    (Some(id.to_owned()), Some(action), context)
                } else {
                    (None, None, None)
                };
                Ok(Self::Image {
                    id,
                    asset,
                    accessibility_label,
                    width: dimension("width")?,
                    height: dimension("height")?,
                    fit,
                    action,
                    context_action,
                })
            }
            "progress" => {
                if !children.is_empty() {
                    return Err("progress cannot have children".into());
                }
                let percent = value
                    .get("percent")
                    .and_then(Value::as_u64)
                    .filter(|percent| *percent <= 100)
                    .ok_or("progress percent must be 0 to 100")?
                    as u8;
                let width = value
                    .get("width")
                    .and_then(Value::as_u64)
                    .filter(|width| (1..=8192).contains(width))
                    .ok_or("progress width must be 1 to 8192")? as u32;
                let height = value
                    .get("height")
                    .and_then(Value::as_u64)
                    .filter(|height| (1..=256).contains(height))
                    .ok_or("progress height must be 1 to 256")? as u32;
                Ok(Self::Progress {
                    percent,
                    width,
                    height,
                })
            }
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
                    drag_action: value
                        .get("dragAction")
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
                close_action: value
                    .get("closeAction")
                    .and_then(Value::as_u64)
                    .and_then(|action| usize::try_from(action).ok()),
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
            Self::Badge {
                label,
                count,
                color,
                ..
            } => AnyView::new(
                Container::new()
                    .width(30.0)
                    .height(24.0)
                    .background(*color)
                    .radius(12.0)
                    .semantic_role(SemanticRole::Status)
                    .accessibility_label(format!("{label}: {count}"))
                    .child(
                        Text::new(count.to_string())
                            .width(30.0)
                            .height(24.0)
                            .align(nickel_ui::TextAlign::Center)
                            .color(0xffffffff)
                            .scale(0.72),
                    ),
            ),
            Self::FileTile {
                asset,
                label,
                x,
                y,
                width,
                height,
                selected,
                hovered,
                dragging,
                foreground,
                outline,
                hover_background,
                selected_background,
                accent,
                complement,
            } => {
                let icon = images.get(asset).map_or_else(
                    || Arc::new(image::RgbaImage::new(1, 1)),
                    |(_, icon)| Arc::clone(icon),
                );
                let icon_id = images.get(asset).map_or(0, |(id, _)| *id);
                let mut tile = FilePlaneItem::new(
                    PluginMessage::Click(usize::MAX),
                    label.clone(),
                    icon_id,
                    icon,
                )
                .position(Point { x: *x, y: *y })
                .width(*width)
                .height(*height)
                .padding(Insets {
                    top: 6.0,
                    right: 3.0,
                    bottom: 8.0,
                    left: 3.0,
                })
                .radius(8.0)
                .semantic_role(SemanticRole::GridCell)
                .accessibility_label(label.clone())
                .interaction_backgrounds(*hover_background, *selected_background)
                .selected_background(*selected, *selected_background)
                .hovered_background(!*selected && *hovered, *hover_background)
                .focus_background_tint(*accent)
                .controller_focus_background_tint(*complement)
                .icon_size(48.0)
                .label_height((*height - 70.0).max(1.0))
                .label_scale(0.85)
                .foreground(*foreground)
                .label_outline(*outline, 1.0)
                .gap(8.0);
                if *dragging {
                    tile = tile.border(*accent, 2.0);
                }
                AnyView::new(tile)
            }
            Self::Box {
                children,
                x,
                y,
                width,
                height,
                background,
                radius,
            } => {
                let mut column = Column::new().fill_width();
                for child in children {
                    column = column.child(child.view(images));
                }
                AnyView::new(
                    Container::new()
                        .position(Point {
                            x: *x as f32,
                            y: *y as f32,
                        })
                        .width(*width as f32)
                        .height(*height as f32)
                        .background(*background)
                        .radius(*radius as f32)
                        .child(column),
                )
            }
            Self::Surface {
                children,
                background,
                width,
                height,
            } => {
                let mut layer = Layer::new().width(*width as f32).height(*height as f32);
                for child in children {
                    if !matches!(child, Self::Dialog { .. }) {
                        layer = layer.child(child.view(images));
                    }
                }
                AnyView::new(
                    Container::new()
                        .width(*width as f32)
                        .height(*height as f32)
                        .background(*background)
                        .child(layer),
                )
            }
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
            Self::Text { value, color } => AnyView::new(
                Container::new()
                    .semantic_role(SemanticRole::Text)
                    .accessibility_label(value.clone())
                    .height(48.0)
                    .padding(Insets::all(10.0))
                    .child(Text::new(value).color(*color).scale(1.0)),
            ),
            Self::Image {
                id,
                asset,
                accessibility_label,
                width,
                height,
                fit,
                action,
                context_action,
            } => {
                let visual = images.get(asset).map_or_else(
                    || {
                        AnyView::new(
                            Container::new()
                                .width(*width as f32)
                                .height(*height as f32)
                                .background(0xff30343d),
                        )
                    },
                    |(image_id, image)| {
                        AnyView::new(
                            Image::new(*image_id, Arc::clone(image))
                                .width(*width as f32)
                                .height(*height as f32)
                                .fit(*fit)
                                .decorative(),
                        )
                    },
                );
                let mut container = Container::new()
                    .width(*width as f32)
                    .height(*height as f32)
                    .child(visual);
                if let Some(action) = action {
                    container = container
                        .id(id.as_ref().expect("image button has an ID").clone())
                        .semantic_role(SemanticRole::Button)
                        .accessibility_label(
                            accessibility_label
                                .as_ref()
                                .expect("image button has a label")
                                .clone(),
                        )
                        .message(PluginMessage::Click(*action));
                    if let Some(context_action) = context_action {
                        container =
                            container.context_message(PluginMessage::Context(*context_action));
                    }
                } else if let Some(label) = accessibility_label {
                    container = container
                        .semantic_role(SemanticRole::Image)
                        .accessibility_label(label.clone());
                }
                AnyView::new(container)
            }
            Self::Progress {
                percent,
                width,
                height,
            } => AnyView::new(
                Container::new()
                    .semantic_role(SemanticRole::Status)
                    .accessibility_label(format!("{percent}%"))
                    .width(*width as f32)
                    .height(*height as f32)
                    .background(0xff4a5262)
                    .radius(*height as f32 / 2.0)
                    .child(
                        Container::new()
                            .width(*width as f32 * f32::from(*percent) / 100.0)
                            .height(*height as f32)
                            .background(0xff7ba6ff)
                            .radius(*height as f32 / 2.0),
                    ),
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
                drag_action,
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
                if let Some(action) = drag_action {
                    container = container.on_drag((PluginMessage::Click(*action), map_plugin_drag));
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
            Self::Box { children, .. }
            | Self::Surface { children, .. }
            | Self::Panel { children, .. }
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
            Self::Box { children, .. }
            | Self::Surface { children, .. }
            | Self::Panel { children, .. }
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
            Self::Box { children, .. }
            | Self::Surface { children, .. }
            | Self::Panel { children, .. }
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
            Self::Image {
                id: Some(id),
                action: Some(action),
                ..
            } if id == requested_id => Some(*action),
            Self::Box { children, .. }
            | Self::Surface { children, .. }
            | Self::Panel { children, .. }
            | Self::Row(children)
            | Self::Column(children)
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.button_action(requested_id)),
            _ => None,
        }
    }

    fn collect_taskbar_badges(
        &self,
        badges: &mut Vec<(String, String, u16, u32)>,
    ) -> Result<(), String> {
        match self {
            Self::Badge {
                item: Some(item),
                label,
                count,
                color,
            } => {
                if badges.len() >= 32 {
                    return Err("extension has too many badges".into());
                }
                badges.push((item.clone(), label.clone(), *count, *color));
                Ok(())
            }
            Self::Row(children) | Self::Column(children) => {
                for child in children {
                    child.collect_taskbar_badges(badges)?;
                }
                Ok(())
            }
            _ => Err("taskbar badge extension must return badges or a row/column of badges".into()),
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

#[derive(Clone, Debug, PartialEq)]
pub enum PluginEffect {
    ShowLauncher,
    RunSubmit(String),
    RunDismiss,
    ToggleLauncher,
    ToggleOnScreenKeyboard,
    ToggleCodexProjects,
    SetLauncherQuery(String),
    SetLauncherPage {
        dashboard: bool,
        page: usize,
    },
    ActivateLauncherResult {
        index: usize,
        id: String,
    },
    LaunchDashboardApplication {
        id: String,
    },
    SetLauncherView(LauncherView),
    ToggleLauncherPin {
        id: String,
    },
    DismissLauncher,
    LauncherOpenSettings,
    LauncherOpenAccount,
    LauncherOpenProject {
        id: String,
    },
    LauncherSeeAllProjects,
    LauncherRequestLogout,
    ActivateTaskbarItem {
        index: usize,
        id: String,
    },
    ContextTaskbarItem {
        index: usize,
        id: String,
    },
    MoveTaskbarPin {
        index: usize,
        id: String,
        direction: i8,
    },
    ToggleTaskbarMenuPin {
        id: String,
    },
    CloseTaskbarMenuWindows,
    InvokeTaskbarWindowMenu {
        page: String,
        index: usize,
    },
    ActivateTrayItem {
        id: String,
    },
    ContextTrayItem {
        id: String,
    },
    ToggleControlCenter,
    InvokeNotification {
        id: u32,
        key: String,
    },
    DismissNotification {
        id: u32,
    },
    CloseNotificationHistory,
    Control(ControlAction),
    Preview(PreviewAction),
}

fn preview_request(effect: &Value) -> Result<(PreviewAction, PluginCapability), String> {
    let action = effect
        .get("action")
        .and_then(Value::as_str)
        .ok_or("preview action is missing")?;
    let window = effect
        .get("window")
        .and_then(Value::as_str)
        .and_then(|value| value.parse::<u64>().ok())
        .map(crate::model::WindowId)
        .ok_or("preview window ID is invalid")?;
    match action {
        "activate" => Ok((
            PreviewAction::Activate(window),
            PluginCapability::WindowsFocus,
        )),
        "close" => Ok((
            PreviewAction::Close(window),
            PluginCapability::WindowsContext,
        )),
        "menu" => Ok((
            PreviewAction::OpenMenu(window),
            PluginCapability::WindowsContext,
        )),
        _ => Err("unknown preview action".into()),
    }
}

fn control_request(effect: &Value) -> Result<(ControlAction, PluginCapability), String> {
    let action = effect
        .get("action")
        .and_then(Value::as_str)
        .ok_or("control action is missing")?;
    let boolean = || {
        effect
            .get("value")
            .and_then(Value::as_bool)
            .ok_or_else(|| "control value must be a boolean".to_owned())
    };
    let id = || {
        effect
            .get("value")
            .and_then(Value::as_str)
            .filter(|id| !id.is_empty() && id.len() <= 256)
            .map(str::to_owned)
            .ok_or_else(|| "control item ID is invalid".to_owned())
    };
    let workspace = || {
        effect
            .get("value")
            .and_then(Value::as_u64)
            .ok_or_else(|| "workspace ID is invalid".to_owned())
    };
    match action {
        "wifi-power" => Ok((
            ControlAction::SetWifiEnabled(boolean()?),
            PluginCapability::NetworkControl,
        )),
        "wifi-activate" => Ok((
            ControlAction::ActivateWifi { id: id()? },
            PluginCapability::NetworkControl,
        )),
        "bluetooth-power" => Ok((
            ControlAction::SetBluetoothPowered(boolean()?),
            PluginCapability::BluetoothControl,
        )),
        "bluetooth-scan" => Ok((
            ControlAction::SetBluetoothDiscovery(boolean()?),
            PluginCapability::BluetoothControl,
        )),
        "bluetooth-device" => Ok((
            ControlAction::ToggleBluetoothDevice { id: id()? },
            PluginCapability::BluetoothControl,
        )),
        "audio-mute" => Ok((
            ControlAction::SetAudioMuted(boolean()?),
            PluginCapability::AudioControl,
        )),
        "audio-volume" => {
            let percent = effect
                .get("value")
                .and_then(Value::as_u64)
                .filter(|value| *value <= 100)
                .ok_or("audio volume must be 0 to 100")? as u8;
            Ok((
                ControlAction::SetAudioVolume(percent),
                PluginCapability::AudioControl,
            ))
        }
        "audio-device" => Ok((
            ControlAction::SelectAudioDevice { id: id()? },
            PluginCapability::AudioControl,
        )),
        "workspace-switch" => Ok((
            ControlAction::SwitchWorkspace(workspace()?),
            PluginCapability::WorkspacesSwitch,
        )),
        "workspace-create" => Ok((
            ControlAction::CreateWorkspace,
            PluginCapability::WorkspacesSwitch,
        )),
        "workspace-remove" => Ok((
            ControlAction::RemoveWorkspace(workspace()?),
            PluginCapability::WorkspacesSwitch,
        )),
        "show-desktop" => Ok((
            ControlAction::ToggleShowDesktop,
            PluginCapability::DesktopControl,
        )),
        "show-notifications" => Ok((
            ControlAction::ShowNotifications,
            PluginCapability::NotificationsRead,
        )),
        "projection-preview" => {
            let mode = match effect.get("value").and_then(Value::as_str) {
                Some("internal") => ProjectionMode::InternalOnly,
                Some("duplicate") => ProjectionMode::Duplicate,
                Some("extend") => ProjectionMode::Extend,
                Some("external") => ProjectionMode::ExternalOnly,
                _ => return Err("display mode is invalid".into()),
            };
            Ok((
                ControlAction::PreviewProjection(mode),
                PluginCapability::DisplayControl,
            ))
        }
        "projection-confirm" => Ok((
            ControlAction::ConfirmProjection,
            PluginCapability::DisplayControl,
        )),
        "projection-cancel" => Ok((
            ControlAction::CancelProjection,
            PluginCapability::DisplayControl,
        )),
        "session-lock" => Ok((
            ControlAction::SessionAction(SessionAction::Lock),
            PluginCapability::SessionControl,
        )),
        "session-prepare" => {
            let action = match effect.get("value").and_then(Value::as_str) {
                Some("suspend") => SessionAction::Suspend,
                Some("restart-shell") => SessionAction::RestartShell,
                Some("logout") => SessionAction::LogOut,
                Some("reboot") => SessionAction::Reboot,
                Some("poweroff") => SessionAction::PowerOff,
                _ => return Err("session action is invalid".into()),
            };
            Ok((
                ControlAction::RequestSessionAction(action),
                PluginCapability::SessionControl,
            ))
        }
        "session-confirm" => Ok((
            ControlAction::ConfirmSessionAction,
            PluginCapability::SessionControl,
        )),
        "session-cancel" => Ok((
            ControlAction::CancelSessionAction,
            PluginCapability::SessionControl,
        )),
        _ => Err("unknown control action".into()),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginMessage {
    Click(usize),
    Context(usize),
    Drag(usize, DragGesture),
    Text(usize, String),
    Scroll,
}

fn map_plugin_drag(seed: PluginMessage, gesture: DragGesture) -> PluginMessage {
    let PluginMessage::Click(action) = seed else {
        unreachable!("plugin drag seed retains its handler")
    };
    PluginMessage::Drag(action, gesture)
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
    pub status: Option<String>,
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
    pub badges: Vec<TaskbarPluginBadge>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarPluginTrayItem {
    pub id: String,
    pub title: String,
    pub icon: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarPluginBadge {
    pub label: String,
    pub count: u16,
    pub color: u32,
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
pub struct TaskbarMenuPluginProjection {
    pub application_id: Option<String>,
    pub pinned: bool,
    pub close_all: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VolumeOsdPluginProjection {
    pub label: String,
    pub percent: u8,
}

impl VolumeOsdPluginProjection {
    fn to_json(&self) -> String {
        serde_json::json!({
            "label": self.label.chars().take(640).collect::<String>(),
            "percent": self.percent.min(100),
        })
        .to_string()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarWindowMenuPluginProjection {
    pub root: Vec<(String, Option<&'static str>)>,
    pub workspaces: Vec<(String, Option<&'static str>)>,
    pub displays: Vec<(String, Option<&'static str>)>,
}

impl TaskbarWindowMenuPluginProjection {
    fn to_json(&self) -> String {
        let entries = |items: &Vec<(String, Option<&'static str>)>| {
            items
                .iter()
                .take(32)
                .map(|(label, navigate)| {
                    serde_json::json!({
                        "label": label.chars().take(120).collect::<String>(),
                        "navigate": navigate,
                    })
                })
                .collect::<Vec<_>>()
        };
        serde_json::json!({
            "root": entries(&self.root),
            "workspaces": entries(&self.workspaces),
            "displays": entries(&self.displays),
        })
        .to_string()
    }
}

impl TaskbarMenuPluginProjection {
    fn to_json(&self) -> String {
        serde_json::json!({
            "applicationId": self.application_id,
            "pinned": self.pinned,
            "closeAll": self.close_all,
        })
        .to_string()
    }
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
                        badges: Vec::new(),
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
            "badges": item.badges.iter().map(|badge| serde_json::json!({
                "label": badge.label, "count": badge.count, "color": badge.color,
            })).collect::<Vec<_>>(),
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
            status: None,
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

    pub(crate) fn with_status(mut self, status: Option<String>) -> Self {
        self.status = status.map(|status| status.chars().take(160).collect());
        self
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
        serde_json::json!({"query": self.query, "status": self.status, "dashboardVisible": self.dashboard_visible, "view": view,
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
    pub(crate) fn button_message(&self, id: &str) -> Option<PluginMessage> {
        self.node.button_action(id).map(PluginMessage::Click)
    }

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

    pub fn from_package_with_settings(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Self, String> {
        let data = serde_json::json!({ "settings": settings }).to_string();
        Self::new_with_manifest(&package.source, &package.manifest, Some(data))
    }

    pub fn from_package_surface(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
        surface: &PluginSurface,
    ) -> Result<Self, String> {
        let data = serde_json::json!({
            "settings": settings,
            "surface": {
                "id": surface.id,
                "kind": surface.kind.as_str(),
                "width": surface.width,
                "height": surface.height,
            },
        })
        .to_string();
        Self::new_with_manifest(&package.source, &package.manifest, Some(data))
    }

    pub fn validate_package(package: &PluginPackage) -> Result<(), String> {
        let settings: std::collections::BTreeMap<_, _> = package
            .manifest
            .settings
            .iter()
            .map(|setting| (setting.id.clone(), setting.kind.default_value()))
            .collect();
        if package.manifest.surfaces.is_empty() {
            let application = Self::from_package_with_settings(package, &settings)?;
            if !package.manifest.contributes.is_empty() {
                application.taskbar_badges()?;
            }
        } else {
            for surface in &package.manifest.surfaces {
                Self::from_package_surface(package, &settings, surface)
                    .map_err(|error| format!("surface {:?}: {error}", surface.id))?;
            }
        }
        Ok(())
    }

    pub fn taskbar_badges(&self) -> Result<Vec<(String, String, u16, u32)>, String> {
        let mut badges = Vec::new();
        self.node.collect_taskbar_badges(&mut badges)?;
        if badges.is_empty() {
            return Err("taskbar badge extension did not return a badge".into());
        }
        Ok(badges)
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

    pub fn taskbar_menu_with_projection(
        projection: &TaskbarMenuPluginProjection,
    ) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/taskbar/menu.js");
        Self::new_with_manifest(source, taskbar_manifest(), Some(projection.to_json()))
    }

    pub fn taskbar_window_menu_with_projection(
        projection: &TaskbarWindowMenuPluginProjection,
    ) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/taskbar/window-menu.js");
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

    pub fn volume_osd_with_projection(
        projection: &VolumeOsdPluginProjection,
    ) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/volume-osd/main.js");
        Self::new_with_manifest(source, volume_osd_manifest(), Some(projection.to_json()))
    }

    pub fn control_center_with_data(data: &Value) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/control-center/main.js");
        Self::new_with_manifest(source, control_center_manifest(), Some(data.to_string()))
    }

    pub fn window_preview_with_data(data: &Value) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/window-preview/main.js");
        Self::new_with_manifest(source, window_preview_manifest(), Some(data.to_string()))
    }

    pub fn desktop_with_data(data: &Value) -> Result<Self, String> {
        let source = include_str!("../../../assets/plugins/desktop/main.js");
        Self::new_with_manifest(source, desktop_manifest(), Some(data.to_string()))
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

    pub fn sync_taskbar_window_menu_projection(
        &mut self,
        projection: &TaskbarWindowMenuPluginProjection,
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

    pub fn sync_volume_osd_projection(
        &mut self,
        projection: &VolumeOsdPluginProjection,
    ) -> Result<bool, String> {
        if self.manifest.id != volume_osd_manifest().id {
            return Err("this plugin is not the volume overlay".into());
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

    pub fn sync_control_center_data(&mut self, data: &Value) -> Result<bool, String> {
        if self.manifest.id != control_center_manifest().id {
            return Err("this plugin is not the control center".into());
        }
        let serialized = data.to_string();
        if self.projection_data.as_deref() == Some(serialized.as_str()) {
            return Ok(false);
        }
        self.context
            .eval(Source::from_bytes(&format!(
                "__nickelSetData({serialized})"
            )))
            .map_err(|error| error.to_string())?;
        self.node = evaluate_tree(&mut self.context, "__nickelRender()")?;
        self.projection_data = Some(serialized);
        Ok(true)
    }

    pub fn sync_window_preview_data(&mut self, data: &Value) -> Result<bool, String> {
        if self.manifest.id != window_preview_manifest().id {
            return Err("this plugin is not the window preview".into());
        }
        let serialized = data.to_string();
        if self.projection_data.as_deref() == Some(serialized.as_str()) {
            return Ok(false);
        }
        self.context
            .eval(Source::from_bytes(&format!(
                "__nickelSetData({serialized})"
            )))
            .map_err(|error| error.to_string())?;
        self.node = evaluate_tree(&mut self.context, "__nickelRender()")?;
        self.projection_data = Some(serialized);
        Ok(true)
    }

    pub fn sync_desktop_data(&mut self, data: &Value) -> Result<bool, String> {
        if self.manifest.id != desktop_manifest().id {
            return Err("this plugin is not the desktop".into());
        }
        let serialized = data.to_string();
        if self.projection_data.as_deref() == Some(serialized.as_str()) {
            return Ok(false);
        }
        self.context
            .eval(Source::from_bytes(&format!(
                "__nickelSetData({serialized})"
            )))
            .map_err(|error| error.to_string())?;
        self.node = evaluate_tree(&mut self.context, "__nickelRender()")?;
        self.projection_data = Some(serialized);
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
            PluginMessage::Drag(action, gesture) => {
                let phase = match gesture.phase {
                    DragPhase::Started => "start",
                    DragPhase::Moved => "move",
                    DragPhase::Ended => "end",
                    DragPhase::Cancelled => "cancel",
                };
                let encoded = serde_json::json!({
                    "phase": phase,
                    "x": gesture.position.x,
                    "y": gesture.position.y,
                    "bounds": {
                        "x": gesture.bounds.origin.x,
                        "y": gesture.bounds.origin.y,
                        "width": gesture.bounds.size.width,
                        "height": gesture.bounds.size.height,
                    },
                });
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
        (|| match (rendered, effects) {
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
                            == Some("taskbar-move-pin")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsPin) =>
                        {
                            let Some(index) = effect
                                .get("index")
                                .and_then(Value::as_u64)
                                .and_then(|index| usize::try_from(index).ok())
                                .filter(|index| *index < 12)
                            else {
                                self.last_error = Some("taskbar pin index is invalid".into());
                                return;
                            };
                            let Some(id) = effect
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|id| !id.is_empty() && id.len() <= 256)
                            else {
                                self.last_error = Some("taskbar pin ID is invalid".into());
                                return;
                            };
                            let direction = match effect.get("direction").and_then(Value::as_str) {
                                Some("left") => -1,
                                Some("right") => 1,
                                _ => {
                                    self.last_error =
                                        Some("taskbar pin direction is invalid".into());
                                    return;
                                }
                            };
                            approved.push(PluginEffect::MoveTaskbarPin {
                                index,
                                id: id.to_owned(),
                                direction,
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
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-menu-toggle-pin")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsPin) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error =
                                    Some("taskbar menu application ID is missing".into());
                                return;
                            };
                            if id.is_empty() || id.len() > 256 {
                                self.last_error =
                                    Some("taskbar menu application ID is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::ToggleTaskbarMenuPin { id: id.to_owned() });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-menu-close-all")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::WindowsContext) =>
                        {
                            approved.push(PluginEffect::CloseTaskbarMenuWindows);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-window-menu-action")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::WindowsContext) =>
                        {
                            let Some(page) = effect.get("page").and_then(Value::as_str) else {
                                self.last_error = Some("window menu page is missing".into());
                                return;
                            };
                            let Some(index) = effect
                                .get("index")
                                .and_then(Value::as_u64)
                                .and_then(|index| usize::try_from(index).ok())
                            else {
                                self.last_error = Some("window menu row is missing".into());
                                return;
                            };
                            if !matches!(page, "root" | "workspaces" | "displays") || index >= 32 {
                                self.last_error = Some("window menu target is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::InvokeTaskbarWindowMenu {
                                page: page.to_owned(),
                                index,
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("preview-action")
                            && self.manifest.id == window_preview_manifest().id =>
                        {
                            match preview_request(&effect) {
                                Ok((action, capability))
                                    if self.manifest.capabilities.contains(&capability) =>
                                {
                                    approved.push(PluginEffect::Preview(action));
                                }
                                Ok(_) => {
                                    self.last_error = Some("preview action is not granted".into());
                                    return;
                                }
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("control-action")
                            && self.manifest.id == control_center_manifest().id =>
                        {
                            match control_request(&effect) {
                                Ok((action, capability))
                                    if self.manifest.capabilities.contains(&capability) =>
                                {
                                    approved.push(PluginEffect::Control(action));
                                }
                                Ok(_) => {
                                    self.last_error = Some("control action is not granted".into());
                                    return;
                                }
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
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
        })();
        let finalizer = if self.last_error.is_some() {
            "__nickelRollbackEvent()"
        } else {
            "__nickelAcceptEvent()"
        };
        if let Err(error) = self.context.eval(Source::from_bytes(finalizer)) {
            self.last_error = Some(format!("could not finalize plugin event: {error}"));
        }
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
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
            || self.manifest.id == window_preview_manifest().id
            || self.manifest.id == desktop_manifest().id
        {
            AnyView::new(self.node.view(&self.images))
        } else {
            AnyView::new(
                Column::new()
                    .fill_width()
                    .height(context.viewport.size.height)
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
                    ..
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

    fn transient_dismissed(&self, id: &OverlayId) -> Option<Self::Message> {
        let dialog_id = id.as_ui_id().as_str().strip_prefix("plugin-")?;
        match self.node.dialog(dialog_id)? {
            PanelNode::Dialog {
                open: true,
                close_action: Some(action),
                ..
            } => Some(PluginMessage::Click(*action)),
            _ => None,
        }
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
    fn bundled_taskbar_drag_requests_validated_pin_move_without_click() {
        let projection = TaskbarPluginProjection {
            items: ["first", "second"]
                .into_iter()
                .enumerate()
                .map(|(index, id)| TaskbarPluginItem {
                    index,
                    id: id.into(),
                    name: id.into(),
                    active: false,
                    pinned: true,
                    icon: false,
                    badges: Vec::new(),
                })
                .collect(),
            tray: Vec::new(),
            clock: "12:00".into(),
            keyboard_enabled: false,
            codex_available: false,
        };
        let app = PluginPanelApplication::taskbar_with_projection(&projection).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 600, 56);
        let bounds = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "first".into(),
            })
            .unwrap()
            .bounds;
        let inside = Point {
            x: bounds.origin.x + bounds.size.width / 2.0,
            y: bounds.origin.y + bounds.size.height / 2.0,
        };
        let outside = Point {
            x: bounds.origin.x + bounds.size.width + 20.0,
            y: inside.y,
        };
        for event in [
            nickel_ui::UiEvent::PointerPressed(inside),
            nickel_ui::UiEvent::PointerMoved(outside),
            nickel_ui::UiEvent::PointerReleased(inside),
        ] {
            host.step(nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(event)],
                ..Default::default()
            });
        }
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::MoveTaskbarPin {
                index: 0,
                id: "first".into(),
                direction: 1,
            }]
        );
    }

    #[test]
    fn progress_component_rejects_out_of_range_geometry() {
        for properties in [
            "percent: 101, width: 100, height: 8",
            "percent: 50, width: 0, height: 8",
            "percent: 50, width: 100, height: 0",
        ] {
            let source = format!(
                "function App() {{ return h(Panel, {{}}, h(Progress, {{{properties}}})); }}"
            );
            assert!(PluginPanelApplication::new(&source).is_err());
        }
    }

    #[test]
    fn image_button_renders_host_image_and_dispatches_its_handler() {
        let source = "function App() { return h(Panel, {height: 180}, h(ImageButton, {id: 'preview', asset: 'window:1', width: 180, height: 110, accessibilityLabel: 'Open window', onClick: () => nickel.request('show-launcher')})); }";
        let mut app = PluginPanelApplication::new(source).unwrap();
        let mut images = PluginImages::new();
        images.insert(
            "window:1".into(),
            (42, Arc::new(image::RgbaImage::new(8, 8))),
        );
        assert!(app.sync_images(images));
        let mut host = nickel_ui::UiHost::new(app, 300, 180);
        let target = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open window".into(),
            })
            .unwrap();
        assert!(!host.commands().is_empty());
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(target.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowLauncher]
        );
    }

    #[test]
    fn bundled_window_preview_renders_and_requests_a_typed_window_action() {
        let data = serde_json::json!({"windows": [{
            "id": "71", "title": "Document", "accessibleName": "Document",
            "closable": true, "index": 0, "imageWidth": 244, "selected": false
        }]});
        let app = PluginPanelApplication::window_preview_with_data(&data).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 300, 214);
        let target = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Document".into(),
            })
            .expect("window preview image button");
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(target.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::Preview(PreviewAction::Activate(
                crate::model::WindowId(71)
            ))]
        );
    }

    #[test]
    fn image_components_reject_unbounded_or_unlabeled_interactive_views() {
        for node in [
            "h(Image, {asset: 'photo', width: 0, height: 20})",
            "h(Image, {asset: 'photo', width: 20, height: 9000})",
            "h(Image, {asset: 'photo', width: 20, height: 20, fit: 'unknown'})",
            "h(ImageButton, {id: 'x', asset: 'photo', width: 20, height: 20, onClick: () => {}})",
        ] {
            let source = format!("function App() {{ return h(Panel, {{}}, {node}); }}");
            assert!(PluginPanelApplication::new(&source).is_err());
        }
    }

    #[test]
    fn bundled_panel_updates_from_javascript_click() {
        let mut panel = PluginPanelApplication::bundled().expect("bundled plugin loads");
        assert!(format!("{:?}", panel.node).contains("Count: 0"));
        panel.update(PluginMessage::Click(0));
        assert!(format!("{:?}", panel.node).contains("Count: 1"));
        assert!(panel.last_error().is_none());
    }

    #[test]
    fn failed_native_render_restores_hook_state_before_next_click() {
        let source = r#"
            function App() {
                const [count, setCount] = useState(0);
                return h(Panel, {},
                    h(Text, {}, `Count: ${count}`),
                    h(Button, {id: 'invalid', onClick: () => setCount(1)}, 'Invalid'),
                    h(Button, {id: 'valid', onClick: () => setCount(value => value + 2)}, 'Valid'),
                    count === 1 ? h('unsupported-component', {}) : null);
            }
        "#;
        let mut panel = PluginPanelApplication::new(source).unwrap();
        let invalid = panel.button_message("invalid").unwrap();
        panel.update(invalid);
        assert!(panel.last_error().is_some());
        assert!(format!("{:?}", panel.node).contains("Count: 0"));

        let valid = panel.button_message("valid").unwrap();
        panel.update(valid);
        assert!(panel.last_error().is_none());
        assert!(format!("{:?}", panel.node).contains("Count: 2"));
    }

    #[test]
    fn failed_handler_discards_effects_and_restores_hook_state() {
        let source = r#"
            function App() {
                const [count, setCount] = useState(0);
                return h(Panel, {},
                    h(Text, {}, `Count: ${count}`),
                    h(Button, {id: 'invalid', onClick: () => {
                        setCount(1);
                        nickel.request('show-launcher');
                        throw Error('handler failed');
                    }}, 'Invalid'),
                    h(Button, {id: 'valid', onClick: () => setCount(value => value + 2)}, 'Valid'));
            }
        "#;
        let mut panel = PluginPanelApplication::new(source).unwrap();
        let invalid = panel.button_message("invalid").unwrap();
        panel.update(invalid);
        assert!(panel.last_error().is_some());
        assert!(panel.take_effects().is_empty());
        assert!(format!("{:?}", panel.node).contains("Count: 0"));

        let valid = panel.button_message("valid").unwrap();
        panel.update(valid);
        assert!(panel.last_error().is_none());
        assert!(format!("{:?}", panel.node).contains("Count: 2"));
    }

    #[test]
    fn denied_effect_restores_previous_tree_and_event_state() {
        let source = r#"
            function App() {
                const [count, setCount] = useState(0);
                return h(Panel, {},
                    h(Text, {}, `Count: ${count}`),
                    h(Button, {id: 'denied', onClick: () => {
                        setCount(1);
                        nickel.request({type: 'ungranted-action'});
                    }}, 'Denied'),
                    h(Button, {id: 'valid', onClick: () => setCount(value => value + 2)}, 'Valid'));
            }
        "#;
        let mut panel = PluginPanelApplication::new(source).unwrap();
        let denied = panel.button_message("denied").unwrap();
        panel.update(denied);
        assert!(panel.last_error().unwrap().contains("not granted"));
        assert!(panel.take_effects().is_empty());
        assert!(format!("{:?}", panel.node).contains("Count: 0"));

        let valid = panel.button_message("valid").unwrap();
        panel.update(valid);
        assert!(panel.last_error().is_none());
        assert!(format!("{:?}", panel.node).contains("Count: 2"));
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
    fn packaged_panel_uses_declared_height_and_dispatches_dialog_action() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.tall-panel".into();
        external_manifest.surfaces[0].height = 400;
        let package = PluginPackage {
            manifest: external_manifest,
            source: r#"
                function App() {
                    return h(Panel, {height: 80},
                        h(Button, {id: 'open', onClick: () => nickel.openDialog('action')}, 'Open dialog'),
                        h(Dialog, {id: 'action', anchor: 'open', open: true, width: 220, height: 120},
                            h(Button, {id: 'show', onClick: () => nickel.request('show-launcher')}, 'Show launcher')));
                }
            "#
            .into(),
        };
        let app = PluginPanelApplication::from_package(&package).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 440, 400);
        let open = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open dialog".into(),
            })
            .unwrap();
        assert!(open.bounds.origin.y > 250.0);
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(open.id),
            )],
            ..Default::default()
        });
        assert!(host.inspect().open_overlay.is_some());
        let show = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Show launcher".into(),
            })
            .unwrap();
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(show.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowLauncher]
        );
    }

    #[test]
    fn host_dismissal_calls_dialog_on_close_and_allows_reopen() {
        let package = PluginPackage {
            manifest: manifest().clone(),
            source: r#"
                function App() {
                    const [open, setOpen] = useState(false);
                    return h(Panel, {},
                        h(Button, {id: 'open', onClick: () => {
                            setOpen(true);
                            nickel.openDialog('confirm');
                        }}, 'Open dialog'),
                        h(Dialog, {id: 'confirm', anchor: 'open', open,
                            onClose: () => setOpen(false)},
                            h(Text, {}, 'Confirm?')));
                }
            "#
            .into(),
        };
        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package(&package).unwrap(),
            440,
            160,
        );
        let open = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open dialog".into(),
            })
            .unwrap();
        for _ in 0..2 {
            host.step(nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(
                    nickel_ui::UiEvent::AccessibilityActivate(open.id.clone()),
                )],
                ..Default::default()
            });
            assert!(host.inspect().open_overlay.is_some());
            assert!(matches!(
                host.application_mut().node.dialog("confirm"),
                Some(PanelNode::Dialog { open: true, .. })
            ));
            host.step(nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Dismiss)],
                ..Default::default()
            });
            assert!(host.inspect().open_overlay.is_none());
            assert!(matches!(
                host.application_mut().node.dialog("confirm"),
                Some(PanelNode::Dialog { open: false, .. })
            ));
            assert!(host.application_mut().last_error().is_none());
        }
    }

    #[test]
    fn installed_package_renders_declared_settings_data() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.settings-panel".into();
        let package = PluginPackage {
            manifest: external_manifest,
            source: "function App() { return h(Panel, {}, h(Text, {}, nickel.data.settings['show-count'] ? 'Shown' : 'Hidden')); }".into(),
        };
        let settings =
            std::collections::BTreeMap::from([("show-count".to_owned(), serde_json::json!(false))]);
        let app = PluginPanelApplication::from_package_with_settings(&package, &settings).unwrap();
        assert!(format!("{:?}", app.node).contains("Hidden"));
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
    fn launcher_plugin_renders_bounded_host_status_updates() {
        let launcher = Launcher::new(Vec::new());
        let projection = LauncherPluginProjection::from_launcher(&launcher)
            .with_status(Some("Could not launch Demo".repeat(30)));
        assert_eq!(projection.status.as_ref().unwrap().chars().count(), 160);
        let mut panel = PluginPanelApplication::launcher_with_projection(&projection).unwrap();
        assert!(format!("{:?}", panel.node).contains("Could not launch Demo"));
        assert!(
            panel
                .sync_launcher_projection(&LauncherPluginProjection::from_launcher(&launcher))
                .unwrap()
        );
        assert!(!format!("{:?}", panel.node).contains("Could not launch Demo"));
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
