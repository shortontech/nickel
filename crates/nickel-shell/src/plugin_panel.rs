//! Experimental JavaScript panel host. The bundled example uses the same small
//! component vocabulary as an external plugin; native surfaces remain shell-owned.

use nickel_jsx_host::SettingsRuntimeExt;
use std::{
    borrow::Cow,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Instant,
};

use nickel_core::package_composition::PackageIdentity;
use nickel_core::plugins::{
    PluginCapability, PluginManifest, PluginPackage, PluginSurface, PluginSurfaceKind,
};
#[cfg(test)]
use nickel_jsx_host::NativePatchCounters;
use nickel_jsx_host::composition_runtime::{
    ComponentEventHandle, ComponentMount, ScheduledExpandedBatch, ShellCompositionRuntime,
};
use nickel_jsx_host::{JsxModuleGraph, JsxRuntime, ModuleSource, ScheduledPatch};
use twinkle_presentation::components::{PanelNode, RetainedPanelTree, render_retained_panel};
pub use twinkle_presentation::components::{PluginImages, PluginMessage};

struct CompositionPanelState {
    host: std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>,
    mount: ComponentMount,
    events: std::collections::BTreeMap<u64, ComponentEventHandle>,
    manifests: std::collections::BTreeMap<PackageIdentity, PluginManifest>,
    snapshots: std::collections::BTreeMap<PackageIdentity, Value>,
}
use serde_json::Value;
use twinkle_ui::{
    AnyView, Column, DragPhase, FrameOverlay, Length, OverlayAnchor, OverlayId, OverlayMenu,
    OverlayStyle, Row, Shortcut, Size, Spacer, TransientSurface, UiFrame, UiId, ViewContext,
};
#[cfg(test)]
use twinkle_ui::{Point, SemanticRole};

/// Advance a native plugin host, including viewport feedback and bounded
/// virtual-row measurement. Embedders must use this instead of stepping the
/// generic UI host alone: virtual collections initially contain no row trees.
/// Continue servicing the returned native deadline when convergence is pending.
pub fn step_host(
    host: &mut twinkle_ui::UiHost<PluginPanelApplication>,
    data: Option<String>,
    batch: twinkle_ui::HostBatch,
) -> Result<(twinkle_ui::HostEventOutcome, u64), String> {
    crate::live_shell::step_plugin_host(host, data, batch)
}

use nickel_core::display_projection::ProjectionMode;
use twinkle_presentation::css::StyleSheet;

use crate::window_preview::PreviewAction;

const MAX_EFFECT_RECONCILIATIONS: usize = 16;
static NEXT_DIAGNOSTIC_MOUNT: AtomicU64 = AtomicU64::new(1);
static NEXT_NATIVE_TEXT_REVISION: AtomicU64 = AtomicU64::new(1);

fn nickel_theme_properties(
    palette: nickel_core::theme::ThemePalette,
) -> std::collections::HashMap<String, String> {
    let blend = |base: u32, foreground: u32, foreground_percent: u32| -> u32 {
        let channel = |shift: u32| {
            let base = (base >> shift) & 0xff;
            let foreground = (foreground >> shift) & 0xff;
            (base * (100 - foreground_percent) + foreground * foreground_percent + 50) / 100
        };
        (channel(16) << 16) | (channel(8) << 8) | channel(0)
    };
    let light = palette.mode == nickel_core::theme::ThemeMode::Light;
    let raised = if light {
        blend(palette.surface, 0x00ff_ffff, 35)
    } else {
        blend(palette.panel, palette.text, 10)
    };
    let control = if light {
        blend(palette.surface, 0x00ff_ffff, 15)
    } else {
        blend(palette.panel, palette.text, 15)
    };
    let mut properties = std::collections::HashMap::new();
    for (name, color) in [
        ("background", palette.background),
        (
            "background-raised",
            if light {
                blend(palette.background, 0x00ff_ffff, 18)
            } else {
                palette.background
            },
        ),
        (
            "panel-raised",
            if light {
                blend(palette.panel, 0x00ff_ffff, 12)
            } else {
                palette.panel
            },
        ),
        ("panel", palette.panel),
        ("surface", palette.surface),
        (
            "card",
            if light {
                nickel_core::theme::shifted_surface_hue(
                    blend(palette.background, 0x00ff_ffff, 18),
                    8.0,
                )
            } else {
                palette.surface
            },
        ),
        ("surface-hover", palette.surface_hover),
        ("text", palette.text),
        ("muted", palette.muted),
        ("accent", palette.accent),
        ("accent-soft", palette.accent_soft),
        ("complement", palette.complement),
        ("raised", raised),
        ("control", control),
        ("border", blend(palette.panel, palette.text, 30)),
        ("soft-text", blend(palette.muted, palette.text, 40)),
        ("selected", blend(control, palette.accent, 25)),
        ("selected-border", blend(palette.accent, palette.text, 35)),
        ("panel-soft", blend(palette.panel, palette.text, 4)),
        ("border-soft", blend(palette.panel, palette.text, 12)),
        ("divider-soft", blend(palette.panel, palette.text, 8)),
        ("control-soft", blend(palette.panel, palette.text, 7)),
        ("text-soft", blend(palette.panel, palette.text, 88)),
        ("selection-soft", blend(palette.panel, palette.accent, 24)),
        ("focus-soft", blend(palette.panel, palette.accent, 48)),
        ("action-soft", blend(palette.panel, palette.accent, 72)),
    ] {
        properties.insert(
            format!("--nickel-{name}"),
            format!("#{:06x}", color & 0x00ff_ffff),
        );
    }
    for (name, value) in [
        ("surface-raised", format!("#{:06x}", raised & 0x00ff_ffff)),
        (
            "text-muted",
            format!("#{:06x}", palette.muted & 0x00ff_ffff),
        ),
        ("radius-control", "8px".into()),
        ("radius-card", "12px".into()),
        ("spacing-control", "8px".into()),
        ("font-size", "14px".into()),
        ("line-height", "20px".into()),
    ] {
        properties.insert(format!("--nickel-{name}"), value);
    }
    properties
}

fn compile_plugin_stylesheet(source: &str) -> Result<StyleSheet, String> {
    StyleSheet::compile_with_host_properties(
        source,
        nickel_theme_properties(nickel_core::theme::ThemePalette::from_appearance(
            nickel_core::theme::Appearance::default(),
        )),
    )
}

fn allocate_native_text_revision(counter: &AtomicU64) -> Option<u64> {
    // Exhaustion disables this optimization instead of aliasing an older source.
    counter
        .try_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
            value.checked_add(1)
        })
        .ok()
}

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

pub fn surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: manifest().id.clone(),
        surface_id: surface().id.clone(),
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
                root.join(&manifest.id),
                manifest,
            )?)
        } else {
            Cow::Borrowed(fallback)
        }
    } else {
        Cow::Borrowed(fallback)
    };
    compile_plugin_stylesheet(&source)
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

type WallpaperDemandCache = Option<(u64, twinkle_ui::Rect, Arc<Vec<String>>)>;

pub struct PluginPanelApplication {
    runtime: std::rc::Rc<std::cell::RefCell<JsxRuntime>>,
    accepted: RetainedPanelTree,
    next_generation: u64,
    effects: Vec<PluginEffect>,
    pending_transient: Option<(OverlayId, UiId)>,
    last_error: Option<String>,
    runtime_failure: Option<String>,
    manifest: PluginManifest,
    expected_surface_id: Option<String>,
    runtime_surface_id: String,
    projection_data: Option<String>,
    projection_value: Option<Value>,
    overlay_open: bool,
    dispatch_removed_focus: bool,
    images: PluginImages,
    stylesheet: StyleSheet,
    composition: Option<CompositionPanelState>,
    surface_snapshot: Value,
    diagnostic_mount: u64,
    native_text_revision: std::cell::Cell<Option<(u64, u64, Option<u64>)>>,
    virtual_feedback_generation: std::cell::Cell<Option<(u64, u64, twinkle_ui::Rect)>>,
    virtual_collection_presence: std::cell::Cell<Option<(u64, bool)>>,
    wallpaper_demand_cache: std::cell::RefCell<WallpaperDemandCache>,
    application_image_demand_cache:
        std::cell::RefCell<Option<(u64, Arc<std::collections::BTreeSet<String>>)>>,
    virtual_measurement_epoch: u64,
    virtual_work_pending: bool,
    #[cfg(test)]
    maintenance_owner_cursor: usize,
    virtual_measurement_generation: Option<(u64, u64, u64, twinkle_ui::Rect, f32)>,
    pending_frame_correlation: Option<twinkle_ui::NativeFrameCorrelation>,
    #[cfg(test)]
    diagnostic_patch_operations: u64,
    #[cfg(test)]
    diagnostic_patch_transport_bytes: u64,
    #[cfg(test)]
    diagnostic_patch_counters: NativePatchCounters,
}

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
pub enum PluginEffect {
    Feature {
        plugin_id: String,
        effect: crate::feature_capabilities::FeatureEffect,
    },
    SearchApplications {
        plugin_id: String,
        query: String,
    },
    MoveApplicationPin {
        id: String,
        direction: i8,
    },
    Preferences {
        plugin_id: String,
        effect: crate::preferences_capabilities::PreferencesEffect,
    },
    ShellPreviewDecision {
        plugin_id: String,
        effect: crate::plugins_capabilities::ShellPreviewDecision,
    },
    ShellSelection {
        plugin_id: String,
        effect: crate::plugins_capabilities::ShellSelectionEffect,
    },
    PluginsSetting {
        plugin_id: String,
        effect: crate::plugins_capabilities::PluginsSettingEffect,
    },
    Plugins {
        plugin_id: String,
        effect: crate::plugins_capabilities::PluginsEffect,
    },
    Associations {
        plugin_id: String,
        effect: crate::associations_capabilities::AssociationsEffect,
    },
    Appearance {
        plugin_id: String,
        effect: crate::appearance_capabilities::AppearanceEffect,
    },
    Connectivity {
        plugin_id: String,
        effect: crate::connectivity_capabilities::ConnectivityEffect,
    },
    InvokeRegisteredSetting {
        caller: String,
        provider: String,
        id: String,
        value: Value,
    },
    SetApplicationScale {
        plugin_id: String,
        effect: crate::application_scale_capability::ApplicationScaleEffect,
    },
    IdentifyDisplays {
        plugin_id: String,
        revision: String,
    },
    SetDisplayLayout {
        plugin_id: String,
        layout: nickel_session_protocol::OutputLayout,
        revision: String,
    },
    ConfirmDisplayLayout {
        plugin_id: String,
    },
    RevertDisplayLayout {
        plugin_id: String,
    },
    ShowLauncher,
    ShowSettings(Option<String>),
    ShowPluginSurface {
        plugin_id: String,
        surface_id: String,
    },
    HidePluginSurface {
        plugin_id: String,
        surface_id: String,
    },
    FocusPluginSurface {
        plugin_id: String,
        surface_id: String,
    },
    SetPluginSurfacePlacement {
        plugin_id: String,
        surface_id: String,
        anchor: nickel_core::plugins::PluginSurfaceAnchor,
        offset_x: i32,
        offset_y: i32,
    },
    SetPluginSetting {
        plugin_id: String,
        key: String,
        value: serde_json::Value,
    },
    RunExecute {
        plugin_id: String,
        execute: crate::run_capabilities::Execute,
    },
    ToggleLauncher,
    ShowControlCenter,
    ActivateWindow(crate::model::WindowId),
    CloseWindow(crate::model::WindowId),
    WindowOperation {
        plugin_id: String,
        operation: String,
        restore_focus: bool,
        destination: Option<String>,
        window: Option<crate::model::WindowId>,
    },
    ToggleOnScreenKeyboard {
        plugin_id: String,
    },
    Keyboard {
        plugin_id: String,
        effect: crate::keyboard_capabilities::KeyboardRequest,
    },
    ProjectsVisibility {
        plugin_id: String,
        toggle: bool,
    },

    LaunchApplication {
        id: String,
    },
    ToggleApplicationPin {
        id: String,
    },
    RetryApplicationPinSave,

    SessionOperation {
        plugin_id: String,
        request: crate::session_capabilities::Request,
    },
    ActivateTrayItem {
        id: String,
    },
    ContextTrayItem {
        id: String,
    },
    ToggleControlCenter,
    InvokeNotification {
        plugin_id: String,
        id: u32,
        key: String,
    },
    DismissNotification {
        plugin_id: String,
        id: u32,
    },
    Workspace {
        plugin_id: String,
        effect: crate::workspace_capabilities::WorkspaceEffect,
    },
    ToggleShowDesktop {
        plugin_id: String,
    },
    PreviewDisplayProjection {
        plugin_id: String,
        mode: ProjectionMode,
        revision: String,
    },
    WindowPreviewRequest {
        plugin_id: String,
        revision: String,
        action: PreviewAction,
    },
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

/// A package may supply synthetic data for each surface's initial validation tree.
fn validation_surface_projection(
    package: &PluginPackage,
    surface: &PluginSurface,
) -> Option<Value> {
    package.manifest.validation_data.get(&surface.id).cloned()
}

fn initial_notifications_data(manifest: &PluginManifest) -> Value {
    if manifest
        .capabilities
        .contains(&PluginCapability::NotificationsRead)
    {
        serde_json::json!({
            "notification": null,
            "history": [],
        })
    } else {
        Value::Null
    }
}

fn package_module_graph(package: &PluginPackage) -> Result<Option<JsxModuleGraph>, String> {
    let is_module = package.source.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with("import ") || line.starts_with("export ")
    });
    if !is_module {
        return Ok(None);
    }
    let mut sources = package
        .modules
        .iter()
        .filter(|module| module.path != package.manifest.entry)
        .map(|module| ModuleSource {
            path: &module.path,
            source: &module.source,
        })
        .collect::<Vec<_>>();
    sources.push(ModuleSource {
        path: &package.manifest.entry,
        source: &package.source,
    });
    let graph = JsxModuleGraph::new(&package.manifest.entry, sources)?;
    let graph = match &package.manifest.composition {
        Some(composition) => graph.with_public_exports(&composition.exports)?,
        None => graph,
    };
    Ok(Some(graph))
}

fn package_runtime(package: &PluginPackage, data: Option<&str>) -> Result<JsxRuntime, String> {
    match package_module_graph(package)? {
        Some(graph) => nickel_jsx_host::create_module_runtime(&graph, data),
        None => nickel_jsx_host::create_runtime(&package.source, data),
    }
}

fn package_stylesheet(package: &PluginPackage) -> Result<StyleSheet, String> {
    let imported = package_module_graph(package)?
        .map(|graph| graph.stylesheet())
        .transpose()?
        .unwrap_or_default();
    compile_plugin_stylesheet(&format!("{}\n{}", package.stylesheet, imported))
}

