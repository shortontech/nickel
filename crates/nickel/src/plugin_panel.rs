//! Experimental JavaScript panel host. The bundled example uses the same small
//! component vocabulary as an external plugin; native surfaces remain shell-owned.

use std::{
    borrow::Cow,
    collections::{BTreeMap, HashSet},
    sync::{Arc, OnceLock},
};

use nickel_core::plugins::{
    PluginCapability, PluginManifest, PluginPackage, PluginSurface, PluginSurfaceKind,
};
use nickel_plugin_runtime::JsxRuntime;
use nickel_ui::{
    AnyView, Column, ComponentBuilderExt, Container, DragGesture, DragPhase, FilePlaneItem,
    FrameOverlay, Grid, Image, ImageFit, Insets, Layer, Length, OverlayAnchor, OverlayId,
    OverlayMenu, OverlayMenuItem, OverlayStyle, Point, Row, SemanticRole, Shortcut, Size, Spacer,
    Text, TextField as UiTextField, TransientSurface, UiId, VerticalScroll, ViewContext,
};
use serde_json::Value;

use crate::control_view::ControlAction;
use crate::platform::SessionAction;
use crate::plugin_css::{ControlStyle, Display, FlexDirection, StyleSheet};
use nickel_codex_ui::{ProjectMenuProjection, ProjectMenuRevision};
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

pub fn launcher_surface() -> &'static PluginSurface {
    launcher_manifest()
        .surfaces
        .first()
        .expect("bundled launcher needs a surface")
}

pub fn launcher_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    let manifest = launcher_manifest();
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: manifest.id.clone(),
        surface_id: launcher_surface().id.clone(),
    }
}

pub fn taskbar_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!("../../../assets/plugins/taskbar/plugin.json"))
            .expect("bundled taskbar plugin manifest must be valid")
    })
}

pub fn taskbar_surface() -> &'static PluginSurface {
    taskbar_manifest()
        .surfaces
        .iter()
        .find(|surface| surface.reserve_work_area)
        .expect("bundled taskbar needs a work-area panel")
}

pub fn taskbar_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    let manifest = taskbar_manifest();
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: manifest.id.clone(),
        surface_id: taskbar_surface().id.clone(),
    }
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

pub fn notification_surface() -> &'static PluginSurface {
    let surface = notification_manifest()
        .surfaces
        .first()
        .expect("bundled notifications need a surface");
    assert_eq!(surface.kind, PluginSurfaceKind::Overlay);
    surface
}

pub fn notification_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: notification_manifest().id.clone(),
        surface_id: notification_surface().id.clone(),
    }
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

pub fn volume_osd_surface() -> &'static PluginSurface {
    volume_osd_manifest()
        .surfaces
        .first()
        .expect("bundled volume overlay needs a surface")
}

pub fn volume_osd_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: volume_osd_manifest().id.clone(),
        surface_id: volume_osd_surface().id.clone(),
    }
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

pub fn control_center_surface() -> &'static PluginSurface {
    let surface = control_center_manifest()
        .surfaces
        .first()
        .expect("bundled Control Center needs a surface");
    assert_eq!(surface.kind, PluginSurfaceKind::Overlay);
    surface
}

pub fn control_center_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: control_center_manifest().id.clone(),
        surface_id: control_center_surface().id.clone(),
    }
}

pub fn codex_projects_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!(
            "../../../assets/plugins/codex-projects/plugin.json"
        ))
        .expect("bundled Codex projects plugin manifest must be valid")
    })
}

pub fn codex_projects_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: codex_projects_manifest().id.clone(),
        surface_id: codex_projects_manifest().surfaces[0].id.clone(),
    }
}

pub fn on_screen_keyboard_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!(
            "../../../assets/plugins/on-screen-keyboard/plugin.json"
        ))
        .expect("bundled keyboard plugin manifest must be valid")
    })
}

pub fn on_screen_keyboard_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: on_screen_keyboard_manifest().id.clone(),
        surface_id: on_screen_keyboard_manifest().surfaces[0].id.clone(),
    }
}

pub fn screenshot_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!(
            "../../../assets/plugins/screenshot/plugin.json"
        ))
        .expect("bundled screenshot plugin manifest must be valid")
    })
}

pub fn screenshot_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: screenshot_manifest().id.clone(),
        surface_id: screenshot_manifest().surfaces[0].id.clone(),
    }
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

pub fn window_preview_surface() -> &'static PluginSurface {
    window_preview_manifest()
        .surfaces
        .first()
        .expect("bundled window preview needs a surface")
}

pub fn window_preview_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: window_preview_manifest().id.clone(),
        surface_id: window_preview_surface().id.clone(),
    }
}

pub fn desktop_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!("../../../assets/plugins/desktop/plugin.json"))
            .expect("bundled desktop plugin manifest must be valid")
    })
}

pub fn desktop_surface() -> &'static PluginSurface {
    desktop_manifest()
        .surfaces
        .iter()
        .find(|surface| surface.kind == PluginSurfaceKind::Desktop)
        .expect("bundled desktop needs a desktop surface")
}

pub fn desktop_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: desktop_manifest().id.clone(),
        surface_id: desktop_surface().id.clone(),
    }
}

pub fn run_manifest() -> &'static PluginManifest {
    static MANIFEST: OnceLock<PluginManifest> = OnceLock::new();
    MANIFEST.get_or_init(|| {
        PluginManifest::from_json(include_str!("../../../assets/plugins/run/plugin.json"))
            .expect("bundled run plugin manifest must be valid")
    })
}

pub fn run_surface() -> &'static PluginSurface {
    run_manifest()
        .surfaces
        .first()
        .expect("bundled Run dialog needs a surface")
}

pub fn run_surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    let manifest = run_manifest();
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: manifest.id.clone(),
        surface_id: run_surface().id.clone(),
    }
}

fn bundled_source(
    manifest: &PluginManifest,
    entry: &str,
    fallback: &'static str,
) -> Result<Cow<'static, str>, String> {
    let Some(root) = std::env::var_os("NICKEL_DEV_BUNDLED_PLUGIN_ROOT") else {
        return Ok(Cow::Borrowed(fallback));
    };
    if !std::path::Path::new(&root).join(&manifest.id).exists() {
        return Ok(Cow::Borrowed(fallback));
    }
    read_bundled_source(std::path::Path::new(&root), manifest, entry).map(Cow::Owned)
}

fn read_bundled_source(
    root: &std::path::Path,
    manifest: &PluginManifest,
    entry: &str,
) -> Result<String, String> {
    let path = root.join(&manifest.id).join(entry);
    let bytes =
        nickel_storage::read_regular_file(&path, nickel_core::plugins::MAX_PLUGIN_ENTRY_BYTES)
            .map_err(|error| {
                format!(
                    "could not read development source {}: {error}",
                    path.display()
                )
            })?
            .ok_or_else(|| format!("development source {} is missing", path.display()))?;
    String::from_utf8(bytes)
        .map_err(|_| format!("development source {} is not UTF-8", path.display()))
}

