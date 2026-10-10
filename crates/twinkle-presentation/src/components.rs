//! Bounded JSX component tree and native Twinkle renderer.

use std::{
    collections::{BTreeMap, HashSet},
    sync::Arc,
};

use serde_json::Value;
use twinkle::{
    AnyView, Column, ComponentBuilderExt, Container, DragGesture, DropGesture, Dropdown,
    DropdownPartStyle, Grid, Icon, Image, ImageFit, Layer, Length, OverlayMenuItem, Point, Row,
    SemanticRole, Shortcut, Slider, Spacer, Text, TextField as UiTextField, VerticalScroll,
};
#[cfg(feature = "jsx")]
use twinkle_jsx_runtime::JsxRuntime;
#[cfg(test)]
use twinkle_protocol::SurfaceBounds;
use twinkle_protocol::{NativePatchEnvelope, NativePatchOperation};
use twinkle_protocol::{
    SurfaceBoundsProvider, SurfaceDefinition as PluginSurface, SurfaceKind as PluginSurfaceKind,
};

use crate::css::{ControlStyle, Display, FlexDirection, InteractionState, StyleSheet};

pub type PluginImages = BTreeMap<String, (u16, Arc<image::RgbaImage>)>;
/// Native option identity, label, callback, CSS classes, and optional artwork.
pub type PluginSelectOption = (String, String, usize, Option<String>, Option<String>);

/// Geometry observed from materialized native rows, never supplied by JSX.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualCollectionMeasurements {
    pub source_revision: u64,
    pub width: f32,
    pub rows: Vec<(usize, f32)>,
    /// First materialized row intersecting its native ancestor clip. The host
    /// revalidates visibility and scroll ownership before capturing an anchor.
    pub anchor: Option<twinkle::UiId>,
}

/// A native focused row retained across one synchronous source update. The
/// source lifetime and key must still exist before it can select a new window.
#[derive(Clone, Debug)]
pub struct VirtualCollectionTarget {
    source_identity: u64,
    key: String,
    viewport: f32,
}