impl PluginPanelApplication {
    pub fn resolved_surface(&self, grant: &PluginSurface) -> Result<PluginSurface, String> {
        let mut surface = self
            .accepted
            .node()
            .requested_surface(grant, &self.stylesheet)
            .map(|surface| surface.unwrap_or_else(|| grant.clone()))?;
        let Some((width, height)) = self
            .accepted
            .node()
            .requested_surface_lengths(&self.stylesheet)
        else {
            return Ok(surface);
        };
        if matches!(width, Length::MaxContent) || matches!(height, Length::MaxContent) {
            let preferred = UiFrame::preferred_size(
                self.accepted.node().view(&self.images, &self.stylesheet),
                Size::new(grant.width as f32, grant.height as f32),
            );
            if matches!(width, Length::MaxContent) {
                surface.width = preferred.width.ceil().max(1.0) as u32;
            }
            if matches!(height, Length::MaxContent) {
                surface.height = preferred.height.ceil().max(1.0) as u32;
            }
        }
        Ok(surface)
    }

    pub(crate) fn button_message(&self, id: &str) -> Option<PluginMessage> {
        self.accepted
            .node()
            .button_action(id)
            .map(PluginMessage::Click)
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
        let mut application = Self::new_with_manifest(source, manifest(), None)?;
        application.stylesheet = bundled_stylesheet(
            manifest(),
            include_str!("../../../assets/plugins/hello-panel/ui.css"),
        )?;
        application.resolved_surface(surface())?;
        Ok(application)
    }

    pub fn from_package(package: &PluginPackage) -> Result<Self, String> {
        let mut application = Self::new_with_manifest_for_surface(
            &package.source,
            &package.manifest,
            None,
            None,
            Some(std::rc::Rc::new(std::cell::RefCell::new(package_runtime(
                package, None,
            )?))),
        )?;
        application.stylesheet = package_stylesheet(package)?;
        if let [surface] = package.manifest.surfaces.as_slice() {
            application.resolved_surface(surface)?;
        }
        application.sync_images(package_images(package)?);
        Ok(application)
    }

    pub fn from_package_with_settings(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<Self, String> {
        let data = serde_json::json!({ "settings": settings, "windows": [] }).to_string();
        let mut application = Self::new_with_manifest_for_surface(
            &package.source,
            &package.manifest,
            Some(data.clone()),
            None,
            Some(std::rc::Rc::new(std::cell::RefCell::new(package_runtime(
                package,
                Some(&data),
            )?))),
        )?;
        application.stylesheet = package_stylesheet(package)?;
        if let [surface] = package.manifest.surfaces.as_slice() {
            application.resolved_surface(surface)?;
        }
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

    fn from_package_surface_with_images(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
        surface: &PluginSurface,
        images: PluginImages,
    ) -> Result<Self, String> {
        Self::from_package_surface_with_runtime(package, settings, surface, images, None)
    }

    pub(crate) fn package_surface_data(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
        surface: &PluginSurface,
    ) -> String {
        serde_json::json!({
            "settings": settings,
            "windows": [],
            "applications": [],
            "displays": {"available": false, "outputs": []},
            "notifications": initial_notifications_data(&package.manifest),
            "surface": {
                "id": surface.id,
                "kind": surface.kind.as_str(),
                "width": surface.width,
                "height": surface.height,
            },
        })
        .to_string()
    }

    pub(crate) fn shared_package_runtime(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
        first_surface: &PluginSurface,
    ) -> Result<std::rc::Rc<std::cell::RefCell<JsxRuntime>>, String> {
        let data = Self::package_surface_data(package, settings, first_surface);
        Ok(std::rc::Rc::new(std::cell::RefCell::new(package_runtime(
            package,
            Some(&data),
        )?)))
    }

    pub(crate) fn shared_runtime(&self) -> std::rc::Rc<std::cell::RefCell<JsxRuntime>> {
        self.runtime.clone()
    }

    pub(crate) fn retire_surface(&self) -> Result<(), String> {
        if let Some(state) = &self.composition {
            return state.host.borrow_mut().unmount(&state.mount);
        }
        self.runtime
            .borrow_mut()
            .drop_surface(&self.runtime_surface_id)
    }

    /// Publish compositor/native-window facts for this exact mount. Callers
    /// publish geometry before focus so focus observers cannot see stale
    /// placement from the same host event.
    pub(crate) fn sync_surface_authority(
        &mut self,
        output: Option<&str>,
        available_size: Option<(f32, f32)>,
        scale_factor: Option<f32>,
        focused: Option<bool>,
        visible: Option<bool>,
    ) -> Result<bool, String> {
        let mut next = self.surface_snapshot.clone();
        let object = next
            .as_object_mut()
            .ok_or("surface snapshot must be an object")?;
        object.insert("output".into(), output.map_or(Value::Null, Value::from));
        let (available_width, available_height) = available_size
            .map(|(width, height)| (Value::from(width), Value::from(height)))
            .unwrap_or((Value::Null, Value::Null));
        object.insert("availableWidth".into(), available_width);
        object.insert("availableHeight".into(), available_height);
        object.insert(
            "scaleFactor".into(),
            scale_factor.map_or(Value::Null, Value::from),
        );
        object.insert("focused".into(), focused.map_or(Value::Null, Value::from));
        object.insert("visible".into(), visible.map_or(Value::Null, Value::from));
        if next == self.surface_snapshot {
            return Ok(false);
        }
        if let Some(state) = &mut self.composition {
            state
                .host
                .borrow_mut()
                .update_mount_surface(&state.mount, next.clone())?;
        } else {
            let mut runtime = self.runtime.borrow_mut();
            runtime.select_surface(&self.runtime_surface_id)?;
            runtime.set_surface_store(
                &format!("plugin-surface:{}", self.runtime_surface_id),
                &next,
            )?;
        }
        self.surface_snapshot = next;
        Ok(true)
    }

    pub(crate) fn reconcile_surface_authority(&mut self) -> Result<bool, String> {
        let Some(state) = &mut self.composition else {
            return Ok(false);
        };
        let mut host = state.host.borrow_mut();
        host.consume_mount_reconciliation(&state.mount)?;
        let (rendered, ()) =
            host.render_expanded_validated(&state.mount, &serde_json::json!({}), |value| {
                self.accepted
                    .readmit(
                        value,
                        &self.manifest,
                        self.expected_surface_id.as_deref(),
                        0,
                    )
                    .map(|_| ())
            })?;
        self.accepted = self.accepted.readmit(
            &rendered.node,
            &self.manifest,
            self.expected_surface_id.as_deref(),
            rendered.generation(),
        )?;
        state.events = rendered.events;
        Ok(true)
    }

    pub(crate) fn reconcile_surface_focus_authority(&mut self) -> Result<bool, String> {
        let Some(state) = &mut self.composition else {
            return self.reconcile_surface_authority();
        };
        if !state
            .host
            .borrow_mut()
            .publish_mount_surface_authority(&state.mount)?
        {
            return Ok(false);
        }
        self.reconcile_surface_authority()
    }

    pub(crate) fn sync_surface_geometry(
        &mut self,
        output: Option<&str>,
        available_size: Option<(f32, f32)>,
        scale_factor: Option<f32>,
        visible: Option<bool>,
    ) -> Result<bool, String> {
        let focused = self
            .surface_snapshot
            .get("focused")
            .and_then(Value::as_bool);
        self.sync_surface_authority(output, available_size, scale_factor, focused, visible)
    }

    pub(crate) fn sync_surface_focus(&mut self, focused: bool) -> Result<bool, String> {
        let output = self
            .surface_snapshot
            .get("output")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let available_size = self
            .surface_snapshot
            .get("availableWidth")
            .and_then(Value::as_f64)
            .zip(
                self.surface_snapshot
                    .get("availableHeight")
                    .and_then(Value::as_f64),
            )
            .map(|(width, height)| (width as f32, height as f32));
        let scale_factor = self
            .surface_snapshot
            .get("scaleFactor")
            .and_then(Value::as_f64)
            .map(|scale| scale as f32);
        let visible = self
            .surface_snapshot
            .get("visible")
            .and_then(Value::as_bool);
        self.sync_surface_authority(
            output.as_deref(),
            available_size,
            scale_factor,
            Some(focused),
            visible,
        )
    }

    #[cfg(test)]
    pub(crate) fn surface_observation(&self) -> &Value {
        &self.surface_snapshot
    }

    pub(crate) fn from_package_surface_with_runtime(
        package: &PluginPackage,
        settings: &std::collections::BTreeMap<String, serde_json::Value>,
        surface: &PluginSurface,
        images: PluginImages,
        runtime: Option<std::rc::Rc<std::cell::RefCell<JsxRuntime>>>,
    ) -> Result<Self, String> {
        let data = Self::package_surface_data(package, settings, surface);
        let runtime = match runtime {
            Some(runtime) => runtime,
            None => std::rc::Rc::new(std::cell::RefCell::new(package_runtime(
                package,
                Some(&data),
            )?)),
        };
        let mut application = Self::new_with_manifest_for_surface(
            &package.source,
            &package.manifest,
            Some(data),
            Some(&surface.id),
            Some(runtime),
        )?;
        application.stylesheet = package_stylesheet(package)?;
        application.resolved_surface(surface)?;
        application.sync_images(images);
        Ok(application)
    }

    /// One native surface adapter over the shared package-lifecycle composition
    /// host. Public child components run in their own package contexts.
    pub(crate) fn from_composed_surface(
        catalog: &std::collections::BTreeMap<String, PluginPackage>,
        active: &str,
        snapshots: &std::collections::BTreeMap<PackageIdentity, Value>,
        surface: &PluginSurface,
        shared: Option<std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>>,
    ) -> Result<Self, String> {
        let host = match shared {
            Some(host) => host,
            None => std::rc::Rc::new(std::cell::RefCell::new(ShellCompositionRuntime::new(
                catalog, active, snapshots,
            )?)),
        };
        let manifest = catalog
            .get(active)
            .ok_or("active shell package is missing")?
            .manifest
            .clone();
        let (runtime, mount, stylesheet, manifests) = {
            let mut host_ref = host.borrow_mut();
            for (owner, data) in snapshots {
                host_ref.update_snapshot(owner, data)?;
            }
            let reference = host_ref
                .component("shell")
                .ok_or("composed shell has no public shell export")?;
            let owner = host_ref.resolution().active.clone();
            let runtime = host_ref.shared_owner_runtime(&owner)?;
            let manifests = host_ref
                .participating_owners()
                .map(|identity| (identity.clone(), catalog[&identity.id].manifest.clone()))
                .collect();
            let css = host_ref
                .participating_owners()
                .map(|identity| {
                    let package = &catalog[&identity.id];
                    let imports = package_module_graph(package)?
                        .map(|graph| graph.stylesheet())
                        .transpose()?
                        .unwrap_or_default();
                    Ok::<_, String>(format!("{}\n{}", package.stylesheet, imports))
                })
                .collect::<Result<Vec<_>, _>>()?
                .join("\n");
            let stylesheet = compile_plugin_stylesheet(&css)?;
            let mount = host_ref.mount(&reference)?;
            (runtime, mount, stylesheet, manifests)
        };
        let images = {
            let host = host.borrow();
            let mut images = PluginImages::new();
            let mut pixels = 0_u64;
            for owner in host.participating_owners() {
                for (name, (_, image)) in package_images(&catalog[&owner.id])? {
                    pixels =
                        pixels.saturating_add(u64::from(image.width()) * u64::from(image.height()));
                    if pixels > 4_000_000 || images.len() >= 0x5fff {
                        return Err("composed package images exceed resource budget".into());
                    }
                    let alias = host
                        .asset_key(owner, &name)
                        .ok_or("missing composed asset owner")?;
                    images.insert(alias.into(), ((images.len() + 1) as u16, image));
                }
            }
            images
        };
        let (rendered, ()) = host.borrow_mut().render_expanded_validated(
            &mount,
            &serde_json::json!({}),
            |value| {
                let node = RetainedPanelTree::admit(value, &manifest, Some(&surface.id), 0)?;
                node.node().requested_surface(surface, &stylesheet)?;
                Ok(())
            },
        )?;
        let accepted = RetainedPanelTree::admit(
            &rendered.node,
            &manifest,
            Some(&surface.id),
            rendered.generation(),
        )?;
        let snapshots = {
            let host = host.borrow();
            host.participating_owners()
                .map(|owner| Ok((owner.clone(), host.snapshot(owner)?.clone())))
                .collect::<Result<std::collections::BTreeMap<_, _>, String>>()?
        };
        let projection_value = snapshots[&host.borrow().resolution().active].clone();
        let surface_snapshot = projection_value
            .get("surface")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));
        let projection_data =
            Some(serde_json::to_string(&projection_value).map_err(|error| error.to_string())?);
        let mut application = Self {
            runtime,
            accepted,
            next_generation: 1,
            effects: Vec::new(),
            pending_transient: None,
            last_error: None,
            runtime_failure: None,
            manifest,
            expected_surface_id: Some(surface.id.clone()),
            runtime_surface_id: surface.id.clone(),
            projection_data,
            projection_value: Some(projection_value),
            overlay_open: false,
            dispatch_removed_focus: false,
            images,
            stylesheet,
            composition: Some(CompositionPanelState {
                host,
                mount,
                events: rendered.events,
                manifests,
                snapshots,
            }),
            surface_snapshot,
            diagnostic_mount: NEXT_DIAGNOSTIC_MOUNT.fetch_add(1, Ordering::Relaxed),
            native_text_revision: std::cell::Cell::new(None),
            virtual_feedback_generation: std::cell::Cell::new(None),
            virtual_collection_presence: std::cell::Cell::new(None),
            wallpaper_demand_cache: Default::default(),
            application_image_demand_cache: Default::default(),
            virtual_measurement_epoch: 0,
            virtual_work_pending: false,
            #[cfg(test)]
            maintenance_owner_cursor: 0,
            virtual_measurement_generation: None,
            pending_frame_correlation: None,
            #[cfg(test)]
            diagnostic_patch_operations: 0,
            #[cfg(test)]
            diagnostic_patch_transport_bytes: 0,
            #[cfg(test)]
            diagnostic_patch_counters: NativePatchCounters::default(),
        };
        application.reconcile_passive_effects()?;
        Ok(application)
    }

    pub(crate) fn retire_composition_owner(&mut self, id: &str) {
        if let Some(state) = &self.composition {
            let owners = state
                .manifests
                .keys()
                .filter(|owner| owner.id == id)
                .cloned()
                .collect::<Vec<_>>();
            for owner in owners {
                state.host.borrow_mut().retire(&owner);
            }
        }
    }