fn bundled_stylesheet(
    manifest: &PluginManifest,
    fallback: &'static str,
) -> Result<StyleSheet, String> {
    let source = if let Some(root) = std::env::var_os("NICKEL_DEV_BUNDLED_PLUGIN_ROOT") {
        let root = std::path::Path::new(&root);
        if root.join(&manifest.id).exists() {
            Cow::Owned(PluginPackage::load_stylesheet(
                &root.join(&manifest.id),
                manifest,
            )?)
        } else {
            Cow::Borrowed(fallback)
        }
    } else {
        Cow::Borrowed(fallback)
    };
    StyleSheet::compile(&source)
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

#[derive(Clone, Debug, PartialEq)]
struct WindowRequest {
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
enum PanelNode {
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

fn apply_container_style(
    mut container: Container<PluginMessage>,
    style: &ControlStyle,
) -> Container<PluginMessage> {
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

fn with_margin(view: AnyView<PluginMessage>, style: &ControlStyle) -> AnyView<PluginMessage> {
    if let Some(margin) = style.margin {
        AnyView::new(Container::new().padding(margin).child(view))
    } else {
        view
    }
}

fn styled_text(mut text: Text<PluginMessage>, style: &ControlStyle) -> Text<PluginMessage> {
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
    fn contribution_bytes(&self) -> u64 {
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
            Self::Div { children, .. }
            | Self::Row { children, .. }
            | Self::Column { children, .. }
            | Self::Viewport { children, .. } => {
                let spare = (children.capacity() - children.len()) * std::mem::size_of::<Self>();
                spare as u64 + children.iter().map(Self::contribution_bytes).sum::<u64>()
            }
            _ => 0,
        }
    }

    fn file_tile_action(&self, id: &str) -> Option<usize> {
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

    fn file_tile_select_action(&self, id: &str) -> Option<usize> {
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

    fn file_tile_move_action(&self, id: &str) -> Option<usize> {
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

    fn file_tile_file_action(&self, id: &str) -> Option<usize> {
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
            "div" => Ok(Self::Div {
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
            }),
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

    fn overlay_menu_item(&self) -> Option<OverlayMenuItem<PluginMessage>> {
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

    fn view(&self, images: &PluginImages, stylesheet: &StyleSheet) -> AnyView<PluginMessage> {
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
                    PluginMessage::Click(action.unwrap_or(usize::MAX)),
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
            } => {
                let style = stylesheet.resolve("div", id.as_deref(), class_name.as_deref());
                let content: AnyView<PluginMessage> = match style.display.unwrap_or_default() {
                    Display::Grid => {
                        let mut grid = style
                            .grid_columns
                            .as_ref()
                            .map_or_else(Grid::new, |tracks| Grid::tracks(tracks.iter().cloned()));
                        if let Some(gap) = style.gap {
                            grid = grid.gap(gap);
                        }
                        for child in children {
                            grid = grid.child(child.view(images, stylesheet));
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
                            row = row.child(child.view(images, stylesheet));
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
                            column = column.child(child.view(images, stylesheet));
                        }
                        AnyView::new(column)
                    }
                };
                let mut container = Container::new().child(content);
                if let Some(id) = id {
                    container = container.id(id.clone());
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
                    column = column.child(child.view(images, stylesheet));
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
                        layer = layer.child(child.view(images, stylesheet));
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
                    column = column.child(child.view(images, stylesheet));
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
                        row = row.child(child.view(images, stylesheet));
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
                let mut row = Row::new().fill_width().height(48.0);
                if let Some(gap) = style.gap {
                    row = row.gap(gap);
                }
                for child in children {
                    row = row.child(child.view(images, stylesheet));
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
                let mut column = Column::new().fill_width();
                if let Some(gap) = style.gap {
                    column = column.gap(gap);
                }
                for child in children {
                    column = column.child(child.view(images, stylesheet));
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
                for child in children {
                    column = column.child(child.view(images, stylesheet));
                }
                let scroll = VerticalScroll::new(PluginMessage::Scroll, 0.0)
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
            } => {
                let style = stylesheet.resolve("text", None, class_name.as_deref());
                let text = styled_text(
                    Text::new(value)
                        .color(style.color.unwrap_or(*color))
                        .scale(1.0),
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
            Self::Spacer { class_name } => {
                let style = stylesheet.resolve("spacer", None, class_name.as_deref());
                AnyView::new(Spacer::flex().grow(style.grow.unwrap_or(1.0)))
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
                let field = if *secure {
                    UiTextField::on_change_masked_with_placeholder_mapped(
                        value,
                        placeholder,
                        '•',
                        {
                            let action = *action;
                            move |value| PluginMessage::Text(action, value)
                        },
                    )
                } else {
                    UiTextField::on_change_with_placeholder_mapped(value, placeholder, {
                        let action = *action;
                        move |value| PluginMessage::Text(action, value)
                    })
                };
                let mut field = field.id(id.clone()).accessibility_label(placeholder);
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
                        || AnyView::new(styled_text(Text::new(label), &style)),
                        |(id, image)| {
                            let icon = Image::new(*id, Arc::clone(image)).width(32.0).height(32.0);
                            if *show_label {
                                AnyView::new(
                                    Row::new()
                                        .gap(8.0)
                                        .child(icon)
                                        .child(styled_text(Text::new(label), &style)),
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
                    .height(height.unwrap_or(42) as f32);
                if let Some(width) = width {
                    container = container.width(*width as f32);
                }
                if let Some(action) = context_action {
                    container = container.context_message(PluginMessage::Context(*action));
                }
                if let Some(drag) = drag_action {
                    container = container.on_drag((
                        PluginMessage::Button {
                            click: *action,
                            drag: *drag,
                        },
                        map_plugin_drag,
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

    fn dialog(&self, requested_id: &str) -> Option<&Self> {
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

    fn menu(&self, requested_id: &str) -> Option<&Self> {
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

    fn transients<'a>(&'a self, output: &mut Vec<&'a Self>) {
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

    fn button_action(&self, requested_id: &str) -> Option<usize> {
        match self {
            Self::Button { id, action, .. } if id == requested_id => Some(*action),
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
            Self::Row { children, .. } | Self::Column { children, .. } => {
                for child in children {
                    child.collect_taskbar_badges(badges)?;
                }
                Ok(())
            }
            _ => Err("taskbar badge extension must return badges or a row/column of badges".into()),
        }
    }

    fn collect_desktop_widgets(
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

    fn collect_taskbar_actions(
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

    fn collect_control_sections(
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
        if matches!(kind, "text-field" | "image-button") && !node.contains_key("id") {
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

fn render_panel(
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

pub struct PluginPanelApplication {
    runtime: JsxRuntime,
    node: PanelNode,
    effects: Vec<PluginEffect>,
    pending_transient: Option<(OverlayId, UiId)>,
    last_error: Option<String>,
    runtime_failure: Option<String>,
    manifest: PluginManifest,
    expected_surface_id: Option<String>,
    projection_data: Option<String>,
    launcher_shortcuts: Option<LauncherShortcutState>,
    notification_shortcuts: Option<(Option<u32>, bool)>,
    overlay_open: bool,
    images: PluginImages,
    stylesheet: StyleSheet,
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

pub(crate) fn package_images(package: &PluginPackage) -> Result<PluginImages, String> {
    let mut total_pixels = 0_u64;
    package
        .images
        .iter()
        .enumerate()
        .map(|(index, (id, bytes))| {
            let dimensions = image::ImageReader::new(std::io::Cursor::new(bytes))
                .with_guessed_format()
                .map_err(|error| format!("plugin image {id:?} has an invalid format: {error}"))?
                .into_dimensions()
                .map_err(|error| format!("plugin image {id:?} has invalid dimensions: {error}"))?;
            total_pixels =
                total_pixels.saturating_add(u64::from(dimensions.0) * u64::from(dimensions.1));
            if dimensions.0 == 0
                || dimensions.1 == 0
                || dimensions.0 > 2048
                || dimensions.1 > 2048
                || total_pixels > 4_000_000
            {
                return Err(format!("plugin images exceed 4 million pixels at {id:?}"));
            }
            let image = image::load_from_memory(bytes)
                .map_err(|error| format!("could not decode plugin image {id:?}: {error}"))?
                .into_rgba8();
            Ok((id.clone(), ((index + 1) as u16, Arc::new(image))))
        })
        .collect()
}

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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DesktopBackgroundAction {
    ToggleIcons,
    SmallIcons,
    MediumIcons,
    LargeIcons,
    SortName,
    SortNameDescending,
    SortKind,
    SortKindDescending,
    SortSize,
    SortSizeDescending,
    SortModified,
    SortModifiedAscending,
    Manual,
    AlignGrid,
    AutoArrange,
    FoldersFirst,
    FoldersMixed,
    Refresh,
    Paste,
    NewFolder,
    DisplaySettings,
    Personalize,
}

impl DesktopBackgroundAction {
    fn parse(value: &str) -> Option<Self> {
        Some(match value {
            "toggle-icons" => Self::ToggleIcons,
            "small-icons" => Self::SmallIcons,
            "medium-icons" => Self::MediumIcons,
            "large-icons" => Self::LargeIcons,
            "sort-name" => Self::SortName,
            "sort-name-descending" => Self::SortNameDescending,
            "sort-kind" => Self::SortKind,
            "sort-kind-descending" => Self::SortKindDescending,
            "sort-size" => Self::SortSize,
            "sort-size-descending" => Self::SortSizeDescending,
            "sort-modified" => Self::SortModified,
            "sort-modified-ascending" => Self::SortModifiedAscending,
            "manual" => Self::Manual,
            "align-grid" => Self::AlignGrid,
            "auto-arrange" => Self::AutoArrange,
            "folders-first" => Self::FoldersFirst,
            "folders-mixed" => Self::FoldersMixed,
            "refresh" => Self::Refresh,
            "paste" => Self::Paste,
            "new-folder" => Self::NewFolder,
            "display-settings" => Self::DisplaySettings,
            "personalize" => Self::Personalize,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PluginEffect {
    ShowLauncher,
    ShowSettings,
    ShowPluginSurface {
        plugin_id: String,
        surface_id: String,
    },
    HidePluginSurface {
        plugin_id: String,
        surface_id: String,
    },
    SetPluginSetting {
        plugin_id: String,
        key: String,
        value: serde_json::Value,
    },
    DesktopSelect {
        id: String,
    },
    DesktopMove {
        id: String,
        dx: f32,
        dy: f32,
    },
    DesktopOpen {
        id: String,
    },
    DesktopFileAction {
        id: String,
        action: nickel_file::desktop::DesktopContextAction,
    },
    DesktopBackgroundAction(DesktopBackgroundAction),
    RunSubmit(String),
    RunDismiss,
    ToggleLauncher,
    ToggleOnScreenKeyboard,
    KeyboardKey {
        id: String,
        generation: u64,
    },
    KeyboardHide {
        generation: u64,
    },
    KeyboardDock {
        generation: u64,
    },
    KeyboardHold {
        generation: u64,
    },
    KeyboardResize {
        delta: i32,
        generation: u64,
    },
    ScreenshotAction {
        action: crate::screenshot::ToolbarAction,
        generation: u64,
    },
    ToggleCodexProjects,
    CodexProjectRefresh,
    CodexProjectClose,
    CodexProjectOpen {
        token: String,
        revision: ProjectMenuRevision,
    },
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
    InvokeTaskbarExtensionAction {
        plugin_id: String,
        id: String,
        application_id: Option<String>,
    },
    InvokePluginSlotAction {
        target_plugin: String,
        slot_id: String,
        plugin_id: String,
        id: String,
    },
    InvokeControlExtensionSection {
        plugin_id: String,
        id: String,
    },
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
    Button { click: usize, drag: usize },
    TileMove(usize, f32, f32),
    FileAction(usize, String),
    Context(usize),
    Drag(usize, DragGesture),
    Text(usize, String),
    Scroll,
}

fn map_plugin_drag(seed: PluginMessage, gesture: DragGesture) -> PluginMessage {
    let PluginMessage::Button { drag, .. } = seed else {
        unreachable!("plugin drag seed retains its handler")
    };
    PluginMessage::Drag(drag, gesture)
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
    pub actions: Vec<TaskbarMenuPluginAction>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TaskbarMenuPluginAction {
    pub plugin_id: String,
    pub id: String,
    pub label: String,
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
            "actions": self.actions.iter().map(|action| serde_json::json!({
                "plugin": action.plugin_id,
                "id": action.id,
                "label": action.label,
            })).collect::<Vec<_>>(),
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

/// Minimal host-owned data for validating a surface without a running shell.
/// These fixtures exercise the same component parser as the live projection;
/// they contain no user windows, files, or device state.
fn validation_surface_projection(
    package: &PluginPackage,
    surface: &PluginSurface,
) -> Option<Value> {
    let id = package.manifest.id.as_str();
    if surface.kind == PluginSurfaceKind::Desktop || id == desktop_manifest().id {
        return Some(serde_json::json!({
            "width": surface.width,
            "height": surface.height,
            "background": 0xff202124_u32,
            "wallpaper": false,
            "surfaceColor": 0xff30343a_u32,
            "text": 0xffffffff_u32,
            "error": null,
            "tiles": [],
            "widgets": [],
            "context": null,
        }));
    }
    if id == launcher_manifest().id {
        return Some(serde_json::json!({
            "query": "",
            "status": null,
            "dashboardVisible": false,
            "view": "favorites",
            "resultPage": 0,
            "resultPageCount": 1,
            "dashboardPage": 0,
            "dashboardPageCount": 1,
            "results": [],
            "dashboard": [],
            "places": [],
            "projects": [],
            "codexAvailable": false,
            "accountName": "User",
            "logoutAvailable": false,
        }));
    }
    if id == control_center_manifest().id {
        return Some(serde_json::json!({
            "height": surface.height,
            "scrollHeight": surface.height.saturating_sub(48).max(1),
            "network": {"available": false, "enabled": false, "networks": []},
            "bluetooth": {"available": false, "powered": false, "discovering": false, "devices": []},
            "audio": {"muted": false, "percent": 50, "devices": []},
            "workspaces": [],
            "activeWorkspace": 0,
            "sections": [],
            "pendingProjection": false,
            "projectionModes": [],
        }));
    }
    if id == on_screen_keyboard_manifest().id {
        return Some(serde_json::json!({
            "generation": 1,
            "height": surface.height,
            "dockTop": false,
            "recipientAvailable": false,
            "rows": nickel_core::on_screen_keyboard::keyboard_display_rows(
                nickel_core::on_screen_keyboard::KeyboardPanel::Letters,
                nickel_core::on_screen_keyboard::VirtualModifiers::default(),
                true,
                false,
            ),
        }));
    }
    if id == window_preview_manifest().id {
        return Some(serde_json::json!({ "windows": [] }));
    }
    if id == volume_osd_manifest().id {
        return Some(serde_json::json!({ "label": "Volume", "percent": 50 }));
    }
    None
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
            let source = bundled_source(manifest(), &manifest().entry, source)?;
            Self::new(source.as_ref())
        }
    }

    pub fn new(source: &str) -> Result<Self, String> {
        Self::new_with_manifest(source, manifest(), None)
    }

    pub fn from_package(package: &PluginPackage) -> Result<Self, String> {
        let mut application = Self::new_with_manifest(&package.source, &package.manifest, None)?;
        application.stylesheet = StyleSheet::compile(&package.stylesheet)?;
        application.sync_images(package_images(package)?);
        Ok(application)
    }

    pub fn from_package_with_settings(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Self, String> {
        let data = serde_json::json!({ "settings": settings, "slots": {} }).to_string();
        let mut application =
            Self::new_with_manifest(&package.source, &package.manifest, Some(data))?;
        application.stylesheet = StyleSheet::compile(&package.stylesheet)?;
        application.sync_images(package_images(package)?);
        Ok(application)
    }

    pub fn from_package_surface(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
        surface: &PluginSurface,
    ) -> Result<Self, String> {
        Self::from_package_surface_with_images(package, settings, surface, package_images(package)?)
    }

    pub(crate) fn from_package_surface_with_images(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
        surface: &PluginSurface,
        images: PluginImages,
    ) -> Result<Self, String> {
        let data = serde_json::json!({
            "settings": settings,
            "slots": {},
            "surface": {
                "id": surface.id,
                "kind": surface.kind.as_str(),
                "width": surface.width,
                "height": surface.height,
            },
        })
        .to_string();
        let mut application = Self::new_with_manifest_for_surface(
            &package.source,
            &package.manifest,
            Some(data),
            Some(&surface.id),
        )?;
        application.stylesheet = StyleSheet::compile(&package.stylesheet)?;
        application.sync_images(images);
        Ok(application)
    }

    pub fn validate_package(package: &PluginPackage) -> Result<(), String> {
        StyleSheet::compile(&package.stylesheet)?;
        package_images(package)?;
        let settings: std::collections::BTreeMap<_, _> = package
            .manifest
            .settings
            .iter()
            .map(|setting| (setting.id.clone(), setting.kind.default_value()))
            .collect();
        if package.manifest.surfaces.is_empty() {
            let application = Self::from_package_with_settings(package, &settings)?;
            if !package.manifest.contributes.is_empty() {
                application.validate_contribution()?;
            }
        } else {
            for surface in &package.manifest.surfaces {
                let mut data = serde_json::json!({
                    "settings": settings,
                    "slots": {},
                    "surface": {
                        "id": surface.id,
                        "kind": surface.kind.as_str(),
                        "width": surface.width,
                        "height": surface.height,
                    },
                });
                if let Some(projection) = validation_surface_projection(package, surface) {
                    data.as_object_mut()
                        .expect("validation data is an object")
                        .extend(
                            projection
                                .as_object()
                                .expect("projection is an object")
                                .clone(),
                        );
                }
                Self::new_with_manifest_for_surface(
                    &package.source,
                    &package.manifest,
                    Some(data.to_string()),
                    Some(&surface.id),
                )
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

    pub fn desktop_widgets(&self) -> Result<Vec<DesktopPluginWidget>, String> {
        let mut widgets = Vec::new();
        self.node.collect_desktop_widgets(&mut widgets)?;
        if widgets.is_empty() {
            return Err("desktop widget extension did not return a widget".into());
        }
        Ok(widgets)
    }

    pub fn taskbar_actions(&self) -> Result<Vec<TaskbarPluginAction>, String> {
        let mut actions = Vec::new();
        self.node.collect_taskbar_actions(&mut actions)?;
        if actions.is_empty() {
            return Err("taskbar action extension did not return an action".into());
        }
        Ok(actions)
    }

    pub fn retained_contribution_bytes(&self) -> u64 {
        self.node.contribution_bytes() + self.stylesheet.estimated_retained_bytes()
    }

    pub fn control_sections(&self) -> Result<Vec<ControlPluginSection>, String> {
        let mut sections = Vec::new();
        self.node.collect_control_sections(&mut sections)?;
        if sections.is_empty() {
            return Err("control section extension did not return a section".into());
        }
        Ok(sections)
    }

    pub fn activate_control_section(&mut self, id: &str) -> bool {
        fn find(node: &PanelNode, id: &str) -> Option<usize> {
            match node {
                PanelNode::Section {
                    id: section_id,
                    action,
                    ..
                } if section_id == id => Some(*action),
                PanelNode::Row { children, .. } | PanelNode::Column { children, .. } => {
                    children.iter().find_map(|child| find(child, id))
                }
                _ => None,
            }
        }
        let Some(action) = find(&self.node, id) else {
            return false;
        };
        nickel_ui::Application::update(self, PluginMessage::Click(action));
        self.last_error.is_none()
    }

    pub fn activate_taskbar_action(&mut self, id: &str, application_id: &str) -> bool {
        let Some(action) = self.find_taskbar_action(id, application_id) else {
            return false;
        };
        nickel_ui::Application::update(
            self,
            PluginMessage::Text(action, application_id.to_owned()),
        );
        self.last_error.is_none()
    }

    fn find_taskbar_action(&self, id: &str, application_id: &str) -> Option<usize> {
        fn find(node: &PanelNode, id: &str, application_id: &str) -> Option<usize> {
            match node {
                PanelNode::Action {
                    id: action_id,
                    item,
                    action,
                    ..
                } if action_id == id
                    && item.as_deref().is_none_or(|item| item == application_id) =>
                {
                    Some(*action)
                }
                PanelNode::Row { children, .. } | PanelNode::Column { children, .. } => children
                    .iter()
                    .find_map(|child| find(child, id, application_id)),
                _ => None,
            }
        }
        find(&self.node, id, application_id)
    }

    pub fn validate_contribution(&self) -> Result<(), String> {
        use nickel_core::plugins::PluginSlotContract;
        let [contribution] = self.manifest.contributes.as_slice() else {
            return Err("extension needs exactly one contribution".into());
        };
        match contribution.contract {
            PluginSlotContract::Badge => self.taskbar_badges().map(|_| ()),
            PluginSlotContract::Widget => self.desktop_widgets().map(|_| ()),
            PluginSlotContract::Action => {
                let actions = self.taskbar_actions()?;
                if contribution.target_plugin != taskbar_manifest().id
                    && actions.iter().any(|action| action.item.is_some())
                {
                    return Err("generic action slots cannot target a taskbar item".into());
                }
                Ok(())
            }
            PluginSlotContract::Section => self.control_sections().map(|_| ()),
        }
    }

    pub fn launcher(launcher: &Launcher) -> Result<Self, String> {
        Self::launcher_with_projection(&LauncherPluginProjection::from_launcher(launcher))
    }

    pub fn launcher_with_projection(projection: &LauncherPluginProjection) -> Result<Self, String> {
        let source = bundled_source(
            launcher_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/launcher/main.js"),
        )?;
        let data = projection.to_json();
        let mut application =
            Self::new_with_manifest(source.as_ref(), launcher_manifest(), Some(data))?;
        application.stylesheet = bundled_stylesheet(
            launcher_manifest(),
            include_str!("../../../assets/plugins/launcher/ui.css"),
        )?;
        application.launcher_shortcuts = Some(projection.into());
        Ok(application)
    }

    #[cfg(test)]
    pub(crate) fn launcher_with_test_source(
        source: &str,
        projection: &LauncherPluginProjection,
    ) -> Result<Self, String> {
        let mut application =
            Self::new_with_manifest(source, launcher_manifest(), Some(projection.to_json()))?;
        application.launcher_shortcuts = Some(projection.into());
        Ok(application)
    }

    pub fn taskbar_with_projection(projection: &TaskbarPluginProjection) -> Result<Self, String> {
        let source = bundled_source(
            taskbar_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/taskbar/main.js"),
        )?;
        let mut application = Self::new_with_manifest(
            source.as_ref(),
            taskbar_manifest(),
            Some(projection.to_json()),
        )?;
        application.stylesheet = bundled_stylesheet(
            taskbar_manifest(),
            include_str!("../../../assets/plugins/taskbar/ui.css"),
        )?;
        Ok(application)
    }

    #[cfg(test)]
    pub(crate) fn taskbar_with_test_source(
        source: &str,
        projection: &TaskbarPluginProjection,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, taskbar_manifest(), Some(projection.to_json()))
    }

    pub fn taskbar_menu_with_projection(
        projection: &TaskbarMenuPluginProjection,
    ) -> Result<Self, String> {
        let source = bundled_source(
            taskbar_manifest(),
            "menu.js",
            include_str!("../../../assets/plugins/taskbar/menu.js"),
        )?;
        Self::new_with_manifest(
            source.as_ref(),
            taskbar_manifest(),
            Some(projection.to_json()),
        )
    }

    pub fn taskbar_window_menu_with_projection(
        projection: &TaskbarWindowMenuPluginProjection,
    ) -> Result<Self, String> {
        let source = bundled_source(
            taskbar_manifest(),
            "window-menu.js",
            include_str!("../../../assets/plugins/taskbar/window-menu.js"),
        )?;
        Self::new_with_manifest(
            source.as_ref(),
            taskbar_manifest(),
            Some(projection.to_json()),
        )
    }

    pub fn notification_with_projection(
        projection: &NotificationPluginProjection,
    ) -> Result<Self, String> {
        let source = bundled_source(
            notification_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/notification/main.js"),
        )?;
        let mut application = Self::new_with_manifest(
            source.as_ref(),
            notification_manifest(),
            Some(projection.to_json()),
        )?;
        application.stylesheet = bundled_stylesheet(
            notification_manifest(),
            include_str!("../../../assets/plugins/notification/ui.css"),
        )?;
        application.notification_shortcuts = Some((
            projection.notification.as_ref().map(|item| item.id),
            projection.history_visible,
        ));
        Ok(application)
    }

    #[cfg(test)]
    pub(crate) fn notification_with_test_source(
        source: &str,
        projection: &NotificationPluginProjection,
    ) -> Result<Self, String> {
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
        let source = bundled_source(
            volume_osd_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/volume-osd/main.js"),
        )?;
        Self::new_with_manifest(
            source.as_ref(),
            volume_osd_manifest(),
            Some(projection.to_json()),
        )
    }

    #[cfg(test)]
    pub(crate) fn volume_osd_with_test_source(
        source: &str,
        projection: &VolumeOsdPluginProjection,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, volume_osd_manifest(), Some(projection.to_json()))
    }

    pub fn control_center_with_data(data: &Value) -> Result<Self, String> {
        let source = bundled_source(
            control_center_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/control-center/main.js"),
        )?;
        Self::new_with_manifest(
            source.as_ref(),
            control_center_manifest(),
            Some(data.to_string()),
        )
    }

    #[cfg(test)]
    pub(crate) fn control_center_with_test_source(
        source: &str,
        data: &Value,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, control_center_manifest(), Some(data.to_string()))
    }

    pub fn codex_projects_with_projection(
        projection: &ProjectMenuProjection,
    ) -> Result<Self, String> {
        let source = bundled_source(
            codex_projects_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/codex-projects/main.js"),
        )?;
        Self::new_with_manifest(
            source.as_ref(),
            codex_projects_manifest(),
            Some(serde_json::to_string(projection).map_err(|error| error.to_string())?),
        )
    }

    pub fn on_screen_keyboard_with_data(data: &Value) -> Result<Self, String> {
        let source = bundled_source(
            on_screen_keyboard_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/on-screen-keyboard/main.js"),
        )?;
        Self::new_with_manifest(
            source.as_ref(),
            on_screen_keyboard_manifest(),
            Some(data.to_string()),
        )
    }

    pub fn screenshot_with_data(data: &Value) -> Result<Self, String> {
        let source = bundled_source(
            screenshot_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/screenshot/main.js"),
        )?;
        Self::new_with_manifest(
            source.as_ref(),
            screenshot_manifest(),
            Some(data.to_string()),
        )
    }

    #[cfg(test)]
    pub(crate) fn on_screen_keyboard_with_test_source(
        source: &str,
        data: &Value,
    ) -> Result<Self, String> {
        Self::new_with_manifest(
            source,
            on_screen_keyboard_manifest(),
            Some(data.to_string()),
        )
    }

    #[cfg(test)]
    pub(crate) fn codex_projects_with_test_source(
        source: &str,
        projection: &ProjectMenuProjection,
    ) -> Result<Self, String> {
        let data = serde_json::to_string(projection).map_err(|error| error.to_string())?;
        Self::new_with_manifest(source, codex_projects_manifest(), Some(data))
    }

    pub fn sync_screenshot_data(&mut self, data: &Value) -> Result<bool, String> {
        if self.manifest.id != screenshot_manifest().id {
            return Err("this plugin is not the screenshot tool".into());
        }
        let serialized = data.to_string();
        if self.projection_data.as_deref() == Some(serialized.as_str()) {
            return Ok(false);
        }
        self.runtime.set_data(&serialized)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
        self.projection_data = Some(serialized);
        Ok(true)
    }

    pub fn sync_on_screen_keyboard_data(&mut self, data: &Value) -> Result<bool, String> {
        if self.manifest.id != on_screen_keyboard_manifest().id {
            return Err("this plugin is not the on-screen keyboard".into());
        }
        let serialized = data.to_string();
        if self.projection_data.as_deref() == Some(serialized.as_str()) {
            return Ok(false);
        }
        self.runtime.set_data(&serialized)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
        self.projection_data = Some(serialized);
        Ok(true)
    }

    pub fn sync_codex_projects_projection(
        &mut self,
        projection: &ProjectMenuProjection,
    ) -> Result<bool, String> {
        if self.manifest.id != codex_projects_manifest().id {
            return Err("this plugin is not Codex projects".into());
        }
        let data = serde_json::to_string(projection).map_err(|error| error.to_string())?;
        if self.projection_data.as_deref() == Some(data.as_str()) {
            return Ok(false);
        }
        self.runtime.set_data(&data)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
        self.projection_data = Some(data);
        Ok(true)
    }

    pub fn window_preview_with_data(data: &Value) -> Result<Self, String> {
        let source = bundled_source(
            window_preview_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/window-preview/main.js"),
        )?;
        Self::new_with_manifest(
            source.as_ref(),
            window_preview_manifest(),
            Some(data.to_string()),
        )
    }

    #[cfg(test)]
    pub(crate) fn window_preview_with_test_source(
        source: &str,
        data: &Value,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, window_preview_manifest(), Some(data.to_string()))
    }

    pub fn desktop_with_data(data: &Value) -> Result<Self, String> {
        let source = bundled_source(
            desktop_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/desktop/main.js"),
        )?;
        Self::new_with_manifest(source.as_ref(), desktop_manifest(), Some(data.to_string()))
    }

    #[cfg(test)]
    pub(crate) fn desktop_with_test_source(source: &str, data: &Value) -> Result<Self, String> {
        Self::new_with_manifest(source, desktop_manifest(), Some(data.to_string()))
    }

    pub fn run_with_status(status: Option<&str>) -> Result<Self, String> {
        let source = bundled_source(
            run_manifest(),
            "main.js",
            include_str!("../../../assets/plugins/run/main.js"),
        )?;
        let data = serde_json::json!({ "status": status }).to_string();
        Self::new_with_manifest(source.as_ref(), run_manifest(), Some(data))
    }

    #[cfg(test)]
    pub(crate) fn run_with_test_source(source: &str) -> Result<Self, String> {
        Self::new_with_manifest(
            source,
            run_manifest(),
            Some(serde_json::json!({ "status": null }).to_string()),
        )
    }

    pub fn sync_run_status(&mut self, status: Option<&str>) -> Result<bool, String> {
        if self.manifest.id != run_manifest().id {
            return Err("this plugin is not the Run dialog".into());
        }
        let data = serde_json::json!({ "status": status }).to_string();
        if self.projection_data.as_deref() == Some(data.as_str()) {
            return Ok(false);
        }
        self.runtime.set_data(&data)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
        self.projection_data = Some(data);
        Ok(true)
    }

    fn new_with_manifest(
        source: &str,
        manifest: &PluginManifest,
        data: Option<String>,
    ) -> Result<Self, String> {
        Self::new_with_manifest_for_surface(source, manifest, data, None)
    }

    fn new_with_manifest_for_surface(
        source: &str,
        manifest: &PluginManifest,
        data: Option<String>,
        expected_surface_id: Option<&str>,
    ) -> Result<Self, String> {
        let mut runtime = JsxRuntime::new(source, data.as_deref())?;
        let node = render_panel(
            &mut runtime,
            manifest,
            expected_surface_id,
            "__nickelRender()",
        )?;
        Ok(Self {
            runtime,
            node,
            effects: Vec::new(),
            pending_transient: None,
            last_error: None,
            runtime_failure: None,
            manifest: manifest.clone(),
            expected_surface_id: expected_surface_id.map(str::to_owned),
            projection_data: data,
            launcher_shortcuts: None,
            notification_shortcuts: None,
            overlay_open: false,
            images: PluginImages::new(),
            stylesheet: StyleSheet::default(),
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

    pub fn retained_image_bytes(&self) -> u64 {
        let mut seen = std::collections::HashSet::new();
        self.retained_image_allocations()
            .filter(|(address, _)| seen.insert(*address))
            .map(|(_, bytes)| bytes)
            .fold(0_u64, u64::saturating_add)
    }

    pub fn retained_image_allocations(&self) -> impl Iterator<Item = (usize, u64)> + '_ {
        self.images
            .values()
            .map(|(_, image)| (Arc::as_ptr(image) as usize, image.as_raw().len() as u64))
    }

    pub(crate) fn sync_external_slots(&mut self, slots: &Value) -> Result<bool, String> {
        let Some(data) = self.projection_data.as_deref() else {
            return Err("plugin has no external projection".into());
        };
        let mut data: Value = serde_json::from_str(data)
            .map_err(|error| format!("invalid external plugin projection: {error}"))?;
        data.as_object_mut()
            .ok_or("external plugin projection must be an object")?
            .insert("slots".into(), slots.clone());
        let data = data.to_string();
        if self.projection_data.as_deref() == Some(data.as_str()) {
            return Ok(false);
        }
        self.runtime.set_data(&data)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
        self.projection_data = Some(data);
        Ok(true)
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
        self.runtime.set_data(&data)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
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
        self.runtime.set_data(&data)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
        self.projection_data = Some(data);
        Ok(true)
    }

    pub fn rendered_taskbar_item_matches(&self, index: usize, id: &str) -> bool {
        if self.manifest.id != taskbar_manifest().id {
            return false;
        }
        self.projection_data
            .as_deref()
            .and_then(|data| serde_json::from_str::<serde_json::Value>(data).ok())
            .is_some_and(|data| {
                data.get("items")
                    .and_then(serde_json::Value::as_array)
                    .is_some_and(|items| {
                        items.iter().any(|item| {
                            item.get("index").and_then(serde_json::Value::as_u64)
                                == u64::try_from(index).ok()
                                && item.get("id").and_then(serde_json::Value::as_str) == Some(id)
                        })
                    })
            })
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
        self.runtime.set_data(&data)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
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
        self.runtime.set_data(&data)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
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
        self.runtime.set_data(&data)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
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
        self.runtime.set_data(&serialized)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
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
        self.runtime.set_data(&serialized)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
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
        self.runtime.set_data(&serialized)?;
        self.node = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            "__nickelRender()",
        )?;
        self.projection_data = Some(serialized);
        Ok(true)
    }

    pub fn activate_desktop_tile(&mut self, id: &str) -> bool {
        if self.manifest.id != desktop_manifest().id {
            return false;
        }
        let Some(action) = self.node.file_tile_action(id) else {
            return false;
        };
        nickel_ui::Application::update(self, PluginMessage::Click(action));
        self.last_error.is_none()
    }

    pub fn select_desktop_tile(&mut self, id: &str) -> bool {
        if self.manifest.id != desktop_manifest().id {
            return false;
        }
        let Some(action) = self.node.file_tile_select_action(id) else {
            return false;
        };
        nickel_ui::Application::update(self, PluginMessage::Click(action));
        self.last_error.is_none()
    }

    pub fn move_desktop_tile(&mut self, id: &str, dx: f32, dy: f32) -> bool {
        if self.manifest.id != desktop_manifest().id || !dx.is_finite() || !dy.is_finite() {
            return false;
        }
        let Some(action) = self.node.file_tile_move_action(id) else {
            return false;
        };
        nickel_ui::Application::update(self, PluginMessage::TileMove(action, dx, dy));
        self.last_error.is_none()
    }

    pub fn file_action_desktop_tile(
        &mut self,
        id: &str,
        action: nickel_file::desktop::DesktopContextAction,
    ) -> bool {
        if self.manifest.id != desktop_manifest().id {
            return false;
        }
        let Some(handler) = self.node.file_tile_file_action(id) else {
            return false;
        };
        nickel_ui::Application::update(
            self,
            PluginMessage::FileAction(handler, action.as_str().to_owned()),
        );
        self.last_error.is_none()
    }

    pub fn take_effects(&mut self) -> Vec<PluginEffect> {
        std::mem::take(&mut self.effects)
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn take_runtime_failure(&mut self) -> Option<String> {
        self.runtime_failure.take()
    }
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
        if self.manifest.id == control_center_manifest().id && shortcut == Shortcut::Escape {
            self.effects.push(PluginEffect::ToggleControlCenter);
            return nickel_ui::ShortcutOutcome::handled(true);
        }
        if self.manifest.id == codex_projects_manifest().id && shortcut == Shortcut::Escape {
            self.effects.push(PluginEffect::CodexProjectClose);
            return nickel_ui::ShortcutOutcome::handled(true);
        }
        if self.manifest.id == on_screen_keyboard_manifest().id && shortcut == Shortcut::Escape {
            if let Some(generation) = self.projection_data.as_deref().and_then(|data| {
                serde_json::from_str::<Value>(data)
                    .ok()
                    .and_then(|data| data.get("generation").and_then(Value::as_u64))
            }) {
                self.effects.push(PluginEffect::KeyboardHide { generation });
                return nickel_ui::ShortcutOutcome::handled(true);
            }
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
            PluginMessage::Click(action)
            | PluginMessage::Button { click: action, .. }
            | PluginMessage::Context(action) => {
                format!("__nickelDispatch({action})")
            }
            PluginMessage::TileMove(action, dx, dy) => {
                let encoded = serde_json::json!({ "dx": dx, "dy": dy });
                format!("__nickelDispatch({action}, {encoded})")
            }
            PluginMessage::FileAction(action, kind) => {
                let encoded = serde_json::json!({ "action": kind });
                format!("__nickelDispatch({action}, {encoded})")
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
        let rendered = render_panel(
            &mut self.runtime,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            &expression,
        );
        let effects = self.runtime.take_effects();
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
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("show-settings")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::SettingsShow) =>
                        {
                            approved.push(PluginEffect::ShowSettings);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("show-plugin-surface") =>
                        {
                            let Some(surface_id) = effect.get("surfaceId").and_then(Value::as_str)
                            else {
                                self.last_error = Some("plugin surface ID is invalid".into());
                                return;
                            };
                            if !self.manifest.surfaces.iter().any(|surface| {
                                surface.id == surface_id
                                    && matches!(
                                        surface.kind,
                                        nickel_core::plugins::PluginSurfaceKind::Window
                                            | nickel_core::plugins::PluginSurfaceKind::Dialog
                                            | nickel_core::plugins::PluginSurfaceKind::Overlay
                                    )
                            }) {
                                self.last_error = Some(
                                    "plugin window, dialog, or overlay is not declared".into(),
                                );
                                return;
                            }
                            approved.push(PluginEffect::ShowPluginSurface {
                                plugin_id: self.manifest.id.clone(),
                                surface_id: surface_id.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("hide-plugin-surface") =>
                        {
                            let Some(surface_id) = effect.get("surfaceId").and_then(Value::as_str)
                            else {
                                self.last_error = Some("plugin surface ID is invalid".into());
                                return;
                            };
                            if !self.manifest.surfaces.iter().any(|surface| {
                                surface.id == surface_id
                                    && matches!(
                                        surface.kind,
                                        nickel_core::plugins::PluginSurfaceKind::Window
                                            | nickel_core::plugins::PluginSurfaceKind::Dialog
                                            | nickel_core::plugins::PluginSurfaceKind::Overlay
                                    )
                            }) {
                                self.last_error = Some(
                                    "plugin window, dialog, or overlay is not declared".into(),
                                );
                                return;
                            }
                            approved.push(PluginEffect::HidePluginSurface {
                                plugin_id: self.manifest.id.clone(),
                                surface_id: surface_id.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("set-plugin-setting")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::SettingsWrite) =>
                        {
                            let Some(key) = effect.get("key").and_then(Value::as_str) else {
                                self.last_error = Some("plugin setting key is invalid".into());
                                return;
                            };
                            let Some(value) = effect.get("value") else {
                                self.last_error = Some("plugin setting value is missing".into());
                                return;
                            };
                            if !self
                                .manifest
                                .settings
                                .iter()
                                .any(|setting| setting.id == key && setting.kind.accepts(value))
                            {
                                self.last_error = Some("plugin setting value is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::SetPluginSetting {
                                plugin_id: self.manifest.id.clone(),
                                key: key.to_owned(),
                                value: value.clone(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("desktop-background-action")
                            && self.manifest.id == desktop_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::DesktopControl) =>
                        {
                            let Some(action) = effect
                                .get("action")
                                .and_then(Value::as_str)
                                .and_then(DesktopBackgroundAction::parse)
                            else {
                                self.last_error =
                                    Some("desktop background action is invalid".into());
                                return;
                            };
                            if matches!(
                                action,
                                DesktopBackgroundAction::Paste | DesktopBackgroundAction::NewFolder
                            ) && !self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::DesktopFilesManage)
                            {
                                self.last_error =
                                    Some("desktop file management grant is missing".into());
                                return;
                            }
                            if !matches!(
                                self.node.menu("desktop-background-actions"),
                                Some(PanelNode::Menu { open: true, .. })
                            ) {
                                self.last_error =
                                    Some("desktop background menu is not active".into());
                                return;
                            }
                            approved.push(PluginEffect::DesktopBackgroundAction(action));
                        }
                        _ if effect.get("type").and_then(Value::as_str) == Some("desktop-open")
                            && self.manifest.id == desktop_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::DesktopFilesOpen) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("desktop file ID is missing".into());
                                return;
                            };
                            let valid_id = id.split_once(':').is_some_and(|(first, second)| {
                                first.parse::<u64>().is_ok() && second.parse::<u64>().is_ok()
                            });
                            if !valid_id || self.node.file_tile_action(id).is_none() {
                                self.last_error = Some("desktop file ID is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::DesktopOpen { id: id.to_owned() });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("desktop-file-action")
                            && self.manifest.id == desktop_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::DesktopFilesManage) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("desktop file ID is missing".into());
                                return;
                            };
                            let Some(action) = effect
                                .get("action")
                                .and_then(Value::as_str)
                                .and_then(nickel_file::desktop::DesktopContextAction::parse)
                            else {
                                self.last_error = Some("desktop file action is invalid".into());
                                return;
                            };
                            let valid_id = id.split_once(':').is_some_and(|(first, second)| {
                                first.parse::<u64>().is_ok() && second.parse::<u64>().is_ok()
                            });
                            if !valid_id || self.node.file_tile_file_action(id).is_none() {
                                self.last_error = Some("desktop file ID is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::DesktopFileAction {
                                id: id.to_owned(),
                                action,
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("desktop-select")
                            && self.manifest.id == desktop_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::DesktopRead) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("desktop file ID is missing".into());
                                return;
                            };
                            let valid_id = id.split_once(':').is_some_and(|(first, second)| {
                                first.parse::<u64>().is_ok() && second.parse::<u64>().is_ok()
                            });
                            if !valid_id || self.node.file_tile_select_action(id).is_none() {
                                self.last_error = Some("desktop file ID is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::DesktopSelect { id: id.to_owned() });
                        }
                        _ if effect.get("type").and_then(Value::as_str) == Some("desktop-move")
                            && self.manifest.id == desktop_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::DesktopArrange) =>
                        {
                            let Some(id) = effect.get("id").and_then(Value::as_str) else {
                                self.last_error = Some("desktop file ID is missing".into());
                                return;
                            };
                            let valid_id = id.split_once(':').is_some_and(|(first, second)| {
                                first.parse::<u64>().is_ok() && second.parse::<u64>().is_ok()
                            });
                            let bounded_delta = |name| {
                                effect.get(name).and_then(Value::as_f64).filter(|delta| {
                                    delta.is_finite() && (-8192.0..=8192.0).contains(delta)
                                })
                            };
                            let (Some(dx), Some(dy)) = (bounded_delta("dx"), bounded_delta("dy"))
                            else {
                                self.last_error = Some("desktop move delta is invalid".into());
                                return;
                            };
                            if !valid_id || self.node.file_tile_move_action(id).is_none() {
                                self.last_error = Some("desktop file ID is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::DesktopMove {
                                id: id.to_owned(),
                                dx: dx as f32,
                                dy: dy as f32,
                            });
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
                            == Some("taskbar-extension-action")
                            && self.manifest.id == taskbar_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::WindowsContext) =>
                        {
                            let plugin_id = effect.get("plugin").and_then(Value::as_str);
                            let id = effect.get("id").and_then(Value::as_str);
                            let application_id =
                                effect.get("applicationId").and_then(Value::as_str);
                            let valid = plugin_id
                                .is_some_and(|value| !value.is_empty() && value.len() <= 128)
                                && id.is_some_and(|value| !value.is_empty() && value.len() <= 64)
                                && application_id
                                    .is_none_or(|value| !value.is_empty() && value.len() <= 256);
                            if !valid {
                                self.last_error =
                                    Some("taskbar extension action is invalid".into());
                                return;
                            }
                            let projection = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok());
                            let projected = projection.as_ref().is_some_and(|data| {
                                data.get("applicationId").and_then(Value::as_str) == application_id
                                    && data.get("actions").and_then(Value::as_array).is_some_and(
                                        |actions| {
                                            actions.iter().any(|action| {
                                                action.get("plugin").and_then(Value::as_str)
                                                    == plugin_id
                                                    && action.get("id").and_then(Value::as_str)
                                                        == id
                                            })
                                        },
                                    )
                            });
                            if !projected {
                                self.last_error = Some("taskbar extension action is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::InvokeTaskbarExtensionAction {
                                plugin_id: plugin_id.unwrap().to_owned(),
                                id: id.unwrap().to_owned(),
                                application_id: application_id.map(str::to_owned),
                            });
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
                            == Some("invoke-plugin-slot-action") =>
                        {
                            let slot_id = effect.get("slot").and_then(Value::as_str);
                            let plugin_id = effect.get("pluginId").and_then(Value::as_str);
                            let id = effect.get("id").and_then(Value::as_str);
                            let valid = slot_id.is_some_and(|value| {
                                self.manifest.provides_slots.iter().any(|slot| {
                                    slot.id == value
                                        && slot.contract
                                            == nickel_core::plugins::PluginSlotContract::Action
                                })
                            }) && plugin_id
                                .is_some_and(|value| !value.is_empty() && value.len() <= 128)
                                && id.is_some_and(|value| !value.is_empty() && value.len() <= 64);
                            if !valid {
                                self.last_error = Some("plugin slot action is invalid".into());
                                return;
                            }
                            let projected = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok())
                                .and_then(|data| {
                                    data.get("slots")?.get(slot_id?)?.as_array().cloned()
                                })
                                .is_some_and(|actions| {
                                    actions.iter().any(|action| {
                                        action.get("pluginId").and_then(Value::as_str) == plugin_id
                                            && action.get("id").and_then(Value::as_str) == id
                                    })
                                });
                            if !projected {
                                self.last_error = Some("plugin slot action is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::InvokePluginSlotAction {
                                target_plugin: self.manifest.id.clone(),
                                slot_id: slot_id.unwrap().to_owned(),
                                plugin_id: plugin_id.unwrap().to_owned(),
                                id: id.unwrap().to_owned(),
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
                            == Some("control-extension-section")
                            && self.manifest.id == control_center_manifest().id =>
                        {
                            let plugin_id = effect.get("plugin").and_then(Value::as_str);
                            let id = effect.get("id").and_then(Value::as_str);
                            if !plugin_id
                                .is_some_and(|value| !value.is_empty() && value.len() <= 128)
                                || !id.is_some_and(|value| !value.is_empty() && value.len() <= 64)
                            {
                                self.last_error =
                                    Some("control extension section is invalid".into());
                                return;
                            }
                            let projected = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok())
                                .and_then(|data| {
                                    data.get("sections").and_then(Value::as_array).cloned()
                                })
                                .is_some_and(|sections| {
                                    sections.iter().any(|section| {
                                        section.get("plugin").and_then(Value::as_str) == plugin_id
                                            && section.get("id").and_then(Value::as_str) == id
                                    })
                                });
                            if !projected {
                                self.last_error = Some("control extension section is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::InvokeControlExtensionSection {
                                plugin_id: plugin_id.unwrap().to_owned(),
                                id: id.unwrap().to_owned(),
                            });
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
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("codex-project-refresh")
                            && self.manifest.id == codex_projects_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ProjectsRead) =>
                        {
                            approved.push(PluginEffect::CodexProjectRefresh);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("codex-project-close")
                            && self.manifest.id == codex_projects_manifest().id =>
                        {
                            approved.push(PluginEffect::CodexProjectClose);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("codex-project-open")
                            && self.manifest.id == codex_projects_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ProjectsOpen) =>
                        {
                            let token = effect.get("id").and_then(Value::as_str);
                            let revision = effect.get("revision").cloned().and_then(|value| {
                                serde_json::from_value::<ProjectMenuRevision>(value).ok()
                            });
                            let projected = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok());
                            let valid = token.is_some_and(|token| {
                                token.len() <= 3
                                    && token.parse::<usize>().ok().is_some_and(|index| index < 100)
                                    && projected.as_ref().is_some_and(|data| {
                                        data.get("status").and_then(Value::as_str) == Some("ready")
                                            && data.get("revision").cloned().and_then(|value| {
                                                serde_json::from_value::<ProjectMenuRevision>(value)
                                                    .ok()
                                            }) == revision
                                            && data
                                                .get("projects")
                                                .and_then(Value::as_array)
                                                .is_some_and(|projects| {
                                                    projects.iter().any(|project| {
                                                        project.get("id").and_then(Value::as_str)
                                                            == Some(token)
                                                    })
                                                })
                                    })
                            });
                            if !valid {
                                self.last_error =
                                    Some("Codex project request is stale or invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::CodexProjectOpen {
                                token: token.unwrap().to_owned(),
                                revision: revision.unwrap(),
                            });
                        }
                        _ if self.manifest.id == on_screen_keyboard_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::OnScreenKeyboardInput)
                            && matches!(
                                effect.get("type").and_then(Value::as_str),
                                Some(
                                    "keyboard-key"
                                        | "keyboard-hide"
                                        | "keyboard-dock"
                                        | "keyboard-hold"
                                        | "keyboard-resize"
                                )
                            ) =>
                        {
                            let projected = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok());
                            let generation = effect.get("generation").and_then(Value::as_u64);
                            if generation.is_none()
                                || generation
                                    != projected.as_ref().and_then(|data| {
                                        data.get("generation").and_then(Value::as_u64)
                                    })
                            {
                                self.last_error = Some("keyboard request is stale".into());
                                return;
                            }
                            let generation = generation.unwrap();
                            match effect.get("type").and_then(Value::as_str) {
                                Some("keyboard-key") => {
                                    let id = effect.get("id").and_then(Value::as_str);
                                    let displayed = id.is_some_and(|id| {
                                        id.len() <= 64
                                            && projected.as_ref().is_some_and(|data| {
                                                data.get("rows")
                                                    .and_then(Value::as_array)
                                                    .is_some_and(|rows| {
                                                        rows.iter().any(|row| {
                                                            row.as_array().is_some_and(|keys| {
                                                                keys.iter().any(|key| {
                                                                    key.get("id")
                                                                        .and_then(Value::as_str)
                                                                        == Some(id)
                                                                        && key
                                                                            .get("enabled")
                                                                            .and_then(
                                                                                Value::as_bool,
                                                                            )
                                                                            == Some(true)
                                                                })
                                                            })
                                                        })
                                                    })
                                            })
                                    });
                                    if !displayed {
                                        self.last_error =
                                            Some("keyboard key is unavailable".into());
                                        return;
                                    }
                                    approved.push(PluginEffect::KeyboardKey {
                                        id: id.unwrap().to_owned(),
                                        generation,
                                    });
                                }
                                Some("keyboard-hide") => {
                                    approved.push(PluginEffect::KeyboardHide { generation });
                                }
                                Some("keyboard-dock") => {
                                    approved.push(PluginEffect::KeyboardDock { generation });
                                }
                                Some("keyboard-hold") => {
                                    approved.push(PluginEffect::KeyboardHold { generation });
                                }
                                Some("keyboard-resize") => {
                                    let delta = effect.get("delta").and_then(Value::as_i64);
                                    if !matches!(delta, Some(-32 | 32)) {
                                        self.last_error = Some("keyboard resize is invalid".into());
                                        return;
                                    }
                                    approved.push(PluginEffect::KeyboardResize {
                                        delta: delta.unwrap() as i32,
                                        generation,
                                    });
                                }
                                _ => unreachable!(),
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("screenshot-action")
                            && self.manifest.id == screenshot_manifest().id
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ScreenshotControl) =>
                        {
                            let projected = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok());
                            let generation = effect.get("generation").and_then(Value::as_u64);
                            let visible = projected.as_ref().is_some_and(|data| {
                                data.get("imageAvailable").and_then(Value::as_bool) == Some(true)
                                    || data.get("errorVisible").and_then(Value::as_bool)
                                        == Some(true)
                            });
                            if !visible
                                || generation.is_none()
                                || generation
                                    != projected.as_ref().and_then(|data| {
                                        data.get("generation").and_then(Value::as_u64)
                                    })
                            {
                                self.last_error = Some("screenshot request is stale".into());
                                return;
                            }
                            let action = match effect.get("action").and_then(Value::as_str) {
                                Some("copy") => crate::screenshot::ToolbarAction::Copy,
                                Some("save") => crate::screenshot::ToolbarAction::Save,
                                Some("temporary-path") => {
                                    crate::screenshot::ToolbarAction::TemporaryPath
                                }
                                Some("cancel") => crate::screenshot::ToolbarAction::Cancel,
                                _ => {
                                    self.last_error = Some("unknown screenshot action".into());
                                    return;
                                }
                            };
                            if action != crate::screenshot::ToolbarAction::Cancel
                                && projected
                                    .as_ref()
                                    .and_then(|data| data.get("confirmed").and_then(Value::as_bool))
                                    != Some(true)
                            {
                                self.last_error =
                                    Some("screenshot selection is not confirmed".into());
                                return;
                            }
                            approved.push(PluginEffect::ScreenshotAction {
                                action,
                                generation: generation.unwrap(),
                            });
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
            (Err(error), _) | (_, Err(error)) => {
                self.runtime_failure = Some(error.clone());
                self.last_error = Some(error);
            }
        })();
        if let Err(error) = self.runtime.finish_event(self.last_error.is_none()) {
            self.runtime_failure = Some(error.clone());
            self.last_error = Some(error);
        }
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        if matches!(
            &self.node,
            PanelNode::Viewport { .. } | PanelNode::Surface { .. }
        ) || self.manifest.id == taskbar_manifest().id
            || self.manifest.id == run_manifest().id
            || self.manifest.id == window_preview_manifest().id
            || self.manifest.id == desktop_manifest().id
        {
            AnyView::new(self.node.view(&self.images, &self.stylesheet))
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
                            .child(self.node.view(&self.images, &self.stylesheet))
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
                        content = content.child(child.view(&self.images, &self.stylesheet));
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
                    point,
                    open: true,
                    items,
                } => {
                    let mut menu = OverlayMenu::new(
                        format!("plugin-menu-{id}"),
                        point.map_or_else(
                            || OverlayAnchor::Node(UiId::from(anchor.clone())),
                            |point| OverlayAnchor::Point {
                                invocation_target: UiId::from(anchor.clone()),
                                point,
                            },
                        ),
                    );
                    for item in items {
                        if let Some(item) = item.overlay_menu_item() {
                            menu = menu.item(item);
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
    fn staged_bundled_source_reads_isolated_profile() {
        let root = tempfile::tempdir().unwrap();
        let plugin = root.path().join(&launcher_manifest().id);
        std::fs::create_dir_all(&plugin).unwrap();
        std::fs::write(
            plugin.join("main.js"),
            "function App() { return h(Panel, {}); }",
        )
        .unwrap();
        assert_eq!(
            read_bundled_source(root.path(), launcher_manifest(), "main.js").unwrap(),
            "function App() { return h(Panel, {}); }"
        );
        assert!(read_bundled_source(root.path(), launcher_manifest(), "menu.js").is_err());
    }

    #[test]
    fn bundled_plugin_packages_validate_with_surface_projection_fixtures() {
        for name in [
            "hello-panel",
            "taskbar",
            "launcher",
            "desktop",
            "notification",
            "run",
            "control-center",
            "window-preview",
            "volume-osd",
        ] {
            let directory = format!("{}/../../assets/plugins/{name}", env!("CARGO_MANIFEST_DIR"));
            let package = PluginPackage::load(directory).unwrap();
            PluginPanelApplication::validate_package(&package)
                .unwrap_or_else(|error| panic!("{name} validation failed: {error}"));
        }
    }

    #[test]
    fn jsx_menu_items_preserve_submenus_disabled_reasons_and_shortcuts() {
        let source = r#"function App() { return h(Panel, {height: 80},
            h(Menu, {id: 'actions', anchor: 'root', open: true},
                h(MenuItem, {id: 'view', label: 'View', separatorBefore: true},
                    h(MenuItem, {id: 'show', shortcut: 'Ctrl+D', onClick: () => nickel.request('show-launcher')}, 'Show'),
                    h(MenuItem, {id: 'paste', disabledReason: 'Clipboard is empty'}, 'Paste')))); }"#;
        let application = PluginPanelApplication::new(source).unwrap();
        let Some(PanelNode::Menu { items, .. }) = application.node.menu("actions") else {
            panic!("JSX menu must be present");
        };
        let view = items[0].overlay_menu_item().unwrap();
        assert_eq!(view.label, "View");
        assert!(view.separator_before);
        assert_eq!(view.children.len(), 2);
        assert_eq!(view.children[0].shortcut.as_deref(), Some("Ctrl+D"));
        assert!(matches!(
            view.children[0].action,
            Some(PluginMessage::Click(_))
        ));
        assert_eq!(
            view.children[1].disabled_reason.as_deref(),
            Some("Clipboard is empty")
        );
        assert!(view.children[1].action.is_none());
    }

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
        assert!(matches!(
            &app.node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        assert_eq!(
            app.stylesheet
                .resolve("window", Some("main"), Some("taskbar"))
                .background,
            Some(0xf2222730)
        );
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
    fn desktop_widget_contribution_validates_and_bounds_its_values() {
        let package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-desktop-widget"
        ))
        .unwrap();
        PluginPanelApplication::validate_package(&package).unwrap();
        let widgets = PluginPanelApplication::from_package(&package)
            .unwrap()
            .desktop_widgets()
            .unwrap();
        assert_eq!(widgets.len(), 1);
        assert_eq!(widgets[0].label, "Unread mail");
        assert_eq!(widgets[0].percent, 60);

        let mut invalid = package.clone();
        invalid.source = invalid.source.replace("Math.min(100, unread * 5)", "101");
        assert_ne!(invalid.source, package.source);
        assert!(PluginPanelApplication::validate_package(&invalid).is_err());
    }

    #[test]
    fn taskbar_action_contribution_dispatches_its_own_granted_callback() {
        let package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-task-action"
        ))
        .unwrap();
        PluginPanelApplication::validate_package(&package).unwrap();
        let mut application = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(application.taskbar_actions().unwrap()[0].id, "find-apps");
        assert!(application.activate_taskbar_action("find-apps", "org.nickel.mail"));
        assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
        assert!(!application.activate_taskbar_action("missing", "org.nickel.mail"));
        assert!(application.take_effects().is_empty());
    }

    #[test]
    fn control_section_contribution_dispatches_its_own_granted_callback() {
        let package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-control-section"
        ))
        .unwrap();
        PluginPanelApplication::validate_package(&package).unwrap();
        let mut application = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(application.control_sections().unwrap()[0].id, "find-apps");
        assert!(application.activate_control_section("find-apps"));
        assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
        assert!(!application.activate_control_section("missing"));
        assert!(application.take_effects().is_empty());
    }

    #[test]
    fn installed_action_slot_dispatches_only_a_projected_contributors_action() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins");
        let provider = PluginPackage::load(format!("{root}/example-widget-host")).unwrap();
        let contributor =
            PluginPackage::load(format!("{root}/example-action-contributor")).unwrap();
        PluginPanelApplication::validate_package(&provider).unwrap();
        PluginPanelApplication::validate_package(&contributor).unwrap();
        let mut invalid = contributor.clone();
        invalid.source = invalid.source.replace(
            "label: \"Open launcher\"",
            "item: \"mail\", label: \"Open launcher\"",
        );
        assert!(
            PluginPanelApplication::from_package(&invalid)
                .unwrap()
                .validate_contribution()
                .unwrap_err()
                .contains("cannot target a taskbar item")
        );
        let mut contributor = PluginPanelApplication::from_package(&contributor).unwrap();
        assert_eq!(
            contributor.taskbar_actions().unwrap()[0].id,
            "open-launcher"
        );
        assert!(contributor.activate_taskbar_action("open-launcher", ""));
        assert_eq!(contributor.take_effects(), vec![PluginEffect::ShowLauncher]);

        let surface = &provider.manifest.surfaces[0];
        let mut app = PluginPanelApplication::from_package_surface(
            &provider,
            &std::collections::BTreeMap::new(),
            surface,
        )
        .unwrap();
        let projected = serde_json::json!({
            "metrics": [],
            "commands": [{"pluginId":"org.example.action-contributor", "id":"open-launcher", "label":"Open launcher"}]
        });
        assert!(app.sync_external_slots(&projected).unwrap());
        let stale_click = app.button_message("slot-action-open-launcher").unwrap();
        let mut host = nickel_ui::UiHost::new(app, surface.width, surface.height);
        let button = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open launcher".into(),
            })
            .unwrap();
        assert!(button.bounds.origin.x < 210.0, "{:?}", button.bounds);
        assert!(
            button.bounds.origin.x + button.bounds.size.width > 210.0,
            "{:?}",
            button.bounds
        );
        assert!(button.bounds.origin.y < 77.0, "{:?}", button.bounds);
        assert!(
            button.bounds.origin.y + button.bounds.size.height > 77.0,
            "{:?}",
            button.bounds
        );
        let app = host.application_mut();
        app.update(stale_click.clone());
        assert_eq!(
            app.take_effects(),
            vec![PluginEffect::InvokePluginSlotAction {
                target_plugin: provider.manifest.id.clone(),
                slot_id: "commands".into(),
                plugin_id: "org.example.action-contributor".into(),
                id: "open-launcher".into(),
            }]
        );
        assert!(
            app.sync_external_slots(&serde_json::json!({"metrics":[], "commands":[]}))
                .unwrap()
        );
        app.update(stale_click);
        assert!(app.take_effects().is_empty());
    }

    #[test]
    fn bundled_desktop_tile_selection_requests_a_typed_effect() {
        let data = serde_json::json!({
            "width": 400, "height": 300, "background": 0xff101820_u32,
            "wallpaper": false, "surfaceColor": 0xff202830_u32,
            "text": 0xfff0f0f0_u32, "error": null, "widgets": [],
            "tiles": [{
                "id": "7:9", "asset": "file", "label": "Example",
                "x": 0, "y": 0, "width": 90, "height": 110,
                "selected": false, "hovered": false, "dragging": false,
                "color": 0xffffffff_u32, "outline": 0xff101820_u32,
                "hoverBackground": 0xff202830_u32,
                "selectedBackground": 0xff304050_u32,
                "accent": 0xff507090_u32, "complement": 0xff90a0b0_u32,
            }],
        });
        let mut application = PluginPanelApplication::desktop_with_data(&data).unwrap();
        assert!(application.select_desktop_tile("7:9"));
        assert_eq!(
            application.take_effects(),
            vec![PluginEffect::DesktopSelect { id: "7:9".into() }]
        );
        assert!(!application.select_desktop_tile("7:10"));
        assert!(application.take_effects().is_empty());
        assert!(application.move_desktop_tile("7:9", 97.0, 0.0));
        assert_eq!(
            application.take_effects(),
            vec![PluginEffect::DesktopMove {
                id: "7:9".into(),
                dx: 97.0,
                dy: 0.0,
            }]
        );
        assert!(!application.move_desktop_tile("7:9", 9000.0, 0.0));
        assert!(application.take_effects().is_empty());
        assert!(
            application.file_action_desktop_tile(
                "7:9",
                nickel_file::desktop::DesktopContextAction::Rename,
            )
        );
        assert_eq!(
            application.take_effects(),
            vec![PluginEffect::DesktopFileAction {
                id: "7:9".into(),
                action: nickel_file::desktop::DesktopContextAction::Rename,
            }]
        );
        assert!(
            !application.file_action_desktop_tile(
                "7:10",
                nickel_file::desktop::DesktopContextAction::Rename,
            )
        );
        assert!(application.take_effects().is_empty());
        application
            .manifest
            .capabilities
            .retain(|capability| *capability != PluginCapability::DesktopFilesManage);
        assert!(
            !application.file_action_desktop_tile(
                "7:9",
                nickel_file::desktop::DesktopContextAction::Rename,
            )
        );
        assert!(application.take_effects().is_empty());
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
    fn retained_image_bytes_counts_shared_image_once() {
        let mut app =
            PluginPanelApplication::new("function App() { return h(Panel, {}); }").unwrap();
        let shared = Arc::new(image::RgbaImage::new(8, 8));
        let mut images = PluginImages::new();
        images.insert("first".into(), (1, Arc::clone(&shared)));
        images.insert("second".into(), (2, Arc::clone(&shared)));
        app.sync_images(images);
        assert_eq!(app.retained_image_bytes(), shared.as_raw().len() as u64);
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
        assert!(
            panel
                .take_runtime_failure()
                .unwrap()
                .contains("handler failed")
        );
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
        assert!(panel.take_runtime_failure().is_none());
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
            images: Default::default(),
            stylesheet: String::new(),
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
    fn plugin_css_styles_controls_without_changing_actions_or_text_editing() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.css-controls".into();
        let package = PluginPackage {
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: "button.primary { background-color: #345678; padding: 8px; border: 2px solid #abc; border-radius: 6px; color: #fff; font-size: 18px; } text-field.entry { background-color: rgba(10, 20, 30, 0.5); padding: 4px; line-height: 24px; }".into(),
            source: "function App() { return h(Panel, {background: 0, height: 120}, h(Button, {id: 'go', className: 'primary', onClick: () => nickel.request('show-launcher')}, 'Go'), h(TextField, {id: 'name', className: 'entry', value: '', onChange: value => {}})); }".into(),
        };
        PluginPanelApplication::validate_package(&package).unwrap();
        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package(&package).unwrap(),
            440,
            120,
        );
        assert!(host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill { color: 0xff345678, radius, .. } if *radius == 4.0
        )));
        assert!(host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill { color: 0xffaabbcc, radius, .. } if *radius == 6.0
        )));
        let button = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Go".into(),
            })
            .unwrap();
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(button.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowLauncher]
        );
        assert!(
            host.query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
                .is_ok()
        );
    }

    #[test]
    fn generic_div_uses_css_grid_tracks_for_plugin_controls() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.css-grid".into();
        let package = PluginPackage {
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: ".grid { display: grid; grid-template-columns: 100px 100px; gap: 10px; width: 210px; } button { width: 100px; }".into(),
            source: "function App() { return h(Panel, {height: 120, background: 0}, h(Div, {className: 'grid'}, h(Button, {id: 'left', onClick: () => nickel.request('show-launcher')}, 'Left'), h(Button, {id: 'right', onClick: () => nickel.request('show-launcher')}, 'Right'))); }".into(),
        };
        PluginPanelApplication::validate_package(&package).unwrap();
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package(&package).unwrap(),
            440,
            120,
        );
        let left = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Left".into(),
            })
            .unwrap();
        let right = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Right".into(),
            })
            .unwrap();
        assert!(right.bounds.origin.x >= left.bounds.origin.x + 100.0);
        assert_eq!(right.bounds.origin.y, left.bounds.origin.y);
    }

    #[test]
    fn fixed_window_jsx_helper_uses_one_manifest_checked_window_root() {
        let mut granted = taskbar_manifest().clone();
        granted.id = "org.example.fixed-window".into();
        let source = "function App() { return h(FixedWindow, {id: 'main', width: '100%', height: 56, output: 'all', edge: 'bottom', reserveWorkArea: true, className: 'bar'}, h(Button, {id: 'open', onClick: () => nickel.request('show-launcher')}, 'Open')); }";
        let mut package = PluginPackage {
            manifest: granted,
            images: Default::default(),
            stylesheet: "window.bar { width: 100%; background: rgba(20, 30, 40, 0.8); }".into(),
            source: source.into(),
        };
        PluginPanelApplication::validate_package(&package).unwrap();
        let app = PluginPanelApplication::from_package(&package).unwrap();
        assert!(matches!(
            app.node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        let host = nickel_ui::UiHost::new(app, 1366, 56);
        assert!(
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Open".into(),
            })
            .is_ok()
        );
        package.source = source.replace("id: 'main', ", "");
        assert!(PluginPanelApplication::from_package(&package).is_ok());

        for invalid in [
            source.replace("id: 'main'", "id: 'other'"),
            source.replace("reserveWorkArea: true", "reserveWorkArea: false"),
            source.replace("height: 56", "height: 64"),
            source.replace("output: 'all'", "output: 'primary'"),
        ] {
            package.source = invalid;
            assert!(PluginPanelApplication::from_package(&package).is_err());
        }

        package.manifest.surfaces[0].kind = PluginSurfaceKind::Window;
        package.manifest.surfaces[0].width = 520;
        package.manifest.surfaces[0].height = 340;
        package.manifest.surfaces[0].output = nickel_core::plugins::PluginOutputScope::Primary;
        package.manifest.surfaces[0].reserve_work_area = false;
        package.source = "function App() { return h(Window, {id: 'main', width: 520, height: 340}, h(Button, {id: 'save', onClick: () => {}}, 'Save')); }".into();
        assert!(PluginPanelApplication::from_package(&package).is_ok());
        let mut sibling = package.manifest.surfaces[0].clone();
        sibling.id = "sibling".into();
        package.manifest.surfaces.push(sibling.clone());
        assert!(
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &sibling,)
                .is_err()
        );
        package.source = "function App() { const id = nickel.data.surface.id; return h(Window, {width: 520, height: 340}, h(Text, {}, id)); }".into();
        for surface in &package.manifest.surfaces {
            assert!(
                PluginPanelApplication::from_package_surface(
                    &package,
                    &Default::default(),
                    surface,
                )
                .is_ok()
            );
        }
        package.source = "function App() { return h(Window, {width: 520, height: 340}, h(TextField, {onChange: () => {}})); }".into();
        assert!(
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &sibling,)
                .is_ok()
        );
        package.source = "function App() { return h(Window, {width: 520, height: 340}, h(Row, {}, ['a', 'b'].map(label => h(Button, {key: label, onClick: () => {}}, label)))); }".into();
        assert!(
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &sibling,)
                .is_ok()
        );
        let duplicate = package.source.replace("['a', 'b']", "['a', 'a']");
        package.source = duplicate;
        assert!(
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &sibling,)
                .is_err()
        );
        package.source = package.source.replace("['a', 'a']", "['a', 'b']");
        package.source = package.source.replace("key: label, ", "");
        assert!(
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &sibling,)
                .is_err()
        );
        package.source = "function App() { return h(Panel, {}, h(Window, {id: 'main', width: 520, height: 340})); }".into();
        assert!(PluginPanelApplication::from_package(&package).is_err());
    }

    #[test]
    fn unstyled_plugin_button_has_no_mandatory_paint() {
        let source = "function App() { return h(Panel, {background: 0}, h(Button, {id: 'go', onClick: () => {}}, 'Go')); }";
        let host = nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 320, 80);
        assert!(!host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill {
                color: 0x66455675,
                ..
            }
        )));
        assert!(
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Go".into(),
            })
            .is_ok()
        );
    }

    #[test]
    fn transparent_css_button_does_not_paint_opaque_black() {
        let source = "function App() { return h(Panel, {background: 0xff112233}, h(Button, {id: 'go', className: 'clear', onClick: () => {}}, 'Go')); }";
        let mut application = PluginPanelApplication::new(source).unwrap();
        application.stylesheet = StyleSheet::compile(
            "button.clear { background: transparent; border: 1px solid transparent; }",
        )
        .unwrap();
        let host = nickel_ui::UiHost::new(application, 320, 80);
        assert!(!host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Fill { color: 0, .. }
                | nickel_ui::backend::PaintCommand::RoundedFill { color: 0, .. }
                | nickel_ui::backend::PaintCommand::Stroke { color: 0, .. }
        )));
    }

    #[test]
    fn installed_viewport_tracks_window_resize_and_keeps_content_inset() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.viewport".into();
        let package = PluginPackage {
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: "function App() { return h(Viewport, {background: 0xff112233, padding: 20}, h(Button, {id: 'open', onClick: () => nickel.request('show-launcher')}, 'Open')); }".into(),
        };
        let app = PluginPanelApplication::from_package(&package).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 400, 200);
        for (width, height) in [(400, 200), (240, 120)] {
            host.step(nickel_ui::HostBatch {
                surface_size: Some((width, height)),
                ..Default::default()
            });
            let button = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Button,
                    name: "Open".into(),
                })
                .unwrap();
            assert_eq!(button.bounds.origin.x, 20.0);
            assert_eq!(button.bounds.origin.y, 20.0);
            assert!(button.bounds.origin.x + button.bounds.size.width <= width as f32);
            assert!(button.bounds.origin.y + button.bounds.size.height <= height as f32);
        }
    }

    #[test]
    fn external_dialog_example_requests_settings_only_with_its_grant() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-dialog"
        );
        let package = PluginPackage::load(directory).unwrap();
        for granted in [true, false] {
            let mut package = package.clone();
            if !granted {
                package.manifest.capabilities.clear();
            }
            let mut host = nickel_ui::UiHost::new(
                PluginPanelApplication::from_package(&package).unwrap(),
                320,
                120,
            );
            for name in ["Open a dialog", "Open Settings"] {
                let button = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: nickel_ui::SemanticRole::Button,
                        name: name.into(),
                    })
                    .unwrap();
                host.step(nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(
                        nickel_ui::UiEvent::AccessibilityActivate(button.id),
                    )],
                    ..Default::default()
                });
            }
            if granted {
                assert_eq!(
                    host.application_mut().take_effects(),
                    vec![PluginEffect::ShowSettings]
                );
            } else {
                assert!(host.application_mut().take_effects().is_empty());
                assert!(host.application_mut().last_error().is_some());
            }
        }
    }

    #[test]
    fn component_window_example_opens_dialog_and_requests_settings() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        );
        let package = PluginPackage::load(directory).unwrap();
        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package(&package).unwrap(),
            520,
            340,
        );
        let open = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open dialog".into(),
            })
            .unwrap();
        let contains = |bounds: nickel_ui::Rect, x: f32, y: f32| {
            x >= bounds.origin.x
                && x < bounds.origin.x + bounds.size.width
                && y >= bounds.origin.y
                && y < bounds.origin.y + bounds.size.height
        };
        assert!(contains(open.bounds, 260.0, 77.0));
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(open.id),
            )],
            ..Default::default()
        });
        assert!(host.inspect().open_overlay.is_some());
        let settings = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open Settings".into(),
            })
            .unwrap();
        assert!(contains(settings.bounds, 80.0, 182.0));
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(settings.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowSettings]
        );
        assert!(host.application_mut().last_error().is_none());
    }

    #[test]
    fn two_window_example_requests_its_declared_sibling() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-two-windows"
        );
        let package = PluginPackage::load(directory).unwrap();
        let home = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "home")
            .unwrap();
        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package_surface(
                &package,
                &std::collections::BTreeMap::new(),
                home,
            )
            .unwrap(),
            home.width,
            home.height,
        );
        let button = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Reopen details".into(),
            })
            .unwrap();
        assert!(button.bounds.size.width > 0.0);
        assert!(button.bounds.size.height > 0.0);
        assert!(button.bounds.origin.x + button.bounds.size.width <= home.width as f32);
        assert!(button.bounds.origin.y + button.bounds.size.height <= home.height as f32);
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(button.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowPluginSurface {
                plugin_id: package.manifest.id.clone(),
                surface_id: "details".into(),
            }]
        );
        let details_surface = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "details")
            .unwrap();
        let mut details = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package_surface(
                &package,
                &std::collections::BTreeMap::new(),
                details_surface,
            )
            .unwrap(),
            details_surface.width,
            details_surface.height,
        );
        let close = details
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Close details".into(),
            })
            .unwrap();
        assert!(close.bounds.size.width > 0.0);
        assert!(close.bounds.size.height > 0.0);
        assert!(close.bounds.origin.x + close.bounds.size.width <= details_surface.width as f32);
        assert!(close.bounds.origin.y + close.bounds.size.height <= details_surface.height as f32);
        details.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(close.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            details.application_mut().take_effects(),
            vec![PluginEffect::HidePluginSurface {
                plugin_id: package.manifest.id.clone(),
                surface_id: "details".into(),
            }]
        );
    }

    #[test]
    fn separate_dialog_example_opens_and_dismisses_its_declared_surface() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-surface-dialog"
        );
        let package = PluginPackage::load(directory).unwrap();
        let home = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "home")
            .unwrap();
        let dialog = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "confirm")
            .unwrap();
        let settings = std::collections::BTreeMap::new();
        let mut home = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package_surface(&package, &settings, home).unwrap(),
            home.width,
            home.height,
        );
        let open = home
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open dialog".into(),
            })
            .unwrap();
        assert!(open.bounds.origin.x < 210.0);
        assert!(open.bounds.origin.x + open.bounds.size.width > 210.0);
        assert!(open.bounds.origin.y < 77.0);
        assert!(open.bounds.origin.y + open.bounds.size.height > 77.0);
        home.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(open.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            home.application_mut().take_effects(),
            vec![PluginEffect::ShowPluginSurface {
                plugin_id: package.manifest.id.clone(),
                surface_id: "confirm".into(),
            }]
        );
        let mut dialog = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package_surface(&package, &settings, dialog).unwrap(),
            dialog.width,
            dialog.height,
        );
        let dismiss = dialog
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Dismiss".into(),
            })
            .unwrap();
        assert!(dismiss.bounds.origin.x < 180.0);
        assert!(dismiss.bounds.origin.x + dismiss.bounds.size.width > 180.0);
        assert!(dismiss.bounds.origin.y < 119.0);
        assert!(dismiss.bounds.origin.y + dismiss.bounds.size.height > 119.0);
        dialog.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(dismiss.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            dialog.application_mut().take_effects(),
            vec![PluginEffect::HidePluginSurface {
                plugin_id: package.manifest.id.clone(),
                surface_id: "confirm".into(),
            }]
        );
    }

    #[test]
    fn overlay_example_opens_and_hides_its_declared_surface() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-overlay"
        );
        let package = PluginPackage::load(directory).unwrap();
        let home = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "home")
            .unwrap();
        let overlay = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "notice")
            .unwrap();
        let settings = std::collections::BTreeMap::new();
        let mut home = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package_surface(&package, &settings, home).unwrap(),
            home.width,
            home.height,
        );
        let show = home
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Show overlay".into(),
            })
            .unwrap();
        assert!(show.bounds.origin.x < 210.0);
        assert!(show.bounds.origin.x + show.bounds.size.width > 210.0);
        assert!(show.bounds.origin.y < 77.0);
        assert!(show.bounds.origin.y + show.bounds.size.height > 77.0);
        home.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(show.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            home.application_mut().take_effects(),
            vec![PluginEffect::ShowPluginSurface {
                plugin_id: package.manifest.id.clone(),
                surface_id: "notice".into(),
            }]
        );
        let mut overlay = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package_surface(&package, &settings, overlay).unwrap(),
            overlay.width,
            overlay.height,
        );
        let hide = overlay
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Close overlay".into(),
            })
            .unwrap();
        assert!(hide.bounds.origin.x < 150.0);
        assert!(hide.bounds.origin.x + hide.bounds.size.width > 150.0);
        assert!(hide.bounds.origin.y < 77.0);
        assert!(hide.bounds.origin.y + hide.bounds.size.height > 77.0);
        overlay.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(hide.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            overlay.application_mut().take_effects(),
            vec![PluginEffect::HidePluginSurface {
                plugin_id: package.manifest.id.clone(),
                surface_id: "notice".into(),
            }]
        );
    }

    #[test]
    fn external_dialog_can_change_only_its_declared_setting_with_a_grant() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-dialog"
        );
        let package = PluginPackage::load(directory).unwrap();
        for (granted, valid) in [(true, true), (false, true), (true, false)] {
            let mut current = package.clone();
            if !granted {
                current.manifest.capabilities.clear();
            }
            if !valid {
                current.source = current.source.replace("Math.min(99, openCount + 1)", "100");
            }
            let mut host = nickel_ui::UiHost::new(
                PluginPanelApplication::from_package(&current).unwrap(),
                320,
                120,
            );
            let open = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Button,
                    name: "Open a dialog".into(),
                })
                .unwrap();
            host.step(nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(
                    nickel_ui::UiEvent::AccessibilityActivate(open.id),
                )],
                ..Default::default()
            });
            let save = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Button,
                    name: "Save count".into(),
                })
                .unwrap();
            host.step(nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(
                    nickel_ui::UiEvent::AccessibilityActivate(save.id),
                )],
                ..Default::default()
            });
            let effects = host.application_mut().take_effects();
            if granted && valid {
                assert_eq!(
                    effects,
                    vec![PluginEffect::SetPluginSetting {
                        plugin_id: current.manifest.id.clone(),
                        key: "open-count".into(),
                        value: serde_json::json!(1),
                    }]
                );
            } else {
                assert!(effects.is_empty());
                assert!(host.application_mut().last_error().is_some());
            }
        }
    }

    #[test]
    fn host_dismissal_calls_dialog_on_close_and_allows_reopen() {
        let package = PluginPackage {
            manifest: manifest().clone(),
            images: Default::default(),
            stylesheet: String::new(),
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
            images: Default::default(),
            stylesheet: String::new(),
            source: "function App() { return h(Panel, {}, h(Text, {}, nickel.data.settings['show-count'] ? 'Shown' : 'Hidden')); }".into(),
        };
        let settings =
            std::collections::BTreeMap::from([("show-count".to_owned(), serde_json::json!(false))]);
        let app = PluginPanelApplication::from_package_with_settings(&package, &settings).unwrap();
        assert!(format!("{:?}", app.node).contains("Hidden"));
    }

    #[test]
    fn installed_window_loads_its_declared_image_into_the_native_host() {
        let package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap();
        PluginPanelApplication::validate_package(&package).unwrap();
        let app = PluginPanelApplication::from_package_surface(
            &package,
            &Default::default(),
            &package.manifest.surfaces[0],
        )
        .unwrap();
        let (_, image) = &app.images["nickel-icon"];
        assert!(image.width() > 0 && image.height() > 0);
        assert!(format!("{:?}", app.node).contains("nickel-icon"));
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
    fn secure_jsx_text_field_masks_paint_and_protects_remote_semantics() {
        let source = "function App() { return h(Panel, {}, h(TextField, {id: 'password', value: 'secret-value', placeholder: 'Password', secure: true, onChange: value => {}})); }";
        let host = nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 320, 100);
        assert!(host.remote_access_protected());
        assert!(matches!(
            host.bounded_semantic_nodes(64, 4096),
            Err(nickel_ui::BoundedSemanticError::ProtectedSurface)
        ));
        assert!(!host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Text { text, .. } if text.contains("secret-value")
        )));
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
    fn bundled_codex_project_menu_renders_only_the_bounded_projection() {
        let package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/codex-projects"
        ))
        .unwrap();
        PluginPanelApplication::validate_package(&package).unwrap();
        let projection = ProjectMenuProjection {
            revision: nickel_codex_ui::ProjectMenuRevision {
                connection: 3,
                projects: 7,
            },
            status: "ready",
            projects: vec![nickel_codex_ui::ProjectMenuEntry {
                id: "0".into(),
                name: "Example project".into(),
            }],
        };
        let mut panel =
            PluginPanelApplication::codex_projects_with_projection(&projection).unwrap();
        let rendered = format!("{:?}", panel.node);
        assert!(rendered.contains("Example project"));
        assert!(!rendered.contains("/private/work"));
        assert!(!panel.sync_codex_projects_projection(&projection).unwrap());
        let mut disconnected = projection.clone();
        disconnected.status = "disconnected";
        disconnected.projects.clear();
        assert!(panel.sync_codex_projects_projection(&disconnected).unwrap());
        assert!(!format!("{:?}", panel.node).contains("Example project"));

        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::codex_projects_with_projection(&projection).unwrap(),
            520,
            680,
        );
        let open = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Example project".into(),
            })
            .unwrap();
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(open.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::CodexProjectOpen {
                token: "0".into(),
                revision: projection.revision,
            }]
        );
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Shortcut(Shortcut::Escape)],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::CodexProjectClose]
        );
    }

    #[test]
    fn bundled_keyboard_renders_host_keys_and_emits_typed_effects() {
        use nickel_core::on_screen_keyboard::{
            KeyboardPanel, VirtualModifiers, keyboard_display_rows,
        };
        let rows = keyboard_display_rows(
            KeyboardPanel::Letters,
            VirtualModifiers::default(),
            false,
            true,
        );
        let data = serde_json::json!({
            "generation": 7,
            "height": 368,
            "dockTop": false,
            "recipientAvailable": true,
            "rows": rows,
        });
        let mut plugin = PluginPanelApplication::on_screen_keyboard_with_data(&data).unwrap();
        let message = plugin.button_message("osk-char-113").unwrap();
        plugin.update(message);
        assert_eq!(
            plugin.take_effects(),
            vec![PluginEffect::KeyboardKey {
                id: "osk-char-113".into(),
                generation: 7,
            }]
        );
        assert!(plugin.last_error().is_none());
        let disabled = serde_json::json!({
            "generation": 8,
            "height": 368,
            "dockTop": false,
            "recipientAvailable": false,
            "rows": keyboard_display_rows(
                KeyboardPanel::Letters,
                VirtualModifiers::default(),
                false,
                false,
            ),
        });
        assert!(plugin.sync_on_screen_keyboard_data(&disabled).unwrap());
        let message = plugin.button_message("osk-char-113").unwrap();
        plugin.update(message);
        assert!(plugin.take_effects().is_empty());
    }

    #[test]
    fn launcher_plugin_uses_window_root_and_keeps_nested_controls_reachable() {
        let launcher = Launcher::new(Vec::new());
        let panel = PluginPanelApplication::launcher(&launcher).unwrap();
        assert!(matches!(
            &panel.node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        assert!(panel.node.button_action("launcher-settings").is_some());
        assert!(panel.node.dialog("launcher-logout-dialog").is_some());
        let host = nickel_ui::UiHost::new(panel, 920, 680);
        let title = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Text,
                name: "Nickel Launcher".into(),
            })
            .unwrap();
        assert!(title.bounds.size.height >= 20.0, "{title:?}");
    }

    #[test]
    fn notification_plugin_uses_styled_window_root() {
        let item = NotificationPluginItem {
            id: 7,
            app_name: "Mail".into(),
            summary: "New message".into(),
            body: "The body".into(),
            actions: vec![NotificationPluginAction {
                key: "open".into(),
                label: "Open".into(),
            }],
        };
        for projection in [
            NotificationPluginProjection {
                notification: None,
                history: Vec::new(),
                history_visible: false,
            },
            NotificationPluginProjection {
                notification: Some(item.clone()),
                history: Vec::new(),
                history_visible: false,
            },
            NotificationPluginProjection {
                notification: Some(item.clone()),
                history: vec![item.clone()],
                history_visible: true,
            },
        ] {
            let panel = PluginPanelApplication::notification_with_projection(&projection).unwrap();
            assert!(matches!(
                &panel.node,
                PanelNode::Surface {
                    window_request: Some(_),
                    ..
                }
            ));
            assert_eq!(
                panel
                    .stylesheet
                    .resolve("window", Some("main"), Some("notification-window"))
                    .background,
                Some(0xf22b303c)
            );
        }
    }

    #[test]
    fn launcher_scroll_area_shrinks_with_the_popup() {
        let launcher = Launcher::new(Vec::new());
        let app = PluginPanelApplication::launcher(&launcher).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 920, 680);
        let large = host.scroll_extent(&PluginMessage::Scroll).unwrap();
        host.step(nickel_ui::HostBatch {
            surface_size: Some((500, 260)),
            ..Default::default()
        });
        let small = host.scroll_extent(&PluginMessage::Scroll).unwrap();
        assert!(small.viewport.height < large.viewport.height);
        assert!(small.viewport.height <= 220.0);
        assert!(small.viewport.height > 0.0);
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