fn virtual_row_id(source: &crate::virtual_source::VirtualSource, ordinal: usize) -> String {
    use std::fmt::Write;
    let mut id = format!("__nickel_virtual_row_{}_", source.identity());
    for byte in source
        .key(ordinal)
        .expect("validated virtual row ordinal")
        .bytes()
    {
        write!(id, "{byte:02x}").expect("writing to String");
    }
    id
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginMessage {
    Click(usize),
    Button { click: usize, drag: usize },
    Context(usize),
    Drag(usize, DragGesture),
    Drop(usize, DropGesture),
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
    fn drop(seed: Self, gesture: DropGesture) -> Self;
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

    fn drop(seed: Self, gesture: DropGesture) -> Self {
        let Self::Click(action) = seed else {
            unreachable!("plugin drop seed retains its handler")
        };
        Self::Drop(action, gesture)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct WindowRequest {
    id: String,
    title: Option<String>,
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
        manifest: &impl SurfaceBoundsProvider,
        width: Length,
        height: Length,
    ) -> Result<(), String> {
        let surface = manifest
            .surface_bounds()
            .iter()
            .find(|surface| surface.id == self.id)
            .ok_or_else(|| format!("window {:?} is not declared by the plugin", self.id))?;
        self.validate_against_surface(surface, width, height)
    }

    fn validate_against_surface(
        &self,
        surface: &PluginSurface,
        width: Length,
        height: Length,
    ) -> Result<(), String> {
        let bounded_size = matches!(
            surface.kind,
            PluginSurfaceKind::Window
                | PluginSurfaceKind::Dialog
                | PluginSurfaceKind::Dock
                | PluginSurfaceKind::Overlay
        );
        let valid_size = |requested: Length, granted: u32| {
            matches!(requested, Length::Percent(1.0))
                || matches!(requested, Length::MaxContent)
                || matches!(requested, Length::Px(value) if value >= 1.0
                    && (value == granted as f32 || (bounded_size && value < granted as f32)))
        };
        if !valid_size(width, surface.width) || !valid_size(height, surface.height) {
            return Err(format!(
                "window {:?} size is outside its declared surface bound",
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
                "all" => surface.output == twinkle_protocol::OutputScope::All,
                "primary" | "active" => true,
                _ => false,
            };
            if !matches {
                return Err(format!("window {:?} output exceeds its grant", self.id));
            }
        }
        if self
            .reserve_work_area
            .is_some_and(|reserve| reserve && !surface.reserve_work_area)
        {
            return Err(format!(
                "window {:?} work-area reservation exceeds its grant",
                self.id
            ));
        }
        if self
            .bottom_offset
            .is_some_and(|offset| offset > surface.bottom_offset)
        {
            return Err(format!(
                "window {:?} bottom offset exceeds its grant",
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
                    twinkle_protocol::SurfaceAnchor::TopLeft
                        | twinkle_protocol::SurfaceAnchor::TopCenter
                        | twinkle_protocol::SurfaceAnchor::TopRight
                ),
                "bottom" => {
                    matches!(
                        surface.kind,
                        PluginSurfaceKind::Panel | PluginSurfaceKind::Dock
                    ) || matches!(
                        surface.anchor,
                        twinkle_protocol::SurfaceAnchor::BottomLeft
                            | twinkle_protocol::SurfaceAnchor::BottomCenter
                            | twinkle_protocol::SurfaceAnchor::BottomRight
                    )
                }
                "left" => matches!(
                    surface.anchor,
                    twinkle_protocol::SurfaceAnchor::TopLeft
                        | twinkle_protocol::SurfaceAnchor::BottomLeft
                ),
                "right" => matches!(
                    surface.anchor,
                    twinkle_protocol::SurfaceAnchor::TopRight
                        | twinkle_protocol::SurfaceAnchor::BottomRight
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

#[derive(Clone, Debug)]
pub struct VirtualCollectionDeclaration {
    pub heights: Arc<twinkle::VirtualHeightIndex>,
    pub range: std::ops::Range<usize>,
    pub gap: f32,
    pub overscan: f32,
    pub source: Option<Arc<crate::virtual_source::VirtualSource>>,
    needs_source_ack: bool,
}

impl PartialEq for VirtualCollectionDeclaration {
    fn eq(&self, other: &Self) -> bool {
        self.range == other.range
            && self.gap == other.gap
            && self.overscan == other.overscan
            && self.needs_source_ack == other.needs_source_ack
            && match (&self.source, &other.source) {
                (Some(left), Some(right)) => {
                    left.revision() == right.revision()
                        && Arc::ptr_eq(&self.heights, &other.heights)
                }
                (None, None) => {
                    Arc::ptr_eq(&self.heights, &other.heights) || self.heights == other.heights
                }
                _ => false,
            }
    }
}

impl VirtualCollectionDeclaration {
    fn parse(
        value: &Value,
        owner: Option<&str>,
        sources: &mut crate::virtual_source::VirtualSourceCatalog,
    ) -> Result<Self, String> {
        let bounded = |name: &str, limit: f64| -> Result<f32, String> {
            value
                .get(name)
                .map_or(Some(0.0), Value::as_f64)
                .filter(|value| value.is_finite() && (0.0..=limit).contains(value))
                .map(|value| value as f32)
                .ok_or_else(|| format!("invalid virtual collection {name}"))
        };
        let ordinal = |name| -> Result<usize, String> {
            value
                .get(name)
                .map_or(Some(0), Value::as_u64)
                .and_then(|value| usize::try_from(value).ok())
                .ok_or_else(|| format!("invalid virtual collection {name}"))
        };
        let start = ordinal("start")?;
        let end = ordinal("end")?;
        let gap = bounded("gap", 1024.0)?;
        let height = |value: &Value| {
            value
                .as_f64()
                .filter(|height| height.is_finite() && (1.0..=8192.0).contains(height))
                .map(|height| height as f32)
                .ok_or("invalid virtual row height")
        };
        let source = if let Some(reference) = value.get("source") {
            if ["keys", "heights", "count", "height"]
                .iter()
                .any(|name| value.get(name).is_some())
            {
                return Err("virtual source reference cannot redefine source data".into());
            }
            let owner = owner.ok_or("registered virtual collection needs native identity")?;
            let revision = reference
                .as_u64()
                .ok_or("invalid virtual source reference")?;
            let source = sources.resolve(owner, revision)?;
            if source.gap() != gap {
                return Err("virtual source gap differs from admission".into());
            }
            Some(source)
        } else if let Some(keys) = value.get("keys") {
            let owner = owner.ok_or("registered virtual collection needs native identity")?;
            let keys = keys
                .as_array()
                .ok_or("virtual source keys must be an array")?;
            if keys.len() > 10_000 {
                return Err("virtual source exceeds 10000 keys".into());
            }
            let keys = keys
                .iter()
                .map(|key| {
                    key.as_str()
                        .filter(|key| {
                            !key.is_empty() && key.len() <= crate::virtual_source::MAX_KEY_BYTES
                        })
                        .map(str::to_owned)
                        .ok_or("invalid virtual source key")
                })
                .collect::<Result<Vec<_>, _>>()?;
            let estimates = if value.get("count").is_some() || value.get("height").is_some() {
                if value.get("heights").is_some() {
                    return Err("ambiguous registered virtual source geometry".into());
                }
                if value.get("count").and_then(Value::as_u64) != Some(keys.len() as u64) {
                    return Err("virtual source count differs from keys".into());
                }
                let height = height(value.get("height").ok_or("virtual source needs height")?)?;
                vec![height; keys.len()]
            } else {
                let estimates = value
                    .get("heights")
                    .and_then(Value::as_array)
                    .ok_or("virtual source needs heights")?;
                if estimates.len() != keys.len() {
                    return Err("virtual source estimates differ from keys".into());
                }
                estimates
                    .iter()
                    .map(height)
                    .collect::<Result<Vec<_>, _>>()?
            };
            Some(sources.admit(owner, &keys, &estimates, gap)?)
        } else {
            None
        };
        let heights = if let Some(source) = &source {
            source.geometry_handle()
        } else if value.get("count").is_some() || value.get("height").is_some() {
            if value.get("heights").is_some() {
                return Err("ambiguous virtual collection geometry".into());
            }
            let count = value
                .get("count")
                .and_then(Value::as_u64)
                .filter(|count| *count <= 10_000)
                .ok_or("invalid virtual collection count")?;
            let height = height(
                value
                    .get("height")
                    .ok_or("virtual collection needs height")?,
            )?;
            Arc::new(twinkle::VirtualHeightIndex::uniform(
                count as usize,
                height,
                gap,
            ))
        } else {
            let source = value
                .get("heights")
                .and_then(Value::as_array)
                .ok_or("virtual collection needs heights")?;
            if source.len() > 10_000 {
                return Err("virtual collection exceeds 10000 items".into());
            }
            let heights = source.iter().map(height).collect::<Result<Vec<_>, _>>()?;
            Arc::new(twinkle::VirtualHeightIndex::new(&heights, gap))
        };
        if start > end || end > heights.len() {
            return Err("invalid virtual collection range".into());
        }
        Ok(Self {
            heights,
            range: start..end,
            gap,
            overscan: bounded("overscan", 8192.0)?,
            needs_source_ack: source.is_some() && value.get("source").is_none(),
            source,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PanelNode {
    Badge {
        item: Option<String>,
        label: String,
        count: u16,
        color: u32,
        class_name: Option<String>,
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
    Layer {
        id: Option<String>,
        children: Vec<Self>,
        class_name: Option<String>,
    },
    Div {
        collection: Option<VirtualCollectionDeclaration>,
        id: Option<String>,
        class_name: Option<String>,
        children: Vec<Self>,
        action: Option<usize>,
        drop_action: Option<usize>,
        role: Option<SemanticRole>,
        accessibility_label: Option<String>,
        accessibility_state: Option<String>,
        disabled: bool,
    },
    Surface {
        children: Vec<Self>,
        id: Option<String>,
        window_request: Option<WindowRequest>,
        accessibility_label: Option<String>,
        escape_action: Option<usize>,
        submit_action: Option<usize>,
        focus_action: Option<usize>,
        blur_action: Option<usize>,
        class_name: Option<String>,
        background: u32,
        width: Length,
        height: Length,
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
        color: Option<u32>,
        class_name: Option<String>,
        wrap: bool,
    },
    Image {
        id: Option<String>,
        class_name: Option<String>,
        asset: String,
        accessibility_label: Option<String>,
        width: u32,
        height: u32,
        fit: ImageFit,
        action: Option<usize>,
        context_action: Option<usize>,
    },
    Progress {
        class_name: Option<String>,
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
        drag_action: Option<usize>,
    },
    Switch {
        id: String,
        class_name: Option<String>,
        state: String,
        label: String,
        action: Option<usize>,
    },
    Checkbox {
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
        options: Vec<PluginSelectOption>,
    },
    Spacer {
        class_name: Option<String>,
    },
    Slot {
        id: String,
        class_name: Option<String>,
    },
    TextField {
        id: String,
        class_name: Option<String>,
        accessibility_label: String,
        value: String,
        placeholder: String,
        secure: bool,
        auto_focus: bool,
        action: usize,
        focus_action: Option<usize>,
        blur_action: Option<usize>,
    },
    Button {
        id: String,
        class_name: Option<String>,
        label: String,
        description: Option<String>,
        accessibility_label: String,
        accessibility_state: Option<String>,
        disabled: bool,
        width: Option<u32>,
        height: Option<u32>,
        icon: Option<String>,
        icon_size: u32,
        icon_above: bool,
        show_label: bool,
        action: usize,
        context_action: Option<usize>,
        drag_action: Option<usize>,
        drop_action: Option<usize>,
        focus_action: Option<usize>,
        blur_action: Option<usize>,
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
        class_name: Option<String>,
        anchor: String,
        point: Option<Point>,
        open: bool,
        items: Vec<Self>,
    },
    MenuItem {
        id: String,
        class_name: Option<String>,
        label: String,
        action: Option<usize>,
        children: Vec<Self>,
        disabled_reason: Option<String>,
        shortcut: Option<String>,
        separator_before: bool,
    },
}

fn dropdown_part(style: &ControlStyle) -> DropdownPartStyle {
    let pixels = |length| match length {
        Some(Length::Px(value)) => value,
        _ => 0.0,
    };
    DropdownPartStyle {
        width: pixels(style.width),
        height: pixels(style.height),
        padding: style.padding.unwrap_or_default(),
        margin: style.margin.unwrap_or_default(),
        background: style.background.filter(|color| *color != 0),
        interaction_backgrounds: [None; 3],
        interaction_paints: [twinkle::InteractionPaint::default(); 3],
        inherited_text: style.inherited_text,
        foreground: style.color.filter(|color| *color != 0),
        border_color: style.border_color.filter(|color| *color != 0),
        border_width: style.border_width.unwrap_or(0.0),
        radius: style.radius.unwrap_or(0.0),
        font_size: style.font_size.unwrap_or(0.0),
        line_height: style.line_height.unwrap_or(0.0),
    }
}

fn interaction_paints(
    stylesheet: &StyleSheet,
    kind: &str,
    id: &str,
    class_name: Option<&str>,
    style: &ControlStyle,
) -> [twinkle::InteractionPaint; 3] {
    [
        InteractionState::Hover,
        InteractionState::Active,
        InteractionState::Focus,
    ]
    .map(|state| {
        stylesheet.resolve_interaction_paint(
            kind,
            Some(id),
            class_name,
            state,
            &style.custom_properties,
            &style.ancestors[..style.ancestors.len().saturating_sub(1)],
        )
    })
}

fn apply_control_interactions<Message>(
    mut control: Container<Message>,
    style: &ControlStyle,
    stylesheet: &StyleSheet,
    kind: &str,
    id: &str,
    class_name: Option<&str>,
) -> Container<Message> {
    control = control.automatic_focus_tint(false).interaction_paints(
        [
            InteractionState::Hover,
            InteractionState::Active,
            InteractionState::Focus,
        ]
        .map(|state| {
            stylesheet.resolve_interaction_paint(
                kind,
                Some(id),
                class_name,
                state,
                &style.custom_properties,
                &style.ancestors[..style.ancestors.len().saturating_sub(1)],
            )
        }),
    );

    control
}

fn apply_container_style<Message>(
    mut container: Container<Message>,
    style: &ControlStyle,
) -> Container<Message> {
    container = container.css_paint(true).automatic_focus_tint(false);
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
        // otherwise be interpreted as legacy opaque RGB black by twinkle.
        container = if background == 0 {
            container.clear_background()
        } else {
            container.background(background)
        };
    }
    if let Some(radius) = style.radius {
        container = container.radius(radius);
    }
    if let Some(shadow) = style.box_shadow {
        container = container.box_shadow(shadow);
    }
    if let Some(blur) = style.backdrop_blur {
        container = container.backdrop_blur(blur);
    }
    if let Some(magnification) = style.proximity_magnification {
        container = container.proximity_magnification(magnification);
    }
    if let Some(duration) = style.transition_duration_ms {
        container = container.transition_duration_ms(duration);
    }
    if style.border_width.is_some() || style.border_color.is_some() {
        container = if style.border_color.or(style.color).unwrap_or(0) == 0 {
            container.clear_border()
        } else {
            container.border(
                style.border_color.or(style.color).unwrap_or(0),
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
    let inherits = if style
        .ancestors
        .last()
        .is_some_and(|ancestor| ancestor.0 == "button")
    {
        [true; 3]
    } else {
        style.inherited_text
    };
    text = text.css_paint(true).inherited_state_text(inherits);
    if let Some(color) = style.color {
        text = text.color(color);
    }
    if let Some(font_size) = style.font_size {
        text = text.font_size(font_size);
    }
    if let Some(line_height) = style.line_height {
        text = text.line_height(line_height);
    }
    if let Some(align) = style.text_align {
        text = text.align(align);
    }
    text
}

#[derive(Clone, Default)]
struct InheritedTextStyle {
    color: Option<u32>,
    font_size: Option<f32>,
    line_height: Option<f32>,
    text_align: Option<twinkle::TextAlign>,
    custom_properties: Arc<std::collections::HashMap<String, String>>,
    ancestors: Vec<(String, Option<String>, Option<String>)>,
}

impl InheritedTextStyle {
    fn extend(&self, style: &ControlStyle) -> Self {
        Self {
            color: style.color.or(self.color),
            font_size: style.font_size.or(self.font_size),
            line_height: style.line_height.or(self.line_height),
            text_align: style.text_align.or(self.text_align),
            custom_properties: style.custom_properties.clone(),
            ancestors: style.ancestors.clone(),
        }
    }

    fn apply(&self, mut style: ControlStyle) -> ControlStyle {
        style.inherited_text = [
            style.color.is_none(),
            style.font_size.is_none(),
            style.line_height.is_none(),
        ];
        style.color = style.color.or(self.color);
        style.font_size = style.font_size.or(self.font_size);
        style.line_height = style.line_height.or(self.line_height);
        style.text_align = style.text_align.or(self.text_align);
        style
    }

    fn resolve(
        &self,
        stylesheet: &StyleSheet,
        kind: &str,
        id: Option<&str>,
        class_name: Option<&str>,
    ) -> ControlStyle {
        stylesheet.resolve_with_ancestors(
            kind,
            id,
            class_name,
            &self.custom_properties,
            &self.ancestors,
        )
    }
}

impl PanelNode {
    fn parse_flow_children(
        children: &[Value],
        sources: &mut crate::virtual_source::VirtualSourceCatalog,
    ) -> Result<Vec<Self>, String> {
        let mut nodes = Vec::new();
        let mut text = String::new();
        let flush_text = |nodes: &mut Vec<Self>, text: &mut String| {
            if !text.trim().is_empty() {
                nodes.push(Self::Text {
                    value: std::mem::take(text),
                    class_name: None,
                    wrap: true,
                    color: None,
                });
            } else {
                text.clear();
            }
        };
        for child in children {
            match child {
                Value::Null => {}
                Value::String(value) => text.push_str(value),
                Value::Number(value) => text.push_str(&value.to_string()),
                Value::Object(_) => {
                    flush_text(&mut nodes, &mut text);
                    nodes.push(Self::parse_with_sources(child, sources)?);
                }
                _ => return Err("component children must be elements, text, or numbers".into()),
            }
        }
        flush_text(&mut nodes, &mut text);
        Ok(nodes)
    }

    pub fn container_children(&self) -> Option<&Vec<Self>> {
        match self {
            Self::Box { children, .. }
            | Self::Layer { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => Some(children),
            _ => None,
        }
    }

    /// Only declarations admitted into this tree may request application artwork.
    /// Unmaterialized virtual rows and closed overlays contribute no demand.
    pub fn application_image_assets(&self) -> std::collections::BTreeSet<String> {
        fn visit(node: &PanelNode, assets: &mut std::collections::BTreeSet<String>) {
            let asset = match node {
                PanelNode::Image { asset, .. } => Some(asset),
                PanelNode::Button { icon, .. } => icon.as_ref(),
                _ => None,
            };
            if let Some(asset) = asset.filter(|asset| asset.starts_with("application:")) {
                assets.insert(asset.clone());
            }
            if let PanelNode::Select {
                options,
                value,
                open,
                ..
            } = node
            {
                for (_, label, _, _, icon) in options {
                    if (*open || label == value)
                        && let Some(asset) = icon
                            .as_ref()
                            .filter(|asset| asset.starts_with("application:"))
                    {
                        assets.insert(asset.clone());
                    }
                }
            }
            let children = match node {
                PanelNode::Dialog {
                    open: true,
                    children,
                    ..
                }
                | PanelNode::Menu {
                    open: true,
                    items: children,
                    ..
                }
                | PanelNode::MenuItem { children, .. } => Some(children),
                _ => node.container_children(),
            };
            if let Some(children) = children {
                for child in children {
                    visit(child, assets);
                }
            }
        }
        let mut assets = Default::default();
        visit(self, &mut assets);
        assets
    }

    /// Resource demand follows resolved native visibility, never package-supplied
    /// viewport claims. The admitted tree already excludes unmaterialized rows.
    pub fn visible_wallpaper_assets(
        &self,
        layout: &twinkle::ResolvedLayout,
        viewport: twinkle::Rect,
    ) -> Vec<String> {
        fn intersects(left: twinkle::Rect, right: twinkle::Rect) -> bool {
            left.size.width > 0.0
                && left.size.height > 0.0
                && right.size.width > 0.0
                && right.size.height > 0.0
                && left.origin.x < right.origin.x + right.size.width
                && right.origin.x < left.origin.x + left.size.width
                && left.origin.y < right.origin.y + right.size.height
                && right.origin.y < left.origin.y + left.size.height
        }
        fn visit(
            node: &PanelNode,
            layout: &twinkle::ResolvedLayout,
            viewport: twinkle::Rect,
            assets: &mut Vec<String>,
        ) {
            if let PanelNode::Image {
                id: Some(id),
                asset,
                ..
            } = node
                && asset.starts_with("wallpaper:")
            {
                let suffix = format!("/{id}");
                if layout.nodes().iter().any(|resolved| {
                    resolved.id.as_str().ends_with(&suffix)
                        && intersects(resolved.allocated, viewport)
                        && resolved.clip.is_none_or(|clip| {
                            intersects(resolved.allocated, clip) && intersects(clip, viewport)
                        })
                }) && !assets.contains(asset)
                {
                    assets.push(asset.clone());
                }
            }
            let children = match node {
                PanelNode::Dialog {
                    open: true,
                    children,
                    ..
                }
                | PanelNode::Menu {
                    open: true,
                    items: children,
                    ..
                }
                | PanelNode::MenuItem { children, .. } => Some(children),
                _ => node.container_children(),
            };
            if let Some(children) = children {
                for child in children {
                    visit(child, layout, viewport, assets);
                }
            }
        }
        let mut assets = Vec::new();
        visit(self, layout, viewport, &mut assets);
        assets
    }

    /// Native-only feedback for collection row construction; ordinary pointer
    /// callbacks never supply the viewport or select the materialized range.
    pub fn virtual_collection_feedback(
        &self,
        layout: &twinkle::ResolvedLayout,
        viewport: twinkle::Rect,
    ) -> Result<Vec<PluginMessage>, String> {
        fn visit(
            node: &PanelNode,
            layout: &twinkle::ResolvedLayout,
            viewport: twinkle::Rect,
            result: &mut Vec<PluginMessage>,
        ) -> Result<(), String> {
            if let PanelNode::Div {
                id: Some(id),
                action: Some(action),
                collection: Some(collection),
                ..
            } = node
            {
                let suffix = format!("/{id}");
                let mut matches = layout
                    .nodes()
                    .iter()
                    .filter(|node| node.id.as_str().ends_with(&suffix));
                if let Some(resolved) = matches.next() {
                    if matches.next().is_some() {
                        return Err("ambiguous virtual collection ID".into());
                    }
                    // Ancestor clips can outlive an output's viewport. The
                    // materialized range must satisfy both authorities.
                    let clip = resolved.clip.map_or(viewport, |clip| {
                        let left = clip.origin.x.max(viewport.origin.x);
                        let top = clip.origin.y.max(viewport.origin.y);
                        let right = (clip.origin.x + clip.size.width)
                            .min(viewport.origin.x + viewport.size.width);
                        let bottom = (clip.origin.y + clip.size.height)
                            .min(viewport.origin.y + viewport.size.height);
                        twinkle::Rect::new(
                            left,
                            top,
                            (right - left).max(0.0),
                            (bottom - top).max(0.0),
                        )
                    });
                    let window = collection.heights.window_in_clip(
                        resolved.content,
                        clip,
                        collection.overscan,
                    );
                    if window.range != collection.range || collection.needs_source_ack {
                        if result.len() >= 64 {
                            return Err("too many virtual collection updates".into());
                        }
                        result.push(PluginMessage::Text(
                            *action,
                            serde_json::json!({"start":window.range.start,"end":window.range.end,
                                "source":collection.source.as_ref().map(|source| source.revision())})
                                .to_string(),
                        ));
                    }
                }
            }
            if let Some(children) = node.container_children() {
                for child in children {
                    visit(child, layout, viewport, result)?;
                }
            }
            Ok(())
        }
        let mut result = Vec::new();
        visit(self, layout, viewport, &mut result)?;
        Ok(result)
    }

    pub fn contains_virtual_collection(&self) -> bool {
        matches!(
            self,
            Self::Div {
                collection: Some(_),
                ..
            }
        ) || self
            .container_children()
            .is_some_and(|children| children.iter().any(Self::contains_virtual_collection))
    }

    /// Prepare native accessibility focus from a current logical key. The
    /// requested collection must itself be admitted; omitted rows are not built
    /// or searched, and a retired/revised source cannot authorize the request.
    pub fn virtual_key_focus_event(
        &self,
        collection: &twinkle::UiId,
        revision: u64,
        key: &str,
        layout: &twinkle::ResolvedLayout,
    ) -> Result<twinkle::UiEvent, String> {
        let logical = layout
            .find(collection)
            .and_then(|node| node.virtual_navigation.as_ref())
            .filter(|logical| logical.revision == revision)
            .ok_or("virtual collection is unavailable or stale")?;
        fn source_for(
            node: &PanelNode,
            revision: u64,
        ) -> Option<&crate::virtual_source::VirtualSource> {
            if let PanelNode::Div {
                collection: Some(collection),
                ..
            } = node
                && let Some(source) = &collection.source
                && source.revision() == revision
            {
                return Some(source);
            }
            node.container_children()?
                .iter()
                .find_map(|child| source_for(child, revision))
        }
        let source = source_for(self, revision).ok_or("virtual source is unavailable or stale")?;
        let ordinal = source.ordinal(key).ok_or("virtual key is unavailable")?;
        if logical.count != source.len() {
            return Err("virtual source admission disagrees with layout".into());
        }
        let geometry = source
            .geometry()
            .window_for_range(ordinal..ordinal + 1)
            .ok_or("virtual row geometry is unavailable")?;
        Ok(twinkle::UiEvent::AccessibilityRevealVirtualRow {
            collection: collection.clone(),
            revision,
            ordinal,
            leading: geometry.leading,
            height: geometry.total - geometry.leading - geometry.trailing,
        })
    }

    pub fn capture_virtual_targets(
        &self,
        target: &twinkle::UiId,
        layout: &twinkle::ResolvedLayout,
    ) -> Result<Vec<VirtualCollectionTarget>, String> {
        if layout.find(target).is_none() {
            return Ok(Vec::new());
        }
        // Index admitted row identities once. Scanning the complete resolved
        // layout for every admitted row made nested focus repair quadratic in
        // visible content even though omitted rows were never constructed.
        let mut resolved = BTreeMap::<&str, Vec<&twinkle::ResolvedNode>>::new();
        for row in layout.nodes() {
            if let Some((_, local)) = row.id.as_str().rsplit_once('/') {
                resolved.entry(local).or_default().push(row);
            }
        }
        fn visit(
            node: &PanelNode,
            target: &twinkle::UiId,
            resolved: &BTreeMap<&str, Vec<&twinkle::ResolvedNode>>,
            result: &mut Vec<VirtualCollectionTarget>,
        ) -> Result<(), String> {
            if let PanelNode::Div {
                collection: Some(collection),
                ..
            } = node
                && let Some(source) = &collection.source
            {
                for ordinal in collection.range.clone() {
                    let local = virtual_row_id(source, ordinal);
                    let mut rows = resolved.get(local.as_str()).into_iter().flatten();
                    if let Some(row) = rows.next() {
                        if rows.next().is_some() {
                            return Err("ambiguous virtual focus row".into());
                        }
                        if target == &row.id
                            || target
                                .as_str()
                                .strip_prefix(row.id.as_str())
                                .is_some_and(|tail| tail.starts_with('/'))
                        {
                            let viewport = row
                                .clip
                                .map_or(row.allocated.size.height, |clip| clip.size.height);
                            if viewport.is_finite() && viewport > 0.0 {
                                result.push(VirtualCollectionTarget {
                                    source_identity: source.identity(),
                                    key: source.key(ordinal).expect("admitted row").to_owned(),
                                    viewport,
                                });
                            }
                            break;
                        }
                    }
                }
            }
            if let Some(children) = node.container_children() {
                for child in children {
                    visit(child, target, resolved, result)?;
                }
            }
            Ok(())
        }
        let mut result = Vec::new();
        visit(self, target, &resolved, &mut result)?;
        Ok(result)
    }

    /// Resolve captured keys using the current native catalog. This runs before
    /// host focus reconciliation, so a far reorder cannot retire a surviving
    /// target merely because the provisional range contains its former ordinal.
    pub fn virtual_target_feedback(
        &self,
        targets: &[VirtualCollectionTarget],
    ) -> Vec<PluginMessage> {
        fn visit(
            node: &PanelNode,
            targets: &[VirtualCollectionTarget],
            result: &mut Vec<PluginMessage>,
        ) {
            if let PanelNode::Div {
                action: Some(action),
                collection: Some(collection),
                ..
            } = node
                && let Some(source) = &collection.source
                && let Some(target) = targets
                    .iter()
                    .find(|target| target.source_identity == source.identity())
                && let Some(ordinal) = source.ordinal(&target.key)
                && !collection.range.contains(&ordinal)
                && let Some(window) =
                    source.reveal_window(&target.key, target.viewport, collection.overscan)
            {
                result.push(PluginMessage::Text(
                    *action,
                    serde_json::json!({
                        "start":window.range.start,"end":window.range.end,"source":source.revision()
                    })
                    .to_string(),
                ));
            }
            if let Some(children) = node.container_children() {
                for child in children {
                    visit(child, targets, result);
                }
            }
        }
        let mut result = Vec::new();
        visit(self, targets, &mut result);
        result
    }

    /// Collect only admitted, materialized row wrappers. Logical keys are read
    /// for the visible range, not scanned; nested collections have independent
    /// native source revisions. Duplicate resolved identities fail closed.
    pub fn virtual_collection_measurements(
        &self,
        layout: &twinkle::ResolvedLayout,
    ) -> Result<Vec<VirtualCollectionMeasurements>, String> {
        let mut resolved = BTreeMap::new();
        for node in layout.nodes() {
            let local = node.id.as_str().rsplit('/').next().unwrap_or_default();
            resolved.entry(local).or_insert_with(Vec::new).push(node);
        }
        fn unique<'a>(
            index: &BTreeMap<&str, Vec<&'a twinkle::ResolvedNode>>,
            id: &str,
        ) -> Result<Option<&'a twinkle::ResolvedNode>, String> {
            if id.contains('/') {
                let suffix = format!("/{id}");
                let mut matches = index
                    .values()
                    .flatten()
                    .filter(|node| node.id.as_str().ends_with(&suffix));
                let first = matches.next().copied();
                return if matches.next().is_some() {
                    Err("ambiguous virtual measurement identity".into())
                } else {
                    Ok(first)
                };
            }
            match index.get(id).map(Vec::as_slice) {
                None | Some([]) => Ok(None),
                Some([node]) => Ok(Some(*node)),
                Some(_) => Err("ambiguous virtual measurement identity".into()),
            }
        }
        fn visit(
            node: &PanelNode,
            index: &BTreeMap<&str, Vec<&twinkle::ResolvedNode>>,
            result: &mut Vec<VirtualCollectionMeasurements>,
        ) -> Result<(), String> {
            if let PanelNode::Div {
                id: Some(id),
                collection: Some(collection),
                ..
            } = node
                && let Some(source) = &collection.source
                && let Some(container) = unique(index, id)?
            {
                let mut rows = Vec::with_capacity(collection.range.len());
                let mut anchor = None;
                for ordinal in collection.range.clone() {
                    let row = unique(index, &virtual_row_id(source, ordinal))?
                        .ok_or("missing materialized virtual row geometry")?;
                    rows.push((ordinal, row.allocated.size.height));
                    let clip = row.clip.unwrap_or(container.content);
                    if anchor.is_none()
                        && row.allocated.size.width > 0.0
                        && row.allocated.size.height > 0.0
                        && clip.size.width > 0.0
                        && clip.size.height > 0.0
                        && row.allocated.origin.y < clip.origin.y + clip.size.height
                        && row.allocated.origin.y + row.allocated.size.height > clip.origin.y
                        && row.allocated.origin.x < clip.origin.x + clip.size.width
                        && row.allocated.origin.x + row.allocated.size.width > clip.origin.x
                    {
                        anchor = Some(row.id.clone());
                    }
                }
                result.push(VirtualCollectionMeasurements {
                    source_revision: source.revision(),
                    width: container.content.size.width,
                    rows,
                    anchor,
                });
            }
            if let Some(children) = node.container_children() {
                for child in children {
                    visit(child, index, result)?;
                }
            }
            Ok(())
        }
        let mut result = Vec::new();
        visit(self, &resolved, &mut result)?;
        Ok(result)
    }

    pub fn direct_child_with_class(&self, class: &str) -> Option<&Self> {
        let children = self.container_children()?;
        let mut matches = children.iter().filter(|child| {
            let class_name = match child {
                Self::Box { class_name, .. }
                | Self::Layer { class_name, .. }
                | Self::Div { class_name, .. }
                | Self::Surface { class_name, .. }
                | Self::Row { class_name, .. }
                | Self::Column { class_name, .. }
                | Self::ScrollView { class_name, .. }
                | Self::Text { class_name, .. }
                | Self::Badge { class_name, .. }
                | Self::Image { class_name, .. }
                | Self::Progress { class_name, .. }
                | Self::Slider { class_name, .. }
                | Self::Switch { class_name, .. }
                | Self::Checkbox { class_name, .. }
                | Self::ColorSwatch { class_name, .. }
                | Self::Select { class_name, .. }
                | Self::Spacer { class_name, .. }
                | Self::Slot { class_name, .. }
                | Self::TextField { class_name, .. }
                | Self::Button { class_name, .. }
                | Self::Menu { class_name, .. } => class_name.as_deref(),
                _ => None,
            };
            class_name
                .is_some_and(|classes| classes.split_ascii_whitespace().any(|name| name == class))
        });
        let first = matches.next()?;
        matches.next().is_none().then_some(first)
    }

    pub fn requested_surface(
        &self,
        grant: &PluginSurface,
        stylesheet: &StyleSheet,
    ) -> Result<Option<PluginSurface>, String> {
        let Self::Surface {
            window_request,
            id,
            class_name,
            width,
            height,
            ..
        } = self
        else {
            return Ok(None);
        };
        let style = stylesheet.resolve("window", id.as_deref(), class_name.as_deref());
        let width = style.width.unwrap_or(*width);
        let height = style.height.unwrap_or(*height);
        let (css_bottom, css_top) = (style.bottom, style.top);
        if css_bottom.is_some() && css_top.is_some() {
            return Err("window CSS cannot set both top and bottom".into());
        }
        let Some(request) = window_request else {
            if css_bottom.is_some() || css_top.is_some() {
                return Err("CSS top or bottom needs a Window root".into());
            }
            return Ok(None);
        };
        if request.id != grant.id {
            return Ok(None);
        }
        request.validate_against_surface(grant, width, height)?;
        let dimension = |length, bound| match length {
            Length::Px(value) => value.ceil() as u32,
            Length::Percent(1.0) => bound,
            _ => bound,
        };
        let mut surface = grant.clone();
        surface.width = dimension(width, grant.width);
        surface.height = dimension(height, grant.height);
        if request.output.as_deref() == Some("primary") {
            surface.output = twinkle_protocol::OutputScope::Primary;
        } else if request.output.as_deref() == Some("active") {
            surface.output = twinkle_protocol::OutputScope::Active;
        }
        if let Some(reserve) = request.reserve_work_area {
            surface.reserve_work_area = reserve;
        }
        if let Some(offset) = request.bottom_offset {
            surface.bottom_offset = offset;
        } else if let Some(offset) = css_bottom {
            if !matches!(
                grant.kind,
                PluginSurfaceKind::Panel | PluginSurfaceKind::Dock
            ) {
                return Err(format!(
                    "window {:?} CSS bottom needs a panel or dock grant",
                    request.id
                ));
            }
            if offset > grant.bottom_offset as f32 {
                return Err(format!(
                    "window {:?} CSS bottom exceeds its grant",
                    request.id
                ));
            }
            surface.bottom_offset = offset.round() as u32;
        }
        if let Some(offset) = css_top {
            if !matches!(
                grant.anchor,
                twinkle_protocol::SurfaceAnchor::TopLeft
                    | twinkle_protocol::SurfaceAnchor::TopCenter
                    | twinkle_protocol::SurfaceAnchor::TopRight
            ) || grant.offset_y < 0
            {
                return Err(format!(
                    "window {:?} CSS top needs a top-anchored grant",
                    request.id
                ));
            }
            if offset > grant.offset_y as f32 {
                return Err(format!("window {:?} CSS top exceeds its grant", request.id));
            }
            surface.offset_y = offset.round() as i32;
        }
        Ok(Some(surface))
    }

    pub fn requested_surface_lengths(&self, stylesheet: &StyleSheet) -> Option<(Length, Length)> {
        let Self::Surface {
            id,
            class_name,
            width,
            height,
            window_request: Some(_),
            ..
        } = self
        else {
            return None;
        };
        let style = stylesheet.resolve("window", id.as_deref(), class_name.as_deref());
        Some((
            style.width.unwrap_or(*width),
            style.height.unwrap_or(*height),
        ))
    }

    pub fn retained_bytes(&self) -> u64 {
        let own = std::mem::size_of::<Self>() as u64;
        let capacity = |value: &String| value.capacity() as u64;
        let descendants = self.container_children().map_or(0, |children| {
            ((children.capacity() - children.len()) * std::mem::size_of::<Self>()) as u64
                + children.iter().map(Self::retained_bytes).sum::<u64>()
        });
        own + descendants
            + match self {
                Self::Button {
                    id,
                    class_name,
                    label,
                    description,
                    accessibility_label,
                    accessibility_state,
                    icon,
                    ..
                } => {
                    capacity(id)
                        + class_name.as_ref().map_or(0, capacity)
                        + capacity(label)
                        + description.as_ref().map_or(0, capacity)
                        + capacity(accessibility_label)
                        + accessibility_state.as_ref().map_or(0, capacity)
                        + icon.as_ref().map_or(0, capacity)
                }
                Self::Badge {
                    item,
                    label,
                    class_name,
                    ..
                } => {
                    item.as_ref().map_or(0, capacity)
                        + capacity(label)
                        + class_name.as_ref().map_or(0, capacity)
                }
                Self::Slot { id, class_name } => {
                    capacity(id) + class_name.as_ref().map_or(0, capacity)
                }
                Self::ColorSwatch {
                    id,
                    class_name,
                    label,
                    ..
                } => capacity(id) + class_name.as_ref().map_or(0, capacity) + capacity(label),
                Self::Image {
                    id,
                    class_name,
                    asset,
                    accessibility_label,
                    ..
                } => {
                    id.as_ref().map_or(0, capacity)
                        + class_name.as_ref().map_or(0, capacity)
                        + capacity(asset)
                        + accessibility_label.as_ref().map_or(0, capacity)
                }
                Self::Switch {
                    id,
                    class_name,
                    state,
                    label,
                    ..
                }
                | Self::Checkbox {
                    id,
                    class_name,
                    state,
                    label,
                    ..
                } => {
                    capacity(id)
                        + capacity(state)
                        + capacity(label)
                        + class_name.as_ref().map_or(0, capacity)
                }
                Self::MenuItem {
                    id,
                    class_name,
                    label,
                    children,
                    disabled_reason,
                    shortcut,
                    ..
                } => {
                    capacity(id)
                        + class_name.as_ref().map_or(0, capacity)
                        + capacity(label)
                        + disabled_reason.as_ref().map_or(0, capacity)
                        + shortcut.as_ref().map_or(0, capacity)
                        + children.iter().map(Self::retained_bytes).sum::<u64>()
                }
                Self::Progress { class_name, .. } => class_name.as_ref().map_or(0, capacity),
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
                        + (options.capacity() * std::mem::size_of::<PluginSelectOption>()) as u64
                        + options
                            .iter()
                            .map(|(id, label, _, class, icon)| {
                                capacity(id)
                                    + capacity(label)
                                    + class.as_ref().map_or(0, capacity)
                                    + icon.as_ref().map_or(0, capacity)
                            })
                            .sum::<u64>()
                }
                Self::Div {
                    id,
                    class_name,
                    accessibility_label,
                    accessibility_state,
                    collection,
                    ..
                } => {
                    id.as_ref().map_or(0, capacity)
                        + class_name.as_ref().map_or(0, capacity)
                        + accessibility_label.as_ref().map_or(0, capacity)
                        + accessibility_state.as_ref().map_or(0, capacity)
                        + collection.as_ref().map_or(0, |collection| {
                            collection.source.as_ref().map_or_else(
                                || collection.heights.retained_bytes() as u64,
                                |source| source.payload_bytes() as u64,
                            )
                        })
                }
                Self::Box { class_name, .. }
                | Self::Row { class_name, .. }
                | Self::Column { class_name, .. } => class_name.as_ref().map_or(0, capacity),
                Self::Layer { id, class_name, .. } => {
                    id.as_ref().map_or(0, capacity) + class_name.as_ref().map_or(0, capacity)
                }
                Self::ScrollView { id, class_name, .. } => {
                    capacity(id) + class_name.as_ref().map_or(0, capacity)
                }
                Self::Surface {
                    id,
                    window_request,
                    accessibility_label,
                    class_name,
                    ..
                } => {
                    id.as_ref().map_or(0, capacity)
                        + accessibility_label.as_ref().map_or(0, capacity)
                        + class_name.as_ref().map_or(0, capacity)
                        + window_request.as_ref().map_or(0, |request| {
                            capacity(&request.id)
                                + capacity(&request.placement)
                                + request.title.as_ref().map_or(0, capacity)
                                + request.output.as_ref().map_or(0, capacity)
                                + request.edge.as_ref().map_or(0, capacity)
                                + request.anchor.as_ref().map_or(0, capacity)
                        })
                }
                Self::Menu {
                    id,
                    class_name,
                    anchor,
                    items,
                    ..
                } => {
                    capacity(id)
                        + capacity(anchor)
                        + class_name.as_ref().map_or(0, capacity)
                        + items.iter().map(Self::retained_bytes).sum::<u64>()
                }
                _ => 0,
            }
    }

    fn parse(value: &Value) -> Result<Self, String> {
        Self::parse_with_sources(value, &mut Default::default())
    }

    fn parse_with_sources(
        value: &Value,
        sources: &mut crate::virtual_source::VirtualSourceCatalog,
    ) -> Result<Self, String> {
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
                    | "layer"
                    | "div"
                    | "window"
                    | "row"
                    | "column"
                    | "scroll-view"
                    | "text"
                    | "spacer"
                    | "slot"
                    | "text-field"
                    | "slider"
                    | "switch"
                    | "checkbox"
                    | "color-swatch"
                    | "select"
                    | "menu"
                    | "menu-item"
                    | "button"
                    | "image"
                    | "image-button"
                    | "progress"
                    | "badge"
            )
        {
            return Err(format!("className is not supported on {kind} yet"));
        }
        match kind {
            "slot" => {
                if !children.is_empty() {
                    return Err("slot cannot have children".into());
                }
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 128)
                    .ok_or("slot needs an id of 1 to 128 bytes")?;
                Ok(Self::Slot {
                    id: id.to_owned(),
                    class_name,
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
                    class_name,
                    color: value
                        .get("color")
                        .and_then(Value::as_u64)
                        .filter(|color| *color <= u32::MAX as u64)
                        .map_or(0xffc9354c, |color| color as u32),
                })
            }
            "div" => {
                let role = value.get("role").and_then(Value::as_str);
                let collection = value
                    .get("collection")
                    .map(|collection| {
                        VirtualCollectionDeclaration::parse(
                            collection,
                            value.get("__nativeId").and_then(Value::as_str),
                            sources,
                        )
                    })
                    .transpose()?;
                if let Some(collection) = &collection {
                    if value.get("id").and_then(Value::as_str).is_none()
                        || value.get("action").and_then(Value::as_u64).is_none()
                    {
                        return Err("virtual collection needs an ID and window callback".into());
                    }
                    if children.len() != collection.range.len() {
                        return Err("virtual collection children do not match its range".into());
                    }
                }
                let interactive =
                    collection.is_none() && value.get("action").and_then(Value::as_u64).is_some();
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
                    collection,
                    id: match value.get("id") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(id)) if !id.is_empty() && id.len() <= 128 => {
                            Some(id.clone())
                        }
                        _ => return Err("div id must contain 1 to 128 bytes".into()),
                    },
                    class_name,
                    children: Self::parse_flow_children(children, sources)?,
                    action: match value.get("action") {
                        None | Some(Value::Null) => None,
                        Some(action) => Some(
                            action
                                .as_u64()
                                .and_then(|action| usize::try_from(action).ok())
                                .ok_or("div action index is invalid")?,
                        ),
                    },
                    drop_action: value
                        .get("dropAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
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
                    children: Self::parse_flow_children(children, sources)?,
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
            "layer" => Ok(Self::Layer {
                id: match value.get("id") {
                    None | Some(Value::Null) => None,
                    Some(Value::String(id)) if !id.is_empty() && id.len() <= 128 => {
                        Some(id.clone())
                    }
                    _ => return Err("layer id must contain 1 to 128 bytes".into()),
                },
                children: Self::parse_flow_children(children, sources)?,
                class_name,
            }),
            "window" => {
                let dimension = |name| match value.get(name) {
                    Some(Value::Number(size)) => size
                        .as_u64()
                        .filter(|size| (1..=8192).contains(size))
                        .map(|size| Length::Px(size as f32))
                        .ok_or_else(|| format!("{kind} {name} must be 1 to 8192")),
                    Some(Value::String(percent)) if percent == "100%" => Ok(Length::Percent(1.0)),
                    Some(Value::String(keyword)) if keyword == "max-content" => {
                        Ok(Length::MaxContent)
                    }
                    None | Some(Value::Null) => Ok(Length::Percent(1.0)),
                    _ => Err(format!(
                        "{kind} {name} must be 1 to 8192, 100%, or max-content"
                    )),
                };
                let id = value
                    .get("id")
                    .and_then(Value::as_str)
                    .filter(|id| !id.is_empty() && id.len() <= 128)
                    .map(str::to_owned);
                let window_request = {
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
                    let title = match value.get("title") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(title))
                            if !title.is_empty()
                                && title.len() <= 128
                                && !title.chars().any(char::is_control) =>
                        {
                            Some(title.clone())
                        }
                        _ => return Err("window title must be 1 to 128 printable bytes".into()),
                    };
                    let output = optional_token("output", &["primary", "active", "all"])?;
                    let edge = optional_token("edge", &["top", "bottom", "left", "right"])?;
                    let anchor = optional_token(
                        "anchor",
                        &[
                            "center",
                            "top-left",
                            "top-center",
                            "top-right",
                            "bottom-left",
                            "bottom-center",
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
                        title,
                        placement,
                        output,
                        edge,
                        anchor,
                        reserve_work_area,
                        bottom_offset,
                    })
                };
                Ok(Self::Surface {
                    class_name,
                    window_request,
                    accessibility_label: match value
                        .get("aria-label")
                        .or_else(|| value.get("accessibilityLabel"))
                    {
                        None | Some(Value::Null) => None,
                        Some(Value::String(label)) if !label.is_empty() && label.len() <= 256 => {
                            Some(label.clone())
                        }
                        _ => {
                            return Err(
                                "window accessible label must contain 1 to 256 bytes".into()
                            );
                        }
                    },
                    escape_action: value
                        .get("escapeAction")
                        .and_then(Value::as_u64)
                        .map(|action| action as usize),
                    submit_action: value
                        .get("submitAction")
                        .and_then(Value::as_u64)
                        .map(|action| action as usize),
                    focus_action: value
                        .get("focusAction")
                        .and_then(Value::as_u64)
                        .map(|action| action as usize),
                    blur_action: value
                        .get("blurAction")
                        .and_then(Value::as_u64)
                        .map(|action| action as usize),
                    children: Self::parse_flow_children(children, sources)?,
                    id,
                    background: value
                        .get("background")
                        .and_then(Value::as_u64)
                        .map_or(0, |color| color as u32),
                    width: dimension("width")?,
                    height: dimension("height")?,
                })
            }
            "row" | "column" | "scroll-view" => {
                let children = Self::parse_flow_children(children, sources)?;
                if kind == "row" {
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
                    .map(|color| color as u32),
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
                    let generated = value
                        .get("__nativeId")
                        .and_then(Value::as_str)
                        .map(|native| {
                            let hash = native.bytes().fold(0xcbf29ce484222325_u64, |hash, byte| {
                                (hash ^ u64::from(byte)).wrapping_mul(0x100000001b3)
                            });
                            format!("native-image-{hash:016x}")
                        });
                    (
                        value
                            .get("id")
                            .map(|id| {
                                id.as_str()
                                    .filter(|id| !id.is_empty() && id.len() <= 128)
                                    .map(str::to_owned)
                                    .ok_or("image ID must contain 1 to 128 characters")
                            })
                            .transpose()?
                            .or(generated),
                        None,
                        None,
                    )
                };
                Ok(Self::Image {
                    id,
                    class_name,
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
                    class_name,
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
                    drag_action: value
                        .get("dragAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
                })
            }
            "switch" | "checkbox" => {
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
                if kind == "checkbox" {
                    Ok(Self::Checkbox {
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
                } else {
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
                    let option_class = match child.get("className") {
                        None => None,
                        Some(Value::String(class))
                            if class.len() <= 256
                                && class.split_ascii_whitespace().all(|name| {
                                    name.len() <= 64
                                        && name.bytes().all(|byte| {
                                            byte.is_ascii_alphanumeric()
                                                || matches!(byte, b'-' | b'_')
                                        })
                                }) =>
                        {
                            Some(class.clone())
                        }
                        _ => return Err("option className is invalid".into()),
                    };
                    let icon = match child.get("icon") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(asset))
                            if !asset.is_empty()
                                && asset.len() <= 512
                                && !asset.chars().any(char::is_control) =>
                        {
                            Some(asset.clone())
                        }
                        _ => return Err("option icon must be a bounded asset key".into()),
                    };
                    options.push((id.to_owned(), label, action, option_class, icon));
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
                accessibility_label: value
                    .get("aria-label")
                    .or_else(|| value.get("accessibilityLabel"))
                    .and_then(Value::as_str)
                    .or_else(|| value.get("placeholder").and_then(Value::as_str))
                    .unwrap_or("")
                    .to_owned(),
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
                auto_focus: match value.get("autoFocus") {
                    None => false,
                    Some(Value::Bool(value)) => *value,
                    _ => return Err("autoFocus must be a boolean".into()),
                },
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
                focus_action: value
                    .get("focusAction")
                    .and_then(Value::as_u64)
                    .and_then(|action| usize::try_from(action).ok()),
                blur_action: value
                    .get("blurAction")
                    .and_then(Value::as_u64)
                    .and_then(|action| usize::try_from(action).ok()),
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
                    icon_size: match value.get("iconSize") {
                        None | Some(Value::Null) => 32,
                        Some(size) => size
                            .as_u64()
                            .filter(|size| (8..=128).contains(size))
                            .map(|size| size as u32)
                            .ok_or("button iconSize must be 8 to 128")?,
                    },
                    icon_above: match value.get("iconPlacement") {
                        None | Some(Value::Null) => false,
                        Some(Value::String(position)) if position == "left" => false,
                        Some(Value::String(position)) if position == "top" => true,
                        _ => return Err("button iconPlacement must be left or top".into()),
                    },
                    show_label: value
                        .get("showLabel")
                        .and_then(Value::as_bool)
                        .unwrap_or(false),
                    description: match value.get("description") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(text)) if text.len() <= 4096 => Some(text.clone()),
                        _ => {
                            return Err(
                                "button description must be a string of at most 4096 bytes".into(),
                            );
                        }
                    },
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
                    drop_action: value
                        .get("dropAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
                    focus_action: value
                        .get("focusAction")
                        .and_then(Value::as_u64)
                        .and_then(|action| usize::try_from(action).ok()),
                    blur_action: value
                        .get("blurAction")
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
                children: Self::parse_flow_children(children, sources)?,
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
                    .map(|value| Self::parse_with_sources(value, sources))
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
                    class_name,
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
                        .map(|value| Self::parse_with_sources(value, sources))
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
                    class_name,
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

    /// Resolve a detached menu through its JSX ancestors so subtree variables
    /// and descendant selectors survive its native overlay boundary.
    pub fn style_overlay_menu_for<Message>(
        &self,
        id: &str,
        menu: twinkle::OverlayMenu<Message>,
        stylesheet: &StyleSheet,
    ) -> twinkle::OverlayMenu<Message> {
        if let Some((node, inherited)) =
            self.menu_style_context(id, stylesheet, InheritedTextStyle::default())
        {
            node.style_overlay_menu_inherited(menu, stylesheet, inherited)
        } else {
            menu
        }
    }

    fn menu_style_context<'a>(
        &'a self,
        requested: &str,
        stylesheet: &StyleSheet,
        inherited: InheritedTextStyle,
    ) -> Option<(&'a Self, InheritedTextStyle)> {
        if matches!(self, Self::Menu { id, .. } if id == requested) {
            return Some((self, inherited));
        }
        let (kind, id, class_name) = match self {
            Self::Surface { id, class_name, .. } => {
                ("window", id.as_deref(), class_name.as_deref())
            }
            Self::Div { id, class_name, .. } => ("div", id.as_deref(), class_name.as_deref()),
            Self::Layer { id, class_name, .. } => ("layer", id.as_deref(), class_name.as_deref()),
            Self::Box { class_name, .. } => ("box", None, class_name.as_deref()),
            Self::Row { class_name, .. } => ("row", None, class_name.as_deref()),
            Self::Column { class_name, .. } => ("column", None, class_name.as_deref()),
            Self::ScrollView { id, class_name, .. } => {
                ("scroll-view", Some(id.as_str()), class_name.as_deref())
            }
            _ => return None,
        };
        let style = inherited.resolve(stylesheet, kind, id, class_name);
        self.container_children()?.iter().find_map(|child| {
            child.menu_style_context(requested, stylesheet, inherited.extend(&style))
        })
    }

    /// Apply package CSS to native menu placement, paint and item metrics.
    pub fn style_overlay_menu<Message>(
        &self,
        menu: twinkle::OverlayMenu<Message>,
        stylesheet: &StyleSheet,
    ) -> twinkle::OverlayMenu<Message> {
        self.style_overlay_menu_inherited(menu, stylesheet, InheritedTextStyle::default())
    }

    fn style_overlay_menu_inherited<Message>(
        &self,
        mut menu: twinkle::OverlayMenu<Message>,
        stylesheet: &StyleSheet,
        inherited: InheritedTextStyle,
    ) -> twinkle::OverlayMenu<Message> {
        let Self::Menu { id, class_name, .. } = self else {
            return menu;
        };
        let style = inherited.resolve(stylesheet, "menu", Some(id), class_name.as_deref());
        let inherited = inherited.extend(&style);
        let item = inherited.apply(inherited.resolve(
            stylesheet,
            "menu-item",
            Some(id),
            class_name.as_deref(),
        ));
        let pixels = |length| match length {
            Some(Length::Px(value)) => value,
            _ => 0.0,
        };
        menu.width = pixels(style.width);
        menu.fit_content = false;
        menu.padding = style.padding.unwrap_or_default();
        menu.radius = style.radius.unwrap_or(0.0);
        menu.background = style.background.unwrap_or(0);
        menu.border = style.border_color.unwrap_or(0);
        menu.border_width = style.border_width.unwrap_or(0.0);
        menu.row_height = pixels(item.height);
        menu.row_gap = style.gap.unwrap_or(0.0);
        menu.foreground = item.color.unwrap_or(0);
        menu.text_scale = item.font_size.unwrap_or(0.0) / 7.0;
        menu.item_padding = item.padding.unwrap_or_default();
        menu.item_line_height = item.line_height.unwrap_or(0.0);
        menu.shortcut_scale = 1.0;
        menu.item_background = item.background;
        menu.item_border = item.border_color;
        menu.item_border_width = item.border_width.unwrap_or(0.0);
        menu.item_radius = item.radius.unwrap_or(0.0);
        let interaction = |state| {
            stylesheet.resolve_interaction_background_with_properties(
                "menu-item",
                Some(id),
                class_name.as_deref(),
                state,
                &item.custom_properties,
                &inherited.ancestors,
            )
        };
        menu.item_hover = interaction(crate::css::InteractionState::Hover);
        menu.item_pressed = interaction(crate::css::InteractionState::Active);
        menu.item_selected = interaction(crate::css::InteractionState::Focus);
        menu.direction = stylesheet.reading_direction();
        if let Self::Menu { items, .. } = self {
            for (source, target) in items.iter().zip(&mut menu.items) {
                source.style_menu_item(target, stylesheet, &inherited, class_name.as_deref());
            }
        }
        menu
    }

    fn style_menu_item<Message>(
        &self,
        item: &mut OverlayMenuItem<Message>,
        stylesheet: &StyleSheet,
        inherited: &InheritedTextStyle,
        owner_class: Option<&str>,
    ) {
        let Self::MenuItem {
            id,
            class_name,
            action,
            children,
            ..
        } = self
        else {
            return;
        };
        let classes = format!(
            "{} {} {}",
            owner_class.unwrap_or(""),
            class_name.as_deref().unwrap_or(""),
            if action.is_none() && children.is_empty() {
                "disabled"
            } else {
                "enabled"
            }
        );
        let style =
            inherited.apply(inherited.resolve(stylesheet, "menu-item", Some(id), Some(&classes)));
        let nested = inherited.extend(&style);
        let shortcut =
            nested.apply(nested.resolve(stylesheet, "menu-shortcut", Some(id), Some(&classes)));
        let indicator =
            nested.apply(nested.resolve(stylesheet, "menu-indicator", Some(id), Some(&classes)));
        let mut frame = dropdown_part(&style);
        frame.interaction_backgrounds = [
            InteractionState::Hover,
            InteractionState::Active,
            InteractionState::Focus,
        ]
        .map(|state| {
            stylesheet.resolve_interaction_background_with_properties(
                "menu-item",
                Some(id),
                Some(&classes),
                state,
                &style.custom_properties,
                &inherited.ancestors,
            )
        });
        frame.interaction_paints = [
            InteractionState::Hover,
            InteractionState::Active,
            InteractionState::Focus,
        ]
        .map(|state| {
            stylesheet.resolve_interaction_paint(
                "menu-item",
                Some(id),
                Some(&classes),
                state,
                &style.custom_properties,
                &inherited.ancestors,
            )
        });
        let mut shortcut_part = dropdown_part(&shortcut);
        shortcut_part.interaction_paints =
            interaction_paints(stylesheet, "menu-shortcut", id, Some(&classes), &shortcut);
        let mut indicator_part = dropdown_part(&indicator);
        indicator_part.interaction_paints =
            interaction_paints(stylesheet, "menu-indicator", id, Some(&classes), &indicator);
        item.presentation = Some(Box::new([frame, shortcut_part, indicator_part]));
        for (source, target) in children.iter().zip(&mut item.children) {
            source.style_menu_item(target, stylesheet, &nested, Some(&classes));
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
            ..
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
        self.view_as_with_slots(images, stylesheet, scope, &mut |_| None)
    }

    pub fn view_as_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        self.view_as_scoped_with_slots(
            images,
            stylesheet,
            scope,
            InheritedTextStyle::default(),
            slots,
        )
    }

    // Keep compound builders outside the recursive renderer to bound its stack frame.
    fn view_slider<Message: PluginUiMessage>(
        &self,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
    ) -> AnyView<Message> {
        let Self::Slider {
            id,
            class_name,
            value,
            label,
            action,
            drag_action,
        } = self
        else {
            unreachable!()
        };
        let style = inherited.resolve(stylesheet, "slider", Some(id), class_name.as_deref());
        let parts = inherited.extend(&style);
        let track_style =
            parts.resolve(stylesheet, "slider-track", Some(id), class_name.as_deref());
        let fill_style = parts.resolve(stylesheet, "slider-fill", Some(id), class_name.as_deref());
        let thumb_style =
            parts.resolve(stylesheet, "slider-thumb", Some(id), class_name.as_deref());
        let mut frame = dropdown_part(&style);
        frame.interaction_paints =
            interaction_paints(stylesheet, "slider", id, class_name.as_deref(), &style);
        let part = |kind: &str, style: &ControlStyle| {
            let mut part = dropdown_part(style);
            part.interaction_paints =
                interaction_paints(stylesheet, kind, id, class_name.as_deref(), style);
            part
        };
        let slider = Slider::on_change_with(
            Message::from_plugin_scoped(PluginMessage::Value(*action, *value), scope),
            Message::value,
            *value,
        )
        .id(id.clone())
        .automatic_focus_tint(false)
        .parts(
            part("slider-track", &track_style),
            part("slider-fill", &fill_style),
            part("slider-thumb", &thumb_style),
        );
        let slider = if let Some(drag) = drag_action {
            slider.on_drag((
                Message::from_plugin_scoped(
                    PluginMessage::Button {
                        click: *action,
                        drag: *drag,
                    },
                    scope,
                ),
                Message::drag,
            ))
        } else {
            slider
        };
        let mut slider = slider
            .accessibility_label(label.clone())
            .css_frame(frame)
            .width_length(style.width.unwrap_or(Length::Auto))
            .height_length(style.height.unwrap_or(Length::Auto))
            .grow(style.grow.unwrap_or(0.0));
        if let Some(value) = style.min_width {
            slider = slider.min_width(value);
        }
        if let Some(value) = style.max_width {
            slider = slider.max_width(value);
        }
        if let Some(value) = style.min_height {
            slider = slider.min_height(value);
        }
        if let Some(value) = style.max_height {
            slider = slider.max_height(value);
        }
        if let Some(value) = style.shrink {
            slider = slider.shrink(value);
        }
        if let Some(value) = style.basis {
            slider = slider.basis(value);
        }
        with_margin(AnyView::new(slider), &style)
    }

    fn view_select<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
    ) -> AnyView<Message> {
        let Self::Select {
            id,
            class_name,
            label,
            value,
            open,
            action,
            options,
        } = self
        else {
            unreachable!()
        };
        let style = inherited.resolve(stylesheet, "select", Some(id), class_name.as_deref());
        let parts = inherited.extend(&style);
        let header_style = parts.apply(parts.resolve(
            stylesheet,
            "select-header",
            Some(id),
            class_name.as_deref(),
        ));
        let option_style =
            parts.apply(parts.resolve(stylesheet, "option", Some(id), class_name.as_deref()));
        let indicator_style = parts.apply(parts.resolve(
            stylesheet,
            "select-indicator",
            Some(id),
            class_name.as_deref(),
        ));
        let with_interactions = |kind: &str, control: &ControlStyle, target_id: &str| {
            let mut part = dropdown_part(control);
            part.interaction_backgrounds = [
                crate::css::InteractionState::Hover,
                crate::css::InteractionState::Active,
                crate::css::InteractionState::Focus,
            ]
            .map(|state| {
                stylesheet.resolve_interaction_background_with_properties(
                    kind,
                    Some(target_id),
                    control
                        .ancestors
                        .last()
                        .and_then(|ancestor| ancestor.2.as_deref()),
                    state,
                    &control.custom_properties,
                    &control.ancestors[..control.ancestors.len().saturating_sub(1)],
                )
            });
            part.interaction_paints = [
                crate::css::InteractionState::Hover,
                crate::css::InteractionState::Active,
                crate::css::InteractionState::Focus,
            ]
            .map(|state| {
                stylesheet.resolve_interaction_paint(
                    kind,
                    Some(target_id),
                    control
                        .ancestors
                        .last()
                        .and_then(|ancestor| ancestor.2.as_deref()),
                    state,
                    &control.custom_properties,
                    &control.ancestors[..control.ancestors.len().saturating_sub(1)],
                )
            });
            part
        };
        let select = Dropdown::new(
            Message::from_plugin_scoped(PluginMessage::Click(*action), scope),
            value,
            options.iter().map(|(_, label, action, _, _)| {
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
        .parts(
            with_interactions("select-header", &header_style, id),
            with_interactions("option", &option_style, id),
            with_interactions("select-indicator", &indicator_style, id),
        )
        .icons(
            options
                .iter()
                .find(|(_, label, _, _, _)| label == value)
                .and_then(|(_, _, _, _, icon)| icon.as_ref())
                .and_then(|asset| images.get(asset))
                .cloned(),
            options.iter().map(|(_, _, _, _, icon)| {
                icon.as_ref().and_then(|asset| images.get(asset)).cloned()
            }),
        )
        .option_parts(options.iter().map(|(option_id, _, _, option_class, _)| {
            let classes = format!(
                "{} {}",
                class_name.as_deref().unwrap_or(""),
                option_class.as_deref().unwrap_or("")
            );
            let option =
                parts.apply(parts.resolve(stylesheet, "option", Some(option_id), Some(&classes)));
            with_interactions("option", &option, option_id)
        }));
        let mut frame = dropdown_part(&style);
        frame.interaction_paints =
            interaction_paints(stylesheet, "select", id, class_name.as_deref(), &style);
        let mut select = select
            .css_frame(frame)
            .width_length(style.width.unwrap_or(Length::Auto))
            .height_length(style.height.unwrap_or(Length::Auto))
            .grow(style.grow.unwrap_or(0.0));
        if let Some(value) = style.min_width {
            select = select.min_width(value);
        }
        if let Some(value) = style.max_width {
            select = select.max_width(value);
        }
        if let Some(value) = style.min_height {
            select = select.min_height(value);
        }
        if let Some(value) = style.max_height {
            select = select.max_height(value);
        }
        if let Some(value) = style.shrink {
            select = select.shrink(value);
        }
        if let Some(value) = style.basis {
            select = select.basis(value);
        }
        with_margin(AnyView::new(select), &style)
    }

    fn view_colorswatch<Message: PluginUiMessage>(
        &self,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
    ) -> AnyView<Message> {
        let Self::ColorSwatch {
            id,
            class_name,
            color,
            selected,
            label,
            action,
        } = self
        else {
            unreachable!()
        };
        let state_classes = format!(
            "{} {}",
            class_name.as_deref().unwrap_or(""),
            if *selected { "selected" } else { "unselected" }
        );
        let class_name = Some(state_classes);
        let style = inherited.resolve(stylesheet, "color-swatch", Some(id), class_name.as_deref());
        let parts = inherited.extend(&style);
        let inner = if let Some(color) = color {
            let fill_style = parts.resolve(
                stylesheet,
                "color-swatch-fill",
                Some(id),
                class_name.as_deref(),
            );
            let fill = apply_container_style(Container::new(), &fill_style)
                .part_interaction_paints(interaction_paints(
                    stylesheet,
                    "color-swatch-fill",
                    id,
                    class_name.as_deref(),
                    &fill_style,
                ));
            // The swatch color is semantic data, like an image source.
            AnyView::new(if *color == 0 {
                fill.clear_background()
            } else {
                fill.background(*color)
            })
        } else {
            let label_style = parts.apply(parts.resolve(
                stylesheet,
                "color-swatch-label",
                Some(id),
                class_name.as_deref(),
            ));
            let text = styled_text(Text::new("+"), &label_style).part_interaction_text(
                interaction_paints(
                    stylesheet,
                    "color-swatch-label",
                    id,
                    class_name.as_deref(),
                    &label_style,
                ),
            );
            with_margin(
                AnyView::new(
                    apply_container_style(Container::new(), &label_style)
                        .part_interaction_paints(interaction_paints(
                            stylesheet,
                            "color-swatch-label",
                            id,
                            class_name.as_deref(),
                            &label_style,
                        ))
                        .child(text),
                ),
                &label_style,
            )
        };
        let control = Container::new()
            .id(id.clone())
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
        let control = apply_control_interactions(
            control,
            &style,
            stylesheet,
            "color-swatch",
            id,
            class_name.as_deref(),
        );
        with_margin(AnyView::new(apply_container_style(control, &style)), &style)
    }

    // Keep the large collection/flex/grid branch off the stack of every
    // recursively rendered non-div node, especially nested overlay controls.
    #[inline(never)]
    fn view_div<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        match self {
            Self::Div {
                id,
                class_name,
                children,
                action,
                drop_action,
                role,
                accessibility_label,
                accessibility_state,
                disabled,
                collection,
            } => {
                let style =
                    inherited.resolve(stylesheet, "div", id.as_deref(), class_name.as_deref());
                let content: AnyView<Message> = if let Some(collection) = collection {
                    let column = twinkle::VirtualColumn::new();
                    let column = if let Some(source) = &collection.source {
                        column.logical_navigation(source.len(), source.revision())
                    } else {
                        column
                    };
                    AnyView::new(
                        column
                            .window(
                                collection
                                    .heights
                                    .window_for_range(collection.range.clone())
                                    .expect("validated collection range"),
                            )
                            .gap(collection.gap)
                            .children(children.iter().enumerate().map(|(offset, child)| {
                                let view = child.view_as_scoped_with_slots::<Message>(
                                    images,
                                    stylesheet,
                                    scope,
                                    inherited.extend(&style),
                                    slots,
                                );
                                if let Some(source) = &collection.source {
                                    AnyView::new(
                                        Container::new()
                                            .id(virtual_row_id(
                                                source,
                                                collection.range.start + offset,
                                            ))
                                            .fill_width()
                                            .semantic_role(SemanticRole::ListItem)
                                            .accessibility_label(
                                                source
                                                    .key(collection.range.start + offset)
                                                    .expect("validated virtual row ordinal"),
                                            )
                                            .accessibility_description(format!(
                                                "item {} of {}",
                                                collection.range.start + offset + 1,
                                                source.len()
                                            ))
                                            // Match native Collection's minimum
                                            // positive virtual item extent.
                                            .min_height(1.0)
                                            .child(view),
                                    )
                                } else {
                                    view
                                }
                            })),
                    )
                } else {
                    match style.display.unwrap_or_default() {
                        Display::Grid => {
                            let mut grid = style
                                .grid_columns
                                .as_ref()
                                .map_or_else(Grid::new, |tracks| {
                                    Grid::tracks(tracks.iter().cloned())
                                });
                            if let Some(gap) = style.gap {
                                grid = grid.gap(gap);
                            }
                            if let Some(align) = style.align_items {
                                grid = grid.align_items(align);
                            }
                            if let Some(justify) = style.justify_content {
                                grid = grid.justify_content(justify);
                            }
                            for child in children {
                                grid = grid.child(child.view_as_scoped_with_slots::<Message>(
                                    images,
                                    stylesheet,
                                    scope,
                                    inherited.extend(&style),
                                    slots,
                                ));
                            }
                            grid = grid.direction(stylesheet.reading_direction());
                            AnyView::new(grid.grow(1.0))
                        }
                        Display::Flex
                            if style.flex_direction.unwrap_or_default() == FlexDirection::Row =>
                        {
                            let mut row = Row::new();
                            if let Some(width) = style.width {
                                row = row.width_length(width);
                            }
                            if let Some(height) = style.height {
                                row = row.height_length(height);
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
                                row = row.child(child.view_as_scoped_with_slots::<Message>(
                                    images,
                                    stylesheet,
                                    scope,
                                    inherited.extend(&style),
                                    slots,
                                ));
                            }
                            if stylesheet.reading_direction()
                                == twinkle::ReadingDirection::RightToLeft
                            {
                                row = row.reverse();
                            }
                            AnyView::new(row.grow(1.0))
                        }
                        Display::Block | Display::Flex => {
                            let mut column = Column::new();
                            if let Some(width) = style.width {
                                column = column.width_length(width);
                            }
                            if let Some(height) = style.height {
                                column = column.height_length(height);
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
                                column = column.child(child.view_as_scoped_with_slots::<Message>(
                                    images,
                                    stylesheet,
                                    scope,
                                    inherited.extend(&style),
                                    slots,
                                ));
                            }
                            AnyView::new(column.grow(1.0))
                        }
                    }
                };
                let mut container = Container::new().child(content);
                if collection.is_some() {
                    container = container.semantic_role(SemanticRole::List);
                }
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
                } else if let Some(action) = action.filter(|_| collection.is_none()) {
                    container = container.message(Message::from_plugin_scoped(
                        PluginMessage::Click(action),
                        scope,
                    ));
                }
                if let Some(action) = drop_action.filter(|_| !*disabled) {
                    container = container.on_drop((
                        Message::from_plugin_scoped(PluginMessage::Click(action), scope),
                        Message::drop,
                    ));
                }
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            _ => unreachable!("view_div requires a Div node"),
        }
    }

    fn view_as_scoped_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        // Compound widget builders have large debug stack frames. Dispatch
        // recursive containers before entering the non-recursive control builder.
        match self {
            Self::Div { .. } => self.view_div(images, stylesheet, scope, inherited, slots),
            Self::Box { .. } => {
                self.view_box_with_slots(images, stylesheet, scope, inherited, slots)
            }
            Self::Layer { .. } => {
                self.view_layer_with_slots(images, stylesheet, scope, inherited, slots)
            }
            Self::Surface { .. } => {
                self.view_surface_with_slots(images, stylesheet, scope, inherited, slots)
            }
            Self::Row { .. } => {
                self.view_row_with_slots(images, stylesheet, scope, inherited, slots)
            }
            Self::Column { .. } => {
                self.view_column_with_slots(images, stylesheet, scope, inherited, slots)
            }
            Self::ScrollView { .. } => {
                self.view_scroll_view_with_slots(images, stylesheet, scope, inherited, slots)
            }
            _ => self.view_control_with_slots(images, stylesheet, scope, inherited, slots),
        }
    }

    fn view_box_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        match self {
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
                let style = inherited.resolve(stylesheet, "box", None, class_name.as_deref());
                let mut column = Column::new().fill_width();
                for child in children {
                    column = column.child(child.view_as_scoped_with_slots::<Message>(
                        images,
                        stylesheet,
                        scope,
                        inherited.extend(&style),
                        slots,
                    ));
                }
                let container = Container::new()
                    .position(Point {
                        x: *x as f32,
                        y: *y as f32,
                    })
                    .width(*width as f32)
                    .height(*height as f32)
                    .radius(*radius as f32)
                    .child(column);
                let container = if *background == 0 {
                    container.clear_background()
                } else {
                    container.background(*background)
                };
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            _ => unreachable!("container variant dispatched by view_as_scoped_with_slots"),
        }
    }

    fn view_layer_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        match self {
            Self::Layer {
                id,
                children,
                class_name,
            } => {
                let style =
                    inherited.resolve(stylesheet, "layer", id.as_deref(), class_name.as_deref());
                let mut layer = Layer::new();
                if let Some(width) = style.width {
                    layer = layer.width_length(width);
                }
                if let Some(height) = style.height {
                    layer = layer.height_length(height);
                }
                if let Some(id) = id {
                    layer = layer.id(id.clone());
                }
                for child in children {
                    layer = layer.child(child.view_as_scoped_with_slots::<Message>(
                        images,
                        stylesheet,
                        scope,
                        inherited.extend(&style),
                        slots,
                    ));
                }
                AnyView::new(layer)
            }
            _ => unreachable!("container variant dispatched by view_as_scoped_with_slots"),
        }
    }

    fn view_surface_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        match self {
            Self::Surface {
                children,
                id,
                accessibility_label,
                class_name,
                background,
                width,
                height,
                window_request,
                ..
            } => {
                let style =
                    inherited.resolve(stylesheet, "window", id.as_deref(), class_name.as_deref());
                // A managed window's declared dimensions request its initial
                // native geometry; they are not a permanent content viewport.
                // Once the compositor resizes the window, its root must follow
                // the current host surface. Fixed shell surfaces keep their
                // declarative dimensions and placement contract.
                let managed = window_request
                    .as_ref()
                    .is_some_and(|request| request.placement == "managed");
                let width = if managed {
                    Length::Percent(1.0)
                } else {
                    style.width.unwrap_or(*width)
                };
                let height = if managed {
                    Length::Percent(1.0)
                } else {
                    style.height.unwrap_or(*height)
                };
                let mut layer = Layer::new().width_length(width).height_length(height);
                for child in children {
                    if !matches!(child, Self::Dialog { .. }) {
                        layer = layer.child(child.view_as_scoped_with_slots::<Message>(
                            images,
                            stylesheet,
                            scope,
                            inherited.extend(&style),
                            slots,
                        ));
                    }
                }
                let mut container = Container::new()
                    .width_length(width)
                    .height_length(height)
                    .child(layer);
                container = if *background == 0 {
                    container.clear_background()
                } else {
                    container.background(*background)
                };
                if let Some(id) = id {
                    container = container.id(id.clone());
                }
                if let Some(label) = accessibility_label {
                    container = container
                        .semantic_role(SemanticRole::ApplicationPresentation)
                        .accessibility_label(label.clone());
                }
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            _ => unreachable!("container variant dispatched by view_as_scoped_with_slots"),
        }
    }

    fn view_row_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        match self {
            Self::Row {
                children,
                class_name,
            } => {
                let style = inherited.resolve(stylesheet, "row", None, class_name.as_deref());
                let mut row = Row::new();
                if let Some(width) = style.width {
                    row = row.width_length(width);
                }
                if let Some(height) = style.height {
                    row = row.height_length(height);
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
                    row = row.child(child.view_as_scoped_with_slots::<Message>(
                        images,
                        stylesheet,
                        scope,
                        inherited.extend(&style),
                        slots,
                    ));
                }
                if stylesheet.reading_direction() == twinkle::ReadingDirection::RightToLeft {
                    row = row.reverse();
                }
                if style == ControlStyle::default() {
                    AnyView::new(row)
                } else {
                    with_margin(
                        AnyView::new(apply_container_style(
                            Container::new().child(row.grow(1.0)),
                            &style,
                        )),
                        &style,
                    )
                }
            }
            _ => unreachable!("container variant dispatched by view_as_scoped_with_slots"),
        }
    }

    fn view_column_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        match self {
            Self::Column {
                children,
                class_name,
            } => {
                let style = inherited.resolve(stylesheet, "column", None, class_name.as_deref());
                let mut column = Column::new();
                if let Some(width) = style.width {
                    column = column.width_length(width);
                }
                if let Some(height) = style.height {
                    column = column.height_length(height);
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
                    column = column.child(child.view_as_scoped_with_slots::<Message>(
                        images,
                        stylesheet,
                        scope,
                        inherited.extend(&style),
                        slots,
                    ));
                }
                if style == ControlStyle::default() {
                    AnyView::new(column)
                } else {
                    with_margin(
                        AnyView::new(apply_container_style(
                            Container::new().child(column.grow(1.0)),
                            &style,
                        )),
                        &style,
                    )
                }
            }
            _ => unreachable!("container variant dispatched by view_as_scoped_with_slots"),
        }
    }

    fn view_scroll_view_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        match self {
            Self::ScrollView {
                id,
                class_name,
                height,
                grow,
                children,
            } => {
                let style =
                    inherited.resolve(stylesheet, "scroll-view", Some(id), class_name.as_deref());
                let mut column = Column::new().fill_width();
                if let Some(gap) = style.gap {
                    column = column.gap(gap);
                }
                for child in children {
                    column = column.child(child.view_as_scoped_with_slots::<Message>(
                        images,
                        stylesheet,
                        scope,
                        inherited.extend(&style),
                        slots,
                    ));
                }
                let inherited_parts = inherited.extend(&style);
                let part = |kind: &str| {
                    let style =
                        inherited_parts.resolve(stylesheet, kind, Some(id), class_name.as_deref());
                    let mut part = dropdown_part(&style);
                    part.interaction_paints =
                        interaction_paints(stylesheet, kind, id, class_name.as_deref(), &style);
                    part
                };
                let scroll = VerticalScroll::new(
                    Message::from_plugin_scoped(PluginMessage::Scroll, scope),
                    0.0,
                )
                .id(id.clone())
                .scrollbar_parts(part("scrollbar-track"), part("scrollbar-thumb"))
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
            _ => unreachable!("container variant dispatched by view_as_scoped_with_slots"),
        }
    }

    fn view_control_with_slots<Message: PluginUiMessage>(
        &self,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
        inherited: InheritedTextStyle,
        slots: &mut dyn FnMut(&str) -> Option<AnyView<Message>>,
    ) -> AnyView<Message> {
        match self {
            Self::Badge {
                label,
                count,
                color,
                class_name,
                ..
            } => {
                let style = inherited.resolve(stylesheet, "badge", None, class_name.as_deref());
                let text = styled_text(
                    Text::new(count.to_string()).color(0xffffffff).scale(0.72),
                    &inherited.apply(style.clone()),
                );
                let container = Container::new()
                    .width(30.0)
                    .height(24.0)
                    .radius(12.0)
                    .align_items(twinkle::Align::Center)
                    .justify_content(twinkle::Justify::Center)
                    .semantic_role(SemanticRole::Status)
                    .accessibility_label(format!("{label}: {count}"))
                    .child(text);
                let container = apply_container_style(container, &style);
                let container = if style.background.is_none() {
                    container.background(*color)
                } else {
                    container
                };
                with_margin(AnyView::new(container), &style)
            }
            Self::Div { .. }
            | Self::Box { .. }
            | Self::Layer { .. }
            | Self::Surface { .. }
            | Self::Row { .. }
            | Self::Column { .. }
            | Self::ScrollView { .. } => unreachable!("containers bypass the control renderer"),
            Self::Text {
                value,
                color,
                class_name,
                wrap,
            } => {
                let style = inherited.resolve(stylesheet, "text", None, class_name.as_deref());
                let text_style = inherited.apply(style.clone());
                let text = styled_text(
                    Text::new(value)
                        .color(
                            style
                                .color
                                .or(*color)
                                .or(inherited.color)
                                .unwrap_or(0xfff4f6fa),
                        )
                        .scale(1.0)
                        .wrap(*wrap),
                    &text_style,
                );
                let container = Container::new()
                    .semantic_role(SemanticRole::Text)
                    .accessibility_label(value.clone())
                    .child(text);
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::Image {
                id,
                class_name,
                asset,
                accessibility_label,
                width,
                height,
                fit,
                action,
                context_action,
            } => {
                let kind = if action.is_some() {
                    "image-button"
                } else {
                    "image"
                };
                let style =
                    inherited.resolve(stylesheet, kind, id.as_deref(), class_name.as_deref());
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
                if let Some(id) = id {
                    container = container.id(id.clone());
                }
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
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::Progress {
                class_name,
                percent,
                width,
                height,
            } => {
                let style = inherited.resolve(stylesheet, "progress", None, class_name.as_deref());
                let parts = inherited.extend(&style);
                let fill_style =
                    parts.resolve(stylesheet, "progress-fill", None, class_name.as_deref());
                let fill = apply_container_style(Container::new(), &fill_style)
                    .width_length(Length::Percent(f32::from(*percent) / 100.0))
                    .fill_height();
                let track = apply_container_style(
                    Container::new()
                        .semantic_role(SemanticRole::Status)
                        .accessibility_label(format!("{percent}%"))
                        .width(*width as f32)
                        .height(*height as f32)
                        .align_items(twinkle::Align::Start)
                        .child(fill),
                    &style,
                );
                with_margin(AnyView::new(track), &style)
            }
            Self::Spacer { class_name } => {
                let style = inherited.resolve(stylesheet, "spacer", None, class_name.as_deref());
                let grow = style
                    .grow
                    .unwrap_or(if style.width.is_some() { 0.0 } else { 1.0 });
                let mut spacer_style = style.clone();
                spacer_style.grow = Some(grow);
                let spacer = apply_container_style(Container::new(), &spacer_style);
                if let Some(margin) = style.margin {
                    AnyView::new(Container::new().padding(margin).grow(grow).child(spacer))
                } else {
                    AnyView::new(spacer)
                }
            }
            Self::Slot { id, class_name } => {
                let style = inherited.resolve(stylesheet, "slot", Some(id), class_name.as_deref());
                let mut container = Container::new().id(id.clone()).fill_width().fill_height();
                if let Some(content) = slots(id) {
                    container = container.child(content);
                }
                with_margin(
                    AnyView::new(apply_container_style(container, &style)),
                    &style,
                )
            }
            Self::Slider { .. } => self.view_slider(stylesheet, scope, inherited),
            Self::Switch {
                id,
                class_name,
                state,
                label,
                action,
            }
            | Self::Checkbox {
                id,
                class_name,
                state,
                label,
                action,
            } => {
                let checkbox = matches!(self, Self::Checkbox { .. });
                let kind = if checkbox { "checkbox" } else { "switch" };
                let state_classes = format!("{} {state}", class_name.as_deref().unwrap_or(""));
                let class_name = Some(state_classes);
                let style = inherited.resolve(stylesheet, kind, Some(id), class_name.as_deref());
                let on = matches!(state.as_str(), "on" | "mixed" | "disabled-on");
                let mixed = matches!(state.as_str(), "mixed" | "mixed-unavailable");
                let parts = inherited.extend(&style);
                let track_style =
                    parts.resolve(stylesheet, "switch-track", Some(id), class_name.as_deref());
                let thumb_style =
                    parts.resolve(stylesheet, "switch-thumb", Some(id), class_name.as_deref());
                let part = |kind: &str, style: &ControlStyle| {
                    apply_container_style(Container::new(), style).part_interaction_paints(
                        interaction_paints(stylesheet, kind, id, class_name.as_deref(), style),
                    )
                };
                let track = if checkbox {
                    let box_style =
                        parts.resolve(stylesheet, "checkbox-box", Some(id), class_name.as_deref());
                    let mark_style = parts.apply(parts.resolve(
                        stylesheet,
                        "checkbox-mark",
                        Some(id),
                        class_name.as_deref(),
                    ));
                    with_margin(
                        AnyView::new(
                            part("checkbox-box", &box_style).child(with_margin(
                                AnyView::new(
                                    part("checkbox-mark", &mark_style).child(
                                        styled_text(
                                            Text::new(if mixed {
                                                "−"
                                            } else if on {
                                                "✓"
                                            } else {
                                                ""
                                            })
                                            .color(0),
                                            &mark_style,
                                        )
                                        .part_interaction_text(interaction_paints(
                                            stylesheet,
                                            "checkbox-mark",
                                            id,
                                            class_name.as_deref(),
                                            &mark_style,
                                        )),
                                    ),
                                ),
                                &mark_style,
                            )),
                        ),
                        &box_style,
                    )
                } else {
                    with_margin(
                        AnyView::new(
                            part("switch-track", &track_style).child(
                                Row::new()
                                    .fill_width()
                                    .justify_content(if mixed {
                                        twinkle::Justify::Center
                                    } else if on {
                                        twinkle::Justify::End
                                    } else {
                                        twinkle::Justify::Start
                                    })
                                    .child(with_margin(
                                        AnyView::new(part("switch-thumb", &thumb_style)),
                                        &thumb_style,
                                    )),
                            ),
                        ),
                        &track_style,
                    )
                };
                let mut control =
                    apply_container_style(Container::new().automatic_focus_tint(false), &style)
                        .id(id.clone())
                        .semantic_role(if checkbox {
                            SemanticRole::Checkbox
                        } else {
                            SemanticRole::Switch
                        })
                        .accessibility_label(label.clone())
                        .accessibility_state(match state.as_str() {
                            "disabled-off" => "off disabled",
                            "disabled-on" => "on disabled",
                            "mixed-unavailable" => "mixed unavailable",
                            other => other,
                        })
                        .child(track);
                if let Some(action) = action {
                    control = control.message(Message::from_plugin_scoped(
                        PluginMessage::Click(*action),
                        scope,
                    ));
                    control = apply_control_interactions(
                        control,
                        &style,
                        stylesheet,
                        kind,
                        id,
                        class_name.as_deref(),
                    );
                }
                with_margin(AnyView::new(control), &style)
            }
            Self::Select { .. } => self.view_select(images, stylesheet, scope, inherited),
            Self::ColorSwatch { .. } => self.view_colorswatch(stylesheet, scope, inherited),
            Self::TextField {
                id,
                class_name,
                accessibility_label,
                value,
                placeholder,
                secure,
                auto_focus,
                action,
                focus_action,
                blur_action,
            } => {
                let style =
                    inherited.resolve(stylesheet, "text-field", Some(id), class_name.as_deref());
                let text_style = inherited.apply(style.clone());
                let focus_message = focus_action
                    .map(|action| Message::from_plugin_scoped(PluginMessage::Click(action), scope));
                let blur_message = blur_action
                    .map(|action| Message::from_plugin_scoped(PluginMessage::Click(action), scope));
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
                    .auto_focus(*auto_focus)
                    .id(id.clone())
                    .accessibility_label(accessibility_label)
                    .grow(1.0)
                    .wrap(false);
                if let Some(message) = focus_message {
                    field = field.focus_message(message);
                }
                if let Some(message) = blur_message {
                    field = field.blur_message(message);
                }
                if let Some(size) = text_style.font_size {
                    field = field.font_size(size);
                }
                if let Some(height) = text_style.line_height {
                    field = field.line_height(height);
                }
                if let Some(color) = text_style.color {
                    field = field.color(color);
                }
                let mut frame_style = dropdown_part(&text_style);
                frame_style.interaction_backgrounds = [
                    InteractionState::Hover,
                    InteractionState::Active,
                    InteractionState::Focus,
                ]
                .map(|state| {
                    stylesheet.resolve_interaction_background_with_properties(
                        "text-field",
                        Some(id),
                        class_name.as_deref(),
                        state,
                        &style.custom_properties,
                        &style.ancestors[..style.ancestors.len().saturating_sub(1)],
                    )
                });
                frame_style.interaction_paints = [
                    InteractionState::Hover,
                    InteractionState::Active,
                    InteractionState::Focus,
                ]
                .map(|state| {
                    stylesheet.resolve_interaction_paint(
                        "text-field",
                        Some(id),
                        class_name.as_deref(),
                        state,
                        &style.custom_properties,
                        &style.ancestors[..style.ancestors.len().saturating_sub(1)],
                    )
                });
                let parts = inherited.extend(&style);
                let caret = dropdown_part(&parts.resolve(
                    stylesheet,
                    "text-field-caret",
                    Some(id),
                    class_name.as_deref(),
                ));
                let selection = dropdown_part(&parts.resolve(
                    stylesheet,
                    "text-field-selection",
                    Some(id),
                    class_name.as_deref(),
                ));
                let menu_style = parts.resolve(
                    stylesheet,
                    "text-field-menu",
                    Some(id),
                    class_name.as_deref(),
                );
                let menu_inherited = parts.extend(&menu_style);
                let menu_parts = |disabled: bool| {
                    let classes = format!(
                        "{} {}",
                        class_name.as_deref().unwrap_or(""),
                        if disabled { "disabled" } else { "enabled" }
                    );
                    let item = menu_inherited.apply(menu_inherited.resolve(
                        stylesheet,
                        "text-field-menu-item",
                        Some(id),
                        Some(&classes),
                    ));
                    let child = menu_inherited.extend(&item);
                    let shortcut = child.apply(child.resolve(
                        stylesheet,
                        "text-field-menu-shortcut",
                        Some(id),
                        Some(&classes),
                    ));
                    let indicator = child.apply(child.resolve(
                        stylesheet,
                        "text-field-menu-indicator",
                        Some(id),
                        Some(&classes),
                    ));
                    let mut frame = dropdown_part(&item);
                    frame.interaction_backgrounds = [
                        InteractionState::Hover,
                        InteractionState::Active,
                        InteractionState::Focus,
                    ]
                    .map(|state| {
                        stylesheet.resolve_interaction_background_with_properties(
                            "text-field-menu-item",
                            Some(id),
                            Some(&classes),
                            state,
                            &item.custom_properties,
                            &menu_inherited.ancestors,
                        )
                    });
                    frame.interaction_paints = [
                        InteractionState::Hover,
                        InteractionState::Active,
                        InteractionState::Focus,
                    ]
                    .map(|state| {
                        stylesheet.resolve_interaction_paint(
                            "text-field-menu-item",
                            Some(id),
                            Some(&classes),
                            state,
                            &item.custom_properties,
                            &menu_inherited.ancestors,
                        )
                    });
                    let mut shortcut_part = dropdown_part(&shortcut);
                    shortcut_part.interaction_paints = interaction_paints(
                        stylesheet,
                        "text-field-menu-shortcut",
                        id,
                        Some(&classes),
                        &shortcut,
                    );
                    let mut indicator_part = dropdown_part(&indicator);
                    indicator_part.interaction_paints = interaction_paints(
                        stylesheet,
                        "text-field-menu-indicator",
                        id,
                        Some(&classes),
                        &indicator,
                    );
                    [frame, shortcut_part, indicator_part]
                };
                let menu = twinkle::OverlayMenuPresentation {
                    frame: dropdown_part(&menu_style),
                    item: menu_parts(false),
                    disabled_item: menu_parts(true),
                    row_gap: menu_style.gap.unwrap_or(0.0),
                };
                field = field
                    .context_menu_presentation(menu)
                    .presentation(frame_style, caret, selection)
                    .width_length(style.width.unwrap_or(Length::Auto))
                    .height_length(style.height.unwrap_or(Length::Auto))
                    .grow(style.grow.unwrap_or(0.0));
                if let Some(value) = style.min_width {
                    field = field.min_width(value);
                }
                if let Some(value) = style.max_width {
                    field = field.max_width(value);
                }
                if let Some(value) = style.min_height {
                    field = field.min_height(value);
                }
                if let Some(value) = style.max_height {
                    field = field.max_height(value);
                }
                if let Some(value) = style.shrink {
                    field = field.shrink(value);
                }
                with_margin(AnyView::new(field), &style)
            }
            Self::Button {
                id,
                class_name,
                label,
                description,
                accessibility_label,
                accessibility_state,
                disabled,
                width,
                height,
                icon,
                icon_size,
                icon_above,
                show_label,
                action,
                context_action,
                drag_action,
                drop_action,
                focus_action,
                blur_action,
            } => {
                let style =
                    inherited.resolve(stylesheet, "button", Some(id), class_name.as_deref());
                let text_style = inherited.apply(style.clone());
                let label_visual = || {
                    let title = styled_text(
                        Text::new(label)
                            .color(text_style.color.unwrap_or(0xfff4f6fa))
                            .wrap(true),
                        &text_style,
                    );
                    if let Some(description) = description {
                        let inherited = inherited.extend(&text_style);
                        let description_style = inherited.apply(inherited.resolve(
                            stylesheet,
                            "text",
                            None,
                            Some("button-description"),
                        ));
                        AnyView::new(
                            Column::new()
                                .gap(2.0)
                                .child(title)
                                .child(styled_text(Text::new(description), &description_style)),
                        )
                    } else {
                        AnyView::new(title)
                    }
                };
                let visual = icon
                    .as_ref()
                    .and_then(|asset| images.get(asset))
                    .map_or_else(label_visual, |(id, image)| {
                        let icon = if let Some(color) = style.icon_color {
                            AnyView::new(
                                Icon::new(*id, Arc::clone(image), color, *icon_size as f32)
                                    .decorative(),
                            )
                        } else {
                            AnyView::new(
                                Image::new(*id, Arc::clone(image))
                                    .width(*icon_size as f32)
                                    .height(*icon_size as f32),
                            )
                        };
                        if *show_label && *icon_above {
                            AnyView::new(
                                Column::new()
                                    .gap(6.0)
                                    .align_items(twinkle::Align::Center)
                                    .child(icon)
                                    .child(label_visual()),
                            )
                        } else if *show_label {
                            AnyView::new(Row::new().gap(8.0).child(icon).child(label_visual()))
                        } else {
                            AnyView::new(icon)
                        }
                    });
                let mut container = Container::new()
                    .id(id.clone())
                    .accessibility_label(accessibility_label)
                    .semantic_role(SemanticRole::Button);
                if let Some(height) = height {
                    container = container.height(*height as f32);
                }
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
                if let Some(action) = drop_action.filter(|_| !*disabled) {
                    container = container.on_drop((
                        Message::from_plugin_scoped(PluginMessage::Click(action), scope),
                        Message::drop,
                    ));
                }
                if let Some(action) = focus_action.filter(|_| !*disabled) {
                    container = container.focus_message(Message::from_plugin_scoped(
                        PluginMessage::Click(action),
                        scope,
                    ));
                }
                if let Some(action) = blur_action.filter(|_| !*disabled) {
                    container = container.blur_message(Message::from_plugin_scoped(
                        PluginMessage::Click(action),
                        scope,
                    ));
                }
                if !*disabled {
                    container = apply_control_interactions(
                        container,
                        &style,
                        stylesheet,
                        "button",
                        id,
                        class_name.as_deref(),
                    );
                } else {
                    container = container.automatic_focus_tint(false);
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

    pub fn window_title(&self) -> Option<&str> {
        match self {
            Self::Surface {
                window_request: Some(request),
                ..
            } => request.title.as_deref(),
            _ => None,
        }
    }

    pub fn window_focus_action(&self, focused: bool) -> Option<usize> {
        let Self::Surface {
            window_request: Some(_),
            focus_action,
            blur_action,
            ..
        } = self
        else {
            return None;
        };
        if focused { *focus_action } else { *blur_action }
    }

    pub fn window_shortcut_action(&self, shortcut: Shortcut) -> Option<usize> {
        let Self::Surface {
            window_request: Some(_),
            escape_action,
            submit_action,
            ..
        } = self
        else {
            return None;
        };
        match shortcut {
            Shortcut::Escape => *escape_action,
            Shortcut::Submit => *submit_action,
            _ => None,
        }
    }

    pub fn dialog(&self, requested_id: &str) -> Option<&Self> {
        match self {
            Self::Dialog { id, .. } if id == requested_id => Some(self),
            Self::Box { children, .. }
            | Self::Layer { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => {
                children.iter().find_map(|child| child.dialog(requested_id))
            }
            _ => None,
        }
    }

    pub fn dialog_content_view<Message: PluginUiMessage>(
        &self,
        requested_id: &str,
        images: &PluginImages,
        stylesheet: &StyleSheet,
        scope: Option<&str>,
    ) -> Option<AnyView<Message>> {
        let Self::Dialog {
            open: true,
            children,
            ..
        } = self.dialog(requested_id)?
        else {
            return None;
        };
        Some(AnyView::new(Column::new().children(children.iter().map(
            |child| child.view_as_scoped(images, stylesheet, scope),
        ))))
    }

    pub fn menu(&self, requested_id: &str) -> Option<&Self> {
        match self {
            Self::Menu { id, .. } if id == requested_id => Some(self),
            Self::Box { children, .. }
            | Self::Layer { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
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
            | Self::Layer { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
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
                collection: None,
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
            }
            | Self::Checkbox {
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
                        .find(|(id, _, _, _, _)| id == requested_id)
                        .map(|(_, _, action, _, _)| *action)
                }
            }
            Self::Image {
                id: Some(id),
                action: Some(action),
                ..
            } if id == requested_id => Some(*action),
            Self::Box { children, .. }
            | Self::Layer { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
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
            | Self::Layer { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
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
            | Self::Layer { children, .. }
            | Self::Div { children, .. }
            | Self::Surface { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::ScrollView { children, .. } => children
                .iter()
                .find_map(|child| child.slider_action(requested_id)),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NativeNodeId(String);

impl NativeNodeId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct HandlerSlotId(String);

impl HandlerSlotId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// One admitted presentation generation. Patch application is intentionally
/// not exposed yet: this type first makes typed tree, identity and event-slot
/// authority transactional under one owner.
#[derive(Clone, Debug)]
pub struct RetainedPanelTree {
    node: PanelNode,
    source: Value,
    generation: u64,
    nodes: BTreeMap<NativeNodeId, String>,
    handler_slots: BTreeMap<HandlerSlotId, usize>,
    sources: crate::virtual_source::VirtualSourceCatalog,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NativePatchApplyCounters {
    pub transport_bytes: usize,
    pub nodes_visited: u64,
    pub nodes_mutated: u64,
}

impl RetainedPanelTree {
    /// Preserve only native-captured focus ancestry during a synchronous data
    /// transaction. Nested owners may temporarily disappear while their outer
    /// keyed row is readmitted. This retains catalogs, never offscreen row trees.
    pub fn begin_virtual_target_repair(&mut self, targets: &[VirtualCollectionTarget]) {
        self.sources
            .set_repair_identities(targets.iter().map(|target| target.source_identity));
    }

    pub fn end_virtual_target_repair(&mut self) {
        self.sources.set_repair_identities(std::iter::empty());
        retain_collection_sources(&self.source, &mut self.sources);
    }

    pub fn admit(
        source: &Value,
        manifest: &impl SurfaceBoundsProvider,
        expected_surface_id: Option<&str>,
        generation: u64,
    ) -> Result<Self, String> {
        Self::admit_with_sources(
            source,
            manifest,
            expected_surface_id,
            generation,
            Default::default(),
        )
    }

    /// Re-admit a candidate using only this generation's source namespace. The
    /// accepted catalog remains unchanged if any source/tree invariant fails.
    pub fn readmit(
        &self,
        source: &Value,
        manifest: &impl SurfaceBoundsProvider,
        expected_surface_id: Option<&str>,
        generation: u64,
    ) -> Result<Self, String> {
        Self::admit_with_sources(
            source,
            manifest,
            expected_surface_id,
            generation,
            self.sources.clone(),
        )
    }

    pub fn readmit_validated(
        &self,
        source: &Value,
        manifest: &impl SurfaceBoundsProvider,
        expected_surface_id: Option<&str>,
        stylesheet: &StyleSheet,
        generation: u64,
    ) -> Result<Self, String> {
        let admitted = self.readmit(source, manifest, expected_surface_id, generation)?;
        if let Some(id) = expected_surface_id {
            let grant = manifest
                .surface_bounds()
                .iter()
                .find(|surface| surface.id == id)
                .ok_or("rendered surface is no longer declared")?;
            admitted.node.requested_surface(grant, stylesheet)?;
        }
        Ok(admitted)
    }

    fn admit_with_sources(
        source: &Value,
        manifest: &impl SurfaceBoundsProvider,
        expected_surface_id: Option<&str>,
        generation: u64,
        mut sources: crate::virtual_source::VirtualSourceCatalog,
    ) -> Result<Self, String> {
        retain_collection_sources(source, &mut sources);
        let node = parse_panel_for_manifest_with_sources(
            source,
            manifest,
            expected_surface_id,
            &mut sources,
        )?;
        let mut nodes = BTreeMap::new();
        let mut handler_slots = BTreeMap::new();
        fn index(
            value: &Value,
            nodes: &mut BTreeMap<NativeNodeId, String>,
            slots: &mut BTreeMap<HandlerSlotId, usize>,
        ) -> Result<(), String> {
            if let Some(array) = value.as_array() {
                for child in array {
                    index(child, nodes, slots)?;
                }
                return Ok(());
            }
            let Some(object) = value.as_object() else {
                return Ok(());
            };
            if let Some(id) = object.get("__nativeId").and_then(Value::as_str) {
                let id = NativeNodeId(id.to_owned());
                let kind = object
                    .get("kind")
                    .and_then(Value::as_str)
                    .ok_or("identified native node has no kind")?;
                if nodes.insert(id, kind.to_owned()).is_some() {
                    return Err("duplicate native node identity".into());
                }
                if let Some(bindings) = object.get("__handlerSlots") {
                    let bindings = bindings
                        .as_object()
                        .ok_or("native handler slots must be an object")?;
                    for (event, slot) in bindings {
                        let slot = HandlerSlotId(
                            slot.as_str()
                                .ok_or("native handler slot must be a string")?
                                .to_owned(),
                        );
                        let action = object
                            .get(event)
                            .and_then(Value::as_u64)
                            .and_then(|action| usize::try_from(action).ok())
                            .ok_or("native handler slot has no bounded action")?;
                        if slots.insert(slot, action).is_some() {
                            return Err("duplicate native handler slot".into());
                        }
                    }
                }
            }
            if let Some(children) = object.get("children") {
                index(children, nodes, slots)?;
            }
            Ok(())
        }
        index(source, &mut nodes, &mut handler_slots)?;
        Ok(Self {
            node,
            source: source.clone(),
            generation,
            nodes,
            handler_slots,
            sources,
        })
    }

    pub fn node(&self) -> &PanelNode {
        &self.node
    }

    /// Constant-time catalog accounting: (sources, logical rows, payload bytes).
    /// Payload bytes are the admission budget, not allocator capacity or RSS.
    pub fn virtual_source_usage(&self) -> (usize, usize, usize) {
        (
            self.sources.source_count(),
            self.sources.logical_rows(),
            self.sources.payload_bytes(),
        )
    }

    /// Commit a native measurement batch without advancing the JSX patch
    /// generation or changing callback/key authority. Callers must rebuild the
    /// native frame when this returns true and reconcile its scroll anchor.
    /// All catalogs and typed collection handles remain unchanged on failure.
    pub fn apply_virtual_measurements(
        &mut self,
        measurements: &[VirtualCollectionMeasurements],
        scale: f32,
        layout_revision: u64,
    ) -> Result<bool, String> {
        if measurements.is_empty() {
            return Ok(false);
        }
        fn collect_ranges(node: &PanelNode, ranges: &mut BTreeMap<u64, std::ops::Range<usize>>) {
            if let PanelNode::Div {
                collection: Some(collection),
                ..
            } = node
                && let Some(source) = &collection.source
            {
                ranges.insert(source.revision(), collection.range.clone());
            }
            if let Some(children) = node.container_children() {
                for child in children {
                    collect_ranges(child, ranges);
                }
            }
        }
        let mut ranges = BTreeMap::new();
        collect_ranges(&self.node, &mut ranges);
        let mut sources = self.sources.clone();
        let mut replacements = BTreeMap::new();
        for batch in measurements {
            if replacements.contains_key(&batch.source_revision) {
                return Err("duplicate virtual measurement source".into());
            }
            let range = ranges
                .get(&batch.source_revision)
                .ok_or("retired virtual measurement source")?;
            if batch.rows.len() != range.len()
                || batch.rows.iter().map(|(row, _)| *row).ne(range.clone())
            {
                return Err("virtual measurements do not match materialized range".into());
            }
            let owner = sources
                .owner_for_revision(batch.source_revision)
                .ok_or("retired virtual measurement source")?
                .to_owned();
            let (source, _) = sources.measure(
                &owner,
                batch.source_revision,
                crate::virtual_source::VirtualMeasurementContext {
                    width: batch.width,
                    scale,
                    layout_revision,
                },
                &batch.rows,
            )?;
            replacements.insert(batch.source_revision, source);
        }
        fn replace(
            node: &mut PanelNode,
            replacements: &BTreeMap<u64, Arc<crate::virtual_source::VirtualSource>>,
        ) -> bool {
            let mut changed = false;
            if let PanelNode::Div {
                collection: Some(collection),
                ..
            } = node
                && let Some(previous) = &collection.source
                && let Some(source) = replacements.get(&previous.revision())
            {
                let geometry = source.geometry_handle();
                changed |= !Arc::ptr_eq(&collection.heights, &geometry);
                collection.heights = geometry;
                collection.source = Some(Arc::clone(source));
            }
            match node {
                PanelNode::Box { children, .. }
                | PanelNode::Layer { children, .. }
                | PanelNode::Div { children, .. }
                | PanelNode::Surface { children, .. }
                | PanelNode::Row { children, .. }
                | PanelNode::Column { children, .. }
                | PanelNode::ScrollView { children, .. } => {
                    for child in children {
                        changed |= replace(child, replacements);
                    }
                }
                _ => {}
            }
            changed
        }
        let changed = replace(&mut self.node, &replacements);
        self.sources = sources;
        Ok(changed)
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }
    pub fn nodes(&self) -> &BTreeMap<NativeNodeId, String> {
        &self.nodes
    }
    pub fn handler_slots(&self) -> &BTreeMap<HandlerSlotId, usize> {
        &self.handler_slots
    }
    pub fn into_node(self) -> PanelNode {
        self.node
    }

    /// Cold-oracle scaffold for a future transactional subtree patch. It
    /// deliberately revalidates the whole candidate today and does not mutate
    /// the accepted generation.
    pub fn validate_candidate(
        &self,
        candidate: &Value,
        manifest: &impl SurfaceBoundsProvider,
        expected_surface_id: Option<&str>,
        stylesheet: &StyleSheet,
    ) -> Result<PanelNode, String> {
        let node = parse_panel_for_manifest_with_sources(
            candidate,
            manifest,
            expected_surface_id,
            &mut self.sources.clone(),
        )?;
        if let Some(surface_id) = expected_surface_id {
            let grant = manifest
                .surface_bounds()
                .iter()
                .find(|surface| surface.id == surface_id)
                .ok_or("rendered surface is no longer declared")?;
            node.requested_surface(grant, stylesheet)?;
        }
        Ok(node)
    }

    pub fn source(&self) -> &Value {
        &self.source
    }

    /// Apply a bounded native delta to clones of the accepted source and typed
    /// presentation. The receiver is changed only after every operation and
    /// root/style invariant succeeds.
    pub fn apply_patch(
        &mut self,
        patch: &NativePatchEnvelope,
        manifest: &impl SurfaceBoundsProvider,
        expected_surface_id: Option<&str>,
        stylesheet: &StyleSheet,
        generation: u64,
        transport_bytes: usize,
    ) -> Result<NativePatchApplyCounters, String> {
        if patch.version != 1 || patch.operations.len() > 256 {
            return Err("unsupported or oversized native patch envelope".into());
        }
        let mut source = self.source.clone();
        let mut node = self.node.clone();
        let sources = std::cell::RefCell::new(self.sources.clone());
        let mut visited = 0_u64;
        for operation in &patch.operations {
            // A fragment retains all source roots, but `node` represents only
            // this host window. Never pair a sibling's source with that node.
            if let Some(roots) = source.as_array() {
                let selected = roots
                    .iter()
                    .find(|root| root.get("id").and_then(Value::as_str) == expected_surface_id)
                    .ok_or("native patch fragment has no selected host window")?;
                let target = match operation {
                    NativePatchOperation::SetPrimitive { target, .. }
                    | NativePatchOperation::ReplaceSubtree { target, .. } => target.clone(),
                    NativePatchOperation::InsertChild { parent, .. }
                    | NativePatchOperation::RemoveChild { parent, .. }
                    | NativePatchOperation::MoveChild { parent, .. } => parent.clone(),
                    NativePatchOperation::ReplaceHandlerSlot { slot, .. } => {
                        find_handler_slot(selected, slot)
                            .ok_or("native patch handler is outside the host window")?
                            .0
                    }
                };
                if selected.get("__nativeId").and_then(Value::as_str) != Some(target.as_str())
                    && !source_contains_id(selected, &target)
                {
                    return Err("native patch target is outside the host window".into());
                }
            }
            match operation {
                NativePatchOperation::SetPrimitive {
                    target,
                    property,
                    value,
                } => {
                    if !is_primitive_patch_value(value, property) {
                        return Err("setPrimitive contains a non-primitive value".into());
                    }
                    if mutate_flattened_target(
                        &mut source,
                        &mut node,
                        target,
                        &sources,
                        &mut |source| {
                            source
                                .as_object_mut()
                                .ok_or("native patch target is not an object")?
                                .insert(property.clone(), value.clone());
                            Ok(())
                        },
                    )? {
                        continue;
                    }
                    mutate_target(
                        &mut source,
                        &mut node,
                        target,
                        &mut visited,
                        &sources,
                        &mut |source, typed| {
                            let object = source
                                .as_object_mut()
                                .ok_or("native patch target is not an object")?;
                            object.insert(property.clone(), value.clone());
                            if property == "className" && typed.container_children().is_some() {
                                set_container_class_name(typed, value)?;
                            } else if matches!(typed, PanelNode::Box { .. })
                                && matches!(
                                    property.as_str(),
                                    "x" | "y" | "width" | "height" | "background" | "radius"
                                )
                            {
                                update_box_scalars(typed, source)?;
                            } else if matches!(typed, PanelNode::Surface { .. }) {
                                update_surface_scalars(typed, source)?;
                            } else {
                                if typed.container_children().is_some() {
                                    return Err(format!(
                                        "setPrimitive property {property:?} requires subtree replacement"
                                    ));
                                }
                                *typed = parse_patched_node(source, typed, &sources)?;
                            }
                            Ok(())
                        },
                    )?;
                }
                NativePatchOperation::ReplaceHandlerSlot { slot, action } => {
                    let expected = self
                        .handler_slots
                        .get(&HandlerSlotId(slot.clone()))
                        .ok_or_else(|| {
                            format!("native patch targets unknown handler slot {slot:?}")
                        })?;
                    let _ = expected;
                    let (target, event) = find_handler_slot(&source, slot)
                        .ok_or_else(|| format!("native handler slot {slot:?} disappeared"))?;
                    mutate_target(
                        &mut source,
                        &mut node,
                        &target,
                        &mut visited,
                        &sources,
                        &mut |source, typed| {
                            let implicit_id = source.get("id").is_none();
                            source
                                .as_object_mut()
                                .ok_or("native handler owner is not an object")?
                                .insert(event.clone(), Value::from(*action as u64));
                            set_typed_handler(typed, &event, *action, implicit_id)
                        },
                    )?;
                }
                NativePatchOperation::InsertChild {
                    parent,
                    key,
                    child_id,
                    index,
                    node: child,
                } => {
                    validate_keyed_child(child, key, child_id)?;
                    if index_native_source(&source)?
                        .0
                        .contains_key(&NativeNodeId(child_id.clone()))
                    {
                        return Err(format!("inserted native child {child_id:?} already exists"));
                    }
                    let typed_child =
                        PanelNode::parse_with_sources(child, &mut sources.borrow_mut())?;
                    mutate_target(
                        &mut source,
                        &mut node,
                        parent,
                        &mut visited,
                        &sources,
                        &mut |source, typed| {
                            let children = source
                                .get_mut("children")
                                .and_then(Value::as_array_mut)
                                .ok_or("insert parent has no source children")?;
                            let typed_children = typed_children_mut(typed)?;
                            if children.len() != typed_children.len() || *index > children.len() {
                                return Err(
                                    "insert child index or typed alignment is invalid".into()
                                );
                            }
                            children.insert(*index, child.clone());
                            typed_children.insert(*index, typed_child.clone());
                            Ok(())
                        },
                    )?;
                }
                NativePatchOperation::RemoveChild {
                    parent,
                    key,
                    child_id,
                    index,
                } => {
                    mutate_target(
                        &mut source,
                        &mut node,
                        parent,
                        &mut visited,
                        &sources,
                        &mut |source, typed| {
                            let children = source
                                .get_mut("children")
                                .and_then(Value::as_array_mut)
                                .ok_or("remove parent has no source children")?;
                            let typed_children = typed_children_mut(typed)?;
                            if children.len() != typed_children.len()
                                || *index >= children.len()
                                || keyed_identity(&children[*index])
                                    != Some((key.as_str(), child_id.as_str()))
                            {
                                return Err(
                                    "remove child parent, index, key, or identity is stale".into(),
                                );
                            }
                            children.remove(*index);
                            typed_children.remove(*index);
                            Ok(())
                        },
                    )?;
                }
                NativePatchOperation::MoveChild {
                    parent,
                    key,
                    child_id,
                    from,
                    to,
                } => {
                    mutate_target(
                        &mut source,
                        &mut node,
                        parent,
                        &mut visited,
                        &sources,
                        &mut |source, typed| {
                            let children = source
                                .get_mut("children")
                                .and_then(Value::as_array_mut)
                                .ok_or("move parent has no source children")?;
                            let typed_children = typed_children_mut(typed)?;
                            if children.len() != typed_children.len()
                                || *from >= children.len()
                                || *to >= children.len()
                                || keyed_identity(&children[*from])
                                    != Some((key.as_str(), child_id.as_str()))
                            {
                                return Err(
                                    "move child parent, index, key, or identity is stale".into()
                                );
                            }
                            let child = children.remove(*from);
                            children.insert(*to, child);
                            let child = typed_children.remove(*from);
                            typed_children.insert(*to, child);
                            Ok(())
                        },
                    )?;
                }
                NativePatchOperation::ReplaceSubtree {
                    target,
                    node: replacement,
                } => {
                    if !self.nodes.contains_key(&NativeNodeId(target.clone())) {
                        return Err(format!("native patch targets unknown node {target:?}"));
                    }
                    let replacement_id = replacement
                        .get("__nativeId")
                        .and_then(Value::as_str)
                        .ok_or("replacement subtree has no native identity")?;
                    if replacement_id != target {
                        return Err("replacement subtree identity differs from its target".into());
                    }
                    if target == "root" {
                        let typed = parse_panel_for_manifest_with_sources(
                            replacement,
                            manifest,
                            expected_surface_id,
                            &mut sources.borrow_mut(),
                        )?;
                        source = replacement.clone();
                        node = typed;
                        visited = visited.saturating_add(1);
                    } else {
                        let typed = if let Some(previous) = find_typed_node(&source, &node, target)
                        {
                            parse_patched_node(replacement, previous, &sources)?
                        } else {
                            PanelNode::parse_with_sources(replacement, &mut sources.borrow_mut())?
                        };
                        replace_source_and_typed(
                            &mut source,
                            &mut node,
                            target,
                            replacement,
                            &typed,
                            &mut visited,
                            &sources,
                        )?;
                    }
                }
            }
        }
        // Re-admission builds authoritative identity/handler indexes and
        // validates manifest/root constraints, but retain the incrementally
        // mutated typed tree rather than reparsing the complete candidate.
        let (nodes, handler_slots) = index_native_source_checked(&source, true)?;
        if let Some(surface_id) = expected_surface_id {
            let grant = manifest
                .surface_bounds()
                .iter()
                .find(|surface| surface.id == surface_id)
                .ok_or("rendered surface is no longer declared")?;
            node.requested_surface(grant, stylesheet)?;
        }
        let mut sources = sources.into_inner();
        retain_collection_sources(&source, &mut sources);
        *self = Self {
            node,
            source,
            generation,
            nodes,
            handler_slots,
            sources,
        };
        Ok(NativePatchApplyCounters {
            transport_bytes,
            nodes_visited: visited,
            nodes_mutated: patch.operations.len() as u64,
        })
    }
}

fn retain_collection_sources(
    source: &Value,
    sources: &mut crate::virtual_source::VirtualSourceCatalog,
) {
    if sources.source_count() == 0 {
        return;
    }
    fn owners<'a>(value: &'a Value, result: &mut std::collections::HashSet<&'a str>) {
        if let Some(values) = value.as_array() {
            for value in values {
                owners(value, result);
            }
        } else if let Some(object) = value.as_object() {
            if object.get("kind").and_then(Value::as_str) == Some("div")
                && object.get("collection").is_some_and(|collection| {
                    collection.get("keys").is_some() || collection.get("source").is_some()
                })
                && let Some(id) = object.get("__nativeId").and_then(Value::as_str)
            {
                result.insert(id);
            }
            if let Some(children) = object.get("children") {
                owners(children, result);
            }
        }
    }
    let mut retained_owners = std::collections::HashSet::new();
    owners(source, &mut retained_owners);
    sources.retain_owners(&retained_owners);
}

/// Reparse only a Surface's bounded scalar declaration. Its already-admitted
/// children stay retained, so a root geometry or accessibility update does not
/// turn into a subtree replacement or walk.
fn update_box_scalars(node: &mut PanelNode, source: &Value) -> Result<(), String> {
    // Validate with the production parser without reparsing retained descendants.
    let mut shallow = source
        .as_object()
        .ok_or("patched box is not an object")?
        .iter()
        .filter(|(key, _)| key.as_str() != "children")
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect::<serde_json::Map<String, Value>>();
    shallow.insert("children".into(), Value::Array(Vec::new()));
    let PanelNode::Box {
        x,
        y,
        width,
        height,
        background,
        radius,
        ..
    } = PanelNode::parse(&Value::Object(shallow))?
    else {
        return Err("patched box no longer declares a box".into());
    };
    let PanelNode::Box {
        x: current_x,
        y: current_y,
        width: current_width,
        height: current_height,
        background: current_background,
        radius: current_radius,
        ..
    } = node
    else {
        unreachable!()
    };
    *current_x = x;
    *current_y = y;
    *current_width = width;
    *current_height = height;
    *current_background = background;
    *current_radius = radius;
    Ok(())
}

fn update_surface_scalars(node: &mut PanelNode, source: &Value) -> Result<(), String> {
    let mut shallow = source.clone();
    let object = shallow
        .as_object_mut()
        .ok_or("patched surface is not an object")?;
    object.insert("children".into(), Value::Array(Vec::new()));
    if object.get("id").is_none()
        && let PanelNode::Surface { id: Some(id), .. } = node
    {
        object.insert("id".into(), Value::String(id.clone()));
    }
    let PanelNode::Surface {
        id,
        window_request,
        accessibility_label,
        escape_action,
        submit_action,
        focus_action,
        blur_action,
        class_name,
        background,
        width,
        height,
        ..
    } = PanelNode::parse(&shallow)?
    else {
        return Err("patched surface no longer declares a surface root".into());
    };
    let PanelNode::Surface {
        id: current_id,
        window_request: current_window_request,
        accessibility_label: current_accessibility_label,
        escape_action: current_escape_action,
        submit_action: current_submit_action,
        focus_action: current_focus_action,
        blur_action: current_blur_action,
        class_name: current_class_name,
        background: current_background,
        width: current_width,
        height: current_height,
        ..
    } = node
    else {
        return Err("surface scalar target is not a surface".into());
    };
    *current_id = id;
    *current_window_request = window_request;
    *current_accessibility_label = accessibility_label;
    *current_escape_action = escape_action;
    *current_submit_action = submit_action;
    *current_focus_action = focus_action;
    *current_blur_action = blur_action;
    *current_class_name = class_name;
    *current_background = background;
    *current_width = width;
    *current_height = height;
    Ok(())
}

type ParseSources = std::cell::RefCell<crate::virtual_source::VirtualSourceCatalog>;

fn parse_patched_node(
    source: &Value,
    previous: &PanelNode,
    sources: &ParseSources,
) -> Result<PanelNode, String> {
    let mut source = source.clone();
    if source.get("id").is_none() {
        let id = match previous {
            PanelNode::Slider { id, .. }
            | PanelNode::Switch { id, .. }
            | PanelNode::Checkbox { id, .. }
            | PanelNode::ColorSwatch { id, .. }
            | PanelNode::Select { id, .. }
            | PanelNode::Slot { id, .. }
            | PanelNode::TextField { id, .. }
            | PanelNode::Button { id, .. }
            | PanelNode::ScrollView { id, .. }
            | PanelNode::Menu { id, .. }
            | PanelNode::MenuItem { id, .. } => Some(id.as_str()),
            PanelNode::Surface { id, .. } | PanelNode::Image { id, .. } => id.as_deref(),
            _ => None,
        };
        if let Some(id) = id {
            source
                .as_object_mut()
                .ok_or("patched native node is not an object")?
                .insert("id".into(), Value::String(id.to_owned()));
        }
    }
    PanelNode::parse_with_sources(&source, &mut sources.borrow_mut())
}

fn find_typed_node<'a>(
    source: &Value,
    typed: &'a PanelNode,
    target: &str,
) -> Option<&'a PanelNode> {
    if source.get("__nativeId").and_then(Value::as_str) == Some(target) {
        return Some(typed);
    }
    let source_children = source.get("children")?.as_array()?;
    let typed_children = typed.container_children()?;
    if source_children.len() != typed_children.len() {
        return None;
    }
    source_children
        .iter()
        .zip(typed_children)
        .find_map(|(source, typed)| find_typed_node(source, typed, target))
}

fn keyed_identity(value: &Value) -> Option<(&str, &str)> {
    Some((
        value.get("key")?.as_str()?,
        value.get("__nativeId")?.as_str()?,
    ))
}

fn validate_keyed_child(value: &Value, key: &str, child_id: &str) -> Result<(), String> {
    if keyed_identity(value) != Some((key, child_id)) {
        return Err("inserted child key or native identity differs from its operation".into());
    }
    Ok(())
}

fn is_primitive_patch_value(value: &Value, property: &str) -> bool {
    value.is_null()
        || value.is_boolean()
        || value.is_number()
        || value.is_string()
        || (property == "children"
            && value.as_array().is_some_and(|values| {
                values
                    .iter()
                    .all(|v| v.is_null() || v.is_string() || v.is_number())
            }))
}

fn set_container_class_name(node: &mut PanelNode, value: &Value) -> Result<(), String> {
    let class = match value {
        Value::Null => None,
        Value::String(value)
            if value.len() <= 256
                && value.split_ascii_whitespace().all(|name| {
                    !name.is_empty()
                        && name.len() <= 64
                        && name
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
                }) =>
        {
            Some(value.clone())
        }
        _ => return Err("className must contain bounded class identifiers".into()),
    };
    match node {
        PanelNode::Box { class_name, .. }
        | PanelNode::Layer { class_name, .. }
        | PanelNode::Div { class_name, .. }
        | PanelNode::Surface { class_name, .. }
        | PanelNode::Row { class_name, .. }
        | PanelNode::Column { class_name, .. }
        | PanelNode::ScrollView { class_name, .. }
        | PanelNode::Menu { class_name, .. }
        | PanelNode::MenuItem { class_name, .. } => {
            *class_name = class;
            Ok(())
        }
        _ => Err("className target is not a container".into()),
    }
}

fn set_typed_handler(
    node: &mut PanelNode,
    event: &str,
    action: usize,
    implicit_id: bool,
) -> Result<(), String> {
    let optional = Some(action);
    match (&mut *node, event) {
        (PanelNode::Div { action: field, .. }, "action")
        | (PanelNode::Image { action: field, .. }, "action")
        | (PanelNode::Switch { action: field, .. }, "action")
        | (PanelNode::Checkbox { action: field, .. }, "action")
        | (PanelNode::MenuItem { action: field, .. }, "action") => *field = optional,
        (
            PanelNode::Div {
                drop_action: field, ..
            },
            "dropAction",
        )
        | (
            PanelNode::Button {
                drop_action: field, ..
            },
            "dropAction",
        ) => *field = optional,
        (
            PanelNode::Surface {
                escape_action: field,
                ..
            },
            "escapeAction",
        ) => *field = optional,
        (
            PanelNode::Surface {
                submit_action: field,
                ..
            },
            "submitAction",
        ) => *field = optional,
        (
            PanelNode::Surface {
                focus_action: field,
                ..
            },
            "focusAction",
        )
        | (
            PanelNode::TextField {
                focus_action: field,
                ..
            },
            "focusAction",
        )
        | (
            PanelNode::Button {
                focus_action: field,
                ..
            },
            "focusAction",
        ) => *field = optional,
        (
            PanelNode::Surface {
                blur_action: field, ..
            },
            "blurAction",
        )
        | (
            PanelNode::TextField {
                blur_action: field, ..
            },
            "blurAction",
        )
        | (
            PanelNode::Button {
                blur_action: field, ..
            },
            "blurAction",
        ) => *field = optional,
        (
            PanelNode::Image {
                context_action: field,
                ..
            },
            "contextAction",
        )
        | (
            PanelNode::Button {
                context_action: field,
                ..
            },
            "contextAction",
        ) => *field = optional,
        (
            PanelNode::Button {
                drag_action: field, ..
            },
            "dragAction",
        ) => *field = optional,
        (
            PanelNode::Dialog {
                close_action: field,
                ..
            },
            "closeAction",
        ) => *field = optional,
        (
            PanelNode::Slider {
                drag_action: field, ..
            },
            "dragAction",
        ) => *field = optional,
        (PanelNode::Slider { action: field, .. }, "action")
        | (PanelNode::ColorSwatch { action: field, .. }, "action")
        | (PanelNode::Select { action: field, .. }, "action")
        | (PanelNode::TextField { action: field, .. }, "action")
        | (PanelNode::Button { action: field, .. }, "action") => *field = action,
        _ => {
            return Err(format!(
                "handler event {event:?} is invalid for its typed node"
            ));
        }
    }
    if implicit_id
        && event == "action"
        && let PanelNode::Button { id, .. } = node
    {
        *id = format!("plugin-button-{action}");
    }
    Ok(())
}

fn find_handler_slot(value: &Value, requested: &str) -> Option<(String, String)> {
    if let Some(object) = value.as_object() {
        if let (Some(id), Some(slots)) = (
            object.get("__nativeId").and_then(Value::as_str),
            object.get("__handlerSlots").and_then(Value::as_object),
        ) && let Some((event, _)) = slots
            .iter()
            .find(|(_, slot)| slot.as_str() == Some(requested))
        {
            return Some((id.to_owned(), event.clone()));
        }
        if let Some(found) = object
            .get("children")
            .and_then(Value::as_array)
            .and_then(|children| {
                children
                    .iter()
                    .find_map(|child| find_handler_slot(child, requested))
            })
        {
            return Some(found);
        }
    }
    value.as_array().and_then(|values| {
        values
            .iter()
            .find_map(|child| find_handler_slot(child, requested))
    })
}

fn mutate_source_only(
    source: &mut Value,
    target: &str,
    sources: &ParseSources,
    operation: &mut impl FnMut(&mut Value, &mut PanelNode) -> Result<(), String>,
) -> Result<(), String> {
    if source.get("__nativeId").and_then(Value::as_str) == Some(target) {
        let mut typed = PanelNode::parse_with_sources(source, &mut sources.borrow_mut())?;
        return operation(source, &mut typed);
    }
    let children = source
        .get_mut("children")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("native patch target {target:?} disappeared"))?;
    for child in children {
        if child.get("__nativeId").and_then(Value::as_str) == Some(target)
            || source_contains_id(child, target)
        {
            return mutate_source_only(child, target, sources, operation);
        }
    }
    Err(format!("native patch target {target:?} disappeared"))
}

fn mutate_target(
    source: &mut Value,
    typed: &mut PanelNode,
    target: &str,
    visited: &mut u64,
    sources: &ParseSources,
    operation: &mut impl FnMut(&mut Value, &mut PanelNode) -> Result<(), String>,
) -> Result<(), String> {
    *visited = visited.saturating_add(1);
    if source.get("__nativeId").and_then(Value::as_str) == Some(target) {
        return operation(source, typed);
    }
    if let Some(roots) = source.as_array_mut() {
        let root = roots
            .iter_mut()
            .find(|root| {
                root.get("__nativeId").and_then(Value::as_str) == Some(target)
                    || source_contains_id(root, target)
            })
            .ok_or_else(|| format!("native patch target {target:?} disappeared"))?;
        return mutate_target(root, typed, target, visited, sources, operation);
    }
    let children = source
        .get_mut("children")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("native patch target {target:?} is not below a container"))?;
    let typed_children = typed_children_mut(typed)?;
    if children.len() != typed_children.len() || children.iter().any(|child| !child.is_object()) {
        // Flow containers coalesce runs of primitive JSX children into typed
        // Text nodes. Their source and typed child indices intentionally do
        // not align. Apply the bounded mutation within this source subtree and
        // reparse only the nearest misaligned ancestor instead of rejecting a
        // valid retained update (for example a clock label beside elements).
        mutate_source_only(source, target, sources, operation)?;
        *typed = parse_patched_node(source, typed, sources)?;
        return Ok(());
    }
    for (source, typed) in children.iter_mut().zip(typed_children) {
        if source.get("__nativeId").and_then(Value::as_str) == Some(target)
            || source_contains_id(source, target)
        {
            return mutate_target(source, typed, target, visited, sources, operation);
        }
    }
    Err(format!("native patch target {target:?} disappeared"))
}

fn mutate_flattened_target(
    source: &mut Value,
    typed: &mut PanelNode,
    target: &str,
    sources: &ParseSources,
    operation: &mut impl FnMut(&mut Value) -> Result<(), String>,
) -> Result<bool, String> {
    if typed_children_mut(typed).is_ok() || !source_contains_id(source, target) {
        return Ok(false);
    }
    fn mutate(
        source: &mut Value,
        target: &str,
        operation: &mut impl FnMut(&mut Value) -> Result<(), String>,
    ) -> Result<(), String> {
        if source.get("__nativeId").and_then(Value::as_str) == Some(target) {
            return operation(source);
        }
        let children = source
            .get_mut("children")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| format!("native patch target {target:?} disappeared"))?;
        for child in children {
            if child.get("__nativeId").and_then(Value::as_str) == Some(target)
                || source_contains_id(child, target)
            {
                return mutate(child, target, operation);
            }
        }
        Err(format!("native patch target {target:?} disappeared"))
    }
    mutate(source, target, operation)?;
    *typed = PanelNode::parse_with_sources(source, &mut sources.borrow_mut())?;
    Ok(true)
}

fn typed_children_mut(node: &mut PanelNode) -> Result<&mut Vec<PanelNode>, String> {
    match node {
        PanelNode::Box { children, .. }
        | PanelNode::Layer { children, .. }
        | PanelNode::Div { children, .. }
        | PanelNode::Surface { children, .. }
        | PanelNode::Row { children, .. }
        | PanelNode::Column { children, .. }
        | PanelNode::ScrollView { children, .. }
        | PanelNode::Dialog { children, .. }
        | PanelNode::Menu {
            items: children, ..
        }
        | PanelNode::MenuItem { children, .. } => Ok(children),
        _ => Err("native patch target is not below a typed container".into()),
    }
}

type NativeSourceIndex = (
    BTreeMap<NativeNodeId, String>,
    BTreeMap<HandlerSlotId, usize>,
);

fn index_native_source(source: &Value) -> Result<NativeSourceIndex, String> {
    index_native_source_checked(source, false)
}

fn index_native_source_checked(
    source: &Value,
    validate_collections: bool,
) -> Result<NativeSourceIndex, String> {
    fn visit(
        value: &Value,
        nodes: &mut BTreeMap<NativeNodeId, String>,
        slots: &mut BTreeMap<HandlerSlotId, usize>,
        validate_collections: bool,
    ) -> Result<(), String> {
        if let Some(values) = value.as_array() {
            for value in values {
                visit(value, nodes, slots, validate_collections)?;
            }
            return Ok(());
        }
        let Some(object) = value.as_object() else {
            return Ok(());
        };
        // Structural deltas can change row count without reparsing their
        // retained parent. Check the final transaction during the existing
        // index walk, not intermediate remove/insert states or the full source
        // height array.
        if validate_collections
            && object.get("kind").and_then(Value::as_str) == Some("div")
            && let Some(collection) = object.get("collection")
        {
            let start = collection.get("start").map_or(Some(0), Value::as_u64);
            let end = collection.get("end").map_or(Some(0), Value::as_u64);
            let count = object
                .get("children")
                .and_then(Value::as_array)
                .map(Vec::len);
            let range_len = start
                .zip(end)
                .and_then(|(start, end)| end.checked_sub(start));
            if !matches!((range_len, count), (Some(length), Some(count)) if length == count as u64)
            {
                return Err("virtual collection children differ from its range".into());
            }
        }
        if let Some(id) = object.get("__nativeId").and_then(Value::as_str) {
            let kind = object
                .get("kind")
                .and_then(Value::as_str)
                .ok_or("identified native node has no kind")?;
            if nodes.insert(NativeNodeId(id.into()), kind.into()).is_some() {
                return Err("duplicate native node identity".into());
            }
            if let Some(bindings) = object.get("__handlerSlots") {
                for (event, slot) in bindings
                    .as_object()
                    .ok_or("native handler slots must be an object")?
                {
                    let slot = HandlerSlotId(
                        slot.as_str()
                            .ok_or("native handler slot must be a string")?
                            .into(),
                    );
                    let action = object
                        .get(event)
                        .and_then(Value::as_u64)
                        .and_then(|value| usize::try_from(value).ok())
                        .ok_or("native handler slot has no bounded action")?;
                    if slots.insert(slot, action).is_some() {
                        return Err("duplicate native handler slot".into());
                    }
                }
            }
        }
        if let Some(children) = object.get("children") {
            visit(children, nodes, slots, validate_collections)?;
        }
        Ok(())
    }
    let mut nodes = BTreeMap::new();
    let mut slots = BTreeMap::new();
    visit(source, &mut nodes, &mut slots, validate_collections)?;
    Ok((nodes, slots))
}

fn replace_source_and_typed(
    source: &mut Value,
    typed: &mut PanelNode,
    target: &str,
    replacement: &Value,
    replacement_typed: &PanelNode,
    visited: &mut u64,
    sources: &ParseSources,
) -> Result<(), String> {
    fn replace_source_only(
        source: &mut Value,
        target: &str,
        replacement: &Value,
    ) -> Result<(), String> {
        if source.get("__nativeId").and_then(Value::as_str) == Some(target) {
            *source = replacement.clone();
            return Ok(());
        }
        let children = source
            .get_mut("children")
            .and_then(Value::as_array_mut)
            .ok_or_else(|| format!("native patch target {target:?} disappeared"))?;
        for child in children {
            if child.get("__nativeId").and_then(Value::as_str) == Some(target)
                || source_contains_id(child, target)
            {
                return replace_source_only(child, target, replacement);
            }
        }
        Err(format!("native patch target {target:?} disappeared"))
    }

    *visited = visited.saturating_add(1);
    if source.get("__nativeId").and_then(Value::as_str) == Some(target) {
        *source = replacement.clone();
        *typed = replacement_typed.clone();
        return Ok(());
    }
    if let Some(roots) = source.as_array_mut() {
        let root = roots
            .iter_mut()
            .find(|root| {
                root.get("__nativeId").and_then(Value::as_str) == Some(target)
                    || source_contains_id(root, target)
            })
            .ok_or_else(|| format!("native patch target {target:?} disappeared"))?;
        return replace_source_and_typed(
            root,
            typed,
            target,
            replacement,
            replacement_typed,
            visited,
            sources,
        );
    }
    let source_children = source
        .get_mut("children")
        .and_then(Value::as_array_mut)
        .ok_or_else(|| format!("native patch target {target:?} is not below its indexed node"))?;
    let typed_children = match typed {
        PanelNode::Box { children, .. }
        | PanelNode::Layer { children, .. }
        | PanelNode::Div { children, .. }
        | PanelNode::Surface { children, .. }
        | PanelNode::Row { children, .. }
        | PanelNode::Column { children, .. }
        | PanelNode::ScrollView { children, .. }
        | PanelNode::Dialog { children, .. }
        | PanelNode::Menu {
            items: children, ..
        }
        | PanelNode::MenuItem { children, .. } => children,
        _ => {
            replace_source_only(source, target, replacement)?;
            *typed = PanelNode::parse_with_sources(source, &mut sources.borrow_mut())?;
            return Ok(());
        }
    };
    if source_children.len() != typed_children.len()
        || source_children.iter().any(|child| !child.is_object())
    {
        replace_source_only(source, target, replacement)?;
        *typed = PanelNode::parse_with_sources(source, &mut sources.borrow_mut())?;
        return Ok(());
    }
    for (child_source, child_typed) in source_children.iter_mut().zip(typed_children) {
        let contains = child_source.get("__nativeId").and_then(Value::as_str) == Some(target)
            || source_contains_id(child_source, target);
        if contains {
            return replace_source_and_typed(
                child_source,
                child_typed,
                target,
                replacement,
                replacement_typed,
                visited,
                sources,
            );
        }
    }
    Err(format!("native patch target {target:?} disappeared"))
}

fn source_contains_id(value: &Value, target: &str) -> bool {
    if let Some(values) = value.as_array() {
        return values.iter().any(|value| {
            value.get("__nativeId").and_then(Value::as_str) == Some(target)
                || source_contains_id(value, target)
        });
    }
    value
        .get("children")
        .and_then(Value::as_array)
        .is_some_and(|children| {
            children.iter().any(|child| {
                child.get("__nativeId").and_then(Value::as_str) == Some(target)
                    || source_contains_id(child, target)
            })
        })
}

pub fn parse_panel_for_manifest(
    value: &Value,
    manifest: &impl SurfaceBoundsProvider,
    expected_surface_id: Option<&str>,
) -> Result<PanelNode, String> {
    parse_panel_for_manifest_with_sources(
        value,
        manifest,
        expected_surface_id,
        &mut Default::default(),
    )
}

fn parse_panel_for_manifest_with_sources(
    value: &Value,
    manifest: &impl SurfaceBoundsProvider,
    expected_surface_id: Option<&str>,
    sources: &mut crate::virtual_source::VirtualSourceCatalog,
) -> Result<PanelNode, String> {
    if let Some(roots) = value.as_array() {
        if roots.len() > 32 {
            return Err("package render exceeds 32 top-level windows".into());
        }
        let mut identities = std::collections::HashSet::new();
        let mut selected = None;
        for root in roots {
            if root.is_null() || root == &Value::Bool(false) {
                continue;
            }
            if root.get("kind").and_then(Value::as_str) != Some("window") {
                return Err("package fragments may contain only top-level windows".into());
            }
            let id = root
                .get("id")
                .and_then(Value::as_str)
                .ok_or("windows in a package fragment need distinct surface identities")?;
            if !identities.insert(id) {
                return Err(format!("duplicate top-level window {id:?}"));
            }
            let node = parse_panel_for_manifest_with_sources(root, manifest, Some(id), sources)?;
            if expected_surface_id == Some(id) {
                selected = Some(node);
            }
        }
        return selected.ok_or_else(|| "package fragment did not render its host window".into());
    }
    let mut root = value.clone();
    if root.get("kind").and_then(Value::as_str) == Some("window") && root.get("id").is_none() {
        let id = expected_surface_id
            .or_else(|| {
                (manifest.surface_bounds().len() == 1)
                    .then(|| manifest.surface_bounds()[0].id.as_str())
            })
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
            "text-field" | "image-button" | "slider" | "color-swatch" | "switch" | "checkbox"
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
    let node = PanelNode::parse_with_sources(&root, sources)?;
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
            | PanelNode::Layer { children, .. }
            | PanelNode::Div { children, .. }
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

#[cfg(feature = "jsx")]
pub fn render_panel(
    runtime: &mut JsxRuntime,
    manifest: &impl SurfaceBoundsProvider,
    expected_surface_id: Option<&str>,
    expression: &str,
) -> Result<PanelNode, String> {
    runtime.render(expression, |value| {
        RetainedPanelTree::admit(value, manifest, expected_surface_id, 0)
            .map(RetainedPanelTree::into_node)
    })
}

#[cfg(feature = "jsx")]
pub fn render_retained_panel(
    runtime: &mut JsxRuntime,
    manifest: &impl SurfaceBoundsProvider,
    expected_surface_id: Option<&str>,
    expression: &str,
    generation: u64,
) -> Result<RetainedPanelTree, String> {
    runtime.render(expression, |value| {
        RetainedPanelTree::admit(value, manifest, expected_surface_id, generation)
    })
}

#[cfg(feature = "jsx")]
pub fn render_panel_validated(
    runtime: &mut JsxRuntime,
    manifest: &impl SurfaceBoundsProvider,
    expected_surface_id: Option<&str>,
    expression: &str,
    stylesheet: &StyleSheet,
    validation_rejected: &mut bool,
) -> Result<PanelNode, String> {
    render_retained_panel_validated(
        runtime,
        manifest,
        expected_surface_id,
        expression,
        stylesheet,
        0,
        validation_rejected,
    )
    .map(RetainedPanelTree::into_node)
}

#[cfg(feature = "jsx")]
pub fn render_retained_panel_validated(
    runtime: &mut JsxRuntime,
    manifest: &impl SurfaceBoundsProvider,
    expected_surface_id: Option<&str>,
    expression: &str,
    stylesheet: &StyleSheet,
    generation: u64,
    validation_rejected: &mut bool,
) -> Result<RetainedPanelTree, String> {
    runtime.render(expression, |value| {
        let retained = RetainedPanelTree::admit(value, manifest, expected_surface_id, generation)
            .inspect_err(|_| *validation_rejected = true)?;
        if let Some(surface_id) = expected_surface_id {
            let grant = manifest
                .surface_bounds()
                .iter()
                .find(|surface| surface.id == surface_id)
                .ok_or_else(|| {
                    *validation_rejected = true;
                    "rendered surface is no longer declared"
                })?;
            retained
                .node()
                .requested_surface(grant, stylesheet)
                .inspect_err(|_| *validation_rejected = true)?;
        }
        Ok(retained)
    })
}

#[cfg(test)]
mod class_lookup_tests {
    use super::{PanelNode, RetainedPanelTree};
    use serde_json::{Value, json};
    use twinkle_protocol::{NativePatchCounters, NativePatchEnvelope, NativePatchOperation};

    #[test]
    fn registered_virtual_sources_survive_compact_patches_and_retire_with_nodes() {
        use std::sync::Arc;
        let manifest = twinkle_protocol::SurfaceBounds {
            surfaces: Vec::new(),
        };
        let source = json!({"kind":"div","id":"rows","__nativeId":"root","action":0,
            "collection":{"keys":(0..10000).map(|i| format!("row-{i}")).collect::<Vec<_>>(),
                "heights":(0..10000).map(|i| if i%2==0 {20} else {70}).collect::<Vec<_>>(),"start":0,"end":0},
            "children":[]});
        let mut tree = RetainedPanelTree::admit(&source, &manifest, None, 1).unwrap();
        let PanelNode::Div {
            collection: Some(collection),
            ..
        } = tree.node()
        else {
            panic!("missing collection")
        };
        let logical = Arc::clone(collection.source.as_ref().unwrap());
        let revision = logical.revision();
        let compact = json!({"kind":"div","id":"rows","__nativeId":"root","action":0,
        "collection":{"source":revision,"start":20,"end":22},"children":[
            {"kind":"text","__nativeId":"root/@row-20","key":"row-20","children":["20"]},
            {"kind":"text","__nativeId":"root/@row-21","key":"row-21","children":["21"]}
        ]});
        let patch = |node| NativePatchEnvelope {
            version: 1,
            counters: Default::default(),
            operations: vec![NativePatchOperation::ReplaceSubtree {
                target: "root".into(),
                node,
            }],
        };
        let sheet = super::StyleSheet::compile("").unwrap();
        tree.apply_patch(&patch(compact.clone()), &manifest, None, &sheet, 2, 512)
            .unwrap();
        let PanelNode::Div {
            collection: Some(collection),
            ..
        } = tree.node()
        else {
            panic!("missing collection")
        };
        assert!(Arc::ptr_eq(&logical, collection.source.as_ref().unwrap()));
        assert!(!collection.needs_source_ack);
        assert!(serde_json::to_vec(tree.source()).unwrap().len() < 1024);
        {
            let mut measured = tree.clone();
            let frame = twinkle::UiFrame::layout(
                twinkle::VerticalScroll::new(super::PluginMessage::Scroll, 0.0)
                    .child(measured.node().view(&super::PluginImages::new(), &sheet)),
                twinkle::Rect::new(0.0, 0.0, 300.0, 200.0),
            );
            let batches = measured
                .node()
                .virtual_collection_measurements(frame.resolved_layout())
                .unwrap();
            assert!(
                measured
                    .apply_virtual_measurements(&batches, 1.0, 1)
                    .unwrap()
            );
            assert_eq!(measured.generation(), tree.generation());
            assert_eq!(measured.source(), tree.source());
            assert_eq!(measured.handler_slots(), tree.handler_slots());
            assert_ne!(measured.node(), tree.node());
            let corrected = measured.sources.resolve("root", revision).unwrap();
            assert_eq!(corrected.geometry().height(20), Some(batches[0].rows[0].1));
            assert!(
                !measured
                    .apply_virtual_measurements(&batches, 1.0, 1)
                    .unwrap()
            );
            assert!(Arc::ptr_eq(
                &corrected,
                &measured.sources.resolve("root", revision).unwrap()
            ));
            let readmitted = measured.readmit(&compact, &manifest, None, 3).unwrap();
            assert_eq!(readmitted.node(), measured.node());
            let before_failure = measured.clone();
            let mut invalid_batches = batches.clone();
            invalid_batches[0].rows[0].1 = 321.0;
            invalid_batches.push(super::VirtualCollectionMeasurements {
                source_revision: u64::MAX,
                width: 300.0,
                rows: vec![],
                anchor: None,
            });
            assert!(
                measured
                    .apply_virtual_measurements(&invalid_batches, 1.0, 1)
                    .is_err()
            );
            assert_eq!(measured.node(), before_failure.node());
            assert_eq!(
                measured.sources.payload_bytes(),
                before_failure.sources.payload_bytes()
            );
            assert!(Arc::ptr_eq(
                &corrected,
                &measured.sources.resolve("root", revision).unwrap()
            ));
            let mut stale_range = batches.clone();
            stale_range[0].rows[0].0 = 0;
            assert!(
                measured
                    .apply_virtual_measurements(&stale_range, 1.0, 1)
                    .is_err()
            );
        }
        let accepted = tree.clone();
        let mut invalid = source.clone();
        invalid["collection"]["end"] = json!(1);
        assert!(
            tree.apply_patch(&patch(invalid), &manifest, None, &sheet, 3, 512)
                .is_err()
        );
        assert_eq!(tree.generation(), accepted.generation());
        assert_eq!(tree.source(), accepted.source());
        assert!(Arc::ptr_eq(
            &logical,
            &tree.sources.resolve("root", revision).unwrap()
        ));
        let readmitted = tree.readmit(&compact, &manifest, None, 3).unwrap();
        assert!(Arc::ptr_eq(
            &logical,
            &readmitted.sources.resolve("root", revision).unwrap()
        ));
        let mut wrong_owner = compact.clone();
        wrong_owner["__nativeId"] = json!("different");
        assert!(tree.readmit(&wrong_owner, &manifest, None, 3).is_err());
        let replacement = json!({"kind":"column","__nativeId":"root","children":[]});
        tree.apply_patch(&patch(replacement), &manifest, None, &sheet, 3, 128)
            .unwrap();
        assert_eq!(tree.sources.source_count(), 0);
        assert!(tree.readmit(&compact, &manifest, None, 4).is_err());
    }

    #[test]
    fn virtual_collection_structural_patch_preserves_range_atomically() {
        let row = json!({"kind":"text","key":"a","__nativeId":"root/@a","children":["A"]});
        let source = json!({"kind":"div","id":"rows","__nativeId":"root","action":0,
            "collection":{"heights":[20],"start":0,"end":1},"children":[row.clone()]});
        let manifest = twinkle_protocol::SurfaceBounds {
            surfaces: Vec::new(),
        };
        let sheet = super::StyleSheet::compile("").unwrap();
        let mut retained = RetainedPanelTree::admit(&source, &manifest, None, 1).unwrap();
        let mut patch = NativePatchEnvelope {
            version: 1,
            operations: vec![NativePatchOperation::RemoveChild {
                parent: "root".into(),
                key: "a".into(),
                child_id: "root/@a".into(),
                index: 0,
            }],
            counters: Default::default(),
        };
        assert!(
            retained
                .apply_patch(&patch, &manifest, None, &sheet, 2, 1)
                .is_err()
        );
        assert_eq!(retained.generation(), 1);
        assert_eq!(retained.source(), &source);
        assert_eq!(retained.node(), &PanelNode::parse(&source).unwrap());
        patch.operations.push(NativePatchOperation::InsertChild {
            parent: "root".into(),
            key: "a".into(),
            child_id: "root/@a".into(),
            index: 0,
            node: row,
        });
        retained
            .apply_patch(&patch, &manifest, None, &sheet, 2, 1)
            .unwrap();
        assert_eq!(retained.source(), &source);
        assert_eq!(retained.node(), &PanelNode::parse(&source).unwrap());
        assert_eq!(retained.generation(), 2);
    }

    #[test]
    fn namespaced_target_below_fragment_root_matches_cold_tree() {
        let target =
            "root/#1/export:shell.taskbar::root/#0/@io.nickel.codex.project.7bad8b9af270552f";
        let root = json!({"kind":"column","__nativeId":"root","children":[
            {"kind":"text","__nativeId":target,"children":["old"]}
        ]});
        let replacement = json!({"kind":"text","__nativeId":target,"children":["new"]});
        let mut source = Value::Array(vec![root.clone()]);
        let mut typed = PanelNode::parse(&root).unwrap();
        let replacement_typed = PanelNode::parse(&replacement).unwrap();
        let mut visited = 0;
        super::replace_source_and_typed(
            &mut source,
            &mut typed,
            target,
            &replacement,
            &replacement_typed,
            &mut visited,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(source[0]["children"][0], replacement);
        assert_eq!(typed, PanelNode::parse(&source[0]).unwrap());
    }

    #[test]
    fn subtree_replacement_rematerializes_the_nearest_misaligned_ancestor() {
        let target = "root/export:shell.settings::root/#0";
        let mut source = json!({"kind":"column","__nativeId":"root","children":[
            {"kind":"text","__nativeId":"root/@title","children":["Settings"]},
            {"kind":"column","__nativeId":target,"children":[]}
        ]});
        // A composed source may retain an ownership wrapper which the typed
        // presentation flattened. Model that divergence directly.
        let mut typed = PanelNode::parse(&json!({
            "kind":"column","__nativeId":"root","children":[
                {"kind":"column","__nativeId":target,"children":[]}
            ]
        }))
        .unwrap();
        let replacement = json!({
            "kind":"column","__nativeId":target,"className":"appearance-page","children":[
                {"kind":"text","__nativeId":format!("{target}/#0"),"children":["Theme"]}
            ]
        });
        let replacement_typed = PanelNode::parse(&replacement).unwrap();
        let mut visited = 0;
        super::replace_source_and_typed(
            &mut source,
            &mut typed,
            target,
            &replacement,
            &replacement_typed,
            &mut visited,
            &Default::default(),
        )
        .unwrap();
        assert_eq!(source["children"][1], replacement);
        assert_eq!(typed, PanelNode::parse(&source).unwrap());
    }

    #[test]
    fn retained_admission_indexes_identity_and_preserves_cold_typed_tree() {
        let source = json!({
            "kind":"column", "__nativeId":"root", "children":[
                {"kind":"text", "__nativeId":"root/@label", "key":"label", "children":["hello"]},
                {"kind":"button", "__nativeId":"root/@go", "key":"go", "label":"Go",
                 "action":0, "__handlerSlots":{"action":"root/@go:action"}, "children":[]}
            ]
        });
        let manifest = twinkle_protocol::SurfaceBounds {
            surfaces: Vec::new(),
        };
        let retained = RetainedPanelTree::admit(&source, &manifest, None, 7).unwrap();
        assert_eq!(retained.generation(), 7);
        assert_eq!(retained.nodes().len(), 3);
        assert_eq!(retained.handler_slots().len(), 1);
        assert_eq!(
            retained.node(),
            &super::parse_panel_for_manifest(&source, &manifest, None).unwrap()
        );

        let invalid = json!({"kind":"column","__nativeId":"root","children":[
            {"kind":"text","__nativeId":"root","children":["duplicate"]}
        ]});
        assert!(RetainedPanelTree::admit(&invalid, &manifest, None, 8).is_err());
        assert_eq!(retained.generation(), 7);
        assert_eq!(retained.source(), &source);
    }

    #[test]
    fn nested_indexed_target_below_typed_leaf_reparses_that_leaf() {
        let source = json!({
            "kind":"select", "__nativeId":"root", "id":"projects", "label":"Projects",
            "value":"project", "open":true, "action":0, "accessibilityLabel":"Projects",
            "__handlerSlots":{"action":"root:action"}, "children":[
                {"kind":"option", "id":"project", "__nativeId":"root/#0/@project",
                 "action":1, "children":["old"]}
            ]
        });
        let manifest = twinkle_protocol::SurfaceBounds {
            surfaces: Vec::new(),
        };
        let stylesheet = super::StyleSheet::compile("").unwrap();
        let mut retained = RetainedPanelTree::admit(&source, &manifest, None, 1).unwrap();
        assert!(
            retained
                .nodes()
                .keys()
                .any(|id| id.as_str() == "root/#0/@project")
        );
        let patch = NativePatchEnvelope {
            version: 1,
            operations: vec![NativePatchOperation::SetPrimitive {
                target: "root/#0/@project".into(),
                property: "children".into(),
                value: json!(["new"]),
            }],
            counters: NativePatchCounters {
                nodes_visited: 1,
                nodes_mutated: 1,
                ..Default::default()
            },
        };
        retained
            .apply_patch(&patch, &manifest, None, &stylesheet, 2, 32)
            .unwrap();
        let expected = json!({
            "kind":"select", "__nativeId":"root", "id":"projects", "label":"Projects",
            "value":"project", "open":true, "action":0, "accessibilityLabel":"Projects",
            "__handlerSlots":{"action":"root:action"}, "children":[
                {"kind":"option", "id":"project", "__nativeId":"root/#0/@project",
                 "action":1, "children":["new"]}
            ]
        });
        assert_eq!(
            retained.node(),
            &super::parse_panel_for_manifest(&expected, &manifest, None).unwrap()
        );
        assert_eq!(retained.source(), &expected);
    }

    #[test]
    fn box_geometry_patches_preserve_children_and_reject_unbounded_positions() {
        let source = json!({"kind":"box","__nativeId":"box","x":0,"y":0,"width":200,"height":100,"children":[{"kind":"text","__nativeId":"child","children":["Display"]}]});
        let manifest = twinkle_protocol::SurfaceBounds {
            surfaces: Vec::new(),
        };
        let stylesheet = super::StyleSheet::default();
        let mut retained = RetainedPanelTree::admit(&source, &manifest, None, 1).unwrap();
        for (generation, property, value) in [
            (2, "x", 150),
            (3, "y", 40),
            (4, "width", 220),
            (5, "height", 120),
        ] {
            let patch = NativePatchEnvelope {
                version: 1,
                operations: vec![NativePatchOperation::SetPrimitive {
                    target: "box".into(),
                    property: property.into(),
                    value: json!(value),
                }],
                counters: Default::default(),
            };
            retained
                .apply_patch(&patch, &manifest, None, &stylesheet, generation, 1)
                .unwrap();
            assert_eq!(
                retained.node(),
                &super::parse_panel_for_manifest(retained.source(), &manifest, None).unwrap()
            );
            assert_eq!(retained.source()["children"], source["children"]);
        }
        let accepted = retained.clone();
        let invalid = NativePatchEnvelope {
            version: 1,
            operations: vec![NativePatchOperation::SetPrimitive {
                target: "box".into(),
                property: "x".into(),
                value: json!(8193),
            }],
            counters: Default::default(),
        };
        assert!(
            retained
                .apply_patch(&invalid, &manifest, None, &stylesheet, 6, 1)
                .is_err()
        );
        assert_eq!(retained.node(), accepted.node());
        assert_eq!(retained.source(), accepted.source());
    }

    #[test]
    fn primitive_and_handler_patches_match_cold_admission_and_reject_atomically() {
        let source = json!({"kind":"column","__nativeId":"root","children":[
            {"kind":"text","__nativeId":"root/@label","key":"label","children":["old"]},
            {"kind":"button","__nativeId":"root/@go","key":"go","label":"Go","action":0,
             "__handlerSlots":{"action":"root/@go:action"},"children":[]}
        ]});
        let manifest = twinkle_protocol::SurfaceBounds {
            surfaces: Vec::new(),
        };
        let stylesheet = super::StyleSheet::compile("").unwrap();
        let mut retained = RetainedPanelTree::admit(&source, &manifest, None, 1).unwrap();
        let patch = NativePatchEnvelope {
            version: 1,
            operations: vec![
                NativePatchOperation::SetPrimitive {
                    target: "root/@label".into(),
                    property: "children".into(),
                    value: json!(["new"]),
                },
                NativePatchOperation::ReplaceHandlerSlot {
                    slot: "root/@go:action".into(),
                    action: 4,
                },
            ],
            counters: NativePatchCounters {
                nodes_visited: 4,
                nodes_mutated: 2,
                ..Default::default()
            },
        };
        let counters = retained
            .apply_patch(&patch, &manifest, None, &stylesheet, 2, 97)
            .unwrap();
        let expected = json!({"kind":"column","__nativeId":"root","children":[
            {"kind":"text","__nativeId":"root/@label","key":"label","children":["new"]},
            {"kind":"button","__nativeId":"root/@go","key":"go","label":"Go","action":4,
             "__handlerSlots":{"action":"root/@go:action"},"children":[]}
        ]});
        assert_eq!(
            retained.node(),
            &super::parse_panel_for_manifest(&expected, &manifest, None).unwrap()
        );
        assert_eq!(retained.source(), &expected);
        assert_eq!(counters.transport_bytes, 97);
        assert_eq!(counters.nodes_visited, 4);
        assert_eq!(counters.nodes_mutated, 2);

        let accepted = retained.clone();
        let invalid = NativePatchEnvelope {
            version: 1,
            operations: vec![NativePatchOperation::SetPrimitive {
                target: "root/@label".into(),
                property: "children".into(),
                value: json!([true]),
            }],
            counters: NativePatchCounters::default(),
        };
        assert!(
            retained
                .apply_patch(&invalid, &manifest, None, &stylesheet, 3, 1)
                .is_err()
        );
        assert_eq!(retained.node(), accepted.node());
        assert_eq!(retained.source(), accepted.source());
        assert_eq!(retained.generation(), 2);
    }

    #[test]
    fn incremental_patch_reparses_only_misaligned_mixed_flow_ancestor() {
        let source = json!({"kind":"column","__nativeId":"root","children":[
            null, "Pinned applications: ", 7, null,
            {"kind":"button","__nativeId":"root/@clock","key":"clock","action":0,
             "__handlerSlots":{"action":"root/@clock:action"},"children":["12:00 PM"]}
        ]});
        let manifest = twinkle_protocol::SurfaceBounds {
            surfaces: Vec::new(),
        };
        let stylesheet = super::StyleSheet::compile("").unwrap();
        let mut retained = RetainedPanelTree::admit(&source, &manifest, None, 1).unwrap();
        let patch = NativePatchEnvelope {
            version: 1,
            operations: vec![
                NativePatchOperation::SetPrimitive {
                    target: "root/@clock".into(),
                    property: "children".into(),
                    value: json!(["12:01 PM"]),
                },
                NativePatchOperation::ReplaceHandlerSlot {
                    slot: "root/@clock:action".into(),
                    action: 4,
                },
            ],
            counters: NativePatchCounters {
                nodes_visited: 1,
                nodes_mutated: 2,
                ..Default::default()
            },
        };
        retained
            .apply_patch(&patch, &manifest, None, &stylesheet, 2, 48)
            .unwrap();
        let expected = json!({"kind":"column","__nativeId":"root","children":[
            null, "Pinned applications: ", 7, null,
            {"kind":"button","__nativeId":"root/@clock","key":"clock","action":4,
             "__handlerSlots":{"action":"root/@clock:action"},"children":["12:01 PM"]}
        ]});
        assert_eq!(retained.source(), &expected);
        assert_eq!(
            retained.node(),
            &super::parse_panel_for_manifest(&expected, &manifest, None).unwrap()
        );
    }

    #[test]
    fn keyed_child_sequence_matches_cold_tree_and_rejects_stale_coordinates() {
        let item = |key: &str| json!({"kind":"text","key":key,"__nativeId":format!("root/@{key}"),"children":[key]});
        let source =
            json!({"kind":"column","__nativeId":"root","children":[item("a"),item("b"),item("c")]});
        let manifest = twinkle_protocol::SurfaceBounds {
            surfaces: Vec::new(),
        };
        let stylesheet = super::StyleSheet::compile("").unwrap();
        let mut retained = RetainedPanelTree::admit(&source, &manifest, None, 1).unwrap();
        let patch = NativePatchEnvelope {
            version: 1,
            operations: vec![
                NativePatchOperation::RemoveChild {
                    parent: "root".into(),
                    key: "a".into(),
                    child_id: "root/@a".into(),
                    index: 0,
                },
                NativePatchOperation::MoveChild {
                    parent: "root".into(),
                    key: "c".into(),
                    child_id: "root/@c".into(),
                    from: 1,
                    to: 0,
                },
                NativePatchOperation::InsertChild {
                    parent: "root".into(),
                    key: "d".into(),
                    child_id: "root/@d".into(),
                    index: 2,
                    node: item("d"),
                },
            ],
            counters: NativePatchCounters {
                nodes_visited: 3,
                nodes_mutated: 3,
                ..Default::default()
            },
        };
        let counters = retained
            .apply_patch(&patch, &manifest, None, &stylesheet, 2, 120)
            .unwrap();
        let expected =
            json!({"kind":"column","__nativeId":"root","children":[item("c"),item("b"),item("d")]});
        assert_eq!(
            retained.node(),
            &super::parse_panel_for_manifest(&expected, &manifest, None).unwrap()
        );
        assert_eq!(retained.source(), &expected);
        assert_eq!(counters.nodes_mutated, 3);

        let bad_operations = [
            NativePatchOperation::RemoveChild {
                parent: "root".into(),
                key: "b".into(),
                child_id: "root/@b".into(),
                index: 0,
            },
            NativePatchOperation::MoveChild {
                parent: "root/missing".into(),
                key: "c".into(),
                child_id: "root/@c".into(),
                from: 0,
                to: 1,
            },
            NativePatchOperation::InsertChild {
                parent: "root".into(),
                key: "b".into(),
                child_id: "root/@b".into(),
                index: 0,
                node: item("b"),
            },
        ];
        for operation in bad_operations {
            let accepted = retained.clone();
            let invalid = NativePatchEnvelope {
                version: 1,
                operations: vec![operation],
                counters: NativePatchCounters::default(),
            };
            assert!(
                retained
                    .apply_patch(&invalid, &manifest, None, &stylesheet, 3, 1)
                    .is_err()
            );
            assert_eq!(retained.source(), accepted.source());
            assert_eq!(retained.node(), accepted.node());
        }
    }

    #[test]
    fn retired_projection_nodes_are_not_native_components() {
        for kind in ["action", "widget", "section"] {
            assert!(PanelNode::parse(&serde_json::json!({"kind":kind,"children":[]})).is_err());
        }
    }

    #[test]
    fn direct_class_lookup_uses_whitespace_tokens_and_rejects_duplicates() {
        let child = |class_name: &str| PanelNode::Column {
            children: Vec::new(),
            class_name: Some(class_name.into()),
        };
        let mut root = PanelNode::Column {
            children: vec![child("content selected"), child("footer")],
            class_name: None,
        };
        assert!(root.direct_child_with_class("selected").is_some());
        assert!(root.direct_child_with_class("select").is_none());
        if let PanelNode::Column { children, .. } = &mut root {
            children.push(child("selected"));
        }
        assert!(root.direct_child_with_class("selected").is_none());
    }

    #[test]
    fn image_and_progress_accept_classes_in_the_rendered_tree() {
        let root = PanelNode::parse(&json!({
            "kind": "column",
            "children": [
                {"kind": "image", "asset": "icon", "width": 24, "height": 24,
                 "className": "icon compact", "children": []},
                {"kind": "progress", "percent": 50, "width": 100, "height": 8,
                 "className": "meter compact", "children": []},
                {"kind": "badge", "count": 3,
                 "className": "task-badge compact", "children": []}
            ]
        }))
        .unwrap();
        assert!(matches!(
            root.direct_child_with_class("icon"),
            Some(PanelNode::Image { .. })
        ));
        assert!(matches!(
            root.direct_child_with_class("meter"),
            Some(PanelNode::Progress { .. })
        ));
        assert!(matches!(
            root.direct_child_with_class("task-badge"),
            Some(PanelNode::Badge { .. })
        ));
    }

    #[test]
    fn layout_elements_accept_and_coalesce_plain_jsx_text() {
        let root = PanelNode::parse(&json!({
            "kind": "div", "children": [
                "Hello ", 42, null,
                {"kind":"row", "children":["nested text"]},
                "  ",
                {"kind":"text", "children":["explicit"]}
            ]
        }))
        .unwrap();
        let PanelNode::Div { children, .. } = root else {
            panic!("expected div");
        };
        assert!(
            matches!(&children[0], PanelNode::Text { value, wrap: true, .. } if value == "Hello 42")
        );
        assert!(matches!(&children[1], PanelNode::Row { children, .. }
            if matches!(&children[0], PanelNode::Text { value, .. } if value == "nested text")));
        assert!(matches!(&children[2], PanelNode::Text { value, .. } if value == "explicit"));
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

#[cfg(test)]
mod tests {
    use super::*;
    use twinkle::{Rect, UiFrame, backend::PaintCommand};

    #[test]
    fn fragment_patch_cannot_mutate_a_sibling_window() {
        let manifest: SurfaceBounds =
            serde_json::from_str(include_str!("../tests/fixtures/two-window-surfaces.json"))
                .unwrap();
        let source = serde_json::json!([
            {"kind":"window","id":"home","width":400,"height":240,
             "__nativeId":"root/#0","children":[
                {"kind":"text","__nativeId":"root/#0/#0","children":["Home"]}]},
            {"kind":"window","id":"details","width":450,"height":260,
             "__nativeId":"root/#1","children":[
                {"kind":"text","__nativeId":"root/#1/#0","children":["Details"]}]}
        ]);
        let mut retained = RetainedPanelTree::admit(&source, &manifest, Some("home"), 1).unwrap();
        let before = retained.node().clone();
        let patch = NativePatchEnvelope {
            version: 1,
            operations: vec![NativePatchOperation::SetPrimitive {
                target: "root/#1/#0".into(),
                property: "children".into(),
                value: serde_json::json!(["wrong window"]),
            }],
            counters: Default::default(),
        };
        let error = retained
            .apply_patch(
                &patch,
                &manifest,
                Some("home"),
                &StyleSheet::default(),
                2,
                0,
            )
            .unwrap_err();
        assert!(error.contains("outside the host window"), "{error}");
        assert_eq!(retained.node(), &before);
        assert_eq!(retained.source(), &source);
        assert_eq!(retained.generation(), 1);
        let mut selected_patch = patch.clone();
        if let NativePatchOperation::SetPrimitive { target, .. } = &mut selected_patch.operations[0]
        {
            *target = "root/#0/#0".into();
        }
        let mut mixed = selected_patch.clone();
        mixed.operations.extend(patch.operations);
        assert!(
            retained
                .apply_patch(
                    &mixed,
                    &manifest,
                    Some("home"),
                    &StyleSheet::default(),
                    2,
                    0
                )
                .is_err()
        );
        assert_eq!(
            retained.source(),
            &source,
            "rejected batches must be atomic"
        );
        retained
            .apply_patch(
                &selected_patch,
                &manifest,
                Some("home"),
                &StyleSheet::default(),
                2,
                0,
            )
            .unwrap();
        assert_eq!(retained.source()[1], source[1]);
        assert_eq!(retained.generation(), 2);
    }

    #[test]
    fn nested_container_builders_fit_the_normal_thread_stack() {
        std::thread::Builder::new()
            .name("nested-presentation-stack".into())
            .stack_size(2 * 1024 * 1024)
            .spawn(|| {
                let mut value = serde_json::json!({"kind":"text","children":["stack-safe leaf"]});
                for (index, kind) in ["box", "layer", "row", "column", "scroll-view", "div"]
                    .into_iter().cycle().take(12).enumerate()
                {
                    value = serde_json::json!({
                        "kind":kind,"id":format!("container-{index}"),
                        "x":0,"y":0,"width":300,"height":200,"children":[value]
                    });
                }
                value = serde_json::json!({"kind":"window","id":"stack-root","width":300,"height":200,"children":[value]});
                let node = PanelNode::parse(&value).unwrap();
                let frame = UiFrame::layout(
                    node.view(&PluginImages::new(), &StyleSheet::default()),
                    Rect::new(0.0, 0.0, 300.0, 200.0),
                );
                assert!(frame.resolved_layout().nodes().iter().any(|node|
                    node.accessibility_label.as_deref() == Some("stack-safe leaf")));
            })
            .unwrap().join().unwrap();
    }

    #[test]
    fn wallpaper_image_demand_follows_native_scroll_clip_and_viewport() {
        let node = PanelNode::parse(&serde_json::json!({
            "kind":"column", "children": (0..20).map(|index| serde_json::json!({
                "kind":"image", "id":format!("preview-{index}"),
                "asset":format!("wallpaper:{index}"), "width":160, "height":40, "children":[]
            })).collect::<Vec<_>>()
        }))
        .unwrap();
        let viewport = Rect::new(0.0, 0.0, 300.0, 100.0);
        let frame_at = |offset| {
            UiFrame::layout(
                VerticalScroll::new(PluginMessage::Scroll, offset)
                    .child(node.view(&PluginImages::new(), &StyleSheet::default())),
                viewport,
            )
        };
        let first = frame_at(0.0);
        assert_eq!(
            node.visible_wallpaper_assets(first.resolved_layout(), viewport),
            vec!["wallpaper:0", "wallpaper:1", "wallpaper:2"]
        );
        let scrolled = frame_at(400.0);
        assert_eq!(
            node.visible_wallpaper_assets(scrolled.resolved_layout(), viewport),
            vec!["wallpaper:10", "wallpaper:11", "wallpaper:12"]
        );
        assert_eq!(
            node.visible_wallpaper_assets(
                scrolled.resolved_layout(),
                Rect::new(0.0, 0.0, 300.0, 40.0)
            ),
            vec!["wallpaper:10"]
        );
        assert!(
            node.visible_wallpaper_assets(
                scrolled.resolved_layout(),
                Rect::new(0.0, 200.0, 300.0, 100.0)
            )
            .is_empty()
        );
    }

    #[test]
    fn virtual_measurements_use_bounded_keyed_native_rows() {
        for count in [100usize, 1_000, 10_000] {
            let keys: Vec<_> = (0..count).map(|row| format!("row/{row}/é")).collect();
            let row =
                |class: &str| serde_json::json!({"kind":"div","className":class,"children":[]});
            let node = PanelNode::parse(&serde_json::json!({
                "kind":"div","id":"settings/rows","className":"rows","__nativeId":"native/rows","action":0,
                "collection":{"keys":keys,"count":count,"height":20,"start":20,"end":22},
                "children":[row("short"),row("tall")]
            }))
            .unwrap();
            let sheet = StyleSheet::compile(
                ".rows { width: 200px; } .short { height: 26px; } .tall { height: 74px; }",
            )
            .unwrap();
            let frame = UiFrame::layout(
                VerticalScroll::new(PluginMessage::Scroll, 400.0)
                    .child(node.view(&PluginImages::new(), &sheet)),
                Rect::new(0.0, 0.0, 300.0, 200.0),
            );
            let measurements = node
                .virtual_collection_measurements(frame.resolved_layout())
                .unwrap();
            assert_eq!(measurements.len(), 1);
            // The native scrollbar reserves 16 logical pixels.
            assert_eq!(measurements[0].width, 284.0);
            assert_eq!(measurements[0].rows, vec![(20, 26.0), (21, 74.0)]);
            let semantics = frame.semantic_nodes();
            assert!(
                semantics
                    .iter()
                    .any(|node| node.role == Some(SemanticRole::List))
            );
            let items: Vec<_> = semantics
                .iter()
                .filter(|node| node.role == Some(SemanticRole::ListItem))
                .collect();
            assert_eq!(
                items.len(),
                2,
                "offscreen logical rows must not become semantic view nodes"
            );
            assert_eq!(
                items[0].description.as_deref(),
                Some(format!("item 21 of {count}").as_str())
            );
            assert_eq!(
                items[1].description.as_deref(),
                Some(format!("item 22 of {count}").as_str())
            );
            assert!(frame.resolved_layout().nodes().len() < 24);
            let PanelNode::Div {
                collection: Some(collection),
                ..
            } = &node
            else {
                unreachable!()
            };
            let source = collection.source.as_ref().unwrap();
            assert!(
                measurements[0]
                    .anchor
                    .as_ref()
                    .unwrap()
                    .as_str()
                    .ends_with(&virtual_row_id(source, 20))
            );
            let clipped_frame = UiFrame::layout(
                VerticalScroll::new(PluginMessage::Scroll, 450.0)
                    .child(node.view(&PluginImages::new(), &sheet)),
                Rect::new(0.0, 0.0, 300.0, 200.0),
            );
            let clipped = node
                .virtual_collection_measurements(clipped_frame.resolved_layout())
                .unwrap();
            assert!(
                clipped[0]
                    .anchor
                    .as_ref()
                    .unwrap()
                    .as_str()
                    .ends_with(&virtual_row_id(source, 21)),
                "fully clipped overscan row is not selected as the anchor"
            );
            let id = virtual_row_id(source, 21);
            let original_id = frame
                .resolved_layout()
                .nodes()
                .iter()
                .find(|row| row.id.as_str().ends_with(&id))
                .unwrap()
                .id
                .clone();
            let mut next = node.clone();
            let PanelNode::Div {
                collection: Some(collection),
                children,
                ..
            } = &mut next
            else {
                unreachable!()
            };
            collection.range = 21..23;
            children.swap(0, 1);
            let next_frame = UiFrame::layout(
                VerticalScroll::new(PluginMessage::Scroll, 420.0)
                    .child(next.view(&PluginImages::new(), &sheet)),
                Rect::new(0.0, 0.0, 300.0, 200.0),
            );
            let next_id = &next_frame
                .resolved_layout()
                .nodes()
                .iter()
                .find(|row| row.id.as_str().ends_with(&id))
                .unwrap()
                .id;
            assert_eq!(
                &original_id, next_id,
                "overlapping keyed row keeps its full native identity"
            );
            assert_eq!(
                next.virtual_collection_measurements(next_frame.resolved_layout())
                    .unwrap()[0]
                    .rows,
                vec![(21, 74.0), (22, 26.0)]
            );
            let next_semantics = next_frame.semantic_nodes();
            let surviving = next_semantics
                .iter()
                .find(|node| node.id == original_id)
                .unwrap();
            assert_eq!(
                surviving.description.as_deref(),
                Some(format!("item 22 of {count}").as_str())
            );
        }
    }

    #[test]
    fn virtual_collection_admission_rejects_invalid_geometry_and_ranges() {
        let valid = serde_json::json!({"kind":"div","id":"rows","action":0,"collection":{"heights":[20,40],"start":0,"end":0},"children":[]});
        let accepted = PanelNode::parse(&valid).unwrap();
        assert!(accepted.contains_virtual_collection());
        assert_eq!(accepted.button_action("rows"), None);
        for (name, value) in [
            ("start", serde_json::json!(-1)),
            ("end", serde_json::json!(3)),
            ("end", serde_json::json!(1)),
            ("gap", serde_json::json!(-1)),
            ("overscan", serde_json::json!(9000)),
            ("heights", serde_json::json!([0])),
            ("heights", serde_json::json!(["20"])),
        ] {
            let mut invalid = valid.clone();
            invalid["collection"][name] = value;
            assert!(
                PanelNode::parse(&invalid).is_err(),
                "accepted invalid {name}"
            );
        }
        let mut missing_action = valid.clone();
        missing_action.as_object_mut().unwrap().remove("action");
        assert!(PanelNode::parse(&missing_action).is_err());
    }

    #[test]
    fn uniform_virtual_collection_admission_is_compact_and_unambiguous() {
        let valid = serde_json::json!({"kind":"div","id":"rows","action":0,"collection":{"count":10000,"height":20,"start":0,"end":0},"children":[]});
        let mut registered = valid.clone();
        registered["__nativeId"] = serde_json::json!("root");
        registered["collection"]["keys"] = serde_json::json!(
            (0..10000)
                .map(|index| index.to_string())
                .collect::<Vec<_>>()
        );
        let PanelNode::Div {
            collection: Some(registered_collection),
            ..
        } = PanelNode::parse(&registered).unwrap()
        else {
            panic!("missing registered source")
        };
        assert_eq!(registered_collection.heights.retained_bytes(), 0);
        assert_eq!(
            registered_collection
                .source
                .as_ref()
                .unwrap()
                .ordinal("9999"),
            Some(9999)
        );
        let mut wrong_count = registered.clone();
        wrong_count["collection"]["count"] = serde_json::json!(9999);
        assert!(PanelNode::parse(&wrong_count).is_err());
        let mut ambiguous = registered.clone();
        ambiguous["collection"]["heights"] = serde_json::json!([20]);
        assert!(PanelNode::parse(&ambiguous).is_err());
        for missing in ["count", "height"] {
            let mut invalid = registered.clone();
            invalid["collection"]
                .as_object_mut()
                .unwrap()
                .remove(missing);
            assert!(PanelNode::parse(&invalid).is_err());
        }
        let PanelNode::Div {
            collection: Some(collection),
            ..
        } = PanelNode::parse(&valid).unwrap()
        else {
            panic!("missing collection")
        };
        assert_eq!(collection.heights.len(), 10_000);
        assert_eq!(collection.heights.retained_bytes(), 0);
        assert_eq!(
            collection.heights.window_for_range(0..0).unwrap().total,
            200_000.0
        );
        for (name, value) in [
            ("count", serde_json::json!(-1)),
            ("count", serde_json::json!(10001)),
            ("count", serde_json::json!(1.5)),
            ("height", serde_json::json!(0)),
            ("height", serde_json::json!(8193)),
            ("height", serde_json::json!("20")),
            ("heights", serde_json::json!([20])),
        ] {
            let mut invalid = valid.clone();
            invalid["collection"][name] = value;
            assert!(
                PanelNode::parse(&invalid).is_err(),
                "accepted invalid {name}"
            );
        }
        for missing in ["count", "height"] {
            let mut invalid = valid.clone();
            invalid["collection"]
                .as_object_mut()
                .unwrap()
                .remove(missing);
            assert!(PanelNode::parse(&invalid).is_err());
        }
        let mut empty = valid;
        empty["collection"]["count"] = serde_json::json!(0);
        assert!(PanelNode::parse(&empty).is_ok());
    }

    #[test]
    fn styled_flex_content_fills_its_allocated_frame() {
        for kind in ["div", "row", "column"] {
            let node = PanelNode::parse(
                &serde_json::json!({"kind":"div","className":"root","children":[
                    {"kind":kind,"className":"body","children":[
                        {"kind":"div","className":"pane","children":[]}
                    ]},
                    {"kind":"div","className":"footer","children":[]}
                ]}),
            )
            .unwrap();
            let sheet = StyleSheet::compile(
                ".root { height: 200px; width: 200px; } .body { display: flex; flex-grow: 1; align-items: stretch; } .pane { flex-grow: 1; background: #123456; } .footer { height: 20px; flex-shrink: 0; }",
            ).unwrap();
            let frame = UiFrame::layout_with_state(
                node.view(&PluginImages::new(), &sheet),
                Rect::new(0.0, 0.0, 200.0, 200.0),
                &mut Default::default(),
            );
            assert!(frame.commands().iter().any(|command| matches!(command,
                PaintCommand::Fill { rect, color } if *color == 0xff123456 && rect.size.height == 180.0
            )), "{kind} content must use the height assigned to its CSS frame: {:?}", frame.commands());
        }
    }

    #[test]
    fn text_alignment_inherits_and_can_be_overridden_for_launcher_labels() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"column","children":[
            {"kind":"text","children":["Centered"]},
            {"kind":"text","className":"time","children":["2m ago"]}
        ]}))
        .unwrap();
        let sheet = StyleSheet::compile(
            "column { text-align: center; color: #eeeeee; } text.time { text-align: end; }",
        )
        .unwrap();
        let frame = UiFrame::layout_with_state(
            node.view(&PluginImages::new(), &sheet),
            Rect::new(0.0, 0.0, 200.0, 80.0),
            &mut Default::default(),
        );
        for (label, expected) in [
            ("Centered", twinkle::TextAlign::Center),
            ("2m ago", twinkle::TextAlign::End),
        ] {
            assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Text{text,align,..} if text==label && *align==expected)));
        }
        assert!(StyleSheet::compile("text { text-align: nonsense; }").is_err());
    }

    #[test]
    fn button_icon_tint_preserves_alpha_and_original_artwork() {
        let node = PanelNode::parse(
            &serde_json::json!({"kind":"button","id":"logo","icon":"app","action":0,"children":[]}),
        )
        .unwrap();
        let original = Arc::new(image::RgbaImage::from_pixel(
            2,
            2,
            image::Rgba([40, 80, 120, 127]),
        ));
        let images = PluginImages::from([("app".into(), (7, Arc::clone(&original)))]);
        for (css, expected) in [
            ("button { icon-color: #eeccaa; }", [238, 204, 170, 127]),
            ("", [40, 80, 120, 127]),
        ] {
            let sheet = StyleSheet::compile(css).unwrap();
            let frame = UiFrame::layout_with_state(
                node.view(&images, &sheet),
                Rect::new(0.0, 0.0, 100.0, 60.0),
                &mut Default::default(),
            );
            assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Image { image, .. } if image.get_pixel(0, 0).0 == expected)));
        }
        assert_eq!(original.get_pixel(0, 0).0, [40, 80, 120, 127]);
    }

    #[test]
    fn button_icon_size_is_bounded_and_controls_rendered_artwork() {
        let mut data = serde_json::json!({"kind":"button","id":"result","icon":"app","iconSize":24,"action":0,"children":["Terminal"]});
        let node = PanelNode::parse(&data).unwrap();
        let images = PluginImages::from([(
            "app".into(),
            (
                7,
                Arc::new(image::RgbaImage::from_pixel(
                    32,
                    32,
                    image::Rgba([40, 80, 120, 255]),
                )),
            ),
        )]);
        let frame = UiFrame::layout_with_state(
            node.view(&images, &StyleSheet::default()),
            Rect::new(0.0, 0.0, 200.0, 60.0),
            &mut Default::default(),
        );
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Image {bounds,..} if bounds.size.width==24.0 && bounds.size.height==24.0)));
        for invalid in [0, 7, 129] {
            data["iconSize"] = invalid.into();
            assert!(PanelNode::parse(&data).is_err());
        }
    }

    #[test]
    fn scroll_view_scrollbar_parts_control_native_paint_geometry_and_states() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"scroll-view", "id":"feed", "className":"custom", "height":100, "children":[{"kind":"div", "className":"tall", "children":[]}]})).unwrap();
        let sheet = StyleSheet::compile(".tall { height: 400px; } scroll-view { --track: #123456; } scrollbar-track.custom { width: 14px; margin: 4px; background: var(--track); border-radius: 7px; } scrollbar-thumb#feed { height: 45px; background: #abcdef; border-radius: 6px; } scrollbar-thumb:hover { background: transparent; border: 2px solid #112233; border-radius: 8px; } scrollbar-track:active { background: #778899; } scrollbar-thumb:focus { background: #fedcba; }").unwrap();
        let mut state = twinkle::UiStateStore::default();
        let build = |state: &mut twinkle::UiStateStore| {
            UiFrame::layout_with_state(
                node.view(&PluginImages::new(), &sheet),
                Rect::new(0.0, 0.0, 200.0, 100.0),
                state,
            )
        };
        let first = build(&mut state);
        assert!(first.commands().iter().any(|command| matches!(command, PaintCommand::RoundedFill { rect, color, radius } if *color == 0xff123456 && rect.size.width == 14.0 && rect.origin.x == 182.0 && *radius == 7.0)));
        assert!(first.commands().iter().any(|command| matches!(command, PaintCommand::RoundedFill { rect, color, .. } if *color == 0xffabcdef && rect.size.height == 45.0)));
        let at = Point { x: 190.0, y: 20.0 };
        first.handle_event(&mut state, twinkle::UiEvent::PointerMoved(at));
        let hover = build(&mut state);
        assert!(hover.commands().iter().any(|command| matches!(command, PaintCommand::RoundedStroke { color,width,radius,.. } if *color == 0xff112233 && *width == 2.0 && *radius == 8.0)));
        hover.handle_event(&mut state, twinkle::UiEvent::PointerPressed(at));
        let active = build(&mut state);
        assert!(active.commands().iter().any(|command| matches!(command,PaintCommand::RoundedFill { color,.. } if *color == 0xff778899)));
        active.handle_event(
            &mut state,
            twinkle::UiEvent::PointerMoved(Point { x: 190.0, y: 75.0 }),
        );
        let moved = build(&mut state);
        assert!(moved.commands().iter().any(|command| matches!(command, PaintCommand::RoundedFill { rect,color,.. } if *color == 0xffabcdef && rect.origin.y > 4.0)));
        moved.handle_event(&mut state, twinkle::UiEvent::PointerReleased(at));
        moved.handle_event(
            &mut state,
            twinkle::UiEvent::PointerMoved(Point { x: 20.0, y: 20.0 }),
        );
        moved.handle_event(&mut state, twinkle::UiEvent::FocusNext);
        let focus = build(&mut state);
        assert!(focus.commands().iter().any(|command| matches!(command, PaintCommand::RoundedFill { color,.. } if *color == 0xfffedcba)));
        let dry = UiFrame::layout(
            node.view(
                &PluginImages::new(),
                &StyleSheet::compile(".tall { height: 400px; }").unwrap(),
            ),
            Rect::new(0.0, 0.0, 200.0, 100.0),
        );
        assert!(
            !dry.commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::RoundedFill { .. }))
        );
    }

    #[test]
    fn slider_frame_and_parts_use_css_states_and_padded_value_geometry() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"slider","id":"volume","value":0.5,"action":4,"accessibilityLabel":"Volume","children":[]})).unwrap();
        let sheet = StyleSheet::compile("slider { width: 200px; height: 50px; padding: 10px; margin: 3px; background: #123456; } slider:hover { background: #334455; border: 2px solid #abcdef; border-radius: 8px; } slider:focus { background: transparent; border: 3px solid #fedcba; border-radius: 7px; } slider:active { background: #556677; } slider-track { width: 150px; margin: 0px 5px; padding: 0px 10px; height: 4px; background: #778899; } slider-fill { background: #aabbcc; } slider-thumb { width: 16px; height: 16px; background: #abcdef; } slider-thumb:hover { background: transparent; border: 2px solid #112233; border-radius: 6px; }").unwrap();
        let mut state = twinkle::UiStateStore::default();
        let build = |state: &mut twinkle::UiStateStore| {
            UiFrame::layout_with_state(
                node.view(&PluginImages::new(), &sheet),
                Rect::new(0.0, 0.0, 300.0, 200.0),
                state,
            )
        };
        let first = build(&mut state);
        let slider = first
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::Slider))
            .unwrap();
        assert_eq!(slider.allocated.size, twinkle::Size::new(200.0, 50.0));
        assert_eq!(slider.content.size, twinkle::Size::new(180.0, 30.0));
        let at = Point {
            x: slider.content.origin.x + 5.0 + 10.0 + 130.0 * 0.25,
            y: slider.content.origin.y + 15.0,
        };
        assert_eq!(
            first.message_at_owned(at),
            Some(PluginMessage::Value(4, 0.25))
        );
        first.handle_event(&mut state, twinkle::UiEvent::PointerMoved(at));
        let hover = build(&mut state);
        assert!(hover.commands().iter().any(|command| matches!(command, PaintCommand::RoundedStroke { color, width, radius, .. } if *color == 0xff112233 && *width == 2.0 && *radius == 6.0)));
        hover.handle_event(&mut state, twinkle::UiEvent::FocusNext);
        let focus = build(&mut state);
        assert!(focus.commands().iter().any(|command| matches!(command, PaintCommand::RoundedStroke { rect, color, width, radius } if rect.size.width == 200.0 && *color == 0xfffedcba && *width == 3.0 && *radius == 7.0)));
        assert!(!focus.commands().iter().any(|command| matches!(command, PaintCommand::Fill { rect, .. } | PaintCommand::RoundedFill { rect, .. } if rect.size.width == 200.0)));
        focus.handle_event(&mut state, twinkle::UiEvent::PointerPressed(at));
        let dragged = focus.handle_event(&mut state, twinkle::UiEvent::PointerMoved(at));
        assert_eq!(dragged.messages, vec![PluginMessage::Value(4, 0.25)]);
        let active = build(&mut state);
        assert!(active.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == 0xff556677)
        ));
    }

    #[test]
    fn decorative_compound_parts_follow_the_native_owner_focus() {
        let sheet = StyleSheet::compile("checkbox, switch, color-swatch { width: 60px; height: 40px; } checkbox-box { width: 30px; height: 30px; } checkbox-box:focus { background: transparent; border: 2px solid #abcdef; border-radius: 5px; } checkbox-mark { font-size: 14px; line-height: 20px; } checkbox-mark:focus { color: #fedcba; font-size: 22px; line-height: 26px; } switch-track { width: 50px; height: 24px; } switch-track:focus { background: #223344; } switch-thumb { width: 20px; height: 20px; } switch-thumb:focus { background: transparent; border: 3px solid #445566; border-radius: 7px; } color-swatch-label { font-size: 14px; line-height: 20px; } color-swatch-label:focus { color: #aabbcc; font-size: 23px; line-height: 27px; background: #334455; }").unwrap();
        for (kind, role) in [
            ("checkbox", SemanticRole::Checkbox),
            ("switch", SemanticRole::Switch),
            ("color-swatch", SemanticRole::Button),
        ] {
            let node = PanelNode::parse(&serde_json::json!({"kind":kind,"id":"control","state":"on","action":1,"accessibilityLabel":"Control","children":[]})).unwrap();
            let mut state = twinkle::UiStateStore::default();
            let build = |state: &mut twinkle::UiStateStore| {
                UiFrame::layout_with_state(
                    Column::new().child(node.view(&PluginImages::new(), &sheet)),
                    Rect::new(0.0, 0.0, 200.0, 100.0),
                    state,
                )
            };
            let first = build(&mut state);
            assert_eq!(
                first
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .filter(|node| node.semantic_role == Some(role))
                    .count(),
                1
            );
            first.handle_event(&mut state, twinkle::UiEvent::FocusNext);
            let focused = build(&mut state);
            match kind {
                "checkbox" => {
                    assert!(focused.commands().iter().any(|command|matches!(command,PaintCommand::RoundedStroke{color,width,radius,..} if *color==0xffabcdef && *width==2.0 && *radius==5.0)));
                    assert!(focused.commands().iter().any(|command|matches!(command,PaintCommand::Text{text,color,scale,..} if text=="✓" && *color==0xfffedcba && *scale == -22.0)));
                }
                "switch" => {
                    assert!(focused.commands().iter().any(|command|matches!(command,PaintCommand::Fill{color,..} if *color==0xff223344)));
                    assert!(focused.commands().iter().any(|command|matches!(command,PaintCommand::RoundedStroke{color,width,radius,..} if *color==0xff445566 && *width==3.0 && *radius==7.0)));
                }
                _ => {
                    assert!(focused.commands().iter().any(|command|matches!(command,PaintCommand::Fill{color,..} if *color==0xff334455)));
                    assert!(focused.commands().iter().any(|command|matches!(command,PaintCommand::Text{text,color,scale,..} if text=="+" && *color==0xffaabbcc && *scale == -23.0)));
                }
            }
        }
    }

    #[test]
    fn select_root_frame_focus_and_padding_reach_native_parts() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"select","id":"choice","value":"One","action":1,"accessibilityLabel":"Choice","children":[{"kind":"option","id":"one","action":2,"children":["One"]}]})).unwrap();
        let sheet = StyleSheet::compile("select { width: 180px; padding: 5px; background: #123456; } select:focus { background: transparent; color: #fedcba; font-size: 19px; line-height: 24px; border: 2px solid #abcdef; border-radius: 6px; } select-header { height: 30px; width: 140px; } select-indicator { width: 20px; height: 20px; font-size: 10px; } select-indicator:focus { color: #112233; font-size: 17px; line-height: 23px; border: 1px solid #445566; border-radius: 4px; } option { height: 20px; }").unwrap();
        let mut state = twinkle::UiStateStore::default();
        let build = |state: &mut twinkle::UiStateStore| {
            UiFrame::layout_with_state(
                Column::new().child(node.view(&PluginImages::new(), &sheet)),
                Rect::new(0.0, 0.0, 300.0, 200.0),
                state,
            )
        };
        let first = build(&mut state);
        let select = first
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.accessibility_label.as_deref() == Some("Choice"))
            .unwrap();
        assert_eq!(select.allocated.size, twinkle::Size::new(180.0, 40.0));
        first.handle_event(&mut state, twinkle::UiEvent::FocusNext);
        let focus = build(&mut state);
        assert!(focus.commands().iter().any(|command| matches!(command, PaintCommand::RoundedStroke { rect, color, radius, .. } if rect.size == twinkle::Size::new(180.0,40.0) && *color == 0xffabcdef && *radius == 6.0)));
        assert!(focus.commands().iter().any(|command| matches!(command, PaintCommand::Text { bounds, scale, text, .. } if text == "One" && bounds.origin.x == 5.0 && bounds.origin.y == 5.0 && bounds.size.width == 140.0 && *scale == -19.0)));
        assert!(focus.commands().iter().any(|command| matches!(command, PaintCommand::Text { color,scale,text,.. } if text != "One" && *color == 0xff112233 && *scale == -17.0)));
        assert!(focus.commands().iter().any(|command| matches!(command, PaintCommand::RoundedStroke { color,radius,.. } if *color == 0xff445566 && *radius == 4.0)));
    }

    #[test]
    fn option_classes_drive_independent_native_geometry_paint_and_hits() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"select","id":"choice","accessibilityLabel":"Choice","value":"Small","action":1,"open":true,"children":[
            {"kind":"option","id":"small","className":"compact","action":2,"children":["Small"]},
            {"kind":"option","id":"large","className":"large","action":3,"children":["Large"]}
        ]})).unwrap();
        let sheet = StyleSheet::compile("select { width: 180px; } select-header { height: 30px; } option { height: 20px; color: #abcdef; font-size: 14px; } option.large { width: 140px; height: 42px; margin: 3px; padding: 5px; background: #123456; font-size: 19px; } option#large:hover { background: transparent; color: #fedcba; border: 2px solid #aabbcc; border-radius: 6px; }").unwrap();
        let mut state = twinkle::UiStateStore::default();
        let build = |state: &mut twinkle::UiStateStore| {
            UiFrame::layout_with_state(
                node.view(&PluginImages::new(), &sheet),
                Rect::new(0.0, 0.0, 300.0, 200.0),
                state,
            )
        };
        let first = build(&mut state);
        first.handle_event(
            &mut state,
            twinkle::UiEvent::PointerPressed(Point { x: 20.0, y: 15.0 }),
        );
        first.handle_event(
            &mut state,
            twinkle::UiEvent::PointerReleased(Point { x: 20.0, y: 15.0 }),
        );
        let frame = build(&mut state);
        let large = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.accessibility_label.as_deref() == Some("Large"))
            .unwrap();
        assert_eq!(large.allocated.size, twinkle::Size::new(140.0, 42.0));
        assert_eq!(large.allocated.origin.y, 53.0);
        assert_eq!(large.content.origin.y, 58.0);
        let at = Point {
            x: large.allocated.origin.x + 20.0,
            y: large.allocated.origin.y + 20.0,
        };
        assert_eq!(frame.message_at(at), Some(&PluginMessage::Click(3)));
        frame.handle_event(&mut state, twinkle::UiEvent::PointerMoved(at));
        let hover = build(&mut state);
        assert!(hover.commands().iter().any(|command| matches!(command, PaintCommand::RoundedStroke { color, radius, .. } if *color == 0xffaabbcc && *radius == 6.0)));
        assert!(hover.commands().iter().any(|command| matches!(command, PaintCommand::Text { color, scale, text, .. } if text == "Large" && *color == 0xfffedcba && *scale == -19.0)));
    }

    #[test]
    fn button_states_paint_css_text_border_and_transparency() {
        let node = PanelNode::parse(
            &serde_json::json!({"kind":"button","id":"action","action":4,"children":["Run"]}),
        )
        .unwrap();
        let sheet = StyleSheet::compile("button { width: 100px; height: 40px; background: #123456; color: #abcdef; } button:hover { background: transparent; color: #fedcba; border: 3px solid #aabbcc; border-radius: 9px; font-size: 19px; line-height: 25px; } button:active { background: #334455; color: #556677; border-color: #778899; } button:focus { color: transparent; border: 2px solid #abcdef; }").unwrap();
        let mut state = twinkle::UiStateStore::default();
        let build = |state: &mut twinkle::UiStateStore| {
            UiFrame::layout_with_state(
                node.view(&PluginImages::new(), &sheet),
                Rect::new(0.0, 0.0, 300.0, 200.0),
                state,
            )
        };
        let first = build(&mut state);
        let at = Point { x: 20.0, y: 20.0 };
        first.handle_event(&mut state, twinkle::UiEvent::PointerMoved(at));
        let hover = build(&mut state);
        assert!(hover.commands().iter().any(|command| matches!(command, PaintCommand::RoundedStroke { color, width, radius, .. } if *color == 0xffaabbcc && *width == 3.0 && *radius == 9.0)));
        assert!(!hover.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Fill { .. } | PaintCommand::RoundedFill { .. }
        )));
        assert!(hover.commands().iter().any(|command| matches!(command, PaintCommand::Text { color, scale, text, .. } if text == "Run" && *color == 0xfffedcba && *scale == -19.0)));
        hover.handle_event(&mut state, twinkle::UiEvent::PointerPressed(at));
        let active = build(&mut state);
        assert!(active.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == 0xff334455)
        ));
        assert!(active.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { color, .. } if *color == 0xff556677)
        ));
        active.handle_event(&mut state, twinkle::UiEvent::PointerReleased(at));
        active.handle_event(&mut state, twinkle::UiEvent::FocusNext);
        let focus = build(&mut state);
        assert!(
            !focus
                .commands()
                .iter()
                .any(|command| matches!(command, PaintCommand::Text { .. }))
        );
    }

    #[test]
    fn editor_focus_uses_css_border_and_exact_text_metrics() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"text-field","id":"entry","value":"abc","action":4,"autoFocus":true,"children":[]})).unwrap();
        let sheet = StyleSheet::compile("text-field { width: 180px; height: 40px; background: #123456; color: #abcdef; } text-field:focus { background: transparent; color: #aabbcc; border: 2px solid #fedcba; border-radius: 7px; font-size: 19px; line-height: 23px; } text-field-caret { width: 2px; background: #abcdef; }").unwrap();
        let mut state = twinkle::UiStateStore::default();
        let frame = UiFrame::layout_with_state(
            node.view(&PluginImages::new(), &sheet),
            Rect::new(0.0, 0.0, 300.0, 200.0),
            &mut state,
        );
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::RoundedStroke { color, width, radius, .. } if *color == 0xfffedcba && *width == 2.0 && *radius == 7.0)));
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Text { color, scale, .. } if *color == 0xffaabbcc && *scale == -19.0)));
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Fill { rect, color } if *color == 0xffabcdef && rect.size.height == 23.0)));
    }

    #[test]
    fn text_field_css_frame_caret_and_selection_share_native_content_geometry() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"text-field","id":"entry","value":"abc","placeholder":"Entry","action":4,"autoFocus":true,"children":[]})).unwrap();
        let sheet = StyleSheet::compile("text-field { width: 180px; height: 50px; padding: 7px 11px; margin: 3px; background: #123456; border: 2px solid #fedcba; border-radius: 8px; color: #abcdef; font-size: 21px; line-height: 30px; }
            text-field-caret { width: 3px; background: #abcdef; }
            text-field-selection { background: #aabbcc; }").unwrap();
        let root = || {
            Column::new()
                .align_items(twinkle::Align::Start)
                .child(node.view(&PluginImages::new(), &sheet))
        };
        let mut state = twinkle::UiStateStore::default();
        let frame =
            UiFrame::layout_with_state(root(), Rect::new(0.0, 0.0, 400.0, 200.0), &mut state);
        let field = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::TextField))
            .unwrap();
        assert_eq!(field.allocated.size, twinkle::Size::new(180.0, 50.0));
        assert_eq!(field.content.origin.x - field.allocated.origin.x, 11.0);
        assert_eq!(field.content.origin.y - field.allocated.origin.y, 7.0);
        assert_eq!(state.focused(), Some(&field.id));
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Text { bounds, scale, text, .. } if text == "abc" && bounds.origin == field.content.origin && *scale == -21.0)));
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Fill { rect, color } if *color == 0xffabcdef && rect.size.width == 3.0 && rect.size.height == 30.0 && rect.origin.y == field.content.origin.y)));
        frame.handle_event(&mut state, twinkle::UiEvent::TextSelectAll);
        let frame =
            UiFrame::layout_with_state(root(), Rect::new(0.0, 0.0, 400.0, 200.0), &mut state);
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Fill { rect, color } if *color == 0xffaabbcc && rect.size.height == 30.0)));
    }

    #[test]
    fn auto_focus_accepts_native_text_and_does_not_steal_focus_after_updates() {
        let field = |id: &str, action: usize| {
            PanelNode::parse(&serde_json::json!({"kind":"text-field","id":id,"value":"abc","placeholder":id,"action":action,"autoFocus":true,"children":[]})).unwrap()
        };
        let node = PanelNode::Column {
            class_name: None,
            children: vec![field("first", 4), field("second", 5)],
        };
        let sheet = StyleSheet::compile(
            "text-field { width: 180px; height: 40px; font-size: 14px; line-height: 20px; }",
        )
        .unwrap();
        let root = || node.view(&PluginImages::new(), &sheet);
        let mut state = twinkle::UiStateStore::default();
        let frame =
            UiFrame::layout_with_state(root(), Rect::new(0.0, 0.0, 400.0, 200.0), &mut state);
        let first = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.accessibility_label.as_deref() == Some("first"))
            .unwrap()
            .id
            .clone();
        let second = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.accessibility_label.as_deref() == Some("second"))
            .unwrap()
            .id
            .clone();
        assert_eq!(state.focused(), Some(&first));
        assert!(
            frame
                .handle_event(&mut state, twinkle::UiEvent::TextInput("x".into()))
                .messages
                .contains(&PluginMessage::Text(4, "abcx".into()))
        );
        frame.handle_event(
            &mut state,
            twinkle::UiEvent::AccessibilityFocus(second.clone()),
        );
        assert_eq!(state.focused(), Some(&second));
        let _ = UiFrame::layout_with_state(root(), Rect::new(0.0, 0.0, 400.0, 200.0), &mut state);
        assert_eq!(state.focused(), Some(&second));
    }

    #[test]
    fn ordinary_editor_and_checkbox_have_no_visible_native_stock_paint() {
        let editor = PanelNode::parse(&serde_json::json!({"kind":"text-field","id":"entry","value":"abc","action":4,"autoFocus":true,"children":[]})).unwrap();
        let check = PanelNode::parse(&serde_json::json!({"kind":"checkbox","id":"check","state":"on","accessibilityLabel":"Checked","action":5,"children":[]})).unwrap();
        let sheet = StyleSheet::default();
        let mut state = twinkle::UiStateStore::default();
        let frame = UiFrame::layout_with_state(
            Column::new()
                .child(editor.view(&PluginImages::new(), &sheet))
                .child(check.view(&PluginImages::new(), &sheet)),
            Rect::new(0.0, 0.0, 300.0, 200.0),
            &mut state,
        );
        assert!(!frame.commands().iter().any(|command| matches!(command, PaintCommand::Fill { color, .. } | PaintCommand::RoundedFill { color, .. } | PaintCommand::Stroke { color, .. } | PaintCommand::Text { color, .. } if *color != 0)));
    }

    #[test]
    fn unstyled_text_keeps_the_native_fallback_foreground() {
        let node = PanelNode::parse(&serde_json::json!({
            "kind": "text",
            "children": ["visible"]
        }))
        .unwrap();
        let frame = UiFrame::layout(
            node.view(&PluginImages::new(), &StyleSheet::default()),
            Rect::new(0.0, 0.0, 160.0, 40.0),
        );
        assert!(frame.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text == "visible"
        )));
    }

    #[test]
    fn native_editor_context_menu_uses_public_css_parts() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"text-field","id":"entry","value":"abc","action":4,"autoFocus":true,"children":[]})).unwrap();
        let sheet = StyleSheet::compile("text-field { width: 180px; height: 40px; font-size: 14px; } text-field-menu { width: 230px; padding: 3px; background: #123456; } text-field-menu-item { height: 37px; color: #abcdef; font-size: 14px; padding: 4px; } text-field-menu-item.disabled { color: #fedcba; } text-field-menu-shortcut { width: 70px; color: #aabbcc; font-size: 7px; }").unwrap();
        let mut state = twinkle::UiStateStore::default();
        let build = |state: &mut twinkle::UiStateStore| {
            UiFrame::layout_with_state(
                node.view(&PluginImages::new(), &sheet),
                Rect::new(0.0, 0.0, 500.0, 400.0),
                state,
            )
        };
        let first = build(&mut state);
        first.handle_event(&mut state, twinkle::UiEvent::KeyboardContextMenu);
        let frame = build(&mut state);
        assert!(
            frame
                .resolved_layout()
                .nodes()
                .iter()
                .any(|node| node.semantic_role == Some(SemanticRole::MenuItem)
                    && node.allocated.size.height == 37.0)
        );
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::RoundedFill { color, .. } | PaintCommand::Fill { color, .. } if *color == 0xff123456)));
        assert!(frame.commands().iter().any(
            |command| matches!(command, PaintCommand::Text { color, .. } if *color == 0xfffedcba)
        ));
    }

    #[test]
    fn checkbox_css_preserves_native_checked_semantics_and_activation() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"checkbox","id":"check","state":"on","className":"custom","accessibilityLabel":"Enabled","action":6,"children":[]})).unwrap();
        let sheet = StyleSheet::compile(
            "checkbox { width: 40px; height: 30px; padding: 2px; }
            checkbox-box { width: 24px; height: 24px; background: #123456; border-radius: 3px; }
            checkbox-mark { color: #abcdef; font-size: 21px; line-height: 24px; }",
        )
        .unwrap();
        let mut state = twinkle::UiStateStore::default();
        let frame = UiFrame::layout_with_state(
            Column::new()
                .align_items(twinkle::Align::Start)
                .child(node.view(&PluginImages::new(), &sheet)),
            Rect::new(0.0, 0.0, 200.0, 100.0),
            &mut state,
        );
        let check = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::Checkbox))
            .unwrap();
        assert_eq!(check.accessibility_state.as_deref(), Some("on"));
        assert_eq!(
            frame.message_for_id(&check.id),
            Some(&PluginMessage::Click(6))
        );
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Text { text, color, scale, .. } if text == "✓" && *color == 0xffabcdef && *scale == -21.0)));
    }

    #[test]
    fn menu_items_and_shortcuts_have_independent_css_and_hit_metrics() {
        let node = PanelNode::parse(&serde_json::json!({"kind":"menu","id":"actions","anchor":"anchor","children":[
            {"kind":"menu-item","id":"small","label":"Small","shortcut":"Ctrl+S","className":"small","action":2,"children":[]},
            {"kind":"menu-item","id":"large","label":"Large","className":"large","action":3,"children":[]}
        ]})).unwrap();
        let sheet = StyleSheet::compile(
            "menu { width: 220px; padding: 2px; }
            menu-item { height: 24px; padding: 4px; color: #abcdef; font-size: 14px; }
            menu-item.large { height: 40px; margin: 3px; background: #123456; }
            menu-shortcut.small { width: 80px; padding: 1px; color: #fedcba; font-size: 7px; } menu-shortcut.small:focus { color: #112233; font-size: 19px; line-height: 23px; border: 2px solid #aabbcc; border-radius: 5px; }",
        )
        .unwrap();
        let mut state = twinkle::UiStateStore::default();
        let mut frame = UiFrame::layout_with_state(
            twinkle::Button::new(PluginMessage::Click(1), "Anchor").id("anchor"),
            Rect::new(0.0, 0.0, 400.0, 200.0),
            &mut state,
        );
        let anchor = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::Button))
            .unwrap()
            .id
            .clone();
        let PanelNode::Menu { items, .. } = &node else {
            unreachable!()
        };
        let menu = twinkle::OverlayMenu::new("actions", twinkle::OverlayAnchor::Node(anchor))
            .item(items[0].overlay_menu_item().unwrap())
            .item(items[1].overlay_menu_item().unwrap());
        frame
            .present_open_menu(&mut state, node.style_overlay_menu(menu, &sheet))
            .unwrap();
        let large = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.accessibility_label.as_deref() == Some("Large"))
            .unwrap();
        assert_eq!(large.allocated.size.height, 40.0);
        assert_eq!(large.allocated.size.width, 210.0);
        assert_eq!(
            frame.message_at(Point {
                x: large.allocated.origin.x + large.allocated.size.width / 2.0,
                y: large.allocated.origin.y + large.allocated.size.height / 2.0
            }),
            Some(&PluginMessage::Click(3))
        );
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Text { text,color,scale,bounds,.. } if text == "Ctrl+S" && *color == 0xff112233 && *scale == -19.0 && bounds.size.width == 78.0)));
        assert!(frame.commands().iter().any(|command|matches!(command,PaintCommand::RoundedStroke{color,width,radius,..} if *color==0xffaabbcc && *width==2.0 && *radius==5.0)));
    }

    #[test]
    fn auto_focus_waits_for_native_surface_focus_and_prefers_declared_editor() {
        let editor = PanelNode::parse(&serde_json::json!({"kind":"text-field","id":"search","value":"","placeholder":"Search","action":4,"autoFocus":true,"children":[]})).unwrap();
        let sheet =
            StyleSheet::compile("text-field { width: 180px; height: 40px; font-size: 14px; }")
                .unwrap();
        let mut state = twinkle::UiStateStore::default();
        let closed = UiFrame::layout(
            twinkle::Button::new(PluginMessage::Click(1), "Open"),
            Rect::new(0.0, 0.0, 400.0, 200.0),
        );
        closed.handle_event(&mut state, twinkle::UiEvent::FocusLost);
        let frame = UiFrame::layout_with_state(
            Column::new()
                .child(twinkle::Button::new(PluginMessage::Click(1), "Earlier"))
                .child(editor.view(&PluginImages::new(), &sheet)),
            Rect::new(0.0, 0.0, 400.0, 200.0),
            &mut state,
        );
        assert!(state.focused().is_none());
        let target = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::TextField))
            .unwrap()
            .id
            .clone();
        frame.handle_event(&mut state, twinkle::UiEvent::FocusGained);
        assert_eq!(state.focused(), Some(&target));
        assert!(
            frame
                .handle_event(&mut state, twinkle::UiEvent::TextInput("query".into()))
                .messages
                .contains(&PluginMessage::Text(4, "query".into()))
        );
    }

    #[cfg(feature = "jsx")]
    #[test]
    fn checkbox_public_jsx_normalizes_flags_and_routes_boolean_changes() {
        let mut runtime = JsxRuntime::new("function App() { return h(Checkbox,{id:'check',checked:true,'aria-label':'Enabled',onChange:checked=>twinkle.request({type:'changed',checked})}); }",None).unwrap();
        let value = runtime
            .render("__nickelRender()", |value| Ok(value.clone()))
            .unwrap();
        assert_eq!(value["kind"], serde_json::json!("checkbox"));
        assert_eq!(value["state"], serde_json::json!("on"));
        assert!(matches!(
            PanelNode::parse(&value).unwrap(),
            PanelNode::Checkbox { .. }
        ));
        let action = value["action"].as_u64().unwrap();
        runtime
            .render(&format!("__nickelDispatch({action})"), |value| {
                Ok(value.clone())
            })
            .unwrap();
        assert_eq!(
            runtime.take_effects().unwrap(),
            vec![serde_json::json!({"type":"changed","checked":false})]
        );
        let mut runtime = JsxRuntime::new("function App(){return h(Checkbox,{id:'disabled',checked:true,disabled:true,accessibilityLabel:'Disabled',onChange:()=>twinkle.request({type:'forbidden'})});}",None).unwrap();
        let value = runtime
            .render("__nickelRender()", |value| Ok(value.clone()))
            .unwrap();
        assert!(matches!(
            PanelNode::parse(&value).unwrap(),
            PanelNode::Checkbox { action: None, .. }
        ));
    }

    #[test]
    fn select_icons_follow_native_header_and_option_paint_and_demand() {
        let mut node = PanelNode::parse(&serde_json::json!({
            "kind":"select", "id":"providers", "accessibilityLabel":"Terminal", "value":"Konsole", "open":false, "action":1,
            "children":[
                {"kind":"option","id":"konsole","icon":"application:konsole","action":2,"children":["Konsole"]},
                {"kind":"option","id":"kitty","icon":"application:kitty","action":3,"children":["Kitty"]}
            ]
        })).unwrap();
        assert_eq!(
            node.application_image_assets(),
            ["application:konsole".to_owned()].into()
        );
        let mut images = PluginImages::new();
        images.insert(
            "application:konsole".into(),
            (1, Arc::new(image::RgbaImage::new(20, 20))),
        );
        images.insert(
            "application:kitty".into(),
            (2, Arc::new(image::RgbaImage::new(20, 20))),
        );
        let sheet = StyleSheet::compile("select { width: 200px; } select-header { height:40px; padding:10px; color:#ffffff; } option { height:40px; padding:10px; color:#ffffff; }").unwrap();
        let frame = UiFrame::layout(
            node.view(&images, &sheet),
            Rect::new(0.0, 0.0, 400.0, 300.0),
        );
        assert_eq!(
            frame
                .commands()
                .iter()
                .filter(|command| matches!(command, PaintCommand::Image { .. }))
                .count(),
            1
        );
        if let PanelNode::Select { open, .. } = &mut node {
            *open = true;
        }
        assert_eq!(node.application_image_assets().len(), 2);
        let frame = UiFrame::layout(
            node.view(&images, &sheet),
            Rect::new(0.0, 0.0, 400.0, 300.0),
        );
        assert_eq!(
            frame
                .commands()
                .iter()
                .filter(|command| matches!(command, PaintCommand::Image { .. }))
                .count(),
            3
        );
        assert!(
            frame
                .commands()
                .iter()
                .filter_map(|command| match command {
                    PaintCommand::Text { bounds, text, .. }
                        if text == "Konsole" || text == "Kitty" =>
                        Some(bounds.origin.x),
                    _ => None,
                })
                .all(|x| x >= 38.0)
        );
    }

    #[test]
    fn select_has_no_stock_paint_without_stylesheet() {
        let node = PanelNode::Select {
            id: "choice".into(),
            class_name: None,
            label: "Choice".into(),
            value: "One".into(),
            open: false,
            action: 1,
            options: vec![("one".into(), "One".into(), 2, None, None)],
        };
        let frame = UiFrame::layout(
            node.view(&PluginImages::new(), &StyleSheet::default()),
            Rect::new(0.0, 0.0, 400.0, 200.0),
        );
        assert!(frame.commands().is_empty());
        assert!(
            frame
                .resolved_layout()
                .nodes()
                .iter()
                .any(|node| node.accessibility_label.as_deref() == Some("Choice"))
        );
    }

    #[test]
    fn select_parts_share_css_geometry_with_option_hits_and_accessibility() {
        let node = PanelNode::Select {
            id: "choice".into(),
            class_name: Some("custom".into()),
            label: "Choice".into(),
            value: "One".into(),
            open: true,
            action: 1,
            options: vec![
                ("one".into(), "One".into(), 2, None, None),
                ("two".into(), "Two".into(), 3, None, None),
            ],
        };
        let sheet = StyleSheet::compile("select.custom { width: 160px; --ink: #abcdef; }
            select-header.custom { height: 44px; background: #123456; color: var(--ink); font-size: 21px; padding: 4px; }
            option.custom { height: 18px; background: #fedcba; color: var(--ink); font-size: 14px; padding: 2px; }
            select-indicator.custom { width: 20px; color: var(--ink); font-size: 7px; }").unwrap();
        let frame = UiFrame::layout(
            Column::new()
                .align_items(twinkle::Align::Start)
                .child(node.view(&PluginImages::new(), &sheet)),
            Rect::new(0.0, 0.0, 400.0, 200.0),
        );
        let option = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.accessibility_label.as_deref() == Some("Two"))
            .unwrap();
        assert_eq!(option.allocated.size.height, 18.0);
        assert_eq!(option.allocated.origin.y, 62.0);
        assert_eq!(
            frame.message_at(Point {
                x: option.allocated.origin.x + 4.0,
                y: option.allocated.origin.y + 4.0
            }),
            Some(&PluginMessage::Click(3))
        );
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Text { color, scale, text, .. } if text == "One" && *color == 0xffabcdef && *scale == -21.0)));
    }

    #[test]
    fn select_header_and_option_interaction_paint_use_css() {
        let node = PanelNode::Select {
            id: "choice".into(),
            class_name: None,
            label: "Choice".into(),
            value: "One".into(),
            open: true,
            action: 1,
            options: vec![("one".into(), "One".into(), 2, None, None)],
        };
        let sheet = StyleSheet::compile(
            "select { width: 160px; }
            select-header { height: 30px; background: #123456; }
            select-header:hover { background: #aabbcc; }
            option { height: 20px; background: #abcdef; }
            option:hover { background: #fedcba; }",
        )
        .unwrap();
        let root = || {
            Column::new()
                .align_items(twinkle::Align::Start)
                .child(node.view(&PluginImages::new(), &sheet))
        };
        let mut state = twinkle::UiStateStore::default();
        let mut frame =
            UiFrame::layout_with_state(root(), Rect::new(0.0, 0.0, 400.0, 200.0), &mut state);
        let header = Point { x: 20.0, y: 15.0 };
        frame.handle_event(&mut state, twinkle::UiEvent::PointerMoved(header));
        frame = UiFrame::layout_with_state(root(), Rect::new(0.0, 0.0, 400.0, 200.0), &mut state);
        assert!(frame.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == 0xffaabbcc)
        ));
        frame.handle_event(&mut state, twinkle::UiEvent::PointerPressed(header));
        frame.handle_event(&mut state, twinkle::UiEvent::PointerReleased(header));
        frame = UiFrame::layout_with_state(root(), Rect::new(0.0, 0.0, 400.0, 200.0), &mut state);
        let option = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.accessibility_label.as_deref() == Some("One"))
            .unwrap()
            .allocated;
        frame.handle_event(
            &mut state,
            twinkle::UiEvent::PointerMoved(Point {
                x: option.origin.x + option.size.width / 2.0,
                y: option.origin.y + option.size.height / 2.0,
            }),
        );
        let frame =
            UiFrame::layout_with_state(root(), Rect::new(0.0, 0.0, 400.0, 200.0), &mut state);
        assert!(frame.commands().iter().any(
            |command| matches!(command, PaintCommand::Fill { color, .. } if *color == 0xfffedcba)
        ));
    }

    #[test]
    fn css_menu_metrics_and_paint_reach_native_overlay() {
        let node = PanelNode::parse(&serde_json::json!({"kind": "menu", "id": "actions", "className": "custom", "anchor": "anchor", "open": true, "children": [{"kind": "menu-item", "id": "one", "label": "One", "action": 2, "children": []}]})).unwrap();
        let sheet = StyleSheet::compile("column.theme { --ink: #abcdef; }
             menu.custom { width: 150px; padding: 3px; background: #123456; }
            menu-item.custom { height: 32px; padding: 4px; color: var(--ink); font-size: 14px; background: #fedcba; }
            menu-item.custom:hover { background: #aabbcc; }").unwrap();
        let mut state = twinkle::UiStateStore::default();
        let mut frame = UiFrame::layout_with_state(
            twinkle::Button::new(PluginMessage::Click(1), "Anchor").id("anchor"),
            Rect::new(0.0, 0.0, 400.0, 200.0),
            &mut state,
        );
        let anchor = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.semantic_role == Some(SemanticRole::Button))
            .unwrap()
            .id
            .clone();
        let menu = twinkle::OverlayMenu::new("actions", twinkle::OverlayAnchor::Node(anchor)).item(
            twinkle::OverlayMenuItem::action("one", "One", PluginMessage::Click(2)),
        );
        let root = PanelNode::Column {
            class_name: Some("theme".into()),
            children: vec![node],
        };
        let menu = root.style_overlay_menu_for("actions", menu, &sheet);
        assert_eq!(menu.item_hover, Some(0xffaabbcc));
        frame.present_open_menu(&mut state, menu).unwrap();
        let item = frame
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.accessibility_label.as_deref() == Some("One"))
            .unwrap();
        assert_eq!(item.allocated.size.height, 32.0);
        assert_eq!(item.allocated.size.width, 144.0);
        assert_eq!(
            frame.message_at(Point {
                x: item.allocated.origin.x + 5.0,
                y: item.allocated.origin.y + 5.0
            }),
            Some(&PluginMessage::Click(2))
        );
        assert!(frame.commands().iter().any(|command| matches!(command, PaintCommand::Text { color, scale, text, .. } if text == "One" && *color == 0xffabcdef && *scale == -14.0)));
    }

    #[test]
    fn slider_parts_compile_into_native_paint_geometry() {
        let node = PanelNode::Slider {
            id: "level".into(),
            class_name: None,
            value: 0.5,
            label: "Level".into(),
            action: 2,
            drag_action: None,
        };
        let sheet = StyleSheet::compile(
            "slider { width: 200px; height: 40px; }
            slider-track { height: 8px; border-radius: 0px; background: #123456; }
            slider-fill { background: #abcdef; }
            slider-thumb { width: 16px; height: 24px; border-radius: 4px; background: #fedcba; }",
        )
        .unwrap();
        let frame = UiFrame::layout(
            Column::new()
                .align_items(twinkle::Align::Start)
                .child(node.view(&PluginImages::new(), &sheet)),
            Rect::new(0.0, 0.0, 400.0, 100.0),
        );
        assert!(frame.commands().iter().any(|command| matches!(command,
            PaintCommand::RoundedFill { rect, color, radius } if *color == 0xfffedcba && rect.size.width == 16.0 && rect.size.height == 24.0 && *radius == 4.0)));
        assert!(frame.commands().iter().any(|command| matches!(command,
            PaintCommand::Fill { rect, color } if *color == 0xffabcdef && rect.size.width == 100.0 && rect.size.height == 8.0)));
    }

    #[test]
    fn progress_css_sizes_track_and_percentage_fill() {
        let node = PanelNode::Progress {
            class_name: Some("custom".into()),
            percent: 25,
            width: 120,
            height: 8,
        };
        let sheet = StyleSheet::compile(
            "progress.custom { width: 200px; height: 20px; background: #123456; }
             progress-fill.custom { background: #abcdef; border-radius: 2px; }",
        )
        .unwrap();
        let frame = UiFrame::layout(
            Column::new()
                .align_items(twinkle::Align::Start)
                .child(node.view(&PluginImages::new(), &sheet)),
            Rect::new(0.0, 0.0, 400.0, 100.0),
        );
        assert!(frame.commands().iter().any(|command| matches!(command,
            PaintCommand::Fill { rect, color } if *color == 0xff123456 && rect.size.width == 200.0 && rect.size.height == 20.0)));
        assert!(frame.commands().iter().any(|command| matches!(command,
            PaintCommand::RoundedFill { rect, color, radius } if *color == 0xffabcdef && rect.size.width == 50.0 && rect.size.height == 20.0 && *radius == 2.0)));
    }

    #[test]
    fn switch_parts_inherit_custom_properties_and_paint_css_dimensions() {
        let node = PanelNode::Switch {
            id: "toggle".into(),
            class_name: None,
            state: "on".into(),
            label: "Toggle".into(),
            action: Some(7),
        };
        let sheet = StyleSheet::compile(
            "switch { --part-color: #abcdef; width: 80px; height: 40px; }
             switch-track { width: 70px; height: 30px; background: #123456; }
             switch-thumb { width: 12px; height: 10px; background: var(--part-color); }",
        )
        .unwrap();
        let frame = UiFrame::layout(
            Column::new()
                .align_items(twinkle::Align::Start)
                .child(node.view(&PluginImages::new(), &sheet)),
            Rect::new(0.0, 0.0, 200.0, 100.0),
        );
        assert!(frame.commands().iter().any(|command| matches!(command,
            PaintCommand::Fill { rect, color } if *color == 0xffabcdef && rect.size.width == 12.0 && rect.size.height == 10.0)));
    }
}