    /// Refresh presentation resources for the host's currently admitted owners.
    /// The root mount is retained so the shell's hooks survive membership changes.
    pub(crate) fn refresh_composition_catalog(
        &mut self,
        catalog: &std::collections::BTreeMap<String, PluginPackage>,
    ) -> Result<bool, String> {
        let Some(state) = &mut self.composition else {
            return Ok(false);
        };
        let host = state.host.borrow();
        let owners = host
            .participating_owners()
            .filter(|owner| catalog.contains_key(&owner.id))
            .cloned()
            .collect::<Vec<_>>();
        let mut css = String::new();
        let mut images = PluginImages::new();
        let mut pixels = 0u64;
        for owner in &owners {
            let package = catalog
                .get(&owner.id)
                .ok_or("admitted composition package is unavailable")?;
            let imports = package_module_graph(package)?
                .map(|graph| graph.stylesheet())
                .transpose()?
                .unwrap_or_default();
            css.push_str(&package.stylesheet);
            css.push('\n');
            css.push_str(&imports);
            css.push('\n');
            for (name, (_, image)) in package_images(package)? {
                pixels =
                    pixels.saturating_add(u64::from(image.width()) * u64::from(image.height()));
                if pixels > 4_000_000 || images.len() >= 0x5fff {
                    return Err("composed package images exceed resource budget".into());
                }
                let alias = host
                    .asset_key(owner, &name)
                    .ok_or("missing composed asset owner")?;
                images.insert(alias.into(), ((images.len() + 1) as u16, image));
            }
        }
        let stylesheet = compile_plugin_stylesheet(&css)?;
        state.manifests = owners
            .iter()
            .map(|owner| (owner.clone(), catalog[&owner.id].manifest.clone()))
            .collect();
        state.snapshots = owners
            .iter()
            .map(|owner| Ok((owner.clone(), host.snapshot(owner)?.clone())))
            .collect::<Result<_, String>>()?;
        drop(host);
        self.stylesheet = stylesheet;
        self.virtual_measurement_epoch = self.virtual_measurement_epoch.wrapping_add(1);
        self.sync_images(images);
        // Contributor membership changes can remove nested ownership
        // boundaries. Re-admit the complete surface while the departing owner
        // is still live, rather than composing incremental patches against a
        // boundary that the same transaction removes.
        self.reconcile_surface_authority()
    }

    pub(crate) fn refresh_composition_snapshots(&mut self) -> Result<bool, String> {
        let Some(state) = &mut self.composition else {
            return Ok(false);
        };
        let host = state.host.borrow();
        for (owner, data) in &mut state.snapshots {
            let surface = data.get("surface").cloned();
            *data = host.snapshot(owner)?.clone();
            if let Some(surface) = surface {
                data.as_object_mut()
                    .ok_or("package snapshot must be an object")?
                    .insert("surface".into(), surface);
            }
        }
        let active = host.resolution().active.clone();
        let serialized =
            serde_json::to_string(&state.snapshots[&active]).map_err(|error| error.to_string())?;
        drop(host);
        self.sync_serialized_data_inner(serialized, true, None)
    }

    pub(crate) fn shared_composition_runtime(
        &self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>> {
        self.composition.as_ref().map(|state| state.host.clone())
    }

    pub(crate) fn validate_provider_resources(package: &PluginPackage) -> Result<(), String> {
        package_stylesheet(package)?;
        package_images(package)?;
        Ok(())
    }

    pub fn validate_package(package: &PluginPackage) -> Result<(), String> {
        if package
            .manifest
            .composition
            .as_ref()
            .is_some_and(|composition| {
                composition.extends.is_some()
                    || composition.exports.contains_key("shell")
                    || composition.replaces.contains_key("shell")
            })
        {
            return Self::validate_package_with_catalog(
                package,
                &crate::bundled_plugin_assets::validation_catalog()?,
            );
        }
        Self::validate_standalone_package(package)
    }

    /// Validate public shell composition through the same owner contexts and native
    /// parser as production. The supplied package always overrides catalog copies.
    pub fn validate_package_with_catalog(
        package: &PluginPackage,
        catalog: &std::collections::BTreeMap<String, PluginPackage>,
    ) -> Result<(), String> {
        let package = package.clone();
        let catalog = catalog.clone();
        std::thread::Builder::new()
            .name("nickel-composition-validation".into())
            .stack_size(16 * 1024 * 1024)
            .spawn(move || Self::validate_composed_package(&package, &catalog))
            .map_err(|error| format!("could not start composition validation: {error}"))?
            .join()
            .map_err(|_| "composition validation panicked".to_owned())?
    }

    fn validate_composed_package(
        package: &PluginPackage,
        catalog: &std::collections::BTreeMap<String, PluginPackage>,
    ) -> Result<(), String> {
        let Some(composition) = &package.manifest.composition else {
            return Self::validate_standalone_package(package);
        };
        if composition.extends.is_none()
            && !composition.exports.contains_key("shell")
            && !composition.replaces.contains_key("shell")
        {
            return Self::validate_standalone_package(package);
        }
        let mut catalog = catalog.clone();
        catalog.insert(package.manifest.id.clone(), package.clone());
        let declarations = catalog
            .iter()
            .filter_map(|(id, source)| {
                source
                    .manifest
                    .composition
                    .clone()
                    .map(|composition| (id.clone(), composition))
            })
            .collect();
        let resolution = nickel_core::package_composition::resolve_shell_package(
            &declarations,
            &package.manifest.id,
        )
        .map_err(|error| format!("composition failed: {error:?}"))?;
        // Available dependencies are a source catalog, not approval to execute
        // unrelated installed contributors during offline authoring validation.
        catalog.retain(|id, _| {
            resolution
                .inheritance_chain
                .iter()
                .any(|owner| &owner.id == id)
        });
        let shared = std::rc::Rc::new(std::cell::RefCell::new(ShellCompositionRuntime::new(
            &catalog,
            &package.manifest.id,
            &Default::default(),
        )?));
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        let owners = shared
            .borrow()
            .participating_owners()
            .cloned()
            .collect::<Vec<_>>();
        for owner in &owners {
            let runtime = shared.borrow().shared_owner_runtime(owner)?;
            runtime
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)?;
        }
        for owner in &owners {
            shared
                .borrow()
                .shared_owner_runtime(owner)?
                .borrow_mut()
                .set_settings_registry(&registry)?;
        }
        for surface in &package.manifest.surfaces {
            let snapshots = owners.iter().map(|owner| {
                let source = &catalog[&owner.id];
                let settings = source.manifest.settings.iter().map(|setting| (setting.id.clone(), setting.kind.default_value())).collect::<std::collections::BTreeMap<_,_>>();
                let mut data = serde_json::json!({"settings":settings,"windows":[],"applications":[],"notifications":initial_notifications_data(&source.manifest),"surface":{"id":surface.id,"kind":surface.kind.as_str(),"width":surface.width,"height":surface.height}});
                if let Some(projection) = validation_surface_projection(source, surface) {
                    data.as_object_mut().unwrap().extend(projection.as_object().unwrap().clone());
                }
                (owner.clone(), data)
            }).collect();
            Self::from_composed_surface(
                &catalog,
                &package.manifest.id,
                &snapshots,
                surface,
                Some(shared.clone()),
            )
            .and_then(|application| application.resolved_surface(surface))
            .map_err(|error| format!("surface {:?}: {error}", surface.id))?;
        }
        Ok(())
    }

