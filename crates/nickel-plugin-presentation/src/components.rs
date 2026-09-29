//! Bounded JSX component tree and native Nickel UI renderer.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use nickel_core::plugins::{PluginManifest, PluginSurfaceKind};
use nickel_plugin_runtime::JsxRuntime;
use nickel_ui::{
    AnyView, Column, ComponentBuilderExt, Container, DragGesture, Dropdown, FilePlaneItem, Grid,
    Image, ImageFit, Insets, Layer, Length, OverlayMenuItem, Point, Row, SemanticRole, Slider,
    Spacer, Text, TextField as UiTextField, VerticalScroll,
};
use serde_json::Value;

use crate::css::{ControlStyle, Display, FlexDirection, StyleSheet};

pub type PluginImages = BTreeMap<String, (u16, Arc<image::RgbaImage>)>;

#[derive(Clone, Debug, PartialEq)]
pub struct DesktopPluginWidget {
    pub label: String,
    pub value: String,
    pub percent: u8,
    pub color: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarPluginAction {
    pub id: String,
    pub item: Option<String>,
    pub label: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ControlPluginSection {
    pub id: String,
    pub label: String,
    pub value: String,
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginMessage {
    Click(usize),
    Button { click: usize, drag: usize },
    TileMove(usize, f32, f32),
    FileAction(usize, String),
    Context(usize),
    Drag(usize, DragGesture),
    Text(usize, String),
    Value(usize, f32),
    Scroll,
}

/// Convert plugin control events at construction time, including controls
/// whose value or drag messages are produced after hit testing.
pub trait PluginUiMessage: Clone + 'static {
    fn from_plugin(message: PluginMessage) -> Self;
    fn from_plugin_scoped(message: PluginMessage, _scope: Option<&str>) -> Self {
        Self::from_plugin(message)
    }
    fn drag(seed: Self, gesture: DragGesture) -> Self;
    fn value(seed: Self, value: f32) -> Self;
}

impl PluginUiMessage for PluginMessage {
    fn from_plugin(message: PluginMessage) -> Self {
        message
    }

    fn drag(seed: Self, gesture: DragGesture) -> Self {
        let Self::Button { drag, .. } = seed else {
            unreachable!("plugin drag seed retains its handler")
        };
        Self::Drag(drag, gesture)
    }

    fn value(seed: Self, value: f32) -> Self {
        let Self::Value(action, _) = seed else {
            unreachable!("plugin slider seed retains its handler")
        };
        Self::Value(action, value)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowRequest {
    id: String,
    placement: String,
    output: Option<String>,
    edge: Option<String>,
    anchor: Option<String>,
    reserve_work_area: Option<bool>,
    bottom_offset: Option<u32>,
}

impl WindowRequest {
    fn validate(
        &self,
        manifest: &PluginManifest,
        width: Length,
        height: Length,
    ) -> Result<(), String> {
        let surface = manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == self.id)
            .ok_or_else(|| format!("window {:?} is not declared by the plugin", self.id))?;
        let valid_size = |requested: Length, granted: u32| {
            matches!(requested, Length::Percent(1.0))
                || matches!(requested, Length::Px(value) if value == granted as f32)
        };
        if !valid_size(width, surface.width) || !valid_size(height, surface.height) {
            return Err(format!(
                "window {:?} size must match its declared surface or use 100%",
                self.id
            ));
        }
        if self.placement == "managed"
            && !matches!(
                surface.kind,
                PluginSurfaceKind::Window | PluginSurfaceKind::Dialog
            )
        {
            return Err(format!("window {:?} needs fixed placement", self.id));
        }
        if let Some(output) = self.output.as_deref() {
            let matches = match output {
                "all" => surface.output == nickel_core::plugins::PluginOutputScope::All,
                "primary" => surface.output == nickel_core::plugins::PluginOutputScope::Primary,
                _ => false,
            };
            if !matches {
                return Err(format!("window {:?} output exceeds its grant", self.id));
            }
        }
        if self
            .reserve_work_area
            .is_some_and(|reserve| reserve != surface.reserve_work_area)
        {
            return Err(format!(
                "window {:?} work-area reservation differs from its grant",
                self.id
            ));
        }
        if self
            .bottom_offset
            .is_some_and(|offset| offset != surface.bottom_offset)
        {
            return Err(format!(
                "window {:?} bottom offset differs from its grant",
                self.id
            ));
        }
        if self
            .anchor
            .as_deref()
            .is_some_and(|anchor| anchor != surface.anchor.as_str())
        {
            return Err(format!(
                "window {:?} anchor differs from its grant",
                self.id
            ));
        }
        if let Some(edge) = self.edge.as_deref() {
            let anchored = match edge {
                "top" => matches!(
                    surface.anchor,
                    nickel_core::plugins::PluginSurfaceAnchor::TopLeft
                        | nickel_core::plugins::PluginSurfaceAnchor::TopRight
                ),
                "bottom" => {
                    surface.kind == PluginSurfaceKind::Panel
                        || matches!(
                            surface.anchor,
                            nickel_core::plugins::PluginSurfaceAnchor::BottomLeft
                                | nickel_core::plugins::PluginSurfaceAnchor::BottomRight
                        )
                }
                "left" => matches!(
                    surface.anchor,
                    nickel_core::plugins::PluginSurfaceAnchor::TopLeft
                        | nickel_core::plugins::PluginSurfaceAnchor::BottomLeft
                ),
                "right" => matches!(
                    surface.anchor,
                    nickel_core::plugins::PluginSurfaceAnchor::TopRight
                        | nickel_core::plugins::PluginSurfaceAnchor::BottomRight
                ),
                _ => false,
            };
            if !anchored {
                return Err(format!("window {:?} edge differs from its grant", self.id));
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PanelNode {
    Badge {
        item: Option<String>,
        label: String,
        count: u16,
        color: u32,
    },
    Widget {
        label: String,
        value: String,
        percent: u8,
        color: u32,
    },
    Action {
        id: String,
        item: Option<String>,
        label: String,
        action: usize,
    },
    Section {
        id: String,
        label: String,
        value: String,
        action: usize,
    },
    FileTile {
        id: String,
        action: Option<usize>,
        select_action: Option<usize>,
        move_action: Option<usize>,
        file_action: Option<usize>,
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
        class_name: Option<String>,
        x: i32,
        y: i32,
        width: u32,
        height: u32,
        background: u32,
        radius: u32,
    },
    Div {
        id: Option<String>,
        class_name: Option<String>,
        children: Vec<Self>,
        action: Option<usize>,
        role: Option<SemanticRole>,
        accessibility_label: Option<String>,
        accessibility_state: Option<String>,
        disabled: bool,
    },
    Surface {
        children: Vec<Self>,
        id: Option<String>,
        window_request: Option<WindowRequest>,
        class_name: Option<String>,
        background: u32,
        width: Length,
        height: Length,
    },
    Viewport {
        children: Vec<Self>,
        background: u32,
        padding: u32,
        class_name: Option<String>,
    },
    Panel {
        children: Vec<Self>,
        background: u32,
        height: u32,
        class_name: Option<String>,
    },
    Row {
        children: Vec<Self>,
        class_name: Option<String>,
    },
    Column {
        children: Vec<Self>,
        class_name: Option<String>,
    },
    ScrollView {
        id: String,
        class_name: Option<String>,
        height: u32,
        grow: bool,
        children: Vec<Self>,
    },
    Text {
        value: String,
        color: u32,
        class_name: Option<String>,
        wrap: bool,
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
    Slider {
        id: String,
        class_name: Option<String>,
        value: f32,
        label: String,
        action: usize,
    },
    Switch {
        id: String,
        class_name: Option<String>,
        state: String,
        label: String,
        action: Option<usize>,
    },
    ColorSwatch {
        id: String,
        class_name: Option<String>,
        color: Option<u32>,
        selected: bool,
        label: String,
        action: usize,
    },
    Select {
        id: String,
        class_name: Option<String>,
        label: String,
        value: String,
        open: bool,
        action: usize,
        options: Vec<(String, String, usize)>,
    },
    Spacer {
        class_name: Option<String>,
    },
    TextField {
        id: String,
        class_name: Option<String>,
        value: String,
        placeholder: String,
        secure: bool,
        action: usize,
    },
    Button {
        id: String,
        class_name: Option<String>,
        label: String,
        accessibility_label: String,
        accessibility_state: Option<String>,
        disabled: bool,
        width: Option<u32>,
        height: Option<u32>,
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
        point: Option<Point>,
        open: bool,
        items: Vec<Self>,
    },
    MenuItem {
        id: String,
        label: String,
        action: Option<usize>,
        children: Vec<Self>,
        disabled_reason: Option<String>,
        shortcut: Option<String>,
        separator_before: bool,
    },
}

fn apply_container_style<Message>(
    mut container: Container<Message>,
    style: &ControlStyle,
) -> Container<Message> {
    if let Some(width) = style.width {
        container = if width == Length::Percent(1.0) {
            container.fill_width()
        } else {
            container.width_length(width)
        };
    }
    if let Some(height) = style.height {
        container = if height == Length::Percent(1.0) {
            container.fill_height()
        } else {
            container.height_length(height)
        };
    }
    if let Some(width) = style.min_width {
        container = container.min_width(width);
    }
    if let Some(width) = style.max_width {
        container = container.max_width(width);
    }
    if let Some(height) = style.min_height {
        container = container.min_height(height);
    }
    if let Some(height) = style.max_height {
        container = container.max_height(height);
    }
    if let Some(padding) = style.padding {
        container = container.padding(padding);
    }
    if let Some(background) = style.background {
        // A transparent CSS background means no paint command. Color 0 would
        // otherwise be interpreted as legacy opaque RGB black by nickel-ui.
        container = if background == 0 {
            container.clear_background()
        } else {
            container.background(background)
        };
    }
    if let Some(radius) = style.radius {
        container = container.radius(radius);
    }
    if style.border_width.is_some() || style.border_color.is_some() {
        container = if style.border_color == Some(0) {
            container.clear_border()
        } else {
            container.border(
                style.border_color.unwrap_or(0xff000000),
                style.border_width.unwrap_or(1.0),
            )
        };
    }
    if let Some(grow) = style.grow {
        container = container.grow(grow);
    }
    if let Some(shrink) = style.shrink {
        container = container.shrink(shrink);
    }
    if let Some(basis) = style.basis {
        container = container.basis(basis);
    }
    container
}

fn with_margin<Message: Clone>(view: AnyView<Message>, style: &ControlStyle) -> AnyView<Message> {
    if let Some(margin) = style.margin {
        AnyView::new(Container::new().padding(margin).child(view))
    } else {
        view
    }
}

fn styled_text<Message>(mut text: Text<Message>, style: &ControlStyle) -> Text<Message> {
    if let Some(color) = style.color {
        text = text.color(color);
    }
    if let Some(font_size) = style.font_size {
        text = text.font_size(font_size);
    }
    if let Some(line_height) = style.line_height {
        text = text.line_height(line_height);
    }
    text
}

impl PanelNode {
    pub fn contribution_bytes(&self) -> u64 {
        let own = std::mem::size_of::<Self>() as u64;
        let capacity = |value: &String| value.capacity() as u64;
        own + match self {
            Self::Badge { item, label, .. } => item.as_ref().map_or(0, capacity) + capacity(label),
            Self::Widget { label, value, .. } => capacity(label) + capacity(value),
            Self::Action {
                id, item, label, ..
            } => capacity(id) + item.as_ref().map_or(0, capacity) + capacity(label),
            Self::Section {
                id, label, value, ..
            } => capacity(id) + capacity(label) + capacity(value),
            Self::ColorSwatch {
                id,
                class_name,
                label,
                ..
            } => capacity(id) + class_name.as_ref().map_or(0, capacity) + capacity(label),
            Self::Select {
                id,
                class_name,
                label,
                value,
                options,
                ..
            } => {
                capacity(id)
                    + class_name.as_ref().map_or(0, capacity)
                    + capacity(label)
                    + capacity(value)
                    + (options.capacity() * std::mem::size_of::<(String, String, usize)>()) as u64
                    + options
                        .iter()
                        .map(|(id, label, _)| capacity(id) + capacity(label))
                        .sum::<u64>()
            }
            Self::Div {
                id,
                class_name,
                children,
                accessibility_label,
                accessibility_state,
                ..
            } => {
                let spare = (children.capacity() - children.len()) * std::mem::size_of::<Self>();
                spare as u64
                    + id.as_ref().map_or(0, capacity)
                    + class_name.as_ref().map_or(0, capacity)
                    + accessibility_label.as_ref().map_or(0, capacity)
                    + accessibility_state.as_ref().map_or(0, capacity)
                    + children.iter().map(Self::contribution_bytes).sum::<u64>()
            }
            Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::Viewport { children, .. } => {
                let spare = (children.capacity() - children.len()) * std::mem::size_of::<Self>();
                spare as u64 + children.iter().map(Self::contribution_bytes).sum::<u64>()
            }
            _ => 0,
        }
    }

    pub fn file_tile_action(&self, id: &str) -> Option<usize> {
        match self {
            Self::FileTile {
                id: tile_id,
                action,
                ..
            } if tile_id == id => *action,
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => {
                children.iter().find_map(|child| child.file_tile_action(id))
            }
            _ => None,
        }
    }

    pub fn file_tile_select_action(&self, id: &str) -> Option<usize> {
        match self {
            Self::FileTile {
                id: tile_id,
                select_action,
                ..
            } if tile_id == id => *select_action,
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.file_tile_select_action(id)),
            _ => None,
        }
    }

    pub fn file_tile_move_action(&self, id: &str) -> Option<usize> {
        match self {
            Self::FileTile {
                id: tile_id,
                move_action,
                ..
            } if tile_id == id => *move_action,
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.file_tile_move_action(id)),
            _ => None,
        }
    }

    pub fn file_tile_file_action(&self, id: &str) -> Option<usize> {
        match self {
            Self::FileTile {
                id: tile_id,
                file_action,
                ..
            } if tile_id == id => *file_action,
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.file_tile_file_action(id)),
            _ => None,
        }
    }

    fn parse(value: &Value) -> Result<Self, String> {
        let kind = value
            .get("kind")
            .and_then(Value::as_str)
            .ok_or("component needs a kind")?;
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("component needs children")?;
        let class_name =
            match value.get("className") {
                None => None,
                Some(Value::String(value))
                    if value.len() <= 256
                        && value.split_ascii_whitespace().all(|name| {
                            !name.is_empty()
                                && name.len() <= 64
                                && name.bytes().all(|byte| {
                                    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_')
                                })
                        }) =>
                {
                    Some(value.clone())
                }
                _ => return Err(
                    "className must contain space-separated class identifiers (at most 256 bytes)"
                        .into(),
                ),
            };
        if class_name.is_some()
            && !matches!(
                kind,
                "box"
                    | "div"
                    | "surface"
                    | "window"
                    | "viewport"
                    | "panel"
                    | "row"
                    | "column"
                    | "scroll-view"
                    | "text"
                    | "spacer"
                    | "text-field"
                    | "slider"
                    | "switch"
                    | "color-swatch"
                    | "select"
                    | "button"
            )
        {
            return Err(format!("className is not supported on {kind} yet"));
        }
        match kind {
            "section" => {
                if !children.is_empty() {
                    return Err("section cannot have children".into());
                }
                let bounded = |name: &str, max: usize| {
                    value
                        .get(name)
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty() && text.len() <= max)
                        .map(str::to_owned)
                        .ok_or_else(|| format!("section {name} must be 1 to {max} bytes"))
                };
                Ok(Self::Section {
                    id: bounded("id", 64)?,
                    label: bounded("label", 120)?,
                    value: bounded("value", 120)?,
                    action: value
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok())
                        .ok_or("section needs an onClick handler")?,
                })
            }
            "action" => {
                if !children.is_empty() {
                    return Err("action cannot have children".into());
                }
                let bounded = |name: &str, max: usize| {
                    value
                        .get(name)
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty() && text.len() <= max)
                        .map(str::to_owned)
                        .ok_or_else(|| format!("action {name} must be 1 to {max} bytes"))
                };
                Ok(Self::Action {
                    id: bounded("id", 64)?,
                    item: match value.get("item") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(item)) if !item.is_empty() && item.len() <= 256 => {
                            Some(item.clone())
                        }
                        _ => return Err("action item must be 1 to 256 bytes".into()),
                    },
                    label: bounded("label", 120)?,
                    action: value
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok())
                        .ok_or("action needs an onClick handler")?,
                })
            }
            "widget" => {
                if !children.is_empty() {
                    return Err("widget cannot have children".into());
                }
                let bounded = |name: &str| {
                    value
                        .get(name)
                        .and_then(Value::as_str)
                        .filter(|text| !text.is_empty() && text.len() <= 64)
                        .map(str::to_owned)
                        .ok_or_else(|| format!("widget {name} must be 1 to 64 bytes"))
                };
                Ok(Self::Widget {
                    label: bounded("label")?,
                    value: bounded("value")?,
                    percent: value
                        .get("percent")
                        .and_then(Value::as_u64)
                        .filter(|percent| *percent <= 100)
                        .ok_or("widget percent must be 0 to 100")?
                        as u8,
                    color: value
                        .get("color")
                        .and_then(Value::as_u64)
                        .filter(|color| *color <= u32::MAX as u64)
                        .map_or(0xffd0d7e2, |color| color as u32),
                })
            }
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
                    id: value
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty() && id.len() <= 64)
                        .ok_or("file tile needs an ID")?
                        .to_owned(),
                    action: value
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
                    select_action: value
                        .get("selectAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
                    move_action: value
                        .get("moveAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
                    file_action: value
                        .get("fileAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
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
            "div" => {
                let role = value.get("role").and_then(Value::as_str);
                let interactive = value.get("action").and_then(Value::as_u64).is_some();
                let label = value
                    .get("aria-label")
                    .or_else(|| value.get("accessibilityLabel"));
                if interactive && label.is_none() {
                    return Err("clickable div needs an accessible label".into());
                }
                if role == Some("radiogroup") && interactive {
                    return Err("radio group cannot have an onClick handler".into());
                }
                if value.get("aria-checked").is_some() && role != Some("radio") {
                    return Err("aria-checked needs role=radio".into());
                }
                if value.get("aria-selected").is_some() && role != Some("option") {
                    return Err("aria-selected needs role=option".into());
                }
                if value.get("aria-checked").is_some() && value.get("aria-selected").is_some() {
                    return Err("div cannot have both checked and selected state".into());
                }
                if value
                    .get("disabled")
                    .is_some_and(|disabled| !disabled.is_boolean())
                {
                    return Err("div disabled must be boolean".into());
                }
                Ok(Self::Div {
                    id: match value.get("id") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(id)) if !id.is_empty() && id.len() <= 128 => {
                            Some(id.clone())
                        }
                        _ => return Err("div id must contain 1 to 128 bytes".into()),
                    },
                    class_name,
                    children: children
                        .iter()
                        .filter(|child| !child.is_null())
                        .map(Self::parse)
                        .collect::<Result<Vec<_>, _>>()?,
                    action: match value.get("action") {
                        None | Some(Value::Null) => None,
                        Some(action) => Some(
                            action
                                .as_u64()
                                .and_then(|action| usize::try_from(action).ok())
                                .ok_or("div action index is invalid")?,
                        ),
                    },
                    role: match value.get("role") {
                        None => interactive.then_some(SemanticRole::Button),
                        Some(Value::String(role)) if role == "button" => Some(SemanticRole::Button),
                        Some(Value::String(role)) if role == "radio" => Some(SemanticRole::Radio),
                        Some(Value::String(role)) if role == "radiogroup" => {
                            Some(SemanticRole::RadioGroup)
                        }
                        Some(Value::String(role)) if role == "group" => Some(SemanticRole::Group),
                        Some(Value::String(role)) if role == "option" => Some(SemanticRole::Option),
                        _ => return Err("div role is unsupported".into()),
                    },
                    accessibility_label: match value
                        .get("aria-label")
                        .or_else(|| value.get("accessibilityLabel"))
                    {
                        None => None,
                        Some(Value::String(label)) if !label.is_empty() && label.len() <= 256 => {
                            Some(label.clone())
                        }
                        _ => return Err("div accessible label must contain 1 to 256 bytes".into()),
                    },
                    accessibility_state: match value
                        .get("aria-checked")
                        .or_else(|| value.get("aria-selected"))
                    {
                        None => None,
                        Some(Value::Bool(selected)) => Some(if *selected {
                            "selected".into()
                        } else {
                            "unselected".into()
                        }),
                        _ => return Err("div checked or selected state must be boolean".into()),
                    },
                    disabled: value
                        .get("disabled")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
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
                    class_name,
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
            "surface" | "window" => {
                let dimension = |name| match value.get(name) {
                    Some(Value::Number(size)) => size
                        .as_u64()
                        .filter(|size| (1..=8192).contains(size))
                        .map(|size| Length::Px(size as f32))
                        .ok_or_else(|| format!("{kind} {name} must be 1 to 8192")),
                    Some(Value::String(percent)) if kind == "window" && percent == "100%" => {
                        Ok(Length::Percent(1.0))
                    }
                    _ => Err(format!("{kind} {name} must be 1 to 8192 or 100%")),
                };
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 128)
                    .map(str::to_owned);
                let window_request = if kind == "window" {
                    let id = id.as_ref().ok_or("window needs a bounded id")?.clone();
                    let optional_token =
                        |name, allowed: &[&str]| -> Result<Option<String>, String> {
                            match value.get(name) {
                                None | Some(Value::Null) => Ok(None),
                                Some(Value::String(token)) if allowed.contains(&token.as_str()) => {
                                    Ok(Some(token.clone()))
                                }
                                _ => Err(format!("window {name} is unsupported")),
                            }
                        };
                    let placement = optional_token("placement", &["managed", "fixed"])?
                        .unwrap_or_else(|| "managed".into());
                    let output = optional_token("output", &["primary", "all"])?;
                    let edge = optional_token("edge", &["top", "bottom", "left", "right"])?;
                    let anchor = optional_token(
                        "anchor",
                        &[
                            "center",
                            "top-left",
                            "top-right",
                            "bottom-left",
                            "bottom-right",
                        ],
                    )?;
                    let reserve_work_area = match value.get("reserveWorkArea") {
                        None | Some(Value::Null) => None,
                        Some(Value::Bool(value)) => Some(*value),
                        _ => return Err("window reserveWorkArea must be a boolean".into()),
                    };
                    let bottom_offset = match value.get("bottomOffset") {
                        None | Some(Value::Null) => None,
                        Some(Value::Number(value)) => value
                            .as_u64()
                            .filter(|value| *value <= 8192)
                            .map(|value| value as u32)
                            .ok_or("window bottomOffset must be 0 to 8192")
                            .map(Some)?,
                        _ => return Err("window bottomOffset must be 0 to 8192".into()),
                    };
                    Some(WindowRequest {
                        id,
                        placement,
                        output,
                        edge,
                        anchor,
                        reserve_work_area,
                        bottom_offset,
                    })
                } else {
                    None
                };
                Ok(Self::Surface {
                    class_name,
                    window_request,
                    children: children
                        .iter()
                        .filter(|value| !value.is_null())
                        .map(Self::parse)
                        .collect::<Result<Vec<_>, _>>()?,
                    id,
                    background: value
                        .get("background")
                        .and_then(Value::as_u64)
                        .map_or(if kind == "window" { 0 } else { 0xff202124 }, |color| {
                            color as u32
                        }),
                    width: dimension("width")?,
                    height: dimension("height")?,
                })
            }
            "viewport" | "panel" | "row" | "column" | "scroll-view" => {
                let children = children
                    .iter()
                    .filter(|value| !value.is_null())
                    .map(Self::parse)
                    .collect::<Result<Vec<_>, _>>()?;
                if kind == "viewport" {
                    let padding = value.get("padding").and_then(Value::as_u64).unwrap_or(0);
                    if padding > 256 {
                        return Err("viewport padding must be 0 to 256".into());
                    }
                    Ok(Self::Viewport {
                        children,
                        class_name,
                        background: value
                            .get("background")
                            .and_then(Value::as_u64)
                            .map_or(0, |color| color as u32),
                        padding: padding as u32,
                    })
                } else if kind == "panel" {
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
                        class_name,
                        background,
                        height,
                    })
                } else if kind == "row" {
                    Ok(Self::Row {
                        children,
                        class_name,
                    })
                } else if kind == "scroll-view" {
                    let id = value
                        .get("id")
                        .and_then(Value::as_str)
                        .ok_or("scroll view needs an id")?;
                    if id.is_empty() || id.len() > 128 {
                        return Err("scroll view ID must contain 1 to 128 characters".into());
                    }
                    let grow = value.get("grow").and_then(Value::as_bool).unwrap_or(false);
                    if grow && value.get("height").is_some() {
                        return Err("scroll view cannot set both grow and height".into());
                    }
                    let height = value.get("height").and_then(Value::as_u64).unwrap_or(480);
                    if !(1..=8192).contains(&height) {
                        return Err("scroll view height must be 1 to 8192".into());
                    }
                    Ok(Self::ScrollView {
                        id: id.to_owned(),
                        class_name,
                        height: height as u32,
                        grow,
                        children,
                    })
                } else {
                    Ok(Self::Column {
                        children,
                        class_name,
                    })
                }
            }
            "text" => Ok(Self::Text {
                value: child_text(children)?,
                class_name,
                wrap: value.get("wrap").and_then(Value::as_bool).unwrap_or(false),
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
            "spacer" => Ok(Self::Spacer { class_name }),
            "slider" => {
                if !children.is_empty() {
                    return Err("slider cannot have children".into());
                }
                let fraction = value
                    .get("value")
                    .and_then(Value::as_f64)
                    .filter(|value| value.is_finite() && (0.0..=1.0).contains(value))
                    .ok_or("slider value must be between 0 and 1")?;
                Ok(Self::Slider {
                    id: value
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty() && id.len() <= 128)
                        .ok_or("slider needs a bounded id")?
                        .to_owned(),
                    class_name,
                    value: fraction as f32,
                    label: value
                        .get("accessibilityLabel")
                        .and_then(Value::as_str)
                        .filter(|label| !label.is_empty() && label.len() <= 256)
                        .ok_or("slider needs an accessibility label")?
                        .to_owned(),
                    action: value
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok())
                        .ok_or("slider needs an onChange handler")?,
                })
            }
            "switch" => {
                if !children.is_empty() {
                    return Err("switch cannot have children".into());
                }
                let state = value
                    .get("state")
                    .and_then(Value::as_str)
                    .filter(|state| {
                        matches!(
                            *state,
                            "off"
                                | "on"
                                | "mixed"
                                | "mixed-unavailable"
                                | "disabled-off"
                                | "disabled-on"
                        )
                    })
                    .ok_or("switch state is invalid")?;
                let action = value
                    .get("action")
                    .and_then(Value::as_u64)
                    .and_then(|action| usize::try_from(action).ok());
                if !matches!(state, "off" | "on" | "mixed") && action.is_some() {
                    return Err("disabled switch cannot have an onClick handler".into());
                }
                Ok(Self::Switch {
                    id: value
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty() && id.len() <= 128)
                        .ok_or("switch needs a bounded id")?
                        .to_owned(),
                    class_name,
                    state: state.to_owned(),
                    label: value
                        .get("accessibilityLabel")
                        .and_then(Value::as_str)
                        .filter(|label| !label.is_empty() && label.len() <= 256)
                        .ok_or("switch needs an accessibility label")?
                        .to_owned(),
                    action,
                })
            }
            "select" => {
                if children.is_empty() || children.len() > 64 {
                    return Err("select needs 1 to 64 options".into());
                }
                let mut options = Vec::with_capacity(children.len());
                let mut seen = HashSet::new();
                for child in children {
                    if child.get("kind").and_then(Value::as_str) != Some("option") {
                        return Err("select children must be Option components".into());
                    }
                    let id = child
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty() && id.len() <= 128)
                        .ok_or("option needs a bounded id")?;
                    if !seen.insert(id) {
                        return Err("select option IDs must be unique".into());
                    }
                    let label = child
                        .get("children")
                        .and_then(Value::as_array)
                        .ok_or_else(|| "option needs text".to_owned())
                        .and_then(|children| child_text(children))?;
                    if label.is_empty() || label.chars().count() > 120 {
                        return Err("option label is invalid".into());
                    }
                    let action = child
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok())
                        .ok_or("option needs an onClick handler")?;
                    options.push((id.to_owned(), label, action));
                }
                Ok(Self::Select {
                    id: value
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty() && id.len() <= 128)
                        .ok_or("select needs a bounded id")?
                        .to_owned(),
                    class_name,
                    label: value
                        .get("accessibilityLabel")
                        .and_then(Value::as_str)
                        .filter(|label| !label.is_empty() && label.len() <= 256)
                        .ok_or("select needs an accessibility label")?
                        .to_owned(),
                    value: value
                        .get("value")
                        .and_then(Value::as_str)
                        .filter(|value| value.len() <= 256)
                        .ok_or("select needs a bounded value")?
                        .to_owned(),
                    open: value.get("open").and_then(Value::as_bool).unwrap_or(false),
                    action: value
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok())
                        .ok_or("select needs an onClick handler")?,
                    options,
                })
            }
            "color-swatch" => {
                if !children.is_empty() {
                    return Err("color swatch cannot have children".into());
                }
                let label = value
                    .get("accessibilityLabel")
                    .and_then(Value::as_str)
                    .filter(|label| !label.is_empty() && label.len() <= 256)
                    .ok_or("color swatch needs an accessibility label")?;
                let color = match value.get("color") {
                    Some(Value::String(color)) => Some(crate::css::color(color)?),
                    Some(Value::Null) | None => None,
                    _ => return Err("color swatch color must be a CSS color".into()),
                };
                Ok(Self::ColorSwatch {
                    id: value
                        .get("id")
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty() && id.len() <= 128)
                        .ok_or("color swatch needs a bounded id")?
                        .to_owned(),
                    class_name,
                    color,
                    selected: value
                        .get("selected")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    label: label.to_owned(),
                    action: value
                        .get("action")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok())
                        .ok_or("color swatch needs an onClick handler")?,
                })
            }
            "text-field" => Ok(Self::TextField {
                class_name,
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
                secure: match value.get("secure") {
                    None => false,
                    Some(Value::Bool(secure)) => *secure,
                    _ => return Err("text field secure must be a boolean".into()),
                },
                action: value
                    .get("action")
                    .and_then(Value::as_u64)
                    .ok_or("text field needs an onChange handler")?
                    as usize,
            }),
            "button" => {
                let label = child_text(children)?;
                let disabled = value
                    .get("disabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let dimension = |name: &str| -> Result<Option<u32>, String> {
                    match value.get(name) {
                        None => Ok(None),
                        Some(value) => value
                            .as_u64()
                            .and_then(|size| u32::try_from(size).ok())
                            .filter(|size| (24..=512).contains(size))
                            .map(Some)
                            .ok_or_else(|| format!("button {name} must be 24 to 512")),
                    }
                };
                Ok(Self::Button {
                    class_name,
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
                    accessibility_state: value
                        .get("state")
                        .and_then(Value::as_str)
                        .filter(|state| state.len() <= 128)
                        .map(str::to_owned),
                    disabled,
                    width: dimension("width")?,
                    height: dimension("height")?,
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
                    action: match value.get("action").and_then(Value::as_u64) {
                        Some(action) => usize::try_from(action)
                            .map_err(|_| "button action index is too large")?,
                        None if disabled => 0,
                        None => return Err("button needs an onClick handler".into()),
                    },
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
                    point: match (value.get("x"), value.get("y")) {
                        (None, None) => None,
                        (Some(x), Some(y)) => {
                            let coordinate = |value: &Value| {
                                value
                                    .as_f64()
                                    .filter(|value| {
                                        value.is_finite() && (-8192.0..=8192.0).contains(value)
                                    })
                                    .map(|value| value as f32)
                                    .ok_or("menu point coordinate is invalid")
                            };
                            Some(Point {
                                x: coordinate(x)?,
                                y: coordinate(y)?,
                            })
                        }
                        _ => return Err("menu point needs both x and y".into()),
                    },
                    open: value.get("open").and_then(Value::as_bool).unwrap_or(false),
                    items,
                })
            }
            "menu-item" => {
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or("menu item needs an id")?;
                let label = if let Some(label) = value.get("label").and_then(Value::as_str) {
                    label.to_owned()
                } else {
                    child_text(children)?
                };
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
                    .and_then(|action| usize::try_from(action).ok());
                let nested = children.iter().any(Value::is_object);
                let items = if nested {
                    if children.len() > 16 {
                        return Err("menu item submenu needs at most 16 items".into());
                    }
                    children
                        .iter()
                        .map(Self::parse)
                        .collect::<Result<Vec<_>, _>>()?
                } else {
                    Vec::new()
                };
                if nested && (action.is_some() || items.is_empty()) {
                    return Err("submenu needs children and no onClick handler".into());
                }
                if nested {
                    let mut seen = HashSet::new();
                    for item in &items {
                        let Self::MenuItem { id, .. } = item else {
                            return Err("submenu children must be MenuItem components".into());
                        };
                        if !seen.insert(id) {
                            return Err("submenu item IDs must be unique".into());
                        }
                    }
                }
                let disabled_reason = value
                    .get("disabledReason")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if !nested && action.is_none() && disabled_reason.is_none() {
                    return Err("menu item needs an onClick handler or disabled reason".into());
                }
                if action.is_some() && disabled_reason.is_some() {
                    return Err("disabled menu item cannot have an onClick handler".into());
                }
                if nested && disabled_reason.is_some() {
                    return Err("submenu cannot have a disabled reason".into());
                }
                if disabled_reason
                    .as_ref()
                    .is_some_and(|reason| reason.is_empty() || reason.len() > 200)
                {
                    return Err("menu item disabled reason is invalid".into());
                }
                let shortcut = value
                    .get("shortcut")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                if shortcut
                    .as_ref()
                    .is_some_and(|shortcut| shortcut.is_empty() || shortcut.len() > 40)
                {
                    return Err("menu item shortcut is invalid".into());
                }
                Ok(Self::MenuItem {
                    id: id.to_owned(),
                    label,
                    action,
                    children: items,
                    disabled_reason,
                    shortcut,
                    separator_before: value
                        .get("separatorBefore")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                })
            }
            _ => Err(format!("unknown component {kind:?}")),
        }
    }

    pub fn overlay_menu_item(&self) -> Option<OverlayMenuItem<PluginMessage>> {
        let Self::MenuItem {
            id,
            label,
            action,
            children,
            disabled_reason,
            shortcut,
            separator_before,
        } = self
        else {
            return None;
        };
        let mut item = if !children.is_empty() {
            OverlayMenuItem::submenu(
                id.clone(),
                label.clone(),
                children.iter().filter_map(Self::overlay_menu_item),
            )
        } else if let Some(action) = action {
            OverlayMenuItem::action(id.clone(), label.clone(), PluginMessage::Click(*action))
        } else {
            OverlayMenuItem::disabled_with_reason(
                id.clone(),
                label.clone(),
                disabled_reason.clone().unwrap_or_default(),
            )
        };
        if let Some(shortcut) = shortcut {
            item = item.shortcut(shortcut.clone());
        }
        Some(item.separator_before(*separator_before))
    }

    pub fn view(&self, images: &PluginImages, stylesheet: &StyleSheet) -> AnyView<PluginMessage> {
        self.view_as::<PluginMessage>(images, stylesheet)
    }

    pub fn view_as<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
    ) -> AnyView<Message> {
        self.view_as_scoped(images, stylesheet, None)
    }

    pub fn view_as_scoped<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
    ) -> AnyView<Message> {
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
            Self::Widget { .. } => AnyView::new(Spacer::fixed(0.0)),
            Self::Action { .. } | Self::Section { .. } => AnyView::new(Spacer::fixed(0.0)),
            Self::FileTile {
                id,
                action,
                select_action: _,
                move_action: _,
                file_action: _,
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
                    Message::from_plugin_scoped(
                        PluginMessage::Click(action.unwrap_or(usize::MAX)),
                        scope,
                    ),
                    label.clone(),
                    icon_id,
                    icon,
                )
                .id(id.clone())
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
            Self::Div {
                id,
                class_name,
                children,
                action,
                role,
                accessibility_label,
                accessibility_state,
                disabled,
            } => {
                let style = stylesheet.resolve("div", id.as_deref(), class_name.as_deref());
                let content: AnyView<Message> = match style.display.unwrap_or_default() {
                    Display::Grid => {
                        let mut grid = style
                            .grid_columns
                            .as_ref()
                            .map_or_else(Grid::new, |tracks| Grid::tracks(tracks.iter().cloned()));
                        if let Some(gap) = style.gap {
                            grid = grid.gap(gap);
                        }
                        for child in children {
                            grid = grid
                                .child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                        }
                        AnyView::new(grid)
                    }
                    Display::Flex
                        if style.flex_direction.unwrap_or_default() == FlexDirection::Row =>
                    {
                        let mut row = Row::new();
                        if style.width == Some(Length::Percent(1.0)) {
                            row = row.fill_width();
                        }
                        if style.height == Some(Length::Percent(1.0)) {
                            row = row.fill_height();
                        }
                        if let Some(gap) = style.gap {
                            row = row.gap(gap);
                        }
                        if let Some(align) = style.align_items {
                            row = row.align_items(align);
                        }
                        if let Some(justify) = style.justify_content {
                            row = row.justify_content(justify);
                        }
                        for child in children {
                            row = row
                                .child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                        }
                        AnyView::new(row)
                    }
                    Display::Block | Display::Flex => {
                        let mut column = Column::new();
                        if style.width == Some(Length::Percent(1.0)) {
                            column = column.fill_width();
                        }
                        if style.height == Some(Length::Percent(1.0)) {
                            column = column.fill_height();
                        }
                        if let Some(gap) = style.gap {
                            column = column.gap(gap);
                        }
                        if let Some(align) = style.align_items {
                            column = column.align_items(align);
                        }
                        if let Some(justify) = style.justify_content {
                            column = column.justify_content(justify);
                        }
                        for child in children {
                            column = column
                                .child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                        }
                        AnyView::new(column)
                    }
                };
                let mut container = Container::new().child(content);
                if let Some(id) = id {
                    container = container.id(id.clone());
                }
                if let Some(role) = role {
                    container = container.semantic_role(*role);
                }
                if let Some(label) = accessibility_label {
                    container = container.accessibility_label(label);
                }
                if let Some(state) = accessibility_state {
                    container = container.accessibility_state(state);
                }
                if *disabled {
                    container = container.enabled(false).accessibility_state("disabled");
                } else if let Some(action) = action {
                    container = container.message(Message::from_plugin_scoped(
                        PluginMessage::Click(*action),
                        scope,
                    ));
                }
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::Box {
                children,
                class_name,
                x,
                y,
                width,
                height,
                background,
                radius,
            } => {
                let style = stylesheet.resolve("box", None, class_name.as_deref());
                let mut column = Column::new().fill_width();
                for child in children {
                    column =
                        column.child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                }
                let container = Container::new()
                    .position(Point {
                        x: *x as f32,
                        y: *y as f32,
                    })
                    .width(*width as f32)
                    .height(*height as f32)
                    .background(*background)
                    .radius(*radius as f32)
                    .child(column);
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::Surface {
                children,
                id,
                window_request,
                class_name,
                background,
                width,
                height,
            } => {
                let style = stylesheet.resolve(
                    if window_request.is_some() {
                        "window"
                    } else {
                        "surface"
                    },
                    id.as_deref(),
                    class_name.as_deref(),
                );
                let mut layer = Layer::new().width_length(*width).height_length(*height);
                for child in children {
                    if !matches!(child, Self::Dialog { .. }) {
                        layer =
                            layer.child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                    }
                }
                let mut container = Container::new()
                    .width_length(*width)
                    .height_length(*height)
                    .background(*background)
                    .child(layer);
                if let Some(id) = id {
                    container = container.id(id.clone());
                }
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::Viewport {
                children,
                background,
                padding,
                class_name,
            } => {
                let mut column = Column::new().fill_width().fill_height();
                for child in children {
                    column =
                        column.child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                }
                let style = stylesheet.resolve("viewport", None, class_name.as_deref());
                let container = Container::new()
                    .fill_width()
                    .fill_height()
                    .padding(
                        style
                            .padding
                            .unwrap_or_else(|| Insets::all(*padding as f32)),
                    )
                    .background(*background)
                    .child(column);
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::Panel {
                children,
                background,
                height,
                class_name,
            } => {
                let mut row = Row::new()
                    .fill_width()
                    .height((*height).saturating_sub(16) as f32);
                for child in children {
                    if !matches!(child, Self::Dialog { .. }) {
                        row = row.child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                    }
                }
                let style = stylesheet.resolve("panel", None, class_name.as_deref());
                let container = Container::new()
                    .height(*height as f32)
                    .background(*background)
                    .radius(style.radius.unwrap_or(16.0))
                    .padding(style.padding.unwrap_or_else(|| Insets::all(8.0)))
                    .child(row);
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::Row {
                children,
                class_name,
            } => {
                let style = stylesheet.resolve("row", None, class_name.as_deref());
                let mut row = Row::new();
                if let Some(gap) = style.gap {
                    row = row.gap(gap);
                }
                for child in children {
                    row = row.child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                }
                if style == ControlStyle::default() {
                    AnyView::new(row)
                } else {
                    with_margin(
                        AnyView::new(apply_container_style(Container::new().child(row), &style)),
                        &style,
                    )
                }
            }
            Self::Column {
                children,
                class_name,
            } => {
                let style = stylesheet.resolve("column", None, class_name.as_deref());
                let mut column = Column::new();
                if let Some(gap) = style.gap {
                    column = column.gap(gap);
                }
                for child in children {
                    column =
                        column.child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                }
                if style == ControlStyle::default() {
                    AnyView::new(column)
                } else {
                    with_margin(
                        AnyView::new(apply_container_style(
                            Container::new().child(column),
                            &style,
                        )),
                        &style,
                    )
                }
            }
            Self::ScrollView {
                id,
                class_name,
                height,
                grow,
                children,
            } => {
                let style = stylesheet.resolve("scroll-view", Some(id), class_name.as_deref());
                let mut column = Column::new().fill_width();
                if let Some(gap) = style.gap {
                    column = column.gap(gap);
                }
                for child in children {
                    column =
                        column.child(child.view_as_scoped::<Message>(images, stylesheet, scope));
                }
                let scroll = VerticalScroll::new(
                    Message::from_plugin_scoped(PluginMessage::Scroll, scope),
                    0.0,
                )
                .id(id.clone())
                .child(column);
                let scroll = if *grow {
                    scroll.grow(1.0)
                } else {
                    scroll.height(*height as f32)
                };
                if style == ControlStyle::default() {
                    AnyView::new(scroll)
                } else {
                    with_margin(
                        AnyView::new(apply_container_style(
                            Container::new().child(scroll),
                            &style,
                        )),
                        &style,
                    )
                }
            }
            Self::Text {
                value,
                color,
                class_name,
                wrap,
            } => {
                let style = stylesheet.resolve("text", None, class_name.as_deref());
                let text = styled_text(
                    Text::new(value)
                        .color(style.color.unwrap_or(*color))
                        .scale(1.0)
                        .wrap(*wrap),
                    &style,
                );
                let container = Container::new()
                    .semantic_role(SemanticRole::Text)
                    .accessibility_label(value.clone())
                    .height(48.0)
                    .child(text);
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
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
                        .message(Message::from_plugin_scoped(
                            PluginMessage::Click(*action),
                            scope,
                        ));
                    if let Some(context_action) = context_action {
                        container = container.context_message(Message::from_plugin_scoped(
                            PluginMessage::Context(*context_action),
                            scope,
                        ));
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
            Self::Spacer { class_name } => {
                let style = stylesheet.resolve("spacer", None, class_name.as_deref());
                AnyView::new(Spacer::flex().grow(style.grow.unwrap_or(1.0)))
            }
            Self::Slider {
                id,
                class_name,
                value,
                label,
                action,
            } => {
                let style = stylesheet.resolve("slider", Some(id), class_name.as_deref());
                let slider = Slider::on_change_with(
                    Message::from_plugin_scoped(PluginMessage::Value(*action, *value), scope),
                    Message::value,
                    *value,
                )
                .id(id.clone())
                .colors(
                    style.background.unwrap_or(0x354158),
                    style.color.unwrap_or(0x68b8ff),
                    style.border_color.unwrap_or(0xf4f7ff),
                );
                let slider = match style.width {
                    Some(Length::Px(width)) => slider.width(width),
                    Some(Length::Percent(1.0)) => slider.grow(1.0),
                    _ => slider,
                }
                .accessibility_label(label.clone());
                let mut wrapper_style = style.clone();
                wrapper_style.background = None;
                wrapper_style.color = None;
                wrapper_style.border_color = None;
                with_margin(
                    AnyView::new(apply_container_style(
                        Container::new().child(slider),
                        &wrapper_style,
                    )),
                    &wrapper_style,
                )
            }
            Self::Switch {
                id,
                class_name,
                state,
                label,
                action,
            } => {
                let style = stylesheet.resolve("switch", Some(id), class_name.as_deref());
                let on = matches!(state.as_str(), "on" | "mixed" | "disabled-on");
                let mixed = matches!(state.as_str(), "mixed" | "mixed-unavailable");
                let track = Container::new()
                    .width(42.0)
                    .height(24.0)
                    .radius(style.radius.unwrap_or(12.0))
                    .border(
                        style.border_color.unwrap_or(0xff646b76),
                        style.border_width.unwrap_or(1.0),
                    )
                    .background(style.background.unwrap_or(if on {
                        0xff456888
                    } else {
                        0xff414958
                    }))
                    .padding(Insets::all(3.0))
                    .child(
                        Row::new()
                            .fill_width()
                            .justify_content(if mixed {
                                nickel_ui::Justify::Center
                            } else if on {
                                nickel_ui::Justify::End
                            } else {
                                nickel_ui::Justify::Start
                            })
                            .child(
                                Container::new()
                                    .width(18.0)
                                    .height(18.0)
                                    .radius(9.0)
                                    .background(style.color.unwrap_or(0xfff4f6fa)),
                            ),
                    );
                let mut control = Container::new()
                    .id(id.clone())
                    .width(44.0)
                    .height(44.0)
                    .semantic_role(SemanticRole::Switch)
                    .accessibility_label(label.clone())
                    .accessibility_state(match state.as_str() {
                        "disabled-off" => "off disabled",
                        "disabled-on" => "on disabled",
                        "mixed-unavailable" => "mixed unavailable",
                        other => other,
                    })
                    .align_items(nickel_ui::Align::Center)
                    .justify_content(nickel_ui::Justify::Center)
                    .child(track);
                if let Some(action) = action {
                    control = control.message(Message::from_plugin_scoped(
                        PluginMessage::Click(*action),
                        scope,
                    ));
                }
                with_margin(AnyView::new(control), &style)
            }
            Self::Select {
                id,
                class_name,
                label,
                value,
                open,
                action,
                options,
            } => {
                let style = stylesheet.resolve("select", Some(id), class_name.as_deref());
                let select = Dropdown::new(
                    Message::from_plugin_scoped(PluginMessage::Click(*action), scope),
                    value,
                    options.iter().map(|(_, label, action)| {
                        (
                            label.as_str(),
                            Message::from_plugin_scoped(PluginMessage::Click(*action), scope),
                        )
                    }),
                )
                .id(id.clone())
                .accessibility_label(label)
                .overlay(true)
                .expanded(*open)
                .colors(
                    style.background.unwrap_or(0xff30343d),
                    style.background.unwrap_or(0xff424957),
                    style.color.unwrap_or(0xfff0f0f0),
                );
                let container = Container::new().width(180.0).child(select);
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::ColorSwatch {
                id,
                class_name,
                color,
                selected,
                label,
                action,
            } => {
                let style = stylesheet.resolve("color-swatch", Some(id), class_name.as_deref());
                let inner = if let Some(color) = color {
                    let mut fill = Container::new().width(32.0).height(32.0).radius(16.0);
                    fill = if *color == 0 {
                        fill.clear_background()
                    } else {
                        fill.background(*color)
                    };
                    AnyView::new(fill)
                } else {
                    AnyView::new(
                        Text::new("+")
                            .font_size(24.0)
                            .color(style.color.unwrap_or(0xff777777)),
                    )
                };
                let control = Container::new()
                    .id(id.clone())
                    .width(42.0)
                    .height(42.0)
                    .radius(21.0)
                    .padding(Insets::all(4.0))
                    .border(
                        style.border_color.unwrap_or(if *selected {
                            0xfff0f0f0
                        } else {
                            0xff777777
                        }),
                        style
                            .border_width
                            .unwrap_or(if *selected { 2.0 } else { 1.0 }),
                    )
                    .align_items(nickel_ui::Align::Center)
                    .justify_content(nickel_ui::Justify::Center)
                    .semantic_role(if color.is_some() {
                        SemanticRole::Radio
                    } else {
                        SemanticRole::Button
                    })
                    .accessibility_label(label)
                    .accessibility_state(if *selected { "selected" } else { "unselected" })
                    .message(Message::from_plugin_scoped(
                        PluginMessage::Click(*action),
                        scope,
                    ))
                    .child(inner);
                with_margin(AnyView::new(apply_container_style(control, &style)), &style)
            }
            Self::TextField {
                id,
                class_name,
                value,
                placeholder,
                secure,
                action,
            } => {
                let style = stylesheet.resolve("text-field", Some(id), class_name.as_deref());
                let scope = scope.map(str::to_owned);
                let field = if *secure {
                    UiTextField::on_change_masked_with_placeholder_mapped(
                        value,
                        placeholder,
                        '•',
                        {
                            let action = *action;
                            move |value| {
                                Message::from_plugin_scoped(
                                    PluginMessage::Text(action, value),
                                    scope.as_deref(),
                                )
                            }
                        },
                    )
                } else {
                    UiTextField::on_change_with_placeholder_mapped(value, placeholder, {
                        let action = *action;
                        move |value| {
                            Message::from_plugin_scoped(
                                PluginMessage::Text(action, value),
                                scope.as_deref(),
                            )
                        }
                    })
                };
                let mut field = field
                    .id(id.clone())
                    .accessibility_label(placeholder)
                    .grow(1.0)
                    .wrap(false);
                if let Some(size) = style.font_size {
                    field = field.font_size(size);
                }
                if let Some(height) = style.line_height {
                    field = field.line_height(height);
                }
                if let Some(color) = style.color {
                    field = field.color(color);
                }
                if style == ControlStyle::default() {
                    AnyView::new(field.height(44.0))
                } else {
                    with_margin(
                        AnyView::new(apply_container_style(Container::new().child(field), &style)),
                        &style,
                    )
                }
            }
            Self::Button {
                id,
                class_name,
                label,
                accessibility_label,
                accessibility_state,
                disabled,
                width,
                height,
                icon,
                show_label,
                action,
                context_action,
                drag_action,
            } => {
                let style = stylesheet.resolve("button", Some(id), class_name.as_deref());
                let visual = icon
                    .as_ref()
                    .and_then(|asset| images.get(asset))
                    .map_or_else(
                        || AnyView::new(styled_text(Text::new(label).wrap(true), &style)),
                        |(id, image)| {
                            let icon = Image::new(*id, Arc::clone(image)).width(32.0).height(32.0);
                            if *show_label {
                                AnyView::new(
                                    Row::new()
                                        .gap(8.0)
                                        .child(icon)
                                        .child(styled_text(Text::new(label).wrap(true), &style)),
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
                    .height(height.unwrap_or(42) as f32);
                if !disabled {
                    container = container.message(Message::from_plugin_scoped(
                        PluginMessage::Click(*action),
                        scope,
                    ));
                }
                if let Some(state) = accessibility_state {
                    container = container.accessibility_state(state);
                }
                if *disabled {
                    container = container.accessibility_state("disabled");
                }
                if let Some(width) = width {
                    container = container.width(*width as f32);
                }
                if let Some(action) = context_action.filter(|_| !*disabled) {
                    container = container.context_message(Message::from_plugin_scoped(
                        PluginMessage::Context(action),
                        scope,
                    ));
                }
                if let Some(drag) = drag_action.filter(|_| !*disabled) {
                    container = container.on_drag((
                        Message::from_plugin_scoped(
                            PluginMessage::Button {
                                click: *action,
                                drag,
                            },
                            scope,
                        ),
                        Message::drag,
                    ));
                }
                with_margin(
                    AnyView::new(apply_container_style(container.child(visual), &style)),
                    &style,
                )
            }
            Self::Dialog { .. } | Self::Menu { .. } | Self::MenuItem { .. } => {
                AnyView::new(Spacer::fixed(0.0))
            }
        }
    }

    pub fn dialog(&self, requested_id: &str) -> Option<&Self> {
        match self {
            Self::Dialog { id, .. } if id == requested_id => Some(self),
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => {
                children.iter().find_map(|child| child.dialog(requested_id))
            }
            _ => None,
        }
    }

    pub fn menu(&self, requested_id: &str) -> Option<&Self> {
        match self {
            Self::Menu { id, .. } if id == requested_id => Some(self),
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => {
                children.iter().find_map(|child| child.menu(requested_id))
            }
            _ => None,
        }
    }

    pub fn transients<'a>(&'a self, output: &mut Vec<&'a Self>) {
        match self {
            Self::Dialog { .. } | Self::Menu { .. } => output.push(self),
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => {
                for child in children {
                    child.transients(output);
                }
            }
            _ => {}
        }
    }

    pub fn button_action(&self, requested_id: &str) -> Option<usize> {
        match self {
            Self::Div {
                id: Some(id),
                action: Some(action),
                disabled: false,
                ..
            } if id == requested_id => Some(*action),
            Self::Button {
                id,
                action,
                disabled: false,
                ..
            } if id == requested_id => Some(*action),
            Self::Switch {
                id,
                action: Some(action),
                ..
            } if id == requested_id => Some(*action),
            Self::ColorSwatch { id, action, .. } if id == requested_id => Some(*action),
            Self::Select {
                id,
                action,
                options,
                ..
            } => {
                if id == requested_id {
                    Some(*action)
                } else {
                    options
                        .iter()
                        .find(|(id, _, _)| id == requested_id)
                        .map(|(_, _, action)| *action)
                }
            }
            Self::Image {
                id: Some(id),
                action: Some(action),
                ..
            } if id == requested_id => Some(*action),
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.button_action(requested_id)),
            _ => None,
        }
    }

    pub fn text_field_action(&self, requested_id: &str) -> Option<usize> {
        match self {
            Self::TextField { id, action, .. } if id == requested_id => Some(*action),
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.text_field_action(requested_id)),
            _ => None,
        }
    }

    pub fn slider_action(&self, requested_id: &str) -> Option<usize> {
        match self {
            Self::Slider { id, action, .. } if id == requested_id => Some(*action),
            Self::Box { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Viewport { children, .. }
            | Self::Panel { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.slider_action(requested_id)),
            _ => None,
        }
    }

    pub fn collect_taskbar_badges(
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
            Self::Row { children, .. } | Self::Column { children, .. } => {
                for child in children {
                    child.collect_taskbar_badges(badges)?;
                }
                Ok(())
            }
            _ => Err("taskbar badge extension must return badges or a row/column of badges".into()),
        }
    }

    pub fn collect_desktop_widgets(
        &self,
        widgets: &mut Vec<DesktopPluginWidget>,
    ) -> Result<(), String> {
        match self {
            Self::Widget {
                label,
                value,
                percent,
                color,
            } => {
                if widgets.len() >= 8 {
                    return Err("extension has too many widgets".into());
                }
                widgets.push(DesktopPluginWidget {
                    label: label.clone(),
                    value: value.clone(),
                    percent: *percent,
                    color: *color,
                });
                Ok(())
            }
            Self::Row { children, .. } | Self::Column { children, .. } => {
                for child in children {
                    child.collect_desktop_widgets(widgets)?;
                }
                Ok(())
            }
            _ => Err(
                "desktop widget extension must return widgets or a row/column of widgets".into(),
            ),
        }
    }

    pub fn collect_taskbar_actions(
        &self,
        actions: &mut Vec<TaskbarPluginAction>,
    ) -> Result<(), String> {
        match self {
            Self::Action {
                id, item, label, ..
            } => {
                if actions.len() >= 8 {
                    return Err("extension has too many actions".into());
                }
                if actions.iter().any(|action| action.id == *id) {
                    return Err("extension action IDs must be unique".into());
                }
                actions.push(TaskbarPluginAction {
                    id: id.clone(),
                    item: item.clone(),
                    label: label.clone(),
                });
                Ok(())
            }
            Self::Row { children, .. } | Self::Column { children, .. } => {
                for child in children {
                    child.collect_taskbar_actions(actions)?;
                }
                Ok(())
            }
            _ => Err(
                "taskbar action extension must return actions or a row/column of actions".into(),
            ),
        }
    }

    pub fn collect_control_sections(
        &self,
        sections: &mut Vec<ControlPluginSection>,
    ) -> Result<(), String> {
        match self {
            Self::Section {
                id, label, value, ..
            } => {
                if sections.len() >= 8 {
                    return Err("extension has too many sections".into());
                }
                if sections.iter().any(|section| section.id == *id) {
                    return Err("extension section IDs must be unique".into());
                }
                sections.push(ControlPluginSection {
                    id: id.clone(),
                    label: label.clone(),
                    value: value.clone(),
                });
                Ok(())
            }
            Self::Row { children, .. } | Self::Column { children, .. } => {
                for child in children {
                    child.collect_control_sections(sections)?;
                }
                Ok(())
            }
            _ => Err(
                "control section extension must return sections or a row/column of sections".into(),
            ),
        }
    }
}

fn parse_panel_for_manifest(
    value: &Value,
    manifest: &PluginManifest,
    expected_surface_id: Option<&str>,
) -> Result<PanelNode, String> {
    let mut root = value.clone();
    if root.get("kind").and_then(Value::as_str) == Some("window") && root.get("id").is_none() {
        let id = expected_surface_id
            .or_else(|| (manifest.surfaces.len() == 1).then(|| manifest.surfaces[0].id.as_str()))
            .ok_or("window needs a host surface or an explicit id")?;
        root.as_object_mut()
            .expect("window render is an object")
            .insert("id".into(), Value::String(id.to_owned()));
    }
    fn assign_control_ids(value: &mut Value, path: &str) {
        let Some(node) = value.as_object_mut() else {
            return;
        };
        let kind = node.get("kind").and_then(Value::as_str).unwrap_or("");
        if matches!(
            kind,
            "text-field" | "image-button" | "slider" | "color-swatch"
        ) && !node.contains_key("id")
        {
            let mut hash = 0xcbf29ce484222325_u64;
            for byte in path.bytes().chain(kind.bytes()) {
                hash = (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3);
            }
            node.insert(
                "id".into(),
                Value::String(format!("auto-{kind}-{hash:016x}")),
            );
        }
        if let Some(children) = node.get_mut("children").and_then(Value::as_array_mut) {
            for (index, child) in children.iter_mut().enumerate() {
                let identity = child
                    .get("key")
                    .and_then(|key| {
                        key.as_str()
                            .map(str::to_owned)
                            .or_else(|| key.as_i64().map(|key| key.to_string()))
                    })
                    .unwrap_or_else(|| format!("#{index}"));
                assign_control_ids(child, &format!("{path}/{identity}"));
            }
        }
    }
    assign_control_ids(&mut root, "root");
    let node = PanelNode::parse(&root)?;
    fn collect_windows<'a>(
        node: &'a PanelNode,
        found: &mut Vec<(&'a WindowRequest, Length, Length)>,
    ) {
        let children = match node {
            PanelNode::Surface {
                window_request,
                width,
                height,
                children,
                ..
            } => {
                if let Some(request) = window_request {
                    found.push((request, *width, *height));
                }
                children
            }
            PanelNode::Box { children, .. }
            | PanelNode::Div { children, .. }
            | PanelNode::Viewport { children, .. }
            | PanelNode::Panel { children, .. }
            | PanelNode::Row { children, .. }
            | PanelNode::Column { children, .. }
            | PanelNode::ScrollView { children, .. }
            | PanelNode::Dialog { children, .. }
            | PanelNode::Menu {
                items: children, ..
            }
            | PanelNode::MenuItem { children, .. } => children,
            _ => return,
        };
        for child in children {
            collect_windows(child, found);
        }
    }
    let mut windows = Vec::new();
    collect_windows(&node, &mut windows);
    if !windows.is_empty() {
        if windows.len() != 1
            || !matches!(
                &node,
                PanelNode::Surface {
                    window_request: Some(_),
                    ..
                }
            )
        {
            return Err("Window must be the single top-level surface root".into());
        }
        let (request, width, height) = windows[0];
        if expected_surface_id.is_some_and(|id| request.id != id) {
            return Err(format!(
                "window {:?} does not match its host surface",
                request.id
            ));
        }
        request.validate(manifest, width, height)?;
    }
    Ok(node)
}

pub fn render_panel(
    runtime: &mut JsxRuntime,
    manifest: &PluginManifest,
    expected_surface_id: Option<&str>,
    expression: &str,
) -> Result<PanelNode, String> {
    runtime.render(expression, |value| {
        parse_panel_for_manifest(value, manifest, expected_surface_id)
    })
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