    fn validate_standalone_package(package: &PluginPackage) -> Result<(), String> {
        package_stylesheet(package)?;
        package_images(package)?;
        let settings: std::collections::BTreeMap<_, _> = package
            .manifest
            .settings
            .iter()
            .map(|setting| (setting.id.clone(), setting.kind.default_value()))
            .collect();
        if package.manifest.surfaces.is_empty() && package.manifest.composition.is_some() {
            let catalog =
                std::collections::BTreeMap::from([(package.manifest.id.clone(), package.clone())]);
            ShellCompositionRuntime::new(&catalog, &package.manifest.id, &Default::default())?;
        } else if package.manifest.surfaces.is_empty() {
            Self::from_package_with_settings(package, &settings)?;
        } else {
            for surface in &package.manifest.surfaces {
                let mut data = serde_json::json!({
                    "settings": settings,
                    "windows": [],
                    "applications": [],
                    "notifications": initial_notifications_data(&package.manifest),
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
                let mut application = Self::new_with_manifest_for_surface(
                    &package.source,
                    &package.manifest,
                    Some(data.to_string()),
                    Some(&surface.id),
                    Some(std::rc::Rc::new(std::cell::RefCell::new(package_runtime(
                        package,
                        Some(&data.to_string()),
                    )?))),
                )
                .map_err(|error| format!("surface {:?}: {error}", surface.id))?;
                application.stylesheet = package_stylesheet(package)?;
                application
                    .resolved_surface(surface)
                    .map_err(|error| format!("surface {:?}: {error}", surface.id))?;
            }
        }
        Ok(())
    }

    /// Refresh the plugin's host-owned data using the same render transaction
    /// regardless of which surface or first-party plugin consumes it.
    pub fn sync_data(&mut self, data: &Value) -> Result<bool, String> {
        let serialized = data.to_string();
        let parsed = (self.composition.is_some()
            && self.projection_data.as_deref() != Some(serialized.as_str()))
        .then(|| data.clone());
        self.sync_serialized_data_inner(serialized, false, parsed)
    }

    /// Refreshes every sibling surface after shared Settings snapshots change.
    pub(crate) fn refresh_settings_render(&mut self) -> Result<bool, String> {
        let serialized = self.projection_data.clone().unwrap_or_else(|| "{}".into());
        self.sync_serialized_data_inner(serialized, true, self.projection_value.clone())
    }

    pub(crate) fn sync_serialized_data(&mut self, serialized: String) -> Result<bool, String> {
        self.sync_serialized_data_inner(serialized, false, None)
    }

    fn sync_serialized_data_inner(
        &mut self,
        serialized: String,
        force: bool,
        parsed: Option<Value>,
    ) -> Result<bool, String> {
        if !force && self.projection_data.as_deref() == Some(serialized.as_str()) {
            return Ok(false);
        }
        self.virtual_measurement_epoch = self.virtual_measurement_epoch.wrapping_add(1);
        if let Some(state) = &mut self.composition {
            // Native field updates already own the validated JSON value. Keep
            // the serialized mirror for legacy consumers, but do not parse our
            // own serialization back into another allocation-heavy snapshot.
            let data: Value = match parsed {
                Some(data) => data,
                None => serde_json::from_str(&serialized).map_err(|error| error.to_string())?,
            };
            let mut host = state.host.borrow_mut();
            let owner = host.resolution().active.clone();
            let accepted_tree = self.accepted.clone();
            let accepted_source = accepted_tree.source().clone();
            let outcome = host.update_snapshot_and_reconcile_expanded_pending_validated(
                &owner,
                &data,
                &state.mount,
                &state.events,
                &accepted_source,
                |patch, _, generation| {
                    let mut candidate = accepted_tree.clone();
                    if patch.operations.is_empty() {
                        return Ok(candidate);
                    }
                    let transport_bytes = serde_json::to_vec(patch)
                        .map_err(|error| error.to_string())?
                        .len();
                    #[cfg(test)]
                    {
                        self.diagnostic_patch_operations = self
                            .diagnostic_patch_operations
                            .saturating_add(patch.operations.len() as u64);
                        self.diagnostic_patch_transport_bytes = self
                            .diagnostic_patch_transport_bytes
                            .saturating_add(transport_bytes as u64);
                        self.diagnostic_patch_counters.nodes_visited = self
                            .diagnostic_patch_counters
                            .nodes_visited
                            .saturating_add(patch.counters.nodes_visited);
                        self.diagnostic_patch_counters.nodes_mutated = self
                            .diagnostic_patch_counters
                            .nodes_mutated
                            .saturating_add(patch.counters.nodes_mutated);
                        self.diagnostic_patch_counters.local_materializations = self
                            .diagnostic_patch_counters
                            .local_materializations
                            .saturating_add(patch.counters.local_materializations);
                        self.diagnostic_patch_counters.expansion_nodes = self
                            .diagnostic_patch_counters
                            .expansion_nodes
                            .saturating_add(patch.counters.expansion_nodes);
                        self.diagnostic_patch_counters.tree_bytes = self
                            .diagnostic_patch_counters
                            .tree_bytes
                            .saturating_add(patch.counters.tree_bytes);
                    }
                    candidate.apply_patch(
                        patch,
                        &self.manifest,
                        self.expected_surface_id.as_deref(),
                        &self.stylesheet,
                        generation,
                        transport_bytes,
                    )?;
                    Ok(candidate)
                },
            )?;
            match outcome {
                ScheduledExpandedBatch::Unchanged => {}
                ScheduledExpandedBatch::Patched {
                    events, validated, ..
                } => {
                    state.events = events;
                    self.accepted = validated;
                }
                ScheduledExpandedBatch::Rendered { .. } => {
                    host.finish_transaction(false)?;
                    return Err("snapshot reconciliation lost typed patch authority".into());
                }
            }
            host.finish_transaction(true)?;
            state.snapshots.insert(owner, data.clone());
            self.projection_data = Some(serialized);
            self.projection_value = Some(data);
            drop(host);
            self.reconcile_passive_effects()?;
            return Ok(true);
        }
        let data: Value = serde_json::from_str(&serialized).map_err(|error| error.to_string())?;
        let generation = self.next_generation;
        let outcome = {
            let mut runtime = self.runtime.borrow_mut();
            runtime.begin_transaction()?;
            let scheduled = (|| {
                runtime.select_surface(&self.runtime_surface_id)?;
                let surface = data
                    .get("surface")
                    .cloned()
                    .unwrap_or_else(|| serde_json::json!({}));
                runtime.set_surface_store(
                    &format!("plugin-surface:{}", self.runtime_surface_id),
                    &surface,
                )?;
                runtime.reconcile_data_patched(data.clone())
            })();
            let scheduled = match scheduled {
                Ok(scheduled) => scheduled,
                Err(error) => {
                    runtime.finish_transaction(false)?;
                    return Err(error);
                }
            };
            match scheduled {
                ScheduledPatch::Unchanged => {
                    runtime.finish_event(true)?;
                    runtime.finish_transaction(true)?;
                    None
                }
                ScheduledPatch::Patched {
                    patch,
                    transport_bytes,
                    ..
                } => {
                    let mut candidate = self.accepted.clone();
                    let application_started = Instant::now();
                    let accepted = candidate.apply_patch(
                        &patch,
                        &self.manifest,
                        self.expected_surface_id.as_deref(),
                        &self.stylesheet,
                        generation,
                        transport_bytes,
                    );
                    let application_micros = application_started
                        .elapsed()
                        .as_micros()
                        .min(u128::from(u64::MAX))
                        as u64;
                    runtime.report_typed_patch_apply(application_micros, accepted.is_ok())?;
                    runtime.finish_patch_render(accepted.is_ok())?;
                    runtime.finish_event(accepted.is_ok())?;
                    runtime.finish_transaction(accepted.is_ok())?;
                    match accepted {
                        Ok(_) => Some((candidate, patch)),
                        Err(error) => {
                            self.last_error = Some(error);
                            return Ok(false);
                        }
                    }
                }
            }
        };
        if let Some((accepted, patch)) = outcome {
            #[cfg(not(test))]
            let _ = &patch;
            #[cfg(test)]
            {
                self.diagnostic_patch_operations = self
                    .diagnostic_patch_operations
                    .saturating_add(patch.operations.len() as u64);
                self.diagnostic_patch_counters.nodes_visited = self
                    .diagnostic_patch_counters
                    .nodes_visited
                    .saturating_add(patch.counters.nodes_visited);
                self.diagnostic_patch_counters.nodes_mutated = self
                    .diagnostic_patch_counters
                    .nodes_mutated
                    .saturating_add(patch.counters.nodes_mutated);
                self.diagnostic_patch_counters.tree_bytes = self
                    .diagnostic_patch_counters
                    .tree_bytes
                    .saturating_add(patch.counters.tree_bytes);
            }
            self.accepted = accepted;
            self.next_generation = generation.saturating_add(1);
        }
        self.projection_value = Some(data);
        self.projection_data = Some(serialized);
        self.last_error = None;
        self.reconcile_passive_effects()?;
        Ok(true)
    }

    fn new_with_manifest(
        source: &str,
        manifest: &PluginManifest,
        data: Option<String>,
    ) -> Result<Self, String> {
        Self::new_with_manifest_for_surface(source, manifest, data, None, None)
    }

    fn new_with_manifest_for_surface(
        source: &str,
        manifest: &PluginManifest,
        data: Option<String>,
        expected_surface_id: Option<&str>,
        shared_runtime: Option<std::rc::Rc<std::cell::RefCell<JsxRuntime>>>,
    ) -> Result<Self, String> {
        Self::new_with_manifest_for_surface_and_scope(
            source,
            manifest,
            data,
            expected_surface_id,
            shared_runtime,
            expected_surface_id.unwrap_or("default"),
        )
    }

    fn new_with_manifest_for_surface_and_scope(
        source: &str,
        manifest: &PluginManifest,
        data: Option<String>,
        expected_surface_id: Option<&str>,
        shared_runtime: Option<std::rc::Rc<std::cell::RefCell<JsxRuntime>>>,
        runtime_surface_id: &str,
    ) -> Result<Self, String> {
        let runtime = if let Some(runtime) = shared_runtime {
            runtime
        } else {
            std::rc::Rc::new(std::cell::RefCell::new(nickel_jsx_host::create_runtime(
                source,
                data.as_deref(),
            )?))
        };
        let accepted = {
            let mut runtime_ref = runtime.borrow_mut();
            runtime_ref.select_surface(runtime_surface_id)?;
            if let Some(data) = data.as_deref() {
                runtime_ref.set_data(data)?;
                let snapshot = serde_json::from_str::<Value>(data)
                    .ok()
                    .and_then(|data| data.get("surface").cloned())
                    .unwrap_or_else(|| serde_json::json!({}));
                runtime_ref.set_surface_store(
                    &format!("plugin-surface:{runtime_surface_id}"),
                    &snapshot,
                )?;
            }
            render_retained_panel(
                &mut runtime_ref,
                manifest,
                expected_surface_id,
                "__twinkleRender()",
                1,
            )?
        };
        let surface_snapshot = data
            .as_deref()
            .and_then(|data| serde_json::from_str::<Value>(data).ok())
            .and_then(|data| data.get("surface").cloned())
            .unwrap_or_else(|| serde_json::json!({}));
        let mut application = Self {
            runtime,
            accepted,
            next_generation: 2,
            effects: Vec::new(),
            pending_transient: None,
            last_error: None,
            runtime_failure: None,
            manifest: manifest.clone(),
            expected_surface_id: expected_surface_id.map(str::to_owned),
            runtime_surface_id: runtime_surface_id.to_owned(),
            projection_value: data
                .as_deref()
                .and_then(|data| serde_json::from_str(data).ok()),
            projection_data: data,
            overlay_open: false,
            dispatch_removed_focus: false,
            images: PluginImages::new(),
            stylesheet: StyleSheet::default(),
            composition: None,
            surface_snapshot,
            diagnostic_mount: NEXT_DIAGNOSTIC_MOUNT.fetch_add(1, Ordering::Relaxed),
            native_text_revision: std::cell::Cell::new(None),
            virtual_feedback_generation: std::cell::Cell::new(None),
            virtual_collection_presence: std::cell::Cell::new(None),
            wallpaper_demand_cache: Default::default(),
            application_image_demand_cache: Default::default(),
            virtual_measurement_epoch: 0,
            virtual_work_pending: false,
            #[cfg(test)]
            maintenance_owner_cursor: 0,
            virtual_measurement_generation: None,
            pending_frame_correlation: None,
            #[cfg(test)]
            diagnostic_patch_operations: 0,
            #[cfg(test)]
            diagnostic_patch_transport_bytes: 0,
            #[cfg(test)]
            diagnostic_patch_counters: NativePatchCounters::default(),
        };
        application.reconcile_passive_effects()?;
        Ok(application)
    }

    pub fn sync_theme_palette(
        &mut self,
        palette: nickel_core::theme::ThemePalette,
    ) -> Result<bool, String> {
        let scheme_changed = self.stylesheet.set_palette(palette)?;
        let properties_changed = self
            .stylesheet
            .set_host_properties(nickel_theme_properties(palette))?;
        let changed = scheme_changed || properties_changed;
        if changed {
            self.virtual_measurement_epoch = self.virtual_measurement_epoch.wrapping_add(1);
        }
        Ok(changed)
    }

    pub fn sync_reading_direction(&mut self, direction: twinkle_ui::ReadingDirection) -> bool {
        let changed = self.stylesheet.set_reading_direction(direction);
        if changed {
            self.virtual_measurement_epoch = self.virtual_measurement_epoch.wrapping_add(1);
        }
        changed
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
            self.virtual_measurement_epoch = self.virtual_measurement_epoch.wrapping_add(1);
        }
        changed
    }

    pub(crate) fn sync_application_images(&mut self, images: PluginImages) -> bool {
        let mut combined = self.images.clone();
        combined.retain(|key, _| {
            !key.starts_with("application:")
                && !key.starts_with("wallpaper:")
                && !key.starts_with("tray:")
        });
        combined.extend(images);
        self.sync_images(combined)
    }

    pub(crate) fn has_wallpaper_images(&self) -> bool {
        self.images
            .range::<str, _>((
                std::ops::Bound::Included("wallpaper:"),
                std::ops::Bound::Unbounded,
            ))
            .next()
            .is_some_and(|(key, _)| key.starts_with("wallpaper:"))
    }

    pub(crate) fn has_application_images(&self) -> bool {
        self.images
            .range::<str, _>((
                std::ops::Bound::Included("application:"),
                std::ops::Bound::Unbounded,
            ))
            .next()
            .is_some_and(|(key, _)| key.starts_with("application:"))
    }

    pub(crate) fn application_image_demand(&self) -> Arc<std::collections::BTreeSet<String>> {
        let generation = self.accepted.generation();
        let mut cache = self.application_image_demand_cache.borrow_mut();
        if let Some((accepted, assets)) = &*cache
            && *accepted == generation
        {
            return Arc::clone(assets);
        }
        let assets = Arc::new(self.accepted.node().application_image_assets());
        *cache = Some((generation, Arc::clone(&assets)));
        assets
    }

    pub(crate) fn wallpaper_preview_demand(
        &self,
        layout: &twinkle_ui::ResolvedLayout,
        viewport: twinkle_ui::Rect,
    ) -> Arc<Vec<String>> {
        if self.virtual_work_pending {
            return Arc::new(Vec::new());
        }
        let generation = self.accepted.generation();
        let mut cache = self.wallpaper_demand_cache.borrow_mut();
        if let Some((accepted, cached_viewport, assets)) = &*cache
            && *accepted == generation
            && *cached_viewport == viewport
        {
            return Arc::clone(assets);
        }
        let assets = Arc::new(
            self.accepted
                .node()
                .visible_wallpaper_assets(layout, viewport),
        );
        *cache = Some((generation, viewport, Arc::clone(&assets)));
        assets
    }

    pub(crate) fn has_virtual_collections(&self) -> bool {
        let generation = self.accepted.generation();
        match self.virtual_collection_presence.get() {
            Some((cached, present)) if cached == generation => present,
            _ => {
                let present = self.accepted.node().contains_virtual_collection();
                self.virtual_collection_presence
                    .set(Some((generation, present)));
                present
            }
        }
    }

    /// The last native view must belong to the admitted declaration before
    /// its layout can supply virtual row keys or scroll anchors. Host resource
    /// projection can admit another declaration before the next host step.
    pub(crate) fn native_view_matches_accepted(&self) -> bool {
        self.native_text_revision
            .get()
            .is_some_and(|(generation, _, _)| generation == self.accepted.generation())
    }

    pub(crate) fn virtual_collection_feedback(
        &self,
        generation: u64,
        layout: &twinkle_ui::ResolvedLayout,
        viewport: twinkle_ui::Rect,
    ) -> Result<Vec<PluginMessage>, String> {
        let generation_present = self.accepted.generation();
        if !self.has_virtual_collections() {
            return Ok(Vec::new());
        }
        let key = (generation, generation_present, viewport);
        if self.virtual_feedback_generation.get() == Some(key) {
            return Ok(Vec::new());
        }
        let feedback = self
            .accepted
            .node()
            .virtual_collection_feedback(layout, viewport)?;
        // A query is not acknowledgement of delivery. Only cache a settled
        // result; pending callbacks and errors must remain retryable. Include
        // accepted source and viewport, not just the host-local frame counter.
        self.virtual_feedback_generation
            .set(feedback.is_empty().then_some(key));
        Ok(feedback)
    }

    pub(crate) fn virtual_measurements(
        &self,
        generation: u64,
        layout: &twinkle_ui::ResolvedLayout,
        viewport: twinkle_ui::Rect,
        scale: f32,
    ) -> Result<Vec<twinkle_presentation::components::VirtualCollectionMeasurements>, String> {
        let accepted = self.accepted.generation();
        if self
            .virtual_collection_presence
            .get()
            .map(|(generation, _)| generation)
            != Some(accepted)
        {
            self.virtual_collection_presence.set(Some((
                accepted,
                self.accepted.node().contains_virtual_collection(),
            )));
        }
        if self.virtual_measurement_generation
            == Some((
                generation,
                accepted,
                self.virtual_measurement_epoch,
                viewport,
                scale,
            ))
            || self.virtual_collection_presence.get() == Some((accepted, false))
        {
            return Ok(Vec::new());
        }
        self.accepted.node().virtual_collection_measurements(layout)
    }

    pub(crate) fn apply_virtual_measurements(
        &mut self,
        generation: u64,
        viewport: twinkle_ui::Rect,
        scale: f32,
        measurements: &[twinkle_presentation::components::VirtualCollectionMeasurements],
    ) -> Result<bool, String> {
        let changed = self.accepted.apply_virtual_measurements(
            measurements,
            scale,
            self.virtual_measurement_epoch,
        )?;
        self.virtual_measurement_generation = Some((
            generation,
            self.accepted.generation(),
            self.virtual_measurement_epoch,
            viewport,
            scale,
        ));
        Ok(changed)
    }

    pub(crate) fn deliver_virtual_feedback(&mut self, feedback: Vec<PluginMessage>) {
        // Native range selection changes materialization, not the row template.
        let epoch = self.virtual_measurement_epoch;
        twinkle_ui::Application::update_messages(self, feedback);
        self.virtual_measurement_epoch = epoch;
    }

    pub(crate) fn virtual_anchor_candidates(
        &self,
        layout: &twinkle_ui::ResolvedLayout,
    ) -> Result<Vec<twinkle_ui::UiId>, String> {
        if !self.accepted.node().contains_virtual_collection() {
            return Ok(Vec::new());
        }
        Ok(self
            .accepted
            .node()
            .virtual_collection_measurements(layout)?
            .into_iter()
            .filter_map(|batch| batch.anchor)
            .collect())
    }

    pub fn virtual_key_focus_event(
        &self,
        collection: &twinkle_ui::UiId,
        revision: u64,
        key: &str,
        layout: &twinkle_ui::ResolvedLayout,
    ) -> Result<twinkle_ui::UiEvent, String> {
        self.accepted
            .node()
            .virtual_key_focus_event(collection, revision, key, layout)
    }

    pub(crate) fn capture_virtual_targets(
        &self,
        target: &twinkle_ui::UiId,
        layout: &twinkle_ui::ResolvedLayout,
    ) -> Result<Vec<twinkle_presentation::components::VirtualCollectionTarget>, String> {
        self.accepted.node().capture_virtual_targets(target, layout)
    }

    pub(crate) fn preserve_virtual_targets(
        &mut self,
        targets: &[twinkle_presentation::components::VirtualCollectionTarget],
    ) {
        if targets.is_empty() {
            return;
        }
        // Bound synchronous repair like ordinary viewport feedback. Each pass
        // can expose another admitted nested collection; no offscreen row tree
        // is retained merely to recover its old focus target.
        for _ in 0..8 {
            let feedback = self.accepted.node().virtual_target_feedback(targets);
            if feedback.is_empty() {
                break;
            }
            self.deliver_virtual_feedback(feedback);
        }
    }

    pub(crate) fn begin_virtual_target_repair(
        &mut self,
        targets: &[twinkle_presentation::components::VirtualCollectionTarget],
    ) {
        self.accepted.begin_virtual_target_repair(targets);
    }

    pub(crate) fn end_virtual_target_repair(&mut self) {
        self.accepted.end_virtual_target_repair();
    }

    pub(crate) fn snapshot_will_change(&self, serialized: &str) -> bool {
        self.projection_data.as_deref() != Some(serialized)
    }

    pub(crate) fn set_virtual_work_pending(&mut self, pending: bool) -> bool {
        let changed = self.virtual_work_pending != pending;
        self.virtual_work_pending = pending;
        changed
    }

    pub(crate) fn virtual_work_pending(&self) -> bool {
        self.virtual_work_pending
    }

    pub fn retained_image_bytes(&self) -> u64 {
        let mut seen = std::collections::HashSet::new();
        self.retained_image_allocations()
            .filter(|(address, _)| seen.insert(*address))
            .map(|(_, bytes)| bytes)
            .fold(0_u64, u64::saturating_add)
    }

    #[cfg(test)]
    pub(crate) fn diagnostic_mount(&self) -> u64 {
        self.diagnostic_mount
    }

    pub fn retained_image_allocations(&self) -> impl Iterator<Item = (usize, u64)> + '_ {
        self.images
            .values()
            .map(|(_, image)| (Arc::as_ptr(image) as usize, image.as_raw().len() as u64))
    }

    #[cfg(test)]
    pub(crate) fn sync_host_data_field(
        &mut self,
        field: &str,
        value: &Value,
    ) -> Result<bool, String> {
        self.sync_host_data_fields(&[(field, value)])
    }

    pub(crate) fn sync_host_data_fields(
        &mut self,
        fields: &[(&str, &Value)],
    ) -> Result<bool, String> {
        if fields.is_empty() {
            return Ok(false);
        }
        Self::validate_host_fields(&self.manifest, fields)?;
        let Some(data) = self.projection_value.as_ref() else {
            return Err("plugin has no external projection".into());
        };
        let object = data
            .as_object()
            .ok_or("external plugin projection must be an object")?;
        if fields
            .iter()
            .all(|(field, value)| object.get(*field) == Some(*value))
        {
            return Ok(false);
        }
        let mut data = data.clone();
        let object = data
            .as_object_mut()
            .ok_or("external plugin projection must be an object")?;
        for (field, value) in fields {
            if object.get(*field) != Some(*value) {
                object.insert((*field).into(), (*value).clone());
            }
        }
        if self.composition.is_some() {
            self.sync_serialized_data_inner(data.to_string(), false, Some(data))
        } else {
            self.sync_data(&data)
        }
    }

    fn validate_host_fields(
        manifest: &PluginManifest,
        fields: &[(&str, &Value)],
    ) -> Result<(), String> {
        if fields.iter().any(|(field, _)| {
            !matches!(
                *field,
                "clock"
                    | "viewport"
                    | "run"
                    | "windows"
                    | "windowMenu"
                    | "windowDestinations"
                    | "windowPreviews"
                    | "applications"
                    | "applicationSearch"
                    | "features"
                    | "shortcuts"
                    | "notifications"
                    | "keyboard"
                    | "workspaces"
                    | "desktop"
                    | "audio"
                    | "displays"
                    | "tray"
                    | "wifi"
                    | "bluetooth"
                    | "associations"
                    | "plugins"
                    | "preferences"
                    | "appearance"
                    | "wallpaper"
                    | "session"
                    | "system"
                    | "navigation"
            )
        }) {
            return Err("unknown host data field".into());
        }
        if fields.iter().any(|(field, _)| *field == "run")
            && !manifest
                .capabilities
                .contains(&PluginCapability::RunCommand)
        {
            return Err("Run data requires run-command".into());
        }
        if fields.iter().any(|(field, _)| *field == "session")
            && !manifest.capabilities.iter().any(|grant| {
                matches!(
                    grant,
                    PluginCapability::SessionControl | PluginCapability::SessionLogoutRequest
                )
            })
        {
            return Err("session data requires a session capability".into());
        }
        if fields.iter().any(|(field, _)| *field == "notifications")
            && !manifest
                .capabilities
                .contains(&PluginCapability::NotificationsRead)
        {
            return Err("notification data requires notifications.read".into());
        }
        if fields.iter().any(|(field, _)| *field == "tray")
            && !manifest.capabilities.contains(&PluginCapability::TrayRead)
        {
            return Err("tray data requires tray-read".into());
        }
        if fields.iter().any(|(field, _)| {
            matches!(
                *field,
                "windows" | "windowMenu" | "windowDestinations" | "windowPreviews"
            )
        }) && !manifest
            .capabilities
            .contains(&PluginCapability::WindowsRead)
        {
            return Err("window data requires windows-read".into());
        }
        if fields
            .iter()
            .any(|(field, _)| matches!(*field, "applications" | "applicationSearch"))
            && !manifest
                .capabilities
                .contains(&PluginCapability::ApplicationsRead)
        {
            return Err("application data requires applications-read".into());
        }
        for (field, capability) in [
            ("keyboard", PluginCapability::OnScreenKeyboardRead),
            ("workspaces", PluginCapability::WorkspacesRead),
            ("desktop", PluginCapability::DesktopControl),
        ] {
            if fields.iter().any(|(name, _)| *name == field)
                && !manifest.capabilities.contains(&capability)
            {
                return Err(format!("{field} read is not granted"));
            }
        }
        if fields.iter().any(|(field, _)| *field == "audio")
            && !manifest.capabilities.contains(&PluginCapability::AudioRead)
        {
            return Err("audio data requires audio-read".into());
        }
        if fields.iter().any(|(field, _)| *field == "displays")
            && !manifest
                .capabilities
                .contains(&PluginCapability::DisplayControl)
        {
            return Err("display data requires display-control".into());
        }
        for (field, capability) in [
            ("appearance", PluginCapability::AppearanceRead),
            ("wallpaper", PluginCapability::WallpaperRead),
            ("features", PluginCapability::FeaturesRead),
            ("shortcuts", PluginCapability::ShortcutsRead),
            ("wifi", PluginCapability::NetworkRead),
            ("bluetooth", PluginCapability::BluetoothRead),
            ("associations", PluginCapability::AssociationsRead),
            ("plugins", PluginCapability::PluginsRead),
            ("navigation", PluginCapability::SettingsRead),
            ("preferences", PluginCapability::PreferencesRead),
        ] {
            if fields.iter().any(|(name, _)| *name == field)
                && !manifest.capabilities.contains(&capability)
            {
                return Err(format!("{field} data requires a read grant"));
            }
        }
        Ok(())
    }

    pub(crate) fn composition_dependency_ids(&self) -> Vec<String> {
        self.composition
            .as_ref()
            .map(|state| {
                state
                    .manifests
                    .keys()
                    .filter(|owner| owner.id != self.manifest.id)
                    .map(|owner| owner.id.clone())
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn sync_composition_dependency_fields(
        &mut self,
        providers: &std::collections::BTreeMap<String, Vec<(&str, Value)>>,
    ) -> Result<bool, String> {
        let Some(state) = &mut self.composition else {
            return Ok(false);
        };
        let mut host = state.host.borrow_mut();
        let mut changed = false;
        for (id, fields) in providers {
            let (owner, manifest) = state
                .manifests
                .iter()
                .find(|(owner, _)| owner.id == *id)
                .ok_or("unknown composition snapshot provider")?;
            let references = fields
                .iter()
                .map(|(name, value)| (*name, value))
                .collect::<Vec<_>>();
            Self::validate_host_fields(manifest, &references)?;
            let mut data = state.snapshots.get(owner).cloned().unwrap_or_else(|| {
                host.snapshot(owner)
                    .expect("known composition owner")
                    .clone()
            });
            let object = data
                .as_object_mut()
                .ok_or("package snapshot must be an object")?;
            let changed_fields = fields
                .iter()
                .filter(|(name, value)| object.get(*name) != Some(value))
                .map(|(name, _)| *name)
                .collect::<Vec<_>>();
            if !changed_fields.is_empty() {
                tracing::warn!(dependency = %id, ?changed_fields, "composition dependency changed");
            }
            for (name, value) in fields {
                object.insert((*name).into(), value.clone());
            }
            let surface_changed = state.snapshots.get(owner) != Some(&data);
            if host.snapshot(owner)? != &data {
                host.update_snapshot(owner, &data)?;
            }
            if surface_changed {
                state.snapshots.insert(owner.clone(), data);
                changed = true;
            }
        }
        drop(host);
        if changed {
            self.refresh_composition_snapshots()?;
        }
        Ok(changed)
    }

    /// One bounded owner batch at a host-owned idle safe point. The host must
    /// schedule other dirty surfaces sharing the serviced runtime separately.
    /// GC-only work must not request layout or paint reconstruction.
    #[cfg(test)]
    pub(crate) fn service_idle_platform_tasks(&mut self) -> Result<bool, String> {
        let serviced = if let Some(state) = &self.composition {
            let mut host = state.host.borrow_mut();
            host.service_next_pending_platform_owner(&mut self.maintenance_owner_cursor)?
                .map_or(0, |(_, count)| count)
        } else {
            self.runtime.borrow_mut().service_platform_tasks()?
        };
        if serviced == 0 {
            return Ok(false);
        }
        self.reconcile_idle_work()
    }

    /// Admit already pending work for this panel without servicing more tasks.
    /// Shared-runtime sibling surfaces use this after another host services
    /// their owner. Clean surfaces retain their accepted native generation.
    pub(crate) fn reconcile_idle_work(&mut self) -> Result<bool, String> {
        if !self.idle_work_pending()? {
            return Ok(false);
        }
        let generation = self.accepted.generation();
        let transient = self.pending_transient.clone();
        self.dispatch_validated_events(Vec::new());
        if let Some(error) = self.last_error() {
            return Err(error.to_owned());
        }
        Ok(self.accepted.generation() != generation || self.pending_transient != transient)
    }

    pub(crate) fn idle_work_pending(&mut self) -> Result<bool, String> {
        if let Some(state) = &self.composition {
            state.host.borrow().mount_work_pending(&state.mount)
        } else {
            self.runtime
                .borrow_mut()
                .surface_work_pending(&self.runtime_surface_id)
        }
    }

    pub(crate) fn uses_runtime(
        &self,
        runtime: &std::rc::Rc<std::cell::RefCell<JsxRuntime>>,
    ) -> bool {
        if let Some(state) = &self.composition {
            let host = state.host.borrow();
            host.participating_owners().any(|owner| {
                host.shared_owner_runtime(owner)
                    .is_ok_and(|candidate| std::rc::Rc::ptr_eq(&candidate, runtime))
            })
        } else {
            std::rc::Rc::ptr_eq(&self.runtime, runtime)
        }
    }

    // Shared admission path for input and eventual idle reconciliation.
    // An empty batch still validates pending effects and dirty components.
    fn dispatch_validated_events(&mut self, events: Vec<Value>) {
        let previous_generation = self.accepted.generation();
        let previous_accepted = self.accepted.clone();
        let previous_effects_len = self.effects.len();
        let previous_transient = self.pending_transient.clone();
        let composition_previous_events =
            self.composition.as_ref().map(|state| state.events.clone());
        let composition_previous_native = self.composition.as_ref().map(|_| {
            (
                self.accepted.clone(),
                self.effects.clone(),
                self.pending_transient.clone(),
            )
        });
        let mut validation_rejected = false;
        let mut native_failure = None;
        #[cfg(test)]
        let applied_patch = std::cell::Cell::new(None);
        let (rendered, effects) = if let Some(state) = &mut self.composition {
            let result = (|| {
                let events = events
                    .iter()
                    .filter_map(|event| {
                        let action = match event[0].as_u64() {
                            Some(action) => action,
                            None => return Some(Err("invalid host action".into())),
                        };
                        // Resource publication can admit a newer frame between
                        // the press and release (or while a removed control's
                        // blur callback is unwinding). The native host is then
                        // allowed to finish the old transition, but its token
                        // no longer grants execution in the current event
                        // table. Reject that bounded callback locally instead
                        // of turning ordinary stale input into package failure.
                        let handle = state.events.get(&action)?.clone();
                        Some(Ok((handle, event.get(1).cloned().unwrap_or(Value::Null))))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                let mut host = state.host.borrow_mut();
                let accepted_tree = self.accepted.clone();
                let accepted_source = accepted_tree.source().clone();
                let outcome = host.dispatch_expanded_batch_scheduled_pending_validated(
                    &state.mount,
                    &events,
                    &state.events,
                    &accepted_source,
                    |patch, _, generation| {
                        let mut candidate = accepted_tree.clone();
                        if patch.operations.is_empty() {
                            return Ok(candidate);
                        }
                        let transport_bytes = serde_json::to_vec(patch)
                            .map_err(|error| error.to_string())?
                            .len();
                        #[cfg(test)]
                        applied_patch.set(Some((
                            patch.operations.len() as u64,
                            transport_bytes as u64,
                            patch.counters,
                        )));
                        candidate.apply_patch(
                            patch,
                            &self.manifest,
                            self.expected_surface_id.as_deref(),
                            &self.stylesheet,
                            generation,
                            transport_bytes,
                        )?;
                        Ok(candidate)
                    },
                )?;
                let accepted = match outcome {
                    ScheduledExpandedBatch::Unchanged => None,
                    ScheduledExpandedBatch::Rendered { rendered, .. } => {
                        let generation = rendered.generation();
                        state.events = rendered.events;
                        Some(self.accepted.readmit(
                            &rendered.node,
                            &self.manifest,
                            self.expected_surface_id.as_deref(),
                            generation,
                        )?)
                    }
                    ScheduledExpandedBatch::Patched {
                        events, validated, ..
                    } => {
                        state.events = events;
                        Some(validated)
                    }
                };
                let effects = host
                    .take_effects()
                    .into_iter()
                    .map(|effect| {
                        host.validate_effect(&effect)?;
                        let manifest = state
                            .manifests
                            .get(effect.owner())
                            .ok_or("effect owner is not installed")?
                            .clone();
                        Ok((manifest, effect.value().clone()))
                    })
                    .collect::<Result<Vec<_>, String>>()?;
                Ok((accepted, effects))
            })();
            match result {
                Ok((node, effects)) => (Ok(node), Ok(effects)),
                Err(error) => (Err(error), Ok(Vec::new())),
            }
        } else {
            let generation = self.next_generation;
            self.next_generation = self.next_generation.saturating_add(1);
            {
                let mut runtime = self.runtime.borrow_mut();
                if let Err(error) = runtime.select_surface(&self.runtime_surface_id) {
                    self.runtime_failure = Some(error.clone());
                    self.last_error = Some(error);
                    return;
                }
                let rendered = runtime
                    .dispatch_batch_patched(events, self.dispatch_removed_focus)
                    .and_then(|outcome| match outcome {
                        ScheduledPatch::Unchanged => Ok(None),
                        ScheduledPatch::Patched {
                            patch,
                            dirty_components,
                            transport_bytes,
                            ..
                        } => {
                            #[cfg(test)]
                            applied_patch.set(Some((
                                patch.operations.len() as u64,
                                transport_bytes as u64,
                                patch.counters,
                            )));
                            if patch.operations.is_empty() {
                                // Hook state can reconcile without changing
                                // this host's native presentation.
                                return Ok(None);
                            }
                            let mut candidate = self.accepted.clone();
                            let application_started = Instant::now();
                            let accepted = candidate.apply_patch(
                                &patch,
                                &self.manifest,
                                self.expected_surface_id.as_deref(),
                                &self.stylesheet,
                                generation,
                                transport_bytes,
                            );
                            let application_micros = application_started
                                .elapsed()
                                .as_micros()
                                .min(u128::from(u64::MAX))
                                as u64;
                            runtime
                                .report_typed_patch_apply(application_micros, accepted.is_ok())?;
                            accepted.inspect_err(|error| {
                                validation_rejected = true;
                                native_failure = Some((dirty_components, error.clone()));
                            })?;
                            Ok(Some(candidate))
                        }
                    });
                let effects = runtime.take_effects();
                (
                    rendered,
                    effects.map(|effects| {
                        effects
                            .into_iter()
                            .map(|effect| (self.manifest.clone(), effect))
                            .collect::<Vec<_>>()
                    }),
                )
            }
        };
        #[cfg(test)]
        if let Some((operations, transport_bytes, counters)) = applied_patch.get() {
            self.diagnostic_patch_operations =
                self.diagnostic_patch_operations.saturating_add(operations);
            self.diagnostic_patch_transport_bytes = self
                .diagnostic_patch_transport_bytes
                .saturating_add(transport_bytes);
            self.diagnostic_patch_counters.nodes_visited = self
                .diagnostic_patch_counters
                .nodes_visited
                .saturating_add(counters.nodes_visited);
            self.diagnostic_patch_counters.nodes_mutated = self
                .diagnostic_patch_counters
                .nodes_mutated
                .saturating_add(counters.nodes_mutated);
            self.diagnostic_patch_counters.local_materializations = self
                .diagnostic_patch_counters
                .local_materializations
                .saturating_add(counters.local_materializations);
            self.diagnostic_patch_counters.expansion_nodes = self
                .diagnostic_patch_counters
                .expansion_nodes
                .saturating_add(counters.expansion_nodes);
            self.diagnostic_patch_counters.tree_bytes = self
                .diagnostic_patch_counters
                .tree_bytes
                .saturating_add(counters.tree_bytes);
        }
        self.apply_rendered_effects(rendered, effects, validation_rejected);
        let candidate_accepted = self.accepted.clone();
        let candidate_effects = self.effects.split_off(previous_effects_len);
        let candidate_transient = self.pending_transient.clone();
        self.accepted = previous_accepted;
        self.pending_transient = previous_transient;
        if let Some(state) = &mut self.composition {
            let accepted = self.last_error.is_none();
            let mut host = state.host.borrow_mut();
            if host.transaction_pending()
                && let Err(error) = host.finish_transaction(accepted)
            {
                self.runtime_failure = Some(error.clone());
                self.last_error = Some(error);
            }
            if self.last_error.is_some() {
                if let Some((accepted, effects, transient)) = composition_previous_native {
                    self.accepted = accepted;
                    self.effects = effects;
                    self.pending_transient = transient;
                }
                if let Some(events) = composition_previous_events {
                    state.events = events;
                }
                // Supported bootstrap state and queued effects were restored;
                // package globals and closure mutations are outside this contract.
            } else {
                self.accepted = candidate_accepted;
                self.effects.extend(candidate_effects);
                self.pending_transient = candidate_transient;
            }
            drop(host);
            if self.last_error.is_none()
                && let Err(error) = self.reconcile_passive_effects()
            {
                self.runtime_failure = Some(error.clone());
                self.last_error = Some(error);
            }
            self.mark_frame_correlation(previous_generation);
            return;
        }

        let accepted = self.last_error.is_none();
        let finalize = {
            let mut runtime = self.runtime.borrow_mut();
            runtime
                .finish_patch_render(accepted)
                .and_then(|()| runtime.finish_event(accepted))
        };
        if let Err(error) = finalize {
            self.runtime_failure = Some(error.clone());
            self.last_error = Some(error);
        } else if let Some((owners, error)) = native_failure {
            let captured = {
                let mut runtime = self.runtime.borrow_mut();
                runtime
                    .select_surface(&self.runtime_surface_id)
                    .and_then(|()| runtime.capture_native_failure(&owners, &error))
            };
            match captured {
                Ok(boundaries) if !boundaries.is_empty() => {
                    self.last_error = None;
                    if let Err(error) = self.reconcile_passive_effects() {
                        self.runtime_failure = Some(error.clone());
                        self.last_error = Some(error);
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    self.runtime_failure = Some(error.clone());
                    self.last_error = Some(error);
                }
            }
        } else if self.last_error.is_none() {
            self.accepted = candidate_accepted;
            self.effects.extend(candidate_effects);
            self.pending_transient = candidate_transient;
            if let Err(error) = self.reconcile_passive_effects() {
                self.runtime_failure = Some(error.clone());
                self.last_error = Some(error);
            }
        }
        self.mark_frame_correlation(previous_generation);
    }

    fn apply_rendered_effects(
        &mut self,
        rendered: Result<Option<RetainedPanelTree>, String>,
        effects: Result<Vec<(PluginManifest, Value)>, String>,
        validation_rejected: bool,
    ) {
        (|| match (rendered, effects) {
            (Ok(node), Ok(effects)) => {
                let mut approved = Vec::new();
                let mut requested_dialog = None;
                let effective_node = node
                    .as_ref()
                    .map(RetainedPanelTree::node)
                    .unwrap_or_else(|| self.accepted.node());
                for (effect_manifest, effect) in effects {
                    match effect.as_str() {
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("applications.movePin") =>
                        {
                            if !effect_manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsPin)
                            {
                                self.last_error =
                                    Some("application pin grant is unavailable".into());
                                return;
                            }
                            let Some(id) = effect
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|id| !id.is_empty() && id.len() <= 256)
                            else {
                                self.last_error = Some("invalid application identity".into());
                                return;
                            };
                            let Some(direction) = effect
                                .get("direction")
                                .and_then(Value::as_i64)
                                .filter(|direction| matches!(direction, -1 | 1))
                            else {
                                self.last_error = Some("invalid pin direction".into());
                                return;
                            };
                            approved.push(PluginEffect::MoveApplicationPin {
                                id: id.into(),
                                direction: direction as i8,
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("session.perform") =>
                        {
                            let mut payload = effect.clone();
                            payload.as_object_mut().unwrap().remove("type");
                            let request = match serde_json::from_value::<
                                crate::session_capabilities::Request,
                            >(payload)
                            {
                                Ok(request) => request,
                                Err(error) => {
                                    self.last_error =
                                        Some(format!("invalid session operation: {error}"));
                                    return;
                                }
                            };
                            let snapshot = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok())
                                .and_then(|data| {
                                    serde_json::from_value::<crate::session_capabilities::Snapshot>(
                                        data.get("session")?.clone(),
                                    )
                                    .ok()
                                });
                            if snapshot.as_ref().is_none_or(|snapshot| {
                                snapshot
                                    .validate(&request, &self.manifest.capabilities)
                                    .is_err()
                            }) {
                                self.last_error = Some(
                                    "session operation is stale, unsupported, locked, or ungranted"
                                        .into(),
                                );
                                return;
                            }
                            approved.push(PluginEffect::SessionOperation {
                                plugin_id: self.manifest.id.clone(),
                                request,
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("settings.invoke") =>
                        {
                            if !effect_manifest
                                .capabilities
                                .contains(&PluginCapability::SettingsWrite)
                            {
                                self.last_error = Some("Settings write is not granted".into());
                                return;
                            }
                            let Some(provider) = effect
                                .get("provider")
                                .and_then(Value::as_str)
                                .filter(|id| !id.is_empty() && id.len() <= 128)
                            else {
                                self.last_error = Some("invalid Settings provider".into());
                                return;
                            };
                            let Some(id) = effect
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|id| !id.is_empty() && id.len() <= 128)
                            else {
                                self.last_error = Some("invalid Settings id".into());
                                return;
                            };
                            let Some(value) = effect
                                .get("value")
                                .filter(|value| value.to_string().len() <= 65536)
                            else {
                                self.last_error = Some("invalid Settings value".into());
                                return;
                            };
                            approved.push(PluginEffect::InvokeRegisteredSetting {
                                caller: effect_manifest.id.clone(),
                                provider: provider.into(),
                                id: id.into(),
                                value: value.clone(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("displays.setApplicationScale") =>
                        {
                            let request =
                                crate::application_scale_capability::ApplicationScaleEffect::parse(
                                    &effect,
                                )
                                .and_then(|request| {
                                    if !self
                                        .manifest
                                        .capabilities
                                        .contains(&PluginCapability::DisplayControl)
                                    {
                                        return Err("display control is not granted".into());
                                    }
                                    let data: Value = self
                                        .projection_data
                                        .as_deref()
                                        .and_then(|data| serde_json::from_str(data).ok())
                                        .ok_or("application scale observation is unavailable")?;
                                    request.validate(&data["displays"]["application_scale"])?;
                                    Ok(request)
                                });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::SetApplicationScale {
                                    plugin_id: self.manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("displays.identify") =>
                        {
                            if !self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::DisplayControl)
                            {
                                self.last_error = Some("display control is not granted".into());
                                return;
                            }
                            let data: Value = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str(data).ok())
                                .unwrap_or(Value::Null);
                            let Some(revision) = effect["revision"]
                                .as_str()
                                .filter(|revision| revision.len() == 16)
                            else {
                                self.last_error = Some("display observation is unavailable".into());
                                return;
                            };
                            if data["displays"]["revision"].as_str() != Some(revision)
                                || data["displays"]["operations"]["identify"] != true
                            {
                                self.last_error =
                                    Some("display identification is unavailable or stale".into());
                                return;
                            }
                            approved.push(PluginEffect::IdentifyDisplays {
                                plugin_id: self.manifest.id.clone(),
                                revision: revision.into(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("displays.setLayout") =>
                        {
                            if !effect_manifest
                                .capabilities
                                .contains(&PluginCapability::DisplayControl)
                            {
                                self.last_error = Some("display control is not granted".into());
                                return;
                            }
                            let Some(value) = effect.get("layout") else {
                                self.last_error = Some("display layout is missing".into());
                                return;
                            };
                            if value.to_string().len() > 16_384 {
                                self.last_error = Some("display layout is too large".into());
                                return;
                            }
                            let Ok(layout) = serde_json::from_value::<
                                nickel_session_protocol::OutputLayout,
                            >(value.clone()) else {
                                self.last_error = Some("display layout is invalid".into());
                                return;
                            };
                            if layout.placements.is_empty()
                                || layout.placements.len() > nickel_session_protocol::MAX_OUTPUTS
                                || layout.primary.is_empty()
                                || layout.primary.len() > 128
                                || layout.placements.iter().any(|placement| {
                                    placement.name.is_empty() || placement.name.len() > 128
                                })
                            {
                                self.last_error = Some("display layout is invalid".into());
                                return;
                            }
                            let Some(revision) = effect
                                .get("revision")
                                .and_then(Value::as_str)
                                .filter(|revision| revision.len() == 16)
                            else {
                                self.last_error = Some("display observation is unavailable".into());
                                return;
                            };
                            let data: Value = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str(data).ok())
                                .unwrap_or(Value::Null);
                            if data["displays"]["available"] != true
                                || data["displays"]["revision"].as_str() != Some(revision)
                            {
                                self.last_error = Some("display observation is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::SetDisplayLayout {
                                plugin_id: effect_manifest.id.clone(),
                                layout,
                                revision: revision.into(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("displays.confirm") =>
                        {
                            if !effect_manifest
                                .capabilities
                                .contains(&PluginCapability::DisplayControl)
                            {
                                self.last_error = Some("display control is not granted".into());
                                return;
                            }
                            approved.push(PluginEffect::ConfirmDisplayLayout {
                                plugin_id: effect_manifest.id.clone(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("displays.revert") =>
                        {
                            if !effect_manifest
                                .capabilities
                                .contains(&PluginCapability::DisplayControl)
                            {
                                self.last_error = Some("display control is not granted".into());
                                return;
                            }
                            approved.push(PluginEffect::RevertDisplayLayout {
                                plugin_id: effect_manifest.id.clone(),
                            });
                        }
                        Some("show-launcher")
                            if effect_manifest
                                .capabilities
                                .contains(&PluginCapability::LauncherShow) =>
                        {
                            approved.push(PluginEffect::ShowLauncher);
                        }

                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("show-settings")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::SettingsShow) =>
                        {
                            let screen = effect.get("screen").and_then(Value::as_str);
                            if effect.get("screen").is_some() && screen.is_none()
                                || screen.is_some_and(|screen| {
                                    !matches!(
                                        screen,
                                        "display"
                                            | "nickel-bar"
                                            | "appearance"
                                            | "network"
                                            | "bluetooth"
                                            | "bluetooth-pair"
                                            | "default-apps"
                                            | "optional-features"
                                            | "plugins"
                                            | "keyboard-shortcuts"
                                            | "about"
                                    )
                                })
                            {
                                self.last_error = Some("Settings screen is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::ShowSettings(screen.map(str::to_owned)));
                        }
                        _ if matches!(
                            effect.get("type").and_then(Value::as_str),
                            Some("show-plugin-surface" | "surface.show")
                        ) =>
                        {
                            let Some(surface_id) = effect
                                .get("surfaceId")
                                .or_else(|| effect.get("id"))
                                .and_then(Value::as_str)
                            else {
                                self.last_error = Some("plugin surface ID is invalid".into());
                                return;
                            };
                            if !effect_manifest.surfaces.iter().any(|surface| {
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
                                plugin_id: effect_manifest.id.clone(),
                                surface_id: surface_id.to_owned(),
                            });
                        }
                        _ if matches!(
                            effect.get("type").and_then(Value::as_str),
                            Some("hide-plugin-surface" | "surface.hide")
                        ) =>
                        {
                            let Some(surface_id) = effect
                                .get("surfaceId")
                                .or_else(|| effect.get("id"))
                                .and_then(Value::as_str)
                            else {
                                self.last_error = Some("plugin surface ID is invalid".into());
                                return;
                            };
                            if !effect_manifest.surfaces.iter().any(|surface| {
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
                                plugin_id: effect_manifest.id.clone(),
                                surface_id: surface_id.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("surface.focus") =>
                        {
                            let Some(surface_id) = effect
                                .get("surfaceId")
                                .or_else(|| effect.get("id"))
                                .and_then(Value::as_str)
                            else {
                                self.last_error = Some("plugin surface ID is invalid".into());
                                return;
                            };
                            if !effect_manifest.surfaces.iter().any(|surface| {
                                surface.id == surface_id
                                    && !surface.passive
                                    && matches!(
                                        surface.kind,
                                        nickel_core::plugins::PluginSurfaceKind::Window
                                            | nickel_core::plugins::PluginSurfaceKind::Dialog
                                            | nickel_core::plugins::PluginSurfaceKind::Overlay
                                    )
                            }) {
                                self.last_error =
                                    Some("focusable plugin surface is not declared".into());
                                return;
                            }
                            approved.push(PluginEffect::FocusPluginSurface {
                                plugin_id: effect_manifest.id.clone(),
                                surface_id: surface_id.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("surface.setPlacement") =>
                        {
                            let Some(surface_id) = effect
                                .get("surfaceId")
                                .or_else(|| effect.get("id"))
                                .and_then(Value::as_str)
                            else {
                                self.last_error = Some("plugin surface ID is invalid".into());
                                return;
                            };
                            if !effect_manifest.surfaces.iter().any(|surface| {
                                surface.id == surface_id
                                    && surface.kind
                                        == nickel_core::plugins::PluginSurfaceKind::Window
                            }) {
                                self.last_error = Some("plugin window is not declared".into());
                                return;
                            }
                            let Some(anchor) = effect
                                .get("anchor")
                                .and_then(|value| serde_json::from_value(value.clone()).ok())
                            else {
                                self.last_error = Some("plugin window anchor is invalid".into());
                                return;
                            };
                            let (Some(offset_x), Some(offset_y)) = (
                                effect.get("offsetX").and_then(Value::as_i64),
                                effect.get("offsetY").and_then(Value::as_i64),
                            ) else {
                                self.last_error =
                                    Some("plugin window anchor offsets are invalid".into());
                                return;
                            };
                            if !(-8192..=8192).contains(&offset_x)
                                || !(-8192..=8192).contains(&offset_y)
                            {
                                self.last_error =
                                    Some("plugin window anchor offsets exceed bounds".into());
                                return;
                            }
                            approved.push(PluginEffect::SetPluginSurfacePlacement {
                                plugin_id: effect_manifest.id.clone(),
                                surface_id: surface_id.to_owned(),
                                anchor,
                                offset_x: offset_x as i32,
                                offset_y: offset_y as i32,
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("set-plugin-setting")
                            && effect_manifest
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
                            if !effect_manifest
                                .settings
                                .iter()
                                .any(|setting| setting.id == key && setting.kind.accepts(value))
                            {
                                self.last_error = Some("plugin setting value is invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::SetPluginSetting {
                                plugin_id: effect_manifest.id.clone(),
                                key: key.to_owned(),
                                value: value.clone(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str) == Some("run.execute")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::RunCommand) =>
                        {
                            let execute = match crate::run_capabilities::Execute::parse(&effect) {
                                Ok(execute) => execute,
                                Err(error) => {
                                    self.last_error = Some(error.into());
                                    return;
                                }
                            };
                            approved.push(PluginEffect::RunExecute {
                                plugin_id: effect_manifest.id.clone(),
                                execute,
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("toggle-launcher")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::LauncherShow) =>
                        {
                            approved.push(PluginEffect::ToggleLauncher);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("toggle-control-center")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::ControlCenterShow) =>
                        {
                            approved.push(PluginEffect::ToggleControlCenter);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("show-control-center")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::ControlCenterShow) =>
                        {
                            approved.push(PluginEffect::ShowControlCenter);
                        }
                        _ if matches!(
                            effect.get("type").and_then(Value::as_str),
                            Some("window-action" | "windows.focus" | "windows.close")
                        ) =>
                        {
                            let action = match effect.get("type").and_then(Value::as_str) {
                                Some("windows.focus") => Some("activate"),
                                Some("windows.close") => Some("close"),
                                _ => effect.get("action").and_then(Value::as_str),
                            };
                            let window = effect
                                .get("window")
                                .or_else(|| effect.get("id"))
                                .and_then(Value::as_str)
                                .and_then(|value| value.parse::<u64>().ok())
                                .filter(|id| *id != 0)
                                .map(crate::model::WindowId);
                            let requested = match (action, window) {
                                (Some("activate"), Some(window))
                                    if effect_manifest
                                        .capabilities
                                        .contains(&PluginCapability::WindowsFocus) =>
                                {
                                    Some(PluginEffect::ActivateWindow(window))
                                }
                                (Some("close"), Some(window))
                                    if effect_manifest
                                        .capabilities
                                        .contains(&PluginCapability::WindowsContext) =>
                                {
                                    Some(PluginEffect::CloseWindow(window))
                                }
                                _ => None,
                            };
                            let Some(requested) = requested else {
                                self.last_error = Some("window action is invalid or denied".into());
                                return;
                            };
                            approved.push(requested);
                        }
                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| {
                                matches!(
                                    kind,
                                    "windows.showMenu"
                                        | "windows.dismissMenu"
                                        | "windows.minimize"
                                        | "windows.maximize"
                                        | "windows.restore"
                                        | "windows.toggleMaximize"
                                        | "windows.toggleFullscreen"
                                        | "windows.snapLeading"
                                        | "windows.snapTrailing"
                                        | "windows.moveToWorkspace"
                                        | "windows.moveToOutput"
                                )
                            }) =>
                        {
                            let operation = effect["type"].as_str().unwrap();
                            let restore_focus = match effect.get("restoreFocus") {
                                None => true,
                                Some(Value::Bool(value)) => *value,
                                _ => {
                                    self.last_error = Some("restoreFocus must be a boolean".into());
                                    return;
                                }
                            };
                            let destination = effect
                                .get("destination")
                                .and_then(Value::as_str)
                                .filter(|value| !value.is_empty() && value.len() <= 256)
                                .map(str::to_owned);
                            let window = effect
                                .get("id")
                                .and_then(Value::as_str)
                                .and_then(|id| id.parse::<u64>().ok())
                                .filter(|id| *id != 0)
                                .map(crate::model::WindowId);
                            if !effect_manifest
                                .capabilities
                                .contains(&PluginCapability::WindowsContext)
                                || (operation != "windows.dismissMenu" && window.is_none())
                                || (matches!(
                                    operation,
                                    "windows.moveToWorkspace" | "windows.moveToOutput"
                                ) && destination.is_none())
                            {
                                self.last_error =
                                    Some("window operation is invalid or denied".into());
                                return;
                            }
                            approved.push(PluginEffect::WindowOperation {
                                plugin_id: effect_manifest.id.clone(),
                                operation: operation.into(),
                                restore_focus,
                                destination,
                                window,
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("keyboard.toggle")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::OnScreenKeyboardShow) =>
                        {
                            approved.push(PluginEffect::ToggleOnScreenKeyboard {
                                plugin_id: effect_manifest.id.clone(),
                            });
                        }
                        _ if matches!(
                            effect.get("type").and_then(Value::as_str),
                            Some("projects.show" | "projects.toggle")
                        ) && effect_manifest
                            .capabilities
                            .contains(&PluginCapability::ProjectsMenuShow) =>
                        {
                            approved.push(PluginEffect::ProjectsVisibility {
                                plugin_id: effect_manifest.id.clone(),
                                toggle: effect.get("type").and_then(Value::as_str)
                                    == Some("projects.toggle"),
                            });
                        }

                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| kind == "tray.activate")
                            && effect_manifest
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
                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| kind == "tray.contextMenu")
                            && effect_manifest
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
                            == Some("windowPreviews.action") =>
                        {
                            match preview_request(&effect) {
                                Ok((action, capability))
                                    if effect_manifest.capabilities.contains(&capability)
                                        && effect_manifest
                                            .capabilities
                                            .contains(&PluginCapability::WindowsRead) =>
                                {
                                    let Some(revision) = effect
                                        .get("revision")
                                        .and_then(Value::as_str)
                                        .filter(|s| !s.is_empty() && s.len() <= 128)
                                    else {
                                        self.last_error =
                                            Some("preview revision is invalid".into());
                                        return;
                                    };
                                    approved.push(PluginEffect::WindowPreviewRequest {
                                        plugin_id: effect_manifest.id.clone(),
                                        revision: revision.to_owned(),
                                        action,
                                    });
                                }
                                _ => {
                                    self.last_error =
                                        Some("preview action is invalid or not granted".into());
                                    return;
                                }
                            }
                        }
                        _ if effect["type"] == "preferences.set" => {
                            let request =
                                crate::preferences_capabilities::PreferencesEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !effect_manifest
                                            .capabilities
                                            .contains(&request.capability())
                                            || !effect_manifest
                                                .capabilities
                                                .contains(&PluginCapability::PreferencesRead)
                                        {
                                            return Err(
                                                "preferences read and control are not granted"
                                                    .into(),
                                            );
                                        }
                                        let data: Value = self
                                            .projection_data
                                            .as_deref()
                                            .and_then(|data| serde_json::from_str(data).ok())
                                            .ok_or("preferences snapshot is unavailable")?;
                                        request.validate(&data["preferences"])?;
                                        Ok(request)
                                    });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::Preferences {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if matches!(
                            effect.get("type").and_then(Value::as_str),
                            Some("plugins.confirmShell" | "plugins.revertShell")
                        ) =>
                        {
                            let request =
                                crate::plugins_capabilities::ShellPreviewDecision::parse(&effect)
                                    .and_then(|request| {
                                        if !effect_manifest
                                            .capabilities
                                            .contains(&PluginCapability::PluginsRead)
                                            || !effect_manifest
                                                .capabilities
                                                .contains(&PluginCapability::PluginsControl)
                                        {
                                            return Err(
                                                "shell preview grants are unavailable".into()
                                            );
                                        }
                                        let data: Value = self
                                            .projection_data
                                            .as_deref()
                                            .and_then(|data| serde_json::from_str(data).ok())
                                            .ok_or("plugin inventory is unavailable")?;
                                        request.validate(&data["plugins"])?;
                                        Ok(request)
                                    });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::ShellPreviewDecision {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("plugins.selectShell") =>
                        {
                            let request =
                                crate::plugins_capabilities::ShellSelectionEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !effect_manifest
                                            .capabilities
                                            .contains(&PluginCapability::PluginsRead)
                                            || !effect_manifest
                                                .capabilities
                                                .contains(&PluginCapability::PluginsControl)
                                        {
                                            return Err(
                                                "shell selection grants are unavailable".into()
                                            );
                                        }
                                        let data: Value = self
                                            .projection_data
                                            .as_deref()
                                            .and_then(|data| serde_json::from_str(data).ok())
                                            .ok_or("plugin inventory is unavailable")?;
                                        request.validate(&data["plugins"])?;
                                        Ok(request)
                                    });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::ShellSelection {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("plugins.setSetting") =>
                        {
                            let request =
                                crate::plugins_capabilities::PluginsSettingEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !effect_manifest
                                            .capabilities
                                            .contains(&PluginCapability::PluginsRead)
                                            || !effect_manifest
                                                .capabilities
                                                .contains(&PluginCapability::PluginsControl)
                                        {
                                            return Err(
                                                "plugin management grants are unavailable".into()
                                            );
                                        }
                                        let data: Value = self
                                            .projection_data
                                            .as_deref()
                                            .and_then(|data| serde_json::from_str(data).ok())
                                            .ok_or("plugin inventory is unavailable")?;
                                        request.validate(&data["plugins"])?;
                                        Ok(request)
                                    });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::PluginsSetting {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|operation| operation.starts_with("plugins.")) =>
                        {
                            let request =
                                crate::plugins_capabilities::PluginsEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !effect_manifest
                                            .capabilities
                                            .contains(&PluginCapability::PluginsRead)
                                            || !effect_manifest
                                                .capabilities
                                                .contains(&PluginCapability::PluginsControl)
                                        {
                                            return Err(
                                                "plugin management grants are unavailable".into()
                                            );
                                        }
                                        let data: Value = self
                                            .projection_data
                                            .as_deref()
                                            .and_then(|data| serde_json::from_str(data).ok())
                                            .ok_or("plugin inventory is unavailable")?;
                                        request.validate(&data["plugins"])?;
                                        Ok(request)
                                    });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::Plugins {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|operation| operation.starts_with("associations.")) =>
                        {
                            let request = crate::associations_capabilities::AssociationsEffect::parse(&effect).and_then(|request| {
                                if !effect_manifest.capabilities.contains(&request.capability()) {
                                    return Err("associations control is not granted".into());
                                }
                                if matches!(request, crate::associations_capabilities::AssociationsEffect::SetDefault { .. }) {
                                    if !effect_manifest.capabilities.contains(&PluginCapability::AssociationsRead) {
                                        return Err("associations read is not granted".into());
                                    }
                                    let data: Value = self.projection_data.as_deref().and_then(|data| serde_json::from_str(data).ok()).ok_or("association snapshot is unavailable")?;
                                    request.validate(&data["associations"])?;
                                }
                                Ok(request)
                            });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::Associations {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str).is_some_and(
                            |operation| {
                                operation.starts_with("appearance.")
                                    || operation.starts_with("wallpaper.")
                            },
                        ) =>
                        {
                            let request =
                                crate::appearance_capabilities::AppearanceEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !effect_manifest
                                            .capabilities
                                            .contains(&request.capability())
                                            || !effect_manifest
                                                .capabilities
                                                .contains(&request.read_capability())
                                        {
                                            return Err(
                                                "appearance or wallpaper control is not granted"
                                                    .into(),
                                            );
                                        }
                                        let data: Value = self
                                            .projection_data
                                            .as_deref()
                                            .and_then(|data| serde_json::from_str(data).ok())
                                            .ok_or("appearance snapshot is unavailable")?;
                                        request.validate(&data[request.resource()])?;
                                        Ok(request)
                                    });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::Appearance {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str).is_some_and(
                            |operation| {
                                operation.starts_with("wifi.")
                                    || operation.starts_with("bluetooth.")
                            },
                        ) =>
                        {
                            let request =
                                crate::connectivity_capabilities::ConnectivityEffect::parse(
                                    &effect,
                                )
                                .and_then(|request| {
                                    if !effect_manifest.capabilities.contains(&request.capability())
                                    {
                                        return Err("connectivity control is not granted".into());
                                    }
                                    let data: Value = self
                                        .projection_data
                                        .as_deref()
                                        .and_then(|data| serde_json::from_str(data).ok())
                                        .ok_or("connectivity snapshot is unavailable")?;
                                    request.validate(&data[request.resource()])?;
                                    Ok(request)
                                });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::Connectivity {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect["type"]
                            .as_str()
                            .is_some_and(|kind| kind.starts_with("workspaces.")) =>
                        {
                            let parsed =
                                crate::workspace_capabilities::WorkspaceEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !effect_manifest
                                            .capabilities
                                            .contains(&request.capability())
                                        {
                                            return Err("workspace control is not granted".into());
                                        }
                                        let data: Value = self
                                            .projection_data
                                            .as_deref()
                                            .and_then(|data| serde_json::from_str(data).ok())
                                            .ok_or("workspace observation unavailable")?;
                                        request.validate(&data["workspaces"])?;
                                        Ok(request)
                                    });
                            match parsed {
                                Ok(effect) => approved.push(PluginEffect::Workspace {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect["type"] == "desktop.toggleShowDesktop" => {
                            if !effect_manifest
                                .capabilities
                                .contains(&PluginCapability::DesktopControl)
                            {
                                self.last_error = Some("desktop control is not granted".into());
                                return;
                            }
                            approved.push(PluginEffect::ToggleShowDesktop {
                                plugin_id: effect_manifest.id.clone(),
                            });
                        }
                        _ if effect["type"] == "displays.previewProjection" => {
                            let mode = effect["mode"]
                                .as_str()
                                .and_then(crate::display_capabilities::projection_mode);
                            let revision = effect["revision"]
                                .as_str()
                                .filter(|revision| revision.len() == 16);
                            if !effect_manifest
                                .capabilities
                                .contains(&PluginCapability::DisplayControl)
                                || mode.is_none()
                                || revision.is_none()
                            {
                                self.last_error =
                                    Some("display projection unavailable or invalid".into());
                                return;
                            }
                            approved.push(PluginEffect::PreviewDisplayProjection {
                                plugin_id: effect_manifest.id.clone(),
                                mode: mode.unwrap(),
                                revision: revision.unwrap().into(),
                            });
                        }
                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| kind.starts_with("keyboard.")) =>
                        {
                            let parsed = (|| -> Result<_, String> {
                                if !effect_manifest
                                    .capabilities
                                    .contains(&PluginCapability::OnScreenKeyboardInput)
                                {
                                    return Err("keyboard input is not granted".into());
                                }
                                let request =
                                    crate::keyboard_capabilities::KeyboardRequest::parse(&effect)?;
                                let data = self
                                    .projection_data
                                    .as_deref()
                                    .and_then(|data| serde_json::from_str::<Value>(data).ok())
                                    .ok_or("keyboard observation unavailable")?;
                                request.validate(&data["keyboard"])?;
                                Ok(request)
                            })();
                            match parsed {
                                Ok(effect) => approved.push(PluginEffect::Keyboard {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        Some(effect) if effect.starts_with("open-dialog:") => {
                            let id = &effect["open-dialog:".len()..];
                            match effective_node.dialog(id) {
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
                            match effective_node.menu(id) {
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
                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|operation| operation.starts_with("features.")) =>
                        {
                            let request =
                                crate::feature_capabilities::FeatureEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !effect_manifest
                                            .capabilities
                                            .contains(&PluginCapability::FeaturesControl)
                                        {
                                            return Err("feature control is not granted".into());
                                        }
                                        let data: Value = self
                                            .projection_data
                                            .as_deref()
                                            .and_then(|data| serde_json::from_str(data).ok())
                                            .ok_or("feature snapshot is unavailable")?;
                                        request.validate(&data["features"])?;
                                        Ok(request)
                                    });
                            match request {
                                Ok(effect) => approved.push(PluginEffect::Feature {
                                    plugin_id: effect_manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("applications.search") =>
                        {
                            let request = effect["query"]
                                .as_str()
                                .ok_or("application search query must be text")
                                .and_then(|query| {
                                    crate::application_capabilities::validate_query(query)
                                        .map(|()| query)
                                        .map_err(|_| "application search query exceeds its bounds")
                                });
                            if !self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsRead)
                            {
                                self.last_error =
                                    Some("application search requires applications-read".into());
                                return;
                            }
                            match request {
                                Ok(query) => approved.push(PluginEffect::SearchApplications {
                                    plugin_id: self.manifest.id.clone(),
                                    query: query.into(),
                                }),
                                Err(error) => {
                                    self.last_error = Some(error.into());
                                    return;
                                }
                            }
                        }

                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("applications.launch")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsLaunch) =>
                        {
                            let Some(id) = effect
                                .get("id")
                                .and_then(Value::as_str)
                                .filter(|id| !id.is_empty() && id.len() <= 256)
                            else {
                                self.last_error = Some("application ID is invalid".into());
                                return;
                            };
                            approved.push(PluginEffect::LaunchApplication { id: id.to_owned() });
                        }

                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("applications.togglePin")
                            && effect_manifest
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
                            approved.push(PluginEffect::ToggleApplicationPin { id: id.to_owned() });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("applications-retry-pin-save")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsPin) =>
                        {
                            approved.push(PluginEffect::RetryApplicationPinSave);
                        }

                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("notifications.invoke")
                            && effect_manifest
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
                                plugin_id: effect_manifest.id.clone(),
                                id,
                                key: key.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("notifications.dismiss")
                            && effect_manifest
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
                            approved.push(PluginEffect::DismissNotification {
                                plugin_id: effect_manifest.id.clone(),
                                id,
                            });
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
                if let Some(accepted) = node {
                    self.accepted = accepted;
                }
                self.last_error = None;
            }
            (Err(error), _) | (_, Err(error)) => {
                if !validation_rejected {
                    self.runtime_failure = Some(error.clone());
                }
                self.last_error = Some(error);
            }
        })();
    }

    fn reconcile_passive_effects(&mut self) -> Result<bool, String> {
        let mut changed = false;
        for pass in 0..MAX_EFFECT_RECONCILIATIONS {
            if self.composition.is_none() {
                let requested = {
                    let mut runtime = self.runtime.borrow_mut();
                    runtime.select_surface(&self.runtime_surface_id)?;
                    runtime.reconciliation_requested()?
                };
                if !requested {
                    return Ok(changed);
                }
            }
            let rendered = if let Some(state) = &mut self.composition {
                let mut host = state.host.borrow_mut();
                let accepted_tree = self.accepted.clone();
                let accepted_source = accepted_tree.source().clone();
                let outcome = host.reconcile_expanded_pending_validated(
                    &state.mount,
                    &state.events,
                    &accepted_source,
                    |patch, _, generation| {
                        let mut candidate = accepted_tree.clone();
                        if patch.operations.is_empty() {
                            return Ok(candidate);
                        }
                        let transport_bytes = serde_json::to_vec(patch)
                            .map_err(|error| error.to_string())?
                            .len();
                        candidate.apply_patch(
                            patch,
                            &self.manifest,
                            self.expected_surface_id.as_deref(),
                            &self.stylesheet,
                            generation,
                            transport_bytes,
                        )?;
                        Ok(candidate)
                    },
                )?;
                match outcome {
                    ScheduledExpandedBatch::Unchanged => {
                        host.finish_transaction(true)?;
                        return Ok(changed);
                    }
                    ScheduledExpandedBatch::Rendered { rendered, .. } => {
                        let candidate = self.accepted.readmit(
                            &rendered.node,
                            &self.manifest,
                            self.expected_surface_id.as_deref(),
                            rendered.generation(),
                        );
                        match candidate {
                            Ok(candidate) => {
                                host.finish_transaction(true)?;
                                state.events = rendered.events;
                                self.accepted = candidate;
                                changed = true;
                                None
                            }
                            Err(error) => {
                                host.finish_transaction(false)?;
                                self.last_error = Some(error);
                                return Ok(changed);
                            }
                        }
                    }
                    ScheduledExpandedBatch::Patched {
                        events, validated, ..
                    } => {
                        host.finish_transaction(true)?;
                        state.events = events;
                        changed |= self.accepted.generation() != validated.generation();
                        self.accepted = validated;
                        None
                    }
                }
            } else {
                let generation = self.next_generation;
                let outcome = {
                    let mut runtime = self.runtime.borrow_mut();
                    runtime.select_surface(&self.runtime_surface_id)?;
                    runtime.dispatch_batch_patched(Vec::new(), false)
                };
                match outcome {
                    Ok(ScheduledPatch::Unchanged) => {
                        self.runtime.borrow_mut().finish_event(true)?;
                        return Ok(changed);
                    }
                    Ok(ScheduledPatch::Patched {
                        patch,
                        dirty_components,
                        transport_bytes,
                        ..
                    }) => {
                        let mut candidate = self.accepted.clone();
                        let application_started = Instant::now();
                        let accepted = candidate.apply_patch(
                            &patch,
                            &self.manifest,
                            self.expected_surface_id.as_deref(),
                            &self.stylesheet,
                            generation,
                            transport_bytes,
                        );
                        let application_micros = application_started
                            .elapsed()
                            .as_micros()
                            .min(u128::from(u64::MAX))
                            as u64;
                        let mut runtime = self.runtime.borrow_mut();
                        runtime.report_typed_patch_apply(application_micros, accepted.is_ok())?;
                        runtime.finish_patch_render(accepted.is_ok())?;
                        runtime.finish_event(accepted.is_ok())?;
                        match accepted {
                            Ok(_) if patch.operations.is_empty() => None,
                            Ok(_) => {
                                self.next_generation = generation.saturating_add(1);
                                changed = true;
                                Some(candidate)
                            }
                            Err(error) => {
                                let captured =
                                    runtime.capture_native_failure(&dirty_components, &error)?;
                                if captured.is_empty() {
                                    self.last_error = Some(error);
                                    return Ok(changed);
                                }
                                self.last_error = None;
                                continue;
                            }
                        }
                    }
                    Err(error) => {
                        let _ = self.runtime.borrow_mut().finish_event(false);
                        return Err(error);
                    }
                }
            };
            if let Some(rendered) = rendered {
                self.accepted = rendered;
            }
            if pass + 1 == MAX_EFFECT_RECONCILIATIONS {
                let error = format!(
                    "passive effect reconciliation exceeded {MAX_EFFECT_RECONCILIATIONS} passes"
                );
                tracing::warn!(%error, plugin = %self.manifest.id, "plugin effect loop stopped");
                self.last_error = Some(error);
            }
        }
        Ok(changed)
    }

    /// Validate callback effects without evaluating an entry, creating a surface
    /// or installing a UI host. The runtime is the registry's existing owner.
    pub(crate) fn validate_provider_effects(
        manifest: &PluginManifest,
        runtime: std::rc::Rc<std::cell::RefCell<JsxRuntime>>,
        effects: Vec<Value>,
        snapshot: &Value,
    ) -> Result<Vec<PluginEffect>, String> {
        let accepted = RetainedPanelTree::admit(
            &serde_json::json!({"kind":"column","children":[]}),
            manifest,
            None,
            1,
        )?;
        let mut scope = Self {
            runtime,
            accepted: accepted.clone(),
            next_generation: 2,
            manifest: manifest.clone(),
            effects: Vec::new(),
            pending_transient: None,
            last_error: None,
            runtime_failure: None,
            expected_surface_id: None,
            runtime_surface_id: String::new(),
            projection_data: Some(snapshot.to_string()),
            projection_value: Some(snapshot.clone()),
            overlay_open: false,
            dispatch_removed_focus: false,
            images: PluginImages::new(),
            stylesheet: StyleSheet::compile("")?,
            composition: None,
            surface_snapshot: serde_json::json!({}),
            diagnostic_mount: NEXT_DIAGNOSTIC_MOUNT.fetch_add(1, Ordering::Relaxed),
            native_text_revision: std::cell::Cell::new(None),
            virtual_feedback_generation: std::cell::Cell::new(None),
            virtual_collection_presence: std::cell::Cell::new(None),
            wallpaper_demand_cache: Default::default(),
            application_image_demand_cache: Default::default(),
            virtual_measurement_epoch: 0,
            virtual_work_pending: false,
            #[cfg(test)]
            maintenance_owner_cursor: 0,
            virtual_measurement_generation: None,
            pending_frame_correlation: None,
            #[cfg(test)]
            diagnostic_patch_operations: 0,
            #[cfg(test)]
            diagnostic_patch_transport_bytes: 0,
            #[cfg(test)]
            diagnostic_patch_counters: NativePatchCounters::default(),
        };
        scope.apply_rendered_effects(
            Ok(Some(accepted)),
            Ok(effects
                .into_iter()
                .map(|value| (manifest.clone(), value))
                .collect()),
            false,
        );
        if let Some(error) = scope.last_error {
            return Err(error);
        }
        Ok(scope.effects)
    }

    pub fn set_overlay_open(&mut self, open: bool) {
        self.overlay_open = open;
    }

    pub fn rendered_taskbar_item_matches(&self, index: usize, id: &str) -> bool {
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

    pub fn take_effects(&mut self) -> Vec<PluginEffect> {
        std::mem::take(&mut self.effects)
    }

    pub fn last_error(&self) -> Option<&str> {
        self.last_error.as_deref()
    }

    pub fn take_runtime_failure(&mut self) -> Option<String> {
        self.runtime_failure.take()
    }

    fn mark_frame_correlation(&mut self, previous_generation: u64) {
        let generation = self.accepted.generation();
        if generation != previous_generation && self.last_error.is_none() {
            self.pending_frame_correlation = Some(twinkle_ui::NativeFrameCorrelation {
                mount: self.diagnostic_mount,
                generation,
            });
        }
    }
}

impl twinkle_ui::Application for PluginPanelApplication {
    type Message = PluginMessage;

    fn window_focus_message(&self, focused: bool) -> Option<Self::Message> {
        self.accepted
            .node()
            .window_focus_action(focused)
            .map(PluginMessage::Click)
    }

    fn shortcut_outcome(&mut self, shortcut: Shortcut) -> twinkle_ui::ShortcutOutcome {
        if self.overlay_open && shortcut == Shortcut::Escape {
            return twinkle_ui::ShortcutOutcome::from_changed(false);
        }
        if self.overlay_open && shortcut == Shortcut::Submit {
            return twinkle_ui::ShortcutOutcome::from_changed(false);
        }
        if let Some(action) = self.accepted.node().window_shortcut_action(shortcut) {
            self.update(PluginMessage::Click(action));
            return twinkle_ui::ShortcutOutcome::handled(true);
        }
        twinkle_ui::ShortcutOutcome::from_changed(false)
    }

    fn update(&mut self, message: Self::Message) {
        self.update_messages(vec![message]);
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        self.virtual_work_pending
            .then_some(std::time::Duration::from_millis(16))
    }

    fn poll(&mut self) -> bool {
        match self.reconcile_idle_work() {
            Ok(changed) => changed,
            Err(error) => {
                self.runtime_failure = Some(error.clone());
                self.last_error = Some(error);
                false
            }
        }
    }

    fn update_removed_focus(&mut self, message: Self::Message) {
        self.dispatch_removed_focus = true;
        self.update_messages(vec![message]);
        self.dispatch_removed_focus = false;
    }

    fn update_messages(&mut self, messages: Vec<Self::Message>) {
        if messages
            .iter()
            .any(|message| !matches!(message, PluginMessage::Scroll))
        {
            self.virtual_measurement_epoch = self.virtual_measurement_epoch.wrapping_add(1);
        }
        let events = messages
            .into_iter()
            .filter_map(|message| match message {
                PluginMessage::Click(action)
                | PluginMessage::Button { click: action, .. }
                | PluginMessage::Context(action) => Some(serde_json::json!([action])),
                PluginMessage::Chart(action, value) => Some(serde_json::json!([action, value])),
                PluginMessage::Text(action, value) => Some(serde_json::json!([action, value])),
                PluginMessage::Value(action, value) => Some(serde_json::json!([action, value])),
                PluginMessage::Drag(action, gesture) => {
                    let phase = match gesture.phase {
                        DragPhase::Started => "start",
                        DragPhase::Moved => "move",
                        DragPhase::Ended => "end",
                        DragPhase::Cancelled => "cancel",
                    };
                    Some(serde_json::json!([action, {
                        "phase": phase,
                        "x": gesture.position.x,
                        "y": gesture.position.y,
                        "bounds": {
                            "x": gesture.bounds.origin.x,
                            "y": gesture.bounds.origin.y,
                            "width": gesture.bounds.size.width,
                            "height": gesture.bounds.size.height,
                        },
                    }]))
                }
                PluginMessage::Drop(action, gesture) => {
                    let bounds = |rect: twinkle_ui::Rect| {
                        serde_json::json!({
                            "x": rect.origin.x,
                            "y": rect.origin.y,
                            "width": rect.size.width,
                            "height": rect.size.height,
                        })
                    };
                    Some(serde_json::json!([action, {
                        "x": gesture.position.x,
                        "y": gesture.position.y,
                        "sourceId": gesture.source_id.as_str(),
                        "sourceBounds": bounds(gesture.source_bounds),
                        "targetId": gesture.target_id.as_str(),
                        "targetBounds": bounds(gesture.target_bounds),
                    }]))
                }
                PluginMessage::Scroll => None,
            })
            .collect::<Vec<_>>();
        if events.is_empty() {
            return;
        }
        self.dispatch_validated_events(events);
    }

    fn take_frame_correlation(&mut self) -> Option<twinkle_ui::NativeFrameCorrelation> {
        self.pending_frame_correlation.take()
    }

    fn view(&self, context: ViewContext) -> impl twinkle_ui::View<Self::Message> {
        // Geometry may change on a native scroll without changing the accepted
        // JSX generation. Paint-only hover bypasses view and keeps this cache.
        self.wallpaper_demand_cache.borrow_mut().take();
        let mut view = if matches!(&self.accepted.node(), PanelNode::Surface { .. }) {
            AnyView::new(self.accepted.node().view(&self.images, &self.stylesheet))
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
                            .child(self.accepted.node().view(&self.images, &self.stylesheet))
                            .child(Spacer::flex()),
                    ),
            )
        };
        let generation = self.accepted.generation();
        let epoch = self.virtual_measurement_epoch;
        let revision = match self.native_text_revision.get() {
            Some((old_generation, old_epoch, revision))
                if old_generation == generation && old_epoch == epoch =>
            {
                revision
            }
            _ => {
                let revision = allocate_native_text_revision(&NEXT_NATIVE_TEXT_REVISION);
                self.native_text_revision
                    .set(Some((generation, epoch, revision)));
                revision
            }
        };
        if let Some(revision) = revision {
            view.set_static_text_content_revision(revision);
        }
        view
    }

    fn retain_view_for_native_layout(&self) -> bool {
        true
    }

    fn retain_view_on_resize(&self) -> bool {
        matches!(self.accepted.node(), PanelNode::Surface { .. })
    }

    fn frame_overlays(&self, _context: ViewContext) -> Vec<FrameOverlay<Self::Message>> {
        let mut overlays = Vec::new();
        let mut transients = Vec::new();
        self.accepted.node().transients(&mut transients);
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
                    ..
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
                    overlays.push(FrameOverlay::Menu(
                        self.accepted
                            .node()
                            .style_overlay_menu_for(id, menu, &self.stylesheet),
                    ));
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
        match self.accepted.node().dialog(dialog_id)? {
            PanelNode::Dialog {
                open: true,
                close_action: Some(action),
                ..
            } => Some(PluginMessage::Click(*action)),
            _ => None,
        }
    }

    fn title(&self) -> &str {
        self.accepted
            .node()
            .window_title()
            .unwrap_or(&self.manifest.name)
    }
}

#[cfg(test)]
mod tests;
