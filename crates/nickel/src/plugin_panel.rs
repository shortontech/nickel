//! Experimental JavaScript panel host. The bundled example uses the same small
//! component vocabulary as an external plugin; native surfaces remain shell-owned.

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
use nickel_plugin_presentation::components::{PanelNode, RetainedPanelTree, render_retained_panel};
pub use nickel_plugin_presentation::components::{PluginImages, PluginMessage};
#[cfg(test)]
use nickel_plugin_runtime::NativePatchCounters;
use nickel_plugin_runtime::composition_runtime::{
    ComponentEventHandle, ComponentMount, ScheduledExpandedBatch, ShellCompositionRuntime,
};
use nickel_plugin_runtime::{JsxModuleGraph, JsxRuntime, ModuleSource, ScheduledPatch};

struct CompositionPanelState {
    host: std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>,
    mount: ComponentMount,
    events: std::collections::BTreeMap<u64, ComponentEventHandle>,
    manifests: std::collections::BTreeMap<PackageIdentity, PluginManifest>,
    snapshots: std::collections::BTreeMap<PackageIdentity, Value>,
}
use nickel_ui::{
    AnyView, Column, DragPhase, FrameOverlay, Length, OverlayAnchor, OverlayId, OverlayMenu,
    OverlayStyle, Row, Shortcut, Size, Spacer, TransientSurface, UiFrame, UiId, ViewContext,
};
#[cfg(test)]
use nickel_ui::{Point, SemanticRole};
use serde_json::Value;

/// Advance a native plugin host, including viewport feedback and bounded
/// virtual-row measurement. Embedders must use this instead of stepping the
/// generic UI host alone: virtual collections initially contain no row trees.
/// Continue servicing the returned native deadline when convergence is pending.
pub fn step_host(
    host: &mut nickel_ui::UiHost<PluginPanelApplication>,
    data: Option<String>,
    batch: nickel_ui::HostBatch,
) -> Result<(nickel_ui::HostEventOutcome, u64), String> {
    crate::live_shell::step_plugin_host(host, data, batch)
}

use nickel_core::display_projection::ProjectionMode;
use nickel_plugin_presentation::css::StyleSheet;

use crate::window_preview::PreviewAction;

const MAX_EFFECT_RECONCILIATIONS: usize = 16;
static NEXT_DIAGNOSTIC_MOUNT: AtomicU64 = AtomicU64::new(1);
static NEXT_NATIVE_TEXT_REVISION: AtomicU64 = AtomicU64::new(1);

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
    StyleSheet::compile(&source)
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

type WallpaperDemandCache = Option<(u64, nickel_ui::Rect, Arc<Vec<String>>)>;

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
    virtual_feedback_generation: std::cell::Cell<Option<(u64, u64, nickel_ui::Rect)>>,
    virtual_collection_presence: std::cell::Cell<Option<(u64, bool)>>,
    wallpaper_demand_cache: std::cell::RefCell<WallpaperDemandCache>,
    application_image_demand_cache:
        std::cell::RefCell<Option<(u64, Arc<std::collections::BTreeSet<String>>)>>,
    virtual_measurement_epoch: u64,
    virtual_work_pending: bool,
    #[cfg(test)]
    maintenance_owner_cursor: usize,
    virtual_measurement_generation: Option<(u64, u64, u64, nickel_ui::Rect, f32)>,
    pending_frame_correlation: Option<nickel_ui::NativeFrameCorrelation>,
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
        Some(graph) => JsxRuntime::new_modules(&graph, data),
        None => JsxRuntime::new(&package.source, data),
    }
}

fn package_stylesheet(package: &PluginPackage) -> Result<StyleSheet, String> {
    let imported = package_module_graph(package)?
        .map(|graph| graph.stylesheet())
        .transpose()?
        .unwrap_or_default();
    StyleSheet::compile(&format!("{}\n{}", package.stylesheet, imported))
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
            let stylesheet = StyleSheet::compile(&css)?;
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
        let stylesheet = StyleSheet::compile(&css)?;
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
        let previous_data = self
            .projection_data
            .as_deref()
            .unwrap_or("{\"query\":\"\",\"results\":[]}")
            .to_owned();
        let mut validation_rejected = false;
        let generation = self.next_generation;
        self.next_generation = self.next_generation.saturating_add(1);
        let accepted = {
            let mut runtime = self.runtime.borrow_mut();
            runtime.select_surface(&self.runtime_surface_id)?;
            runtime.set_data(&serialized)?;
            let surface = serde_json::from_str::<Value>(&serialized)
                .ok()
                .and_then(|data| data.get("surface").cloned())
                .unwrap_or_else(|| serde_json::json!({}));
            runtime.set_surface_store(
                &format!("plugin-surface:{}", self.runtime_surface_id),
                &surface,
            )?;
            let result = runtime.render_current(|value| {
                self.accepted
                    .readmit_validated(
                        value,
                        &self.manifest,
                        self.expected_surface_id.as_deref(),
                        &self.stylesheet,
                        generation,
                    )
                    .inspect_err(|_| validation_rejected = true)
            });
            if result.is_err() {
                runtime.set_data(&previous_data)?;
            }
            match result {
                Ok(accepted) => accepted,
                Err(error) if validation_rejected => {
                    self.last_error = Some(error);
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        };
        self.accepted = accepted;
        self.projection_value = serde_json::from_str(&serialized).ok();
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
            std::rc::Rc::new(std::cell::RefCell::new(JsxRuntime::new(
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
                "__nickelRender()",
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
        let changed = self.stylesheet.set_palette(palette)?;
        if changed {
            self.virtual_measurement_epoch = self.virtual_measurement_epoch.wrapping_add(1);
        }
        Ok(changed)
    }

    pub fn sync_reading_direction(&mut self, direction: nickel_ui::ReadingDirection) -> bool {
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
        layout: &nickel_ui::ResolvedLayout,
        viewport: nickel_ui::Rect,
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
        layout: &nickel_ui::ResolvedLayout,
        viewport: nickel_ui::Rect,
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
        layout: &nickel_ui::ResolvedLayout,
        viewport: nickel_ui::Rect,
        scale: f32,
    ) -> Result<Vec<nickel_plugin_presentation::components::VirtualCollectionMeasurements>, String>
    {
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
        viewport: nickel_ui::Rect,
        scale: f32,
        measurements: &[nickel_plugin_presentation::components::VirtualCollectionMeasurements],
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
        nickel_ui::Application::update_messages(self, feedback);
        self.virtual_measurement_epoch = epoch;
    }

    pub(crate) fn virtual_anchor_candidates(
        &self,
        layout: &nickel_ui::ResolvedLayout,
    ) -> Result<Vec<nickel_ui::UiId>, String> {
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
        collection: &nickel_ui::UiId,
        revision: u64,
        key: &str,
        layout: &nickel_ui::ResolvedLayout,
    ) -> Result<nickel_ui::UiEvent, String> {
        self.accepted
            .node()
            .virtual_key_focus_event(collection, revision, key, layout)
    }

    pub(crate) fn capture_virtual_targets(
        &self,
        target: &nickel_ui::UiId,
        layout: &nickel_ui::ResolvedLayout,
    ) -> Result<Vec<nickel_plugin_presentation::components::VirtualCollectionTarget>, String> {
        self.accepted.node().capture_virtual_targets(target, layout)
    }

    pub(crate) fn preserve_virtual_targets(
        &mut self,
        targets: &[nickel_plugin_presentation::components::VirtualCollectionTarget],
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
        targets: &[nickel_plugin_presentation::components::VirtualCollectionTarget],
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
            self.pending_frame_correlation = Some(nickel_ui::NativeFrameCorrelation {
                mount: self.diagnostic_mount,
                generation,
            });
        }
    }
}

impl nickel_ui::Application for PluginPanelApplication {
    type Message = PluginMessage;

    fn window_focus_message(&self, focused: bool) -> Option<Self::Message> {
        self.accepted
            .node()
            .window_focus_action(focused)
            .map(PluginMessage::Click)
    }

    fn shortcut_outcome(&mut self, shortcut: Shortcut) -> nickel_ui::ShortcutOutcome {
        if self.overlay_open && shortcut == Shortcut::Escape {
            return nickel_ui::ShortcutOutcome::from_changed(false);
        }
        if self.overlay_open && shortcut == Shortcut::Submit {
            return nickel_ui::ShortcutOutcome::from_changed(false);
        }
        if let Some(action) = self.accepted.node().window_shortcut_action(shortcut) {
            self.update(PluginMessage::Click(action));
            return nickel_ui::ShortcutOutcome::handled(true);
        }
        nickel_ui::ShortcutOutcome::from_changed(false)
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
                    let bounds = |rect: nickel_ui::Rect| {
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

    fn take_frame_correlation(&mut self) -> Option<nickel_ui::NativeFrameCorrelation> {
        self.pending_frame_correlation.take()
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
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
mod tests {
    use super::*;
    use nickel_ui::Application;

    fn with_package_runtime_stack(test: impl FnOnce() + Send + 'static) {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(test)
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn wallpaper_preview_demand_uses_admitted_rows_and_retires_on_page_change() {
        with_package_runtime_stack(|| {
            let application = PluginPanelApplication::new(
                r#"
                const items = Array.from({length:1000}, (_, index) => index);
                function App() { return h(FixedWindow,{width:'100%',height:'100%'},
                    nickel.data.hide ? h(Text,null,'Keyboard shortcuts') :
                    h(ScrollView,{id:'scroll',height:100},h(VirtualColumn,{
                        id:'rows',items,itemKey:String,itemHeight:40,overscan:80,
                        renderItem:item=>h(Image,{asset:'wallpaper:'+item,width:160,height:40})
                    }))); }
            "#,
            )
            .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 320, 100);
            step_host(&mut host, None, Default::default()).unwrap();
            let viewport = nickel_ui::Rect::new(0.0, 0.0, 320.0, 100.0);
            let demand = host
                .application()
                .wallpaper_preview_demand(host.resolved_layout(), viewport);
            assert_eq!(&*demand, &["wallpaper:0", "wallpaper:1", "wallpaper:2"]);
            assert!(host.resolved_layout().nodes().len() < 100);
            for _ in 0..100 {
                let unchanged = host
                    .application()
                    .wallpaper_preview_demand(host.resolved_layout(), viewport);
                assert!(
                    Arc::ptr_eq(&demand, &unchanged),
                    "unchanged frames must not walk the image tree again"
                );
            }
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                        point: nickel_ui::Point { x: 40.0, y: 50.0 },
                        delta_y: 400.0,
                    })],
                    ..Default::default()
                },
            )
            .unwrap();
            let scrolled = host
                .application()
                .wallpaper_preview_demand(host.resolved_layout(), viewport);
            assert_eq!(
                &*scrolled,
                &["wallpaper:10", "wallpaper:11", "wallpaper:12"]
            );
            step_host(
                &mut host,
                Some(serde_json::json!({"hide":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert!(
                host.application()
                    .wallpaper_preview_demand(host.resolved_layout(), viewport)
                    .is_empty()
            );
            assert!(host.application_mut().take_effects().is_empty());
        });
    }

    #[test]
    fn virtual_collection_feedback_is_retryable_before_delivery() {
        with_package_runtime_stack(|| {
            let application = PluginPanelApplication::new(
                r#"
                const items = Array.from({length:100}, (_, index) => index);
                function App() { return h(FixedWindow, {width:'100%',height:'100%'},
                    h(ScrollView, {id:'scroller',height:100},
                        h(VirtualColumn, {id:'rows',items,itemKey:String,itemHeight:20,overscan:0,
                            renderItem:item=>h(Text, null, 'Row '+item)}))); }
                "#,
            )
            .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 320, 100);
            let viewport = nickel_ui::Rect::new(0.0, 0.0, 320.0, 100.0);
            // Zero is a valid initial generation, not an initialized-cache marker.
            let first = host
                .application()
                .virtual_collection_feedback(0, host.resolved_layout(), viewport)
                .unwrap();
            assert!(!first.is_empty());
            let retry = host
                .application()
                .virtual_collection_feedback(0, host.resolved_layout(), viewport)
                .unwrap();
            assert_eq!(
                retry, first,
                "querying must not acknowledge undelivered feedback"
            );
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            let generation = host.resolved_frame_generation();
            assert!(
                host.application()
                    .virtual_collection_feedback(generation, host.resolved_layout(), viewport,)
                    .unwrap()
                    .is_empty()
            );
            let source_generation = host.application().accepted.generation();
            assert_eq!(
                host.application().virtual_feedback_generation.get(),
                Some((generation, source_generation, viewport))
            );
            let changed_viewport = nickel_ui::Rect::new(0.0, 0.0, 320.0, 80.0);
            let resized = host
                .application()
                .virtual_collection_feedback(generation, host.resolved_layout(), changed_viewport)
                .unwrap();
            assert!(
                !resized.is_empty(),
                "a changed viewport must reselect rows even with an ancestor clip"
            );
            assert_eq!(host.application().virtual_feedback_generation.get(), None);
            assert_eq!(
                host.application()
                    .virtual_collection_feedback(
                        generation,
                        host.resolved_layout(),
                        changed_viewport,
                    )
                    .unwrap(),
                resized
            );
        });
    }

    fn logical_source(
        node: &PanelNode,
    ) -> Option<&std::sync::Arc<nickel_plugin_presentation::virtual_source::VirtualSource>> {
        if let PanelNode::Div {
            collection: Some(collection),
            ..
        } = node
            && let Some(source) = &collection.source
        {
            return Some(source);
        }
        node.container_children()
            .and_then(|children| children.iter().find_map(logical_source))
    }
    #[test]
    fn navigation_batch_can_introduce_its_first_virtual_collection() {
        with_package_runtime_stack(|| {
            let source = r#"
                const items = Array.from({length:100}, (_, i) => i);
                function App() {
                    const [open, setOpen] = useState(false);
                    return h(FixedWindow, {width:'100%',height:'100%'},
                        h(Button, {id:'open',onClick:()=>setOpen(true)}, 'Open rows'),
                        open ? h(ScrollView, {id:'scroller',height:100},
                            h(VirtualColumn, {id:'rows',items,itemKey:String,itemHeight:36,overscan:96,
                                renderItem:i=>h(Button, {id:'row-'+i,onClick:()=>{}}, 'Row '+i)})) : null);
                }
                "#;
            let make_host = || {
                let mut host =
                    nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 320, 140);
                step_host(&mut host, None, Default::default()).unwrap();
                assert!(!host.application().has_virtual_collections());
                let open = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: "Open rows".into(),
                    })
                    .unwrap();
                host.request_focus(open.id);
                step_host(&mut host, None, Default::default()).unwrap();
                host
            };
            let mut host = make_host();
            let mut sequential = make_host();
            let events = || {
                let mut events = vec![nickel_ui::HostEvent::Ui(
                    nickel_ui::UiEvent::KeyboardNavigateActivate,
                )];
                events.extend(
                    (0..33).map(|_| nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::FocusNext)),
                );
                events
            };
            for event in events() {
                step_host(
                    &mut sequential,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![event],
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            let (outcome, _) = step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: events(),
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(outcome.telemetry.events_processed, 34);
            assert!(host.application().has_virtual_collections());
            // The scroll viewport is also a focus stop: 33 advances reach row 31.
            for host in [&host, &sequential] {
                let last = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: "Row 31".into(),
                    })
                    .expect("later batch events must see rows introduced by the first event");
                assert_eq!(host.inspect().keyboard_focus, Some(last.id));
                assert!(host.resolved_layout().nodes().len() < 100);
            }
        });
    }

    #[test]
    fn virtual_controller_batches_preserve_admission_and_paired_releases() {
        with_package_runtime_stack(|| {
            use nickel_ui::{
                ControllerAction, ControllerExecutionAuthority, ControllerExecutionBinding,
                ControllerExecutionDisposition, HostBatch, HostEvent,
            };
            let source = r#"
                const items = Array.from({length:100}, (_, i) => i);
                const reversed = [...items].reverse();
                function App() { return h(FixedWindow, {width:'100%',height:'100%'},
                    h(ScrollView, {id:'scroller',height:100},
                        h(VirtualColumn, {id:'rows',items:nickel.data.reversed ? reversed : items,itemKey:String,itemHeight:36,overscan:96,
                            renderItem:i=>h(Button, {id:'row-'+i,onClick:()=>{}}, 'Row '+i)}))); }
            "#;
            let make = || {
                let mut host =
                    nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 320, 100);
                step_host(&mut host, None, Default::default()).unwrap();
                host
            };
            let mut host = make();
            let mut sequential = make();
            let authority = ControllerExecutionAuthority {
                routing_epoch: 9,
                lease_epoch: 7,
                connection_generation: 3,
                stream_generation: 2,
                cutoff: None,
                surface_generation: Some(10),
            };
            let events = |action, count: u64, start: u64| {
                (0..count * 2)
                    .map(|index| HostEvent::AdmittedController {
                        action: Some(action),
                        binding: ControllerExecutionBinding {
                            device_generation: 5,
                            edge: if index % 2 == 0 {
                                nickel_input::KeyEdge::Pressed
                            } else {
                                nickel_input::KeyEdge::Released
                            },
                            event_id: start + index,
                            routing_epoch: 9,
                            lease_epoch: 7,
                            connection_generation: 3,
                            stream_generation: 2,
                            cutoff: None,
                            surface_generation: Some(10),
                            repeat: false,
                        },
                    })
                    .collect::<Vec<_>>()
            };
            for supplied in [
                None,
                Some(ControllerExecutionAuthority {
                    routing_epoch: 8,
                    ..authority
                }),
            ] {
                let (outcome, _) = step_host(
                    &mut host,
                    None,
                    HostBatch {
                        controller_authority: supplied,
                        events: events(ControllerAction::Down, 32, 1),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(outcome.controller_executions.len(), 64);
                assert!(outcome.controller_executions.iter().all(
                    |event| event.disposition == ControllerExecutionDisposition::RejectedStale
                ));
                assert!(host.inspect().controller_target.is_none());
            }
            for (action, count, start) in [
                (ControllerAction::Down, 32, 1),
                (ControllerAction::Up, 16, 65),
            ] {
                let (outcome, _) = step_host(
                    &mut host,
                    None,
                    HostBatch {
                        controller_authority: Some(authority),
                        events: events(action, count, start),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(outcome.controller_executions.len(), (count * 2) as usize);
                assert!(
                    outcome
                        .controller_executions
                        .iter()
                        .all(|event| event.disposition == ControllerExecutionDisposition::Executed)
                );
                for event in events(action, count, start) {
                    step_host(
                        &mut sequential,
                        None,
                        HostBatch {
                            controller_authority: Some(authority),
                            events: vec![event],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
                let selected = |host: &nickel_ui::UiHost<PluginPanelApplication>| {
                    host.inspect()
                        .controller_target
                        .unwrap()
                        .as_str()
                        .rsplit('/')
                        .next()
                        .unwrap()
                        .to_owned()
                };
                assert_eq!(selected(&host), selected(&sequential));
                let row: usize = selected(&host)
                    .strip_prefix("row-")
                    .expect("controller must reach a row")
                    .parse()
                    .unwrap();
                assert!(if action == ControllerAction::Down {
                    row >= 20
                } else {
                    row < 20
                });
                assert!(host.resolved_layout().nodes().len() < 100);
            }
            let selected = host.inspect().controller_target.unwrap();
            let prior_bounds = host.resolved_layout().find(&selected).unwrap().allocated;
            step_host(
                &mut host,
                Some(serde_json::json!({"reversed":true}).to_string()),
                HostBatch::default(),
            )
            .unwrap();
            assert_eq!(host.inspect().controller_target, Some(selected.clone()));
            assert_eq!(
                host.inspect().modality,
                nickel_ui::InputModality::Controller
            );
            let bounds = host.resolved_layout().find(&selected).unwrap().allocated;
            assert!(
                bounds.origin.y >= 0.0 && bounds.origin.y + bounds.size.height <= 100.01,
                "controller bounds before={prior_bounds:?}, after={bounds:?}"
            );
            assert!(host.resolved_layout().nodes().len() < 100);
        });
    }

    #[test]
    fn virtual_navigation_batches_preserve_normalized_authority_and_replay_fences() {
        with_package_runtime_stack(|| {
            use nickel_input::{
                DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation,
                LogicalKey, ModifierState, NamedKey, PhysicalKey,
            };
            let application = PluginPanelApplication::new(
                r#"
                const items = Array.from({length:100}, (_, i) => i);
                function App() { return h(FixedWindow, {width:'100%',height:'100%'},
                    h(ScrollView, {id:'scroller',height:100},
                        h(VirtualColumn, {id:'rows',items,itemKey:String,itemHeight:36,overscan:96,
                            renderItem:i=>h(Button, {id:'row-'+i,onClick:()=>{}}, 'Row '+i)}))); }
            "#,
            )
            .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 320, 100);
            step_host(&mut host, None, Default::default()).unwrap();
            let first = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Row 0".into(),
                })
                .unwrap();
            host.request_focus(first.id.clone());
            step_host(&mut host, None, Default::default()).unwrap();
            assert_eq!(host.inspect().keyboard_focus, Some(first.id));
            let mut envelopes = Vec::new();
            let mut authorities = Vec::new();
            for order in 0..64 {
                let (event, authority) = crate::live_shell::internal_normalized_ingress(
                    InputEvent::Key(KeyEvent {
                        device: DeviceId(1),
                        order: EventOrder(order + 1),
                        physical: PhysicalKey::Code(KeyCode::Tab),
                        logical: LogicalKey::Named(NamedKey::Tab),
                        location: KeyLocation::Standard,
                        edge: if order % 2 == 0 {
                            KeyEdge::Pressed
                        } else {
                            KeyEdge::Released
                        },
                        repeat: false,
                        modifiers: ModifierState::default(),
                    }),
                    None,
                    "virtual-navigation-regression",
                    host.input_lease(),
                    None,
                );
                let nickel_ui::HostEvent::NormalizedIngress(envelope) = event else {
                    unreachable!()
                };
                envelopes.push(envelope);
                authorities.push(authority);
            }
            for (mode, expected) in [
                ("missing", 0),
                ("mismatched", 0),
                ("valid", 32),
                ("replay", 32),
            ] {
                let mut supplied = authorities.clone();
                if mode == "missing" {
                    supplied.clear();
                }
                if mode == "mismatched" {
                    for authority in &mut supplied {
                        authority.source.reconnect_generation += 1;
                    }
                }
                let (outcome, _) = step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        normalized_authorities: supplied,
                        events: envelopes
                            .iter()
                            .cloned()
                            .map(nickel_ui::HostEvent::NormalizedIngress)
                            .collect(),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(outcome.telemetry.events_processed, 64);
                let expected = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: format!("Row {expected}"),
                    })
                    .unwrap();
                assert_eq!(host.inspect().keyboard_focus, Some(expected.id), "{mode}");
                let rows = host
                    .application()
                    .accepted
                    .node()
                    .virtual_collection_measurements(host.resolved_layout())
                    .unwrap();
                // Native button height is 20.8 here, not the 36-pixel estimate.
                // Bound by the actual viewport plus overscan and edge rows.
                let minimum_height = rows[0]
                    .rows
                    .iter()
                    .map(|(_, height)| *height)
                    .fold(f32::INFINITY, f32::min);
                assert!(minimum_height > 0.0);
                let row_bound = ((100.0 + 2.0 * 96.0) / minimum_height).ceil() as usize + 2;
                assert!(
                    rows[0].rows.len() <= row_bound,
                    "{mode}: accumulated hidden rows: {:?}",
                    rows[0]
                );
                assert!(rows[0].rows.len() < 100);
                assert!(host.application_mut().take_effects().is_empty());
            }
        });
    }

    #[test]
    fn native_virtual_column_constructs_only_the_visible_jsx_rows() {
        with_package_runtime_stack(|| {
            for (mode, height) in [
                ("uniform", "20"),
                ("mixed", "item => item % 2 === 0 ? 20 : 70"),
            ] {
                let mut baseline_work = None;
                for count in [100, 1_000, 10_000] {
                    let source = format!(
                        r#"
                    let built = 0;
                    let keyed = 0;
                    const items = Array.from({{length:{count}}}, (_, index) => index);
                    function App() {{ return h(FixedWindow, {{width:'100%',height:'100%'}},
                        h(ScrollView, {{id:'scroller',height:100}},
                            h(VirtualColumn, {{id:'rows',items,itemKey:item=>{{keyed++; return String(item);}},itemHeight:{height},overscan:0,
                                renderItem:item=>{{built++; return h(Button, {{id:'row-'+item,onClick:()=>{{}}}}, 'Row '+item);}}
                            }}))); }}
                "#
                    );
                    let application = PluginPanelApplication::new(&source).unwrap();
                    assert_eq!(
                        application
                            .runtime
                            .borrow_mut()
                            .eval_json::<usize>("built")
                            .unwrap(),
                        0
                    );
                    let mut host = nickel_ui::UiHost::new(application, 320, 100);
                    crate::live_shell::step_plugin_host(&mut host, None, Default::default())
                        .unwrap();
                    let built: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    assert!(
                        built > 0 && built <= 24,
                        "constructed {built} rows from {count}"
                    );
                    assert!(host.inspect().resources.accessibility_node_count < 80);
                    let before = built;
                    crate::live_shell::step_plugin_host(&mut host, None, Default::default())
                        .unwrap();
                    let after: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    assert_eq!(after, before, "unchanged frame constructed rows");
                    let transport_before = host.application().diagnostic_patch_transport_bytes;
                    let source_before = logical_source(host.application().accepted.node()).cloned();
                    let measured = host
                        .application()
                        .accepted
                        .node()
                        .virtual_collection_measurements(host.resolved_layout())
                        .unwrap();
                    for &(ordinal, height) in &measured[0].rows {
                        assert_eq!(
                            source_before.as_ref().unwrap().geometry().height(ordinal),
                            Some(height)
                        );
                    }
                    assert!(
                        host.application()
                            .virtual_measurements(
                                host.resolved_frame_generation(),
                                host.resolved_layout(),
                                nickel_ui::Rect::new(0.0, 0.0, 320.0, 100.0),
                                1.0
                            )
                            .unwrap()
                            .is_empty(),
                        "settled frame repeated measurement collection"
                    );
                    assert!(
                        source_before.is_some(),
                        "logical keys must exist independently of visible rows"
                    );
                    assert_eq!(
                        source_before
                            .as_ref()
                            .unwrap()
                            .ordinal(&(count - 1).to_string()),
                        Some(count - 1)
                    );
                    crate::live_shell::step_plugin_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                                point: nickel_ui::Point { x: 40.0, y: 50.0 },
                                delta_y: 400.0,
                            })],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let scrolled: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    assert!(
                        scrolled > after && scrolled - after <= 24,
                        "scroll constructed {} rows from {count}",
                        scrolled - after
                    );
                    assert!(host.inspect().resources.accessibility_node_count < 80);
                    // Count all convergence passes, not only the final range.
                    let work = (built, scrolled - after);
                    if let Some(baseline) = baseline_work {
                        assert_eq!(
                            work, baseline,
                            "row-factory work grew with logical source size"
                        );
                    } else {
                        baseline_work = Some(work);
                    }
                    let keyed: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("keyed")
                        .unwrap();
                    assert_eq!(keyed, count, "scroll revalidated the full source");
                    if let Some(source) = source_before {
                        let current = logical_source(host.application().accepted.node()).unwrap();
                        assert_eq!(source.revision(), current.revision());
                        assert_eq!(
                            source.key(0).map(str::as_ptr),
                            current.key(0).map(str::as_ptr),
                            "measurement correction must share logical key storage"
                        );
                    }
                    let source_bytes = serde_json::to_vec(host.application().accepted.source())
                        .unwrap()
                        .len();
                    let transport_bytes =
                        host.application().diagnostic_patch_transport_bytes - transport_before;
                    assert!(
                        transport_bytes > 0,
                        "scroll transport measurement was not recorded"
                    );
                    assert!(
                        source_bytes < 16_384,
                        "{mode} collection retained {source_bytes} declaration bytes for {count} items"
                    );
                    assert!(
                        transport_bytes < 32_768,
                        "scroll transported {transport_bytes} bytes for {count} items"
                    );
                    eprintln!(
                        "virtual collection: mode={mode}, count={count}, source_bytes={source_bytes}, scroll_patch_bytes={transport_bytes}, rows={}",
                        scrolled - after
                    );
                }
            }
        });
    }

    #[test]
    fn virtual_zero_content_rows_have_bounded_positive_native_slots() {
        with_package_runtime_stack(|| {
            let mut baseline = None;
            for count in [100, 1_000, 10_000] {
                let source = format!(
                    r#"
                    let built = 0;
                    const items = Array.from({{length:{count}}}, (_, index) => index);
                    const itemKey = item => String(item);
                    const renderItem = item => {{ built++; return h('div',null); }};
                    function App() {{ return h(FixedWindow,{{width:'100%',height:'100%'}},
                        h(ScrollView,{{id:'scroller',height:20}},h(VirtualColumn,{{
                            id:'rows',items,itemKey,itemHeight:20,renderItem,overscan:0
                        }}))); }}
                "#
                );
                let mut host =
                    nickel_ui::UiHost::new(PluginPanelApplication::new(&source).unwrap(), 320, 20);
                let mut before = 0usize;
                let mut work = Vec::new();
                for turn in 0..16 {
                    let now = host.next_deadline().unwrap_or_else(Instant::now);
                    crate::live_shell::step_plugin_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            now: Some(now),
                            events: if turn == 0 {
                                vec![]
                            } else {
                                vec![nickel_ui::HostEvent::Poll]
                            },
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let built: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    assert!(
                        built - before <= 168,
                        "collapsed rows escaped per-turn work bound"
                    );
                    work.push(built - before);
                    before = built;
                    let rows = host
                        .application()
                        .accepted
                        .node()
                        .virtual_collection_measurements(host.resolved_layout())
                        .unwrap();
                    assert!(
                        rows[0].rows.len() <= 21,
                        "zero-height rows accumulated in the materialized slice"
                    );
                    assert!(rows[0].rows.iter().all(|(_, height)| *height == 1.0));
                    assert!(host.resolved_layout().nodes().len() < 180);
                    if !host.application().virtual_work_pending {
                        break;
                    }
                }
                assert!(!host.application().virtual_work_pending);
                assert!(host.next_deadline().is_none());
                if let Some(expected) = &baseline {
                    assert_eq!(&work, expected);
                } else {
                    baseline = Some(work);
                }
            }
        });
    }

    #[test]
    fn virtual_estimate_convergence_yields_and_retires_native_deadline() {
        with_package_runtime_stack(|| {
            let mut baseline = None;
            for count in [100, 1_000, 10_000] {
                let source = format!(
                    r#"
                    let built = 0;
                    const items = Array.from({{length:{count}}}, (_, index) => index);
                    const itemKey = item => String(item);
                    const renderItem = item => {{ built++; return h('div',{{className:'row'}},
                        h(Button,{{onClick:()=>{{}}}},'Row '+item)); }};
                    function App() {{ return h(FixedWindow,{{width:'100%',height:'100%'}},
                        nickel.data.hidden ? h(Text,null,'Hidden') :
                        h(ScrollView,{{id:'scroller',height:120}},h(VirtualColumn,{{
                            id:'rows',items,itemKey,itemHeight:8192,renderItem,overscan:0
                        }}))); }}
                "#
                );
                let make_host = || {
                    let mut application = PluginPanelApplication::new(&source).unwrap();
                    application.stylesheet = StyleSheet::compile(".row { height: 20px; }").unwrap();
                    nickel_ui::UiHost::new(application, 320, 120)
                };
                let mut host = make_host();
                let mut counts = Vec::new();
                let mut before = 0usize;
                let mut now = Instant::now();
                for turn in 0..8 {
                    let (outcome, _) = crate::live_shell::step_plugin_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            now: Some(now),
                            events: if turn == 0 {
                                vec![]
                            } else {
                                vec![nickel_ui::HostEvent::Poll]
                            },
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let built: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    let work = built - before;
                    assert!(work <= 32, "unbounded synchronous convergence: {work}");
                    counts.push(work);
                    before = built;
                    assert_eq!(outcome.next_deadline, host.next_deadline());
                    if !host.application().virtual_work_pending {
                        assert!(turn > 0, "fixture must exercise deferred convergence");
                        assert!(
                            host.next_deadline().is_none(),
                            "settled collection kept polling"
                        );
                        break;
                    }
                    now = host
                        .next_deadline()
                        .expect("pending geometry needs a native wakeup");
                }
                assert!(
                    !host.application().virtual_work_pending,
                    "positive-height estimates failed to converge"
                );
                if let Some(expected) = &baseline {
                    assert_eq!(&counts, expected);
                } else {
                    baseline = Some(counts);
                }
                let mut hidden = make_host();
                crate::live_shell::step_plugin_host(&mut hidden, None, Default::default()).unwrap();
                assert!(hidden.application().virtual_work_pending);
                crate::live_shell::step_plugin_host(
                    &mut hidden,
                    Some(serde_json::json!({"hidden":true}).to_string()),
                    Default::default(),
                )
                .unwrap();
                assert!(!hidden.application().virtual_work_pending);
                assert!(hidden.next_deadline().is_none());
                assert!(logical_source(hidden.application().accepted.node()).is_none());
            }
        });
    }

    #[test]
    fn projected_virtual_rows_resolve_new_geometry_before_anchor_measurement() {
        with_package_runtime_stack(|| {
            let source = r#"
                const items = Array.from({length:1000}, (_, index) => index);
                const reversed = [...items].reverse();
                const itemKey = item => String(item);
                function App() { return h(FixedWindow,{width:'100%',height:'100%'},
                    h(ScrollView,{id:'scroller',height:120},h(VirtualColumn,{
                        id:'rows',items:nickel.data.reversed ? reversed : items,
                        itemKey,itemHeight:24,overscan:48,
                        renderItem:item=>h(Button,{id:'row-'+item,onClick:()=>{}},'Row '+item)
                    }))); }
            "#;
            let mut host =
                nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 320, 120);
            step_host(&mut host, None, Default::default()).unwrap();
            assert!(
                host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Row 0".into(),
                })
                .is_ok()
            );
            // Match production projection: admission occurs before host.step,
            // so the old native frame cannot measure the newly admitted keys.
            host.application_mut()
                .sync_data(&serde_json::json!({"reversed":true}))
                .unwrap();
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    application_changed: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(
                host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Row 999".into(),
                })
                .is_ok()
            );
            assert!(host.application().last_error().is_none());
        });
    }

    #[test]
    fn virtual_measurements_preserve_anchor_on_preview_height_change() {
        with_package_runtime_stack(|| {
            let source = r#"
                const items = Array.from({length:1000}, (_, index) => index);
                const inserted = [-1,...items];
                const itemKey = item => String(item);
                const renderItem = item => h('div',{className:nickel.data.height === 60 ? 'tall' : 'short'},
                    h(Button,{id:'row-'+item,onClick:()=>{}},'Row '+item));
                function App() { return h(FixedWindow,{width:'100%',height:'100%'},
                    h(ScrollView,{id:'scroller',height:120},h(VirtualColumn,{
                        id:'rows',items:nickel.data.inserted ? inserted : items,itemKey,itemHeight:20,renderItem,overscan:80
                    }))); }
            "#;
            let mut application = PluginPanelApplication::new(source).unwrap();
            application.stylesheet =
                StyleSheet::compile(".short { height: 20px; } .tall { height: 60px; }").unwrap();
            let mut host = nickel_ui::UiHost::new(application, 320, 120);
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            crate::live_shell::step_plugin_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                        point: nickel_ui::Point { x: 40.0, y: 50.0 },
                        delta_y: 405.0,
                    })],
                    ..Default::default()
                },
            )
            .unwrap();
            let anchor = host
                .application()
                .virtual_anchor_candidates(host.resolved_layout())
                .unwrap()
                .remove(0);
            let y = host
                .resolved_layout()
                .find(&anchor)
                .unwrap()
                .allocated
                .origin
                .y;
            let previous_epoch = logical_source(host.application().accepted.node())
                .unwrap()
                .measurement_context()
                .unwrap();
            crate::live_shell::step_plugin_host(
                &mut host,
                Some(serde_json::json!({"height":60}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert!(
                (host
                    .resolved_layout()
                    .find(&anchor)
                    .unwrap()
                    .allocated
                    .origin
                    .y
                    - y)
                    .abs()
                    < 0.02,
                "preview arrival moved the visible keyed row"
            );
            let current = logical_source(host.application().accepted.node()).unwrap();
            assert_ne!(
                current.measurement_context().unwrap().layout_revision,
                previous_epoch.layout_revision
            );
            let rows = host
                .application()
                .accepted
                .node()
                .virtual_collection_measurements(host.resolved_layout())
                .unwrap();
            for &(row, height) in &rows[0].rows {
                assert_eq!(height, 60.0);
                assert_eq!(current.geometry().height(row), Some(height));
            }
            crate::live_shell::step_plugin_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    scale_factor: Some(1.5),
                    surface_size: Some((400, 120)),
                    ..Default::default()
                },
            )
            .unwrap();
            let context = logical_source(host.application().accepted.node())
                .unwrap()
                .measurement_context()
                .unwrap();
            assert_eq!(context.scale, 1.5);
            assert_ne!(context.width, previous_epoch.width);
            let before_insert = logical_source(host.application().accepted.node())
                .unwrap()
                .clone();
            let y_before_insert = host
                .resolved_layout()
                .find(&anchor)
                .unwrap()
                .allocated
                .origin
                .y;
            crate::live_shell::step_plugin_host(
                &mut host,
                Some(serde_json::json!({"height":60,"inserted":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            let after_insert = logical_source(host.application().accepted.node()).unwrap();
            assert_ne!(before_insert.revision(), after_insert.revision());
            assert_eq!(before_insert.identity(), after_insert.identity());
            assert!(
                (host
                    .resolved_layout()
                    .find(&anchor)
                    .unwrap()
                    .allocated
                    .origin
                    .y
                    - y_before_insert)
                    .abs()
                    < 0.02,
                "prefix insertion moved the surviving keyed anchor"
            );
        });
    }

    #[test]
    fn virtual_accessibility_key_focus_admits_only_the_requested_window() {
        with_package_runtime_stack(|| {
            for (count, mixed) in [100, 1_000, 10_000]
                .into_iter()
                .flat_map(|count| [(count, false), (count, true)])
            {
                let source = format!(
                    r#"
                    const items=Array.from({{length:{count}}},(_,index)=>index);
                    let built=0;let clicks=[];
                    function App(){{return h(FixedWindow,{{width:'100%',height:'100%'}},
                        h(ScrollView,{{id:'scroll',height:120}},h(VirtualColumn,{{
                            id:'rows',items,itemKey:String,itemHeight:24,overscan:48,
                            renderItem:item=>{{built++;return h(Button,{{id:'row-'+item,className:item%2?'tall':'short',
                                onClick:()=>clicks.push(item)}},'Row '+item)}}
                        }})))}}
                "#
                );
                let mut application = PluginPanelApplication::new(&source).unwrap();
                if mixed {
                    application.stylesheet =
                        StyleSheet::compile(".short { height: 36px; } .tall { height: 96px; }")
                            .unwrap();
                }
                let mut host = nickel_ui::UiHost::new(application, 320, 120);
                step_host(&mut host, None, Default::default()).unwrap();
                let column = host
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .find(|node| node.virtual_navigation.is_some())
                    .unwrap();
                let collection = column.id.clone();
                let revision = column.virtual_navigation.as_ref().unwrap().revision;
                let ordinal = count / 2 + 3;
                let selector = nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: format!("Row {ordinal}"),
                };
                assert!(host.query_unique(&selector).is_err());
                let before: usize = host
                    .application()
                    .runtime
                    .borrow_mut()
                    .eval_json("built")
                    .unwrap();
                assert!(
                    host.application()
                        .virtual_key_focus_event(
                            &collection,
                            revision + 1,
                            &ordinal.to_string(),
                            host.resolved_layout()
                        )
                        .is_err()
                );
                assert!(
                    host.application()
                        .virtual_key_focus_event(
                            &collection,
                            revision,
                            "missing",
                            host.resolved_layout()
                        )
                        .is_err()
                );
                let event = host
                    .application()
                    .virtual_key_focus_event(
                        &collection,
                        revision,
                        &ordinal.to_string(),
                        host.resolved_layout(),
                    )
                    .unwrap();
                assert_eq!(
                    host.application()
                        .runtime
                        .borrow_mut()
                        .eval_json::<usize>("built")
                        .unwrap(),
                    before,
                    "resolving a logical key must not build rows"
                );
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(event)],
                        ..Default::default()
                    },
                )
                .unwrap();
                let target = host.query_unique(&selector).unwrap();
                assert_eq!(host.inspect().keyboard_focus, Some(target.id));
                assert!(
                    target.bounds.origin.y >= 0.0
                        && target.bounds.origin.y + target.bounds.size.height <= 120.01
                );
                assert!(
                    host.application()
                        .runtime
                        .borrow_mut()
                        .eval_json::<usize>("built")
                        .unwrap()
                        - before
                        < 50
                );
                assert!(host.resolved_layout().nodes().len() < 150);
                assert!(
                    host.application()
                        .runtime
                        .borrow_mut()
                        .eval_json::<Vec<usize>>("JSON.stringify(clicks)")
                        .unwrap()
                        .is_empty()
                );
            }
        });
    }

    #[test]
    fn virtual_keyboard_boundaries_reach_never_materialized_rows() {
        with_package_runtime_stack(|| {
            for (count, mixed) in [100, 1_000, 10_000]
                .into_iter()
                .flat_map(|count| [(count, false), (count, true)])
            {
                let source = format!(
                    r#"
                    const items = Array.from({{length:{count}}}, (_, index) => index);
                    let built = 0;
                    let clicks = [];
                    function App() {{ return h(FixedWindow,{{width:'100%',height:'100%'}},
                        h(ScrollView,{{id:'scroll',height:120}},h(VirtualColumn,{{
                            id:'rows',items,itemKey:String,itemHeight:24,overscan:48,
                            renderItem:item=>{{ built++; return h(Button,{{id:'row-'+item,
                                className:item % 2 ? 'tall' : 'short',
                                onClick:()=>clicks.push(item)}},'Row '+item); }}
                        }}))); }}
                "#
                );
                let mut application = PluginPanelApplication::new(&source).unwrap();
                if mixed {
                    application.stylesheet =
                        StyleSheet::compile(".short { height: 36px; } .tall { height: 96px; }")
                            .unwrap();
                }
                let mut host = nickel_ui::UiHost::new(application, 320, 120);
                step_host(&mut host, None, Default::default()).unwrap();
                let selector = |ordinal| nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: format!("Row {ordinal}"),
                };
                assert!(host.query_unique(&selector(count - 1)).is_err());
                let first = host.query_unique(&selector(0)).unwrap().id;
                host.request_focus(first);
                let mut expected_clicks = Vec::new();
                for (event, ordinal) in [
                    (nickel_ui::UiEvent::KeyboardNavigateEnd, count - 1),
                    (nickel_ui::UiEvent::KeyboardNavigateStart, 0),
                ] {
                    let before: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    step_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            events: vec![nickel_ui::HostEvent::Ui(event)],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let target = host.query_unique(&selector(ordinal)).expect(
                        "boundary navigation must admit the logical boundary, not stop at overscan",
                    );
                    assert_eq!(host.inspect().controller_target, Some(target.id));
                    assert!(
                        target.bounds.origin.y >= 0.0
                            && target.bounds.origin.y + target.bounds.size.height <= 120.01
                    );
                    let after: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    assert!(after - before < 50, "boundary jump built intermediate rows");
                    assert!(host.resolved_layout().nodes().len() < 150);
                    eprintln!(
                        "virtual-boundary count={count} mixed={mixed} target={ordinal} built={} nodes={}",
                        after - before,
                        host.resolved_layout().nodes().len(),
                    );
                    assert_eq!(
                        host.application()
                            .runtime
                            .borrow_mut()
                            .eval_json::<Vec<usize>>("JSON.stringify(clicks)")
                            .unwrap(),
                        expected_clicks
                    );
                    step_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            events: vec![nickel_ui::HostEvent::Ui(
                                nickel_ui::UiEvent::KeyboardNavigateActivate,
                            )],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    expected_clicks.push(ordinal);
                    assert_eq!(
                        host.application()
                            .runtime
                            .borrow_mut()
                            .eval_json::<Vec<usize>>("JSON.stringify(clicks)")
                            .unwrap(),
                        expected_clicks
                    );
                }
            }
        });
    }

    #[test]
    fn virtual_accessibility_keys_cross_nested_sources_without_eager_rows() {
        with_package_runtime_stack(|| {
            let source = r#"
                const groups=Array.from({length:8},(_,index)=>index);
                const items=Array.from({length:128},(_,index)=>index);
                let built=0;
                function App(){return h(FixedWindow,{width:'100%',height:'100%'},
                    h(ScrollView,{id:'scroll',height:120},h(VirtualColumn,{
                        id:'groups',items:groups,itemKey:String,itemHeight:3072,overscan:0,
                        renderItem:group=>h(VirtualColumn,{
                            id:'items-'+group,items,itemKey:String,itemHeight:24,overscan:48,
                            renderItem:item=>{built++;return h(Button,{onClick:()=>{}},'Row '+group+'/'+item)}
                        })
                    })));}
            "#;
            let mut host =
                nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 320, 120);
            step_host(&mut host, None, Default::default()).unwrap();
            for (count, key, label) in [(8, "7", "Row 7/0"), (128, "77", "Row 7/77")] {
                let column = host
                    .resolved_layout()
                    .nodes()
                    .iter()
                    .find(|node| {
                        node.virtual_navigation
                            .as_ref()
                            .is_some_and(|logical| logical.count == count)
                            && (count == 8 || node.id.as_str().contains("/items-7/"))
                    })
                    .unwrap();
                let revision = column.virtual_navigation.as_ref().unwrap().revision;
                let event = host
                    .application()
                    .virtual_key_focus_event(&column.id, revision, key, host.resolved_layout())
                    .unwrap();
                let before: usize = host
                    .application()
                    .runtime
                    .borrow_mut()
                    .eval_json("built")
                    .unwrap();
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(event)],
                        ..Default::default()
                    },
                )
                .unwrap();
                let target = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: label.into(),
                    })
                    .unwrap_or_else(|error| panic!("nested key {key} in {count} expected {label}: {error:?}; admitted={:?}; collections={:?}",
                        host.accessibility_nodes().iter().filter_map(|node| node.label.as_deref()).collect::<Vec<_>>(),
                        host.resolved_layout().nodes().iter().filter(|node| node.virtual_navigation.is_some())
                            .map(|node| (&node.id, &node.virtual_navigation, node.allocated, node.clip)).collect::<Vec<_>>()));
                assert_eq!(host.inspect().keyboard_focus, Some(target.id));
                assert!(
                    target.bounds.origin.y >= 0.0
                        && target.bounds.origin.y + target.bounds.size.height <= 120.01
                );
                assert!(
                    host.application()
                        .runtime
                        .borrow_mut()
                        .eval_json::<usize>("built")
                        .unwrap()
                        - before
                        < 100
                );
                assert!(host.resolved_layout().nodes().len() < 200);
            }
        });
    }

    #[test]
    fn virtual_keyboard_boundaries_cross_nested_sources() {
        with_package_runtime_stack(|| {
            for nested in [false, true] {
                let source = r#"
                const groups = Array.from({length:8}, (_, index) => index);
                const middles = [0,1];
                const items = Array.from({length:128}, (_, index) => index);
                let built = 0;
                function rows(prefix) { return h(VirtualColumn,{
                    id:'items-'+prefix,items,itemKey:String,itemHeight:24,overscan:48,
                    renderItem:item=>{built++; return h(Button,{
                        id:'row-'+prefix+'-'+item,onClick:()=>{}
                    },'Row '+prefix+'/'+item);}
                }); }
                function App() { return h(FixedWindow,{width:'100%',height:'100%'},
                    h(ScrollView,{id:'scroll',height:120},h(VirtualColumn,{
                        id:'groups',items:groups,itemKey:String,itemHeight:nested?6144:3072,overscan:0,
                        renderItem:group=>nested ? h(VirtualColumn,{
                            id:'middle-'+group,items:middles,itemKey:String,itemHeight:3072,overscan:0,
                            renderItem:middle=>rows(group+'/'+middle)
                        }) : rows(group)
                    }))); }
            "#;
                let source = format!("const nested = {nested};\n{source}");
                let mut host =
                    nickel_ui::UiHost::new(PluginPanelApplication::new(&source).unwrap(), 320, 120);
                step_host(&mut host, None, Default::default()).unwrap();
                for (event, label) in [
                    (
                        nickel_ui::UiEvent::KeyboardNavigateEnd,
                        if nested { "Row 7/1/127" } else { "Row 7/127" },
                    ),
                    (
                        nickel_ui::UiEvent::KeyboardNavigateStart,
                        if nested { "Row 0/0/0" } else { "Row 0/0" },
                    ),
                ] {
                    let before: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    step_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            events: vec![nickel_ui::HostEvent::Ui(event)],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    let target = host
                        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                            role: SemanticRole::Button,
                            name: label.into(),
                        })
                        .expect("nested boundary must cross omitted outer and inner sources");
                    let semantic = host
                        .accessibility_nodes()
                        .iter()
                        .find(|node| node.id == target.id)
                        .unwrap();
                    assert_eq!(
                        semantic.collection_position,
                        Some((if label.ends_with("/127") { 128 } else { 1 }, 128))
                    );
                    assert_eq!(host.inspect().controller_target, Some(target.id));
                    assert!(
                        target.bounds.origin.y >= 0.0
                            && target.bounds.origin.y + target.bounds.size.height <= 120.01
                    );
                    let after: usize = host
                        .application()
                        .runtime
                        .borrow_mut()
                        .eval_json("built")
                        .unwrap();
                    assert!(after - before < 100);
                    assert!(host.resolved_layout().nodes().len() < 200);
                }
            }
        });
    }

    #[test]
    fn virtual_source_far_reorder_preserves_focused_key_and_reveals_it() {
        with_package_runtime_stack(|| {
            let source = r#"
                const items = Array.from({length:1000}, (_, index) => index);
                const reversed = [...items].reverse();
                const removed = reversed.filter(item=>item!==2);
                const itemKey = String;
                let clicks = [];
                function App() { return h(FixedWindow,{width:'100%',height:'100%'},
                    h(ScrollView,{id:'scroll',height:120},h(VirtualColumn,{
                        id:'rows',items:nickel.data.removed ? removed : nickel.data.reversed ? reversed : items,
                        itemKey,itemHeight:24,overscan:48,
                        renderItem:item=>h(Button,{id:'row-'+item,onClick:()=>clicks.push(item)},'Row '+item)
                    }))); }
            "#;
            let mut host =
                nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 320, 120);
            step_host(&mut host, None, Default::default()).unwrap();
            let selector = nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Row 2".into(),
            };
            let target = host.query_unique(&selector).unwrap().id;
            assert_eq!(
                host.accessibility_nodes()
                    .iter()
                    .find(|node| node.id == target)
                    .unwrap()
                    .collection_position,
                Some((3, 1000))
            );
            host.request_focus(target.clone());
            step_host(
                &mut host,
                Some(serde_json::json!({"reversed":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert_eq!(host.inspect().keyboard_focus, Some(target.clone()));
            let item = host
                .query_unique(&selector)
                .expect("focused logical key must materialize");
            assert_eq!(item.id, target);
            assert_eq!(
                host.accessibility_nodes()
                    .iter()
                    .find(|node| node.id == target)
                    .unwrap()
                    .collection_position,
                Some((998, 1000))
            );
            assert!(
                item.bounds.origin.y >= 0.0
                    && item.bounds.origin.y + item.bounds.size.height <= 120.01
            );
            assert!(host.resolved_layout().nodes().len() < 100);
            assert!(host.application().last_error().is_none());
            host.perform_semantic_action(
                target.clone(),
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert_eq!(
                host.application()
                    .runtime
                    .borrow_mut()
                    .eval_json::<Vec<usize>>("JSON.stringify(clicks)")
                    .unwrap(),
                vec![2]
            );
            step_host(
                &mut host,
                Some(serde_json::json!({"removed":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert_ne!(host.inspect().keyboard_focus, Some(target.clone()));
            assert!(host.query_unique(&selector).is_err());
            host.perform_semantic_action(
                target.clone(),
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert_eq!(
                host.application()
                    .runtime
                    .borrow_mut()
                    .eval_json::<Vec<usize>>("JSON.stringify(clicks)")
                    .unwrap(),
                vec![2]
            );
            step_host(
                &mut host,
                Some(serde_json::json!({"reversed":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert_ne!(host.inspect().keyboard_focus, Some(target));
            // Snapshot anchor repair precedes, rather than undoing, the wheel
            // event that arrived in the same native batch.
            assert!(
                host.resolved_layout()
                    .nodes()
                    .iter()
                    .find_map(|node| node.scroll)
                    .unwrap()
                    .offset
                    > 1000.0
            );
            step_host(
                &mut host,
                Some(serde_json::json!({"reversed":true,"label":"updated"}).to_string()),
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                        point: Point { x: 40.0, y: 50.0 },
                        delta_y: -100_000.0,
                    })],
                    ..Default::default()
                },
            )
            .unwrap();
            let scroll = host
                .resolved_layout()
                .nodes()
                .iter()
                .find_map(|node| node.scroll)
                .unwrap();
            assert_eq!(
                scroll.offset, 0.0,
                "snapshot repair undid the current wheel event"
            );
        });
    }

    #[test]
    fn virtual_nested_reorder_preserves_focused_descendant() {
        with_package_runtime_stack(|| {
            let source = r#"
                const groups = Array.from({length:8}, (_, index) => index);
                const reversedGroups = [...groups].reverse();
                const removedGroups = reversedGroups.filter(group=>group!==0);
                const rows = Array.from({length:128}, (_, index) => index);
                const reversedRows = [...rows].reverse();
                let clicks = [];
                function App() { return h(FixedWindow,{width:'100%',height:'100%'},
                    h(ScrollView,{id:'scroll',height:120},h(VirtualColumn,{
                        id:'groups',items:nickel.data.removed ? removedGroups : nickel.data.reversed ? reversedGroups : groups,
                        itemKey:String,itemHeight:3072,overscan:0,
                        renderItem:group=>h(VirtualColumn,{
                            id:'rows-'+group,items:nickel.data.reversed ? reversedRows : rows,
                            itemKey:String,itemHeight:24,overscan:48,
                            renderItem:row=>h(Button,{id:'row-'+group+'-'+row,
                                onClick:()=>clicks.push([group,row])},'Row '+group+'/'+row)
                        })
                    }))); }
            "#;
            let mut host =
                nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 320, 120);
            for _ in 0..32 {
                let now = host.next_deadline().unwrap_or_else(Instant::now);
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        now: Some(now),
                        ..Default::default()
                    },
                )
                .unwrap();
                if !host.application().virtual_work_pending() {
                    break;
                }
            }
            assert!(!host.application().virtual_work_pending());
            let selector = nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Row 0/2".into(),
            };
            let target = host.query_unique(&selector).unwrap().id;
            host.request_focus(target.clone());
            step_host(
                &mut host,
                Some(serde_json::json!({"reversed":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            for _ in 0..32 {
                assert!(
                    host.application().last_error().is_none(),
                    "{:?}",
                    host.application().last_error()
                );
                assert_eq!(host.inspect().keyboard_focus, Some(target.clone()));
                assert!(host.resolved_layout().nodes().len() < 200);
                if !host.application().virtual_work_pending() {
                    break;
                }
                let now = host
                    .next_deadline()
                    .expect("pending virtual work needs a deadline");
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        now: Some(now),
                        events: vec![nickel_ui::HostEvent::Poll],
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            assert!(!host.application().virtual_work_pending());
            let item = host.query_unique(&selector).unwrap();
            assert_eq!(item.id, target);
            assert!(
                item.bounds.origin.y >= 0.0
                    && item.bounds.origin.y + item.bounds.size.height <= 120.01
            );
            host.perform_semantic_action(
                target.clone(),
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert_eq!(
                host.application()
                    .runtime
                    .borrow_mut()
                    .eval_json::<Vec<(usize, usize)>>("JSON.stringify(clicks)")
                    .unwrap(),
                vec![(0, 2)]
            );
            step_host(
                &mut host,
                Some(serde_json::json!({"removed":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert_ne!(host.inspect().keyboard_focus, Some(target.clone()));
            assert!(host.query_unique(&selector).is_err());
            host.perform_semantic_action(
                target.clone(),
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert_eq!(
                host.application()
                    .runtime
                    .borrow_mut()
                    .eval_json::<Vec<(usize, usize)>>("JSON.stringify(clicks)")
                    .unwrap(),
                vec![(0, 2)]
            );
            step_host(
                &mut host,
                Some(serde_json::json!({"reversed":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert_ne!(host.inspect().keyboard_focus, Some(target));
            assert!(host.application().last_error().is_none());
        });
    }

    #[test]
    fn virtual_source_lifecycle_preserves_readmission_and_retires_removed_keys() {
        with_package_runtime_stack(|| {
            let source = r#"
                let clicks = [];
                const original = [{id:'a',height:20},{id:'b',height:70},{id:'c',height:40}];
                const reordered = [{id:'c',height:40},{id:'a',height:20},{id:'d',height:70}];
                const empty = [];
                const itemKey = item => item.id;
                const itemHeight = item => item.height;
                const renderItem = item => h(Button,{id:'row-'+item.id,onClick:()=>clicks.push(item.id)},'Row '+item.id);
                function App() { return h(FixedWindow,{width:'100%',height:'100%'},
                    h(Column,null,h(Text,null,nickel.data.label || 'initial'),
                        nickel.data.show === false ? h(Text,null,'Hidden') :
                            h(ScrollView,{id:'scroll',height:180},h(VirtualColumn,{
                                id:'rows',items:nickel.data.empty ? empty : nickel.data.reordered ? reordered : original,
                                itemKey,itemHeight,renderItem,overscan:0
                            })))); }
            "#;
            let application = PluginPanelApplication::new(source).unwrap();
            let mut host = nickel_ui::UiHost::new(application, 320, 240);
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            let initial = logical_source(host.application().accepted.node())
                .unwrap()
                .clone();
            let weak_initial = std::sync::Arc::downgrade(&initial);
            crate::live_shell::step_plugin_host(
                &mut host,
                Some(serde_json::json!({"label":"unrelated update"}).to_string()),
                Default::default(),
            )
            .unwrap();
            let current = logical_source(host.application().accepted.node()).unwrap();
            assert_eq!(initial.revision(), current.revision());
            assert_eq!(
                initial.key(0).map(str::as_ptr),
                current.key(0).map(str::as_ptr),
                "layout invalidation must share unchanged logical key storage"
            );
            let selector = |name: &str| nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: name.into(),
            };
            let removed = host.query_unique(&selector("Row b")).unwrap().id;
            let surviving = host.query_unique(&selector("Row c")).unwrap().id;
            host.request_focus(surviving.clone());
            assert_eq!(host.inspect().keyboard_focus, Some(surviving.clone()));
            crate::live_shell::step_plugin_host(
                &mut host,
                Some(serde_json::json!({"reordered":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            let replacement = logical_source(host.application().accepted.node())
                .unwrap()
                .clone();
            assert_ne!(replacement.revision(), initial.revision());
            assert_eq!(replacement.identity(), initial.identity());
            assert_eq!(host.query_unique(&selector("Row c")).unwrap().id, surviving);
            assert_eq!(
                host.inspect().keyboard_focus,
                Some(surviving.clone()),
                "a surviving keyed row lost focus during source replacement"
            );
            assert_eq!(replacement.ordinal("c"), Some(0));
            assert_eq!(replacement.ordinal("a"), Some(1));
            assert_eq!(replacement.ordinal("b"), None);
            assert_eq!(replacement.ordinal("d"), Some(2));
            assert!(host.query_unique(&selector("Row b")).is_err());
            host.perform_semantic_action(
                removed,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert_eq!(
                host.application()
                    .runtime
                    .borrow_mut()
                    .eval_json::<Vec<String>>("JSON.stringify(clicks)")
                    .unwrap(),
                Vec::<String>::new()
            );
            let first = host.query_unique(&selector("Row c")).unwrap();
            host.perform_semantic_action(
                first.id,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert_eq!(
                host.application()
                    .runtime
                    .borrow_mut()
                    .eval_json::<Vec<String>>("JSON.stringify(clicks)")
                    .unwrap(),
                vec!["c"]
            );
            let weak_replacement = std::sync::Arc::downgrade(&replacement);
            drop(initial);
            drop(replacement);
            assert!(
                weak_initial.upgrade().is_none(),
                "replacement retained the previous source"
            );
            crate::live_shell::step_plugin_host(
                &mut host,
                Some(serde_json::json!({"empty":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            let empty = logical_source(host.application().accepted.node()).unwrap();
            assert!(empty.is_empty());
            assert!(
                weak_replacement.upgrade().is_none(),
                "empty-list transition retained previous rows"
            );
            let weak_empty = std::sync::Arc::downgrade(empty);
            assert!(host.query_unique(&selector("Row c")).is_err());
            crate::live_shell::step_plugin_host(
                &mut host,
                Some(serde_json::json!({"show":false}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert!(logical_source(host.application().accepted.node()).is_none());
            assert!(
                weak_empty.upgrade().is_none(),
                "hidden empty collection retained its registration"
            );
            assert!(
                weak_initial.upgrade().is_none(),
                "old source survived replacement"
            );
            assert!(
                weak_replacement.upgrade().is_none(),
                "hidden collection retained native source"
            );
            crate::live_shell::step_plugin_host(
                &mut host,
                Some(serde_json::json!({"show":true}).to_string()),
                Default::default(),
            )
            .unwrap();
            assert!(host.query_unique(&selector("Row b")).is_ok());
            assert_ne!(
                host.query_unique(&selector("Row c")).unwrap().id,
                surviving,
                "a retired collection must not revive its old native row identity"
            );
            assert_eq!(
                logical_source(host.application().accepted.node())
                    .unwrap()
                    .ordinal("b"),
                Some(1)
            );
        });
    }

    #[test]
    fn public_session_callback_emits_native_operation_and_rejects_ungranted_or_stale_requests() {
        use nickel_ui::Application;
        let snapshot = serde_json::json!({"session":{"revision":"current","account":{"displayName":"User","username":"user"},"locked":false,"support":{"lock":true,"logout":true,"suspend":false,"reboot":false,"powerOff":false,"restartShell":false}}});
        let source = "function App(){return h(FixedWindow,{width:'100%',height:'100%'},h(Button,{onClick:()=>nickel.session.logout()},'Logout'))}";
        let mut granted_manifest = manifest().clone();
        granted_manifest
            .capabilities
            .push(PluginCapability::SessionControl);
        let mut granted = PluginPanelApplication::new_with_manifest(
            source,
            &granted_manifest,
            Some(snapshot.to_string()),
        )
        .unwrap();
        granted.update(PluginMessage::Click(0));
        let effects = granted.take_effects();
        assert!(
            matches!(&effects[..],[PluginEffect::SessionOperation { plugin_id, request }] if plugin_id==&granted_manifest.id && request.action==crate::session_capabilities::Action::Logout && request.revision=="current")
        );
        let mut ungranted_manifest = granted_manifest.clone();
        ungranted_manifest.capabilities.retain(|grant| {
            !matches!(
                grant,
                PluginCapability::SessionControl | PluginCapability::SessionLogoutRequest
            )
        });
        let mut ungranted = PluginPanelApplication::new_with_manifest(
            source,
            &ungranted_manifest,
            Some(snapshot.to_string()),
        )
        .unwrap();
        ungranted.update(PluginMessage::Click(0));
        assert!(ungranted.take_effects().is_empty());
        assert!(ungranted.last_error.is_some());
        let source = "function App(){return h(FixedWindow,{width:'100%',height:'100%'},h(Button,{onClick:()=>nickel.request({type:'session.perform',action:'logout',revision:'old'})},'Logout'))}";
        let mut stale = PluginPanelApplication::new_with_manifest(
            source,
            &granted_manifest,
            Some(snapshot.to_string()),
        )
        .unwrap();
        stale.update(PluginMessage::Click(0));
        assert!(stale.take_effects().is_empty());
        assert!(stale.last_error.is_some());
    }

    #[test]
    fn copied_derived_shell_validates_against_explicit_dependency_catalog() {
        let root = tempfile::tempdir().unwrap();
        for entry in std::fs::read_dir(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-shell"
        ))
        .unwrap()
        {
            let entry = entry.unwrap();
            std::fs::copy(entry.path(), root.path().join(entry.file_name())).unwrap();
        }
        let package = PluginPackage::load(root.path()).unwrap();
        let base = crate::bundled_plugin_assets::load_package("nickel-default").unwrap();
        let mut catalog = std::collections::BTreeMap::from([("nickel-default".into(), base)]);
        let mut stale = package.clone();
        stale
            .manifest
            .composition
            .as_mut()
            .unwrap()
            .requires
            .insert("nickel-default".into(), "^99".into());
        catalog.insert(package.manifest.id.clone(), stale);
        // The copy being checked overrides any installed package with the same ID.
        PluginPanelApplication::validate_package_with_catalog(&package, &catalog).unwrap();
        catalog.remove("nickel-default");
        assert!(
            PluginPanelApplication::validate_package_with_catalog(&package, &catalog)
                .unwrap_err()
                .contains("nickel-default")
        );
    }

    #[test]
    fn embedded_default_shell_surfaces_share_runtime_and_settings_registry() {
        std::thread::Builder::new()
            .stack_size(16 * 1024 * 1024)
            .spawn(|| {
        let package = crate::bundled_plugin_assets::load_package("nickel-default").unwrap();
        assert_eq!(package.manifest.entry, "src/Shell.tsx");
        assert!(
            package
                .manifest
                .capabilities
                .contains(&PluginCapability::PluginsRead)
        );
        assert!(
            package
                .manifest
                .capabilities
                .contains(&PluginCapability::PluginsControl)
        );
        assert_eq!(
            package.manifest.composition.as_ref().unwrap().exports["shell.settings.plugins"],
            "./src/Plugins.tsx#Plugins"
        );
        assert!(
            package
                .modules
                .iter()
                .any(|module| module.path == "src/Shell.tsx")
        );
        let runtime = PluginPanelApplication::shared_package_runtime(
            &package,
            &Default::default(),
            &package.manifest.surfaces[0],
        )
        .unwrap();
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        runtime
            .borrow_mut()
            .publish_settings(&mut registry, &package.manifest.id)
            .unwrap();
        let pages = registry.settings_pages_snapshot();
        assert!(
            pages
                .pages
                .iter()
                .any(|page| page.registration.id == "plugins")
        );
        assert!(
            pages
                .pages
                .iter()
                .any(|page| page.registration.id == "appearance")
        );
        assert!(
            pages
                .pages
                .iter()
                .any(|page| page.registration.id == "default-apps")
        );
        let mut applications = Vec::new();
        for surface in &package.manifest.surfaces {
            let application = PluginPanelApplication::from_package_surface_with_runtime(
                &package,
                &Default::default(),
                surface,
                PluginImages::new(),
                Some(runtime.clone()),
            )
            .unwrap_or_else(|error| panic!("surface {}: {error}", surface.id));
            assert_eq!(
                application.resolved_surface(surface).unwrap().id,
                surface.id
            );
            assert!(std::rc::Rc::ptr_eq(&application.shared_runtime(), &runtime));
            applications.push(application);
        }
        assert!(!applications.is_empty());
        // Constructing additional surfaces must not initialize registration modules again.
        runtime
            .borrow_mut()
            .publish_settings(&mut registry, &package.manifest.id)
            .unwrap();
        assert_eq!(registry.settings_pages_snapshot().pages, pages.pages);
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn hidden_registered_setting_effects_use_existing_owner_without_rendering_an_entry() {
        let mut manifest = super::manifest().clone();
        manifest.capabilities = vec![PluginCapability::LauncherShow];
        let runtime=std::rc::Rc::new(std::cell::RefCell::new(JsxRuntime::new(
            "registerSetting({id:'toggle',group:'Test',label:'Toggle',type:'switch',defaultValue:false,onChange:()=>nickel.request('show-launcher')}); function App(){throw Error('hidden entry must not be evaluated');}",None).unwrap()));
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        runtime
            .borrow_mut()
            .publish_settings(&mut registry, &manifest.id)
            .unwrap();
        runtime
            .borrow_mut()
            .invoke_setting(&manifest.id, "toggle", &Value::Bool(true))
            .unwrap();
        let effects = runtime.borrow_mut().take_effects().unwrap();
        assert_eq!(
            PluginPanelApplication::validate_provider_effects(
                &manifest,
                runtime.clone(),
                effects,
                &serde_json::json!({})
            )
            .unwrap(),
            vec![PluginEffect::ShowLauncher]
        );
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<usize>("JSON.stringify(__surfaceApps.size)")
                .unwrap(),
            0
        );
        manifest.capabilities.clear();
        let accepted_revision = runtime.borrow().settings_revision();
        runtime.borrow_mut().begin_transaction().unwrap();
        runtime
            .borrow_mut()
            .invoke_setting(&manifest.id, "toggle", &Value::Bool(false))
            .unwrap();
        let effects = runtime.borrow_mut().take_effects().unwrap();
        assert!(
            PluginPanelApplication::validate_provider_effects(
                &manifest,
                runtime.clone(),
                effects,
                &serde_json::json!({})
            )
            .is_err()
        );
        runtime.borrow_mut().finish_transaction(false).unwrap();
        assert_eq!(runtime.borrow().settings_revision(), accepted_revision);
        assert!(runtime.borrow_mut().take_effects().unwrap().is_empty());
    }

    #[test]
    fn composed_shell_replacement_uses_production_parser_and_owner_effect_validation() {
        fn package(
            id: &str,
            source: &str,
            base: Option<&str>,
            grants: Vec<PluginCapability>,
        ) -> PluginPackage {
            let mut manifest = super::manifest().clone();
            manifest.id = id.into();
            manifest.capabilities = grants;
            let composition = serde_json::from_value(serde_json::json!({
                "api_version": 1, "id": id, "version":"0.1.0",
                "exports": if base.is_none() { serde_json::json!({"shell":"./main.js#Shell", "shell.taskbar":"./main.js#Taskbar"}) } else { serde_json::json!({}) },
                "extends": base,
                "requires": base.map(|base| serde_json::json!({base:"^0.1"})).unwrap_or_else(|| serde_json::json!({})),
                "replaces": if base.is_some() { serde_json::json!({"shell.taskbar":"./main.js#Taskbar"}) } else { serde_json::json!({}) }
            })).unwrap();
            manifest.composition = Some(composition);
            PluginPackage {
                manifest,
                source: source.into(),
                stylesheet: String::new(),
                modules: Vec::new(),
                images: std::collections::BTreeMap::new(),
            }
        }
        for granted in [true, false] {
            let mut base = package(
                "base-shell",
                "globalThis.origin = 'base';\nexport function Shell() { return h(Window, {id:'main',placement:'fixed',width:440,height:220,edge:'bottom',bottomOffset:24,output:'all'}, h(nickel.component('shell.taskbar'), {onChange:()=>nickel.request('show-launcher')}, h(Button,{id:'owned-child',icon:'shared',onClick:()=>nickel.request('show-launcher')},'Owned child')), nickel.contributions('taskbar.items').map(entry => h(entry.component, {key:entry.key}))); }\nexport function Taskbar() { return h(Button, {id:'base',onClick:()=>nickel.request('show-launcher')}, origin); }\nexport default Shell;",
                None,
                vec![
                    PluginCapability::LauncherShow,
                    PluginCapability::WindowsRead,
                ],
            );
            let mut child = package(
                "derived-shell",
                "globalThis.origin = 'derived';\nexport function Taskbar(props) { const [count,setCount] = useState(0); const [hidden,setHidden] = useState(0); globalThis.setIdleHidden = setHidden; globalThis.setIdleCount = setCount; return h(Column,null,h(Button,{id:'callback-control',onClick:()=>props.onChange('changed')},'Callback'),h(Button, {id:'replacement',icon:'shared',onClick:()=>{setCount(count+1); nickel.request('show-launcher');}}, origin + count),...props.children); }\nexport function Widget() { return h(Button, {id:'contribution',onClick:()=>nickel.request('show-launcher')}, 'Owned contribution'); }\nexport default Taskbar;",
                Some("base-shell"),
                if granted {
                    vec![PluginCapability::LauncherShow]
                } else {
                    Vec::new()
                },
            );
            child
                .manifest
                .composition
                .as_mut()
                .unwrap()
                .contributions
                .push(nickel_core::package_composition::SemanticContribution {
                    collection: "taskbar.items".into(),
                    id: "widget".into(),
                    implementation: "./main.js#Widget".into(),
                    priority: 10,
                });
            fn png(color: [u8; 4]) -> Vec<u8> {
                let image = image::RgbaImage::from_pixel(1, 1, image::Rgba(color));
                let mut output = std::io::Cursor::new(Vec::new());
                image::DynamicImage::ImageRgba8(image)
                    .write_to(&mut output, image::ImageFormat::Png)
                    .unwrap();
                output.into_inner()
            }
            base.images.insert("shared".into(), png([255, 0, 0, 255]));
            child.images.insert("shared".into(), png([0, 0, 255, 255]));
            let surface = child.manifest.surfaces[0].clone();
            let catalog = std::collections::BTreeMap::from([
                ("base-shell".into(), base),
                ("derived-shell".into(), child),
            ]);
            let mut application = PluginPanelApplication::from_composed_surface(
                &catalog,
                "derived-shell",
                &std::collections::BTreeMap::new(),
                &surface,
                None,
            )
            .unwrap();
            assert_eq!(application.images.len(), 2);
            let shared = application.shared_composition_runtime().unwrap();
            let asset = |id: &str| {
                let host = shared.borrow();
                let owner = host
                    .participating_owners()
                    .find(|owner| owner.id == id)
                    .unwrap();
                host.asset_key(owner, "shared").unwrap().to_owned()
            };
            let base_asset = asset("base-shell");
            let child_asset = asset("derived-shell");
            assert_ne!(base_asset, child_asset);
            assert_eq!(
                application.images[&base_asset].1.get_pixel(0, 0).0,
                [255, 0, 0, 255]
            );
            assert_eq!(
                application.images[&child_asset].1.get_pixel(0, 0).0,
                [0, 0, 255, 255]
            );
            let native_tree = format!("{:?}", application.accepted.node());
            assert!(native_tree.contains(&base_asset));
            assert!(native_tree.contains(&child_asset));
            let windows = serde_json::json!([{"id":"observed"}]);
            assert!(
                application
                    .sync_host_data_fields(&[("windows", &windows)])
                    .is_err()
            );
            assert!(
                application
                    .sync_composition_dependency_fields(&std::collections::BTreeMap::from([(
                        "base-shell".into(),
                        vec![("windows", windows.clone())]
                    )]))
                    .unwrap()
            );
            let shared = application.shared_composition_runtime().unwrap();
            let base_owner = shared
                .borrow()
                .resolution()
                .inheritance_chain
                .iter()
                .find(|owner| owner.id == "base-shell")
                .unwrap()
                .clone();
            assert_eq!(
                shared.borrow().snapshot(&base_owner).unwrap()["windows"],
                windows
            );
            assert!(
                application
                    .sync_composition_dependency_fields(&std::collections::BTreeMap::from([(
                        "base-shell".into(),
                        vec![("preferences", serde_json::json!({}))]
                    )]))
                    .is_err()
            );
            assert!(application.button_message("base").is_none());
            application.update(application.button_message("callback-control").unwrap());
            assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
            assert!(application.last_error().is_none());

            application.update(application.button_message("owned-child").unwrap());
            assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
            let event = application.button_message("replacement").unwrap();
            application.update(event);
            if granted {
                assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
                assert!(application.last_error().is_none());
                application.update(application.button_message("contribution").unwrap());
                assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);

                let replacement = application.button_message("replacement").unwrap();
                let contribution = application.button_message("contribution").unwrap();
                application.update_messages(vec![replacement, contribution]);
                assert_eq!(
                    application.take_effects(),
                    vec![PluginEffect::ShowLauncher, PluginEffect::ShowLauncher]
                );
                assert!(application.last_error().is_none());

                let host = application.shared_composition_runtime().unwrap();
                let owner = host.borrow().resolution().active.clone();
                let mut data = host.borrow().snapshot(&owner).unwrap().clone();
                data["settings"] = serde_json::json!({"test": "changed"});
                host.borrow_mut().update_snapshot(&owner, &data).unwrap();
                application.refresh_composition_snapshots().unwrap();
                assert!(std::rc::Rc::ptr_eq(
                    &host,
                    &application.shared_composition_runtime().unwrap()
                ));
                assert_eq!(
                    application.composition.as_ref().unwrap().snapshots[&owner]["settings"]["test"],
                    "changed"
                );
                application.update(application.button_message("replacement").unwrap());
                assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
                // Idle state from the replacement owner must use composed
                // patch admission, preserving the base owner's callbacks.
                let runtime = host.borrow().shared_owner_runtime(&owner).unwrap();
                let generation = application.accepted.generation();
                runtime.borrow_mut().eval("setIdleHidden(1)").unwrap();
                assert!(!application.reconcile_idle_work().unwrap());
                assert_eq!(application.accepted.generation(), generation);
                runtime.borrow_mut().eval("setIdleCount(99)").unwrap();
                application.dispatch_validated_events(Vec::new());
                assert!(
                    format!("{:?}", application.accepted.node()).contains("derived99"),
                    "idle composed update failed: {:?}; runtime failure: {:?}",
                    application.last_error(),
                    application.runtime_failure
                );
                assert!(application.last_error().is_none());
                assert!(application.take_effects().is_empty());
                let generation = application.accepted.generation();
                application.dispatch_validated_events(Vec::new());
                assert_eq!(application.accepted.generation(), generation);
                application.update(application.button_message("callback-control").unwrap());
                assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
            } else {
                assert!(application.take_effects().is_empty());
                assert!(application.last_error().is_some());
                // Denial restores supported hook/presentation state; the base
                // grant still cannot authorize a replacement's callback.
                assert!(application.take_runtime_failure().is_none());
                let node = format!("{:?}", application.accepted.node());
                assert!(node.contains("derived0"));
                assert!(!node.contains("derived1"));
                application.update(application.button_message("replacement").unwrap());
                assert!(application.take_effects().is_empty());
                assert!(format!("{:?}", application.accepted.node()).contains("derived0"));
            }
        }
    }

    #[test]
    fn composed_production_noop_retains_node_events_and_mount_generation() {
        let mut manifest = super::manifest().clone();
        manifest.id = "scheduled-shell".into();
        manifest.composition = Some(
            serde_json::from_value(serde_json::json!({
                "api_version": 1,
                "id": "scheduled-shell",
                "version": "0.1.0",
                "exports": {
                    "shell": "./main.js#Shell",
                    "shell.taskbar": "./main.js#Taskbar"
                }
            }))
            .unwrap(),
        );
        let package = PluginPackage {
            manifest: manifest.clone(),
            source: "globalThis.renders=0;\nexport function Shell(){renders++;const [value,setValue]=useState(0);return h(Window,{id:'main',placement:'fixed',width:440,height:220,edge:'bottom',bottomOffset:24,output:'all'},h(Button,{id:'noop',onClick:()=>setValue(current=>current)},String(value)));}\nexport function Taskbar(){return h(Text,null,'taskbar');}\nexport default Shell;".into(),
            stylesheet: String::new(),
            modules: Vec::new(),
            images: std::collections::BTreeMap::new(),
        };
        let surface = manifest.surfaces[0].clone();
        let catalog = std::collections::BTreeMap::from([("scheduled-shell".into(), package)]);
        let mut application = PluginPanelApplication::from_composed_surface(
            &catalog,
            "scheduled-shell",
            &std::collections::BTreeMap::new(),
            &surface,
            None,
        )
        .unwrap();
        let accepted_node = application.accepted.node().clone();
        let message = application.button_message("noop").unwrap();
        let PluginMessage::Click(action) = message else {
            panic!("noop must be a button click");
        };
        let accepted_event =
            application.composition.as_ref().unwrap().events[&(action as u64)].clone();

        application.update(PluginMessage::Click(action.saturating_add(10_000)));
        assert_eq!(application.accepted.node(), &accepted_node);
        assert!(application.last_error().is_none());
        assert!(application.take_runtime_failure().is_none());

        application.update(PluginMessage::Click(action));

        assert_eq!(application.accepted.node(), &accepted_node);
        assert!(application.last_error().is_none());
        let shared = application.shared_composition_runtime().unwrap();
        assert!(
            shared
                .borrow_mut()
                .dispatch_scheduled(&accepted_event, &Value::Null)
                .unwrap()
                .rendered
                .is_none(),
            "the accepted event generation must remain live after a no-op"
        );
        let owner = shared.borrow().resolution().active.clone();
        let runtime = shared.borrow().shared_owner_runtime(&owner).unwrap();
        assert_eq!(
            runtime
                .borrow_mut()
                .eval_json::<u64>("JSON.stringify(renders)")
                .unwrap(),
            1,
            "the production composed path must not call the root component"
        );
    }

    #[test]
    fn idle_reconciliation_updates_only_the_dirty_shared_runtime_surface() {
        let mut package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-two-windows"
        ))
        .unwrap();
        package.source = "export default function App(){const [value,setValue]=useState(0);globalThis.latestSetter=setValue;return [h(Window,{id:'home',width:400,height:240},h(Text,null,'Home:'+value)),h(Window,{id:'details',width:450,height:260},h(Text,null,'Details:'+value))]}".into();
        let runtime = PluginPanelApplication::shared_package_runtime(
            &package,
            &Default::default(),
            &package.manifest.surfaces[0],
        )
        .unwrap();
        let mut panels = Vec::new();
        for (index, surface) in package.manifest.surfaces.iter().enumerate() {
            panels.push(
                PluginPanelApplication::from_package_surface_with_runtime(
                    &package,
                    &Default::default(),
                    surface,
                    PluginImages::new(),
                    Some(runtime.clone()),
                )
                .unwrap(),
            );
            if index == 0 {
                runtime
                    .borrow_mut()
                    .eval("globalThis.firstSurfaceSetter=latestSetter")
                    .unwrap();
            }
        }
        assert_eq!(panels.len(), 2);
        let generations = panels
            .iter()
            .map(|panel| panel.accepted.generation())
            .collect::<Vec<_>>();
        let clean_node = format!("{:?}", panels[1].accepted.node());
        runtime
            .borrow_mut()
            .eval("globalThis.secondSurfaceSetter=latestSetter;firstSurfaceSetter(7)")
            .unwrap();
        assert!(!panels[1].reconcile_idle_work().unwrap());
        assert_eq!(panels[1].accepted.generation(), generations[1]);
        assert!(panels[0].reconcile_idle_work().unwrap());
        assert!(panels[0].accepted.generation() > generations[0]);
        assert!(format!("{:?}", panels[0].accepted.node()).contains(":7"));
        assert_eq!(format!("{:?}", panels[1].accepted.node()), clean_node);
        let first_node = format!("{:?}", panels[0].accepted.node());
        runtime
            .borrow_mut()
            .eval("secondSurfaceSetter(11)")
            .unwrap();
        assert!(!panels[0].reconcile_idle_work().unwrap());
        assert!(panels[1].reconcile_idle_work().unwrap());
        assert!(format!("{:?}", panels[1].accepted.node()).contains(":11"));
        assert_eq!(format!("{:?}", panels[0].accepted.node()), first_node);
        for panel in &mut panels {
            assert!(!panel.reconcile_idle_work().unwrap());
            assert!(panel.take_effects().is_empty());
        }
    }

    #[test]
    fn idle_reconciliation_of_non_host_child_preserves_visible_window() {
        let mut package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-two-windows"
        ))
        .unwrap();
        package.source = "function Hidden(){const [value,setValue]=useState(0);globalThis.hiddenSetter=setValue;return h(Text,null,'Hidden:'+value)}\nexport default function App(){return [h(Window,{id:'home',width:400,height:240},h(Text,null,'Home')),h(Window,{id:'details',width:450,height:260},h(Hidden))]}".into();
        let mut panel = PluginPanelApplication::from_package_surface_with_runtime(
            &package,
            &Default::default(),
            &package.manifest.surfaces[0],
            PluginImages::new(),
            None,
        )
        .unwrap();
        let generation = panel.accepted.generation();
        let node = format!("{:?}", panel.accepted.node());
        panel.runtime.borrow_mut().eval("hiddenSetter(7)").unwrap();
        assert!(!panel.reconcile_idle_work().unwrap());
        assert_eq!(panel.accepted.generation(), generation);
        assert_eq!(format!("{:?}", panel.accepted.node()), node);
        assert!(!panel.reconcile_idle_work().unwrap());
        let materialized = panel
            .runtime
            .borrow_mut()
            .eval_json::<u64>("__runtimeCounters.nativeNodesMaterialized")
            .unwrap();
        for value in 8..16 {
            panel
                .runtime
                .borrow_mut()
                .eval(&format!("hiddenSetter({value})"))
                .unwrap();
            assert!(!panel.reconcile_idle_work().unwrap());
            assert_eq!(panel.accepted.generation(), generation);
        }
        assert_eq!(
            panel
                .runtime
                .borrow_mut()
                .eval_json::<u64>("__runtimeCounters.nativeNodesMaterialized")
                .unwrap(),
            materialized
        );
    }

    #[test]
    fn one_package_entry_renders_sibling_windows_with_shared_modules() {
        let mut package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-two-windows"
        ))
        .unwrap();
        package.source = "export default function App() { return [h(Window, {id:'home',width:400,height:240}, h(Text, {}, 'Home')), h(Window, {id:'details',width:450,height:260}, h(Text, {}, 'Details'))]; }".into();
        PluginPanelApplication::validate_package(&package).unwrap();
        let runtime = PluginPanelApplication::shared_package_runtime(
            &package,
            &Default::default(),
            &package.manifest.surfaces[0],
        )
        .unwrap();
        for surface in &package.manifest.surfaces {
            let app = PluginPanelApplication::from_package_surface_with_runtime(
                &package,
                &Default::default(),
                surface,
                PluginImages::new(),
                Some(runtime.clone()),
            )
            .unwrap();
            assert_eq!(app.resolved_surface(surface).unwrap().id, surface.id);
            assert!(std::rc::Rc::ptr_eq(&app.shared_runtime(), &runtime));
        }
        package.source = package.source.replace("id:'details'", "id:'home'");
        assert!(
            PluginPanelApplication::validate_package(&package)
                .unwrap_err()
                .contains("duplicate")
        );
        package.source = package
            .source
            .replace("id:'home',width:450", "id:'ungranted',width:450");
        assert!(PluginPanelApplication::validate_package(&package).is_err());
    }

    #[test]
    fn package_host_loads_shared_modules_and_imported_css() {
        let mut package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap();
        package.source = "import { title } from './state.js';\nimport './module.css';\nexport default function App() { return h(Window, {id:'main',width:520,height:340}, h(Text, {}, title)); }".into();
        package.modules = vec![
            nickel_core::plugins::PluginSourceFile {
                path: "state.js".into(),
                source: "export const title = 'Module window';".into(),
            },
            nickel_core::plugins::PluginSourceFile {
                path: "module.css".into(),
                source: "text { color: #123456; }".into(),
            },
        ];
        PluginPanelApplication::validate_package(&package).unwrap();
        let app = PluginPanelApplication::from_package(&package).unwrap();
        let title: String = app
            .runtime
            .borrow_mut()
            .eval_json("JSON.stringify(__nickelRequireModule('state.js').title)")
            .unwrap();
        assert_eq!(title, "Module window");
        assert!(
            package_module_graph(&package)
                .unwrap()
                .unwrap()
                .stylesheet()
                .unwrap()
                .contains("#123456")
        );
    }

    #[test]
    fn display_policy_effects_require_grants_current_revisions_and_availability() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        let source = r#"function App(){return h(Window,{id:'main',width:520,height:340},h(Button,{id:'scale',onClick:()=>nickel.request({type:'displays.setApplicationScale',revision:'0123456789abcdef',policy:{policy:'custom',scale_120:180}})},'Scale'),h(Button,{id:'identify',onClick:()=>nickel.request({type:'displays.identify',revision:'0123456789abcdef'})},'Identify'));}"#;
        manifest.capabilities.clear();
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        for id in ["scale", "identify"] {
            denied.update(denied.button_message(id).unwrap());
            assert!(denied.take_effects().is_empty());
        }
        manifest.capabilities.push(PluginCapability::DisplayControl);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        let snapshot = serde_json::json!({"available":true,"revision":"0123456789abcdef","operations":{"identify":true},"application_scale":{"available":true,"revision":"0123456789abcdef"}});
        granted.sync_host_data_field("displays", &snapshot).unwrap();
        granted.update(granted.button_message("scale").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::SetApplicationScale { .. }]
        ));
        granted.update(granted.button_message("identify").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::IdentifyDisplays { .. }]
        ));
        let stale = serde_json::json!({"available":true,"revision":"fedcba9876543210","operations":{"identify":false},"application_scale":{"available":true,"revision":"fedcba9876543210"}});
        granted.sync_host_data_field("displays", &stale).unwrap();
        for id in ["scale", "identify"] {
            granted.update(granted.button_message(id).unwrap());
            assert!(granted.take_effects().is_empty());
        }
    }

    #[test]
    fn display_layout_effect_requires_capability_and_valid_shape() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        let source = r#"function App() { return h(Window, {id:'main',width:520,height:340}, h(Button, {id:'apply',onClick:()=>nickel.request({type:'displays.setLayout',revision:'0123456789abcdef',layout:{primary:'DP-1',placements:[{name:'DP-1',x:0,y:0,enabled:true,scale_120:120}]}})}, 'Apply')); }"#;
        manifest.capabilities.clear();
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        assert_eq!(
            denied.sync_host_data_field(
                "displays",
                &serde_json::json!({"available":true,"revision":"0123456789abcdef","outputs":[]})
            ),
            Err("display data requires display-control".into())
        );
        denied.update(denied.button_message("apply").unwrap());
        assert!(denied.take_effects().is_empty());
        assert_eq!(denied.last_error(), Some("display control is not granted"));

        manifest.capabilities.push(PluginCapability::DisplayControl);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        assert!(
            granted
                .sync_host_data_field(
                    "displays",
                    &serde_json::json!({"available":true,"revision":"0123456789abcdef","outputs":[]})
                )
                .unwrap()
        );
        granted.update(granted.button_message("apply").unwrap());
        assert!(
            matches!(granted.take_effects().as_slice(), [PluginEffect::SetDisplayLayout { plugin_id, layout, .. }] if plugin_id == &manifest.id && layout.placements.len() == 1)
        );

        granted
            .sync_host_data_field(
                "displays",
                &serde_json::json!({"available":true,"revision":"fedcba9876543210","outputs":[]}),
            )
            .unwrap();
        granted.update(granted.button_message("apply").unwrap());
        assert!(granted.take_effects().is_empty());
        assert_eq!(granted.last_error(), Some("display observation is stale"));

        let malformed = r#"function App() { return h(Window, {id:'main',width:520,height:340}, h(Button, {id:'apply',onClick:()=>nickel.request({type:'displays.setLayout',revision:'0123456789abcdef',layout:{primary:'DP-1',placements:'bad'}})}, 'Apply')); }"#;
        let mut rejected =
            PluginPanelApplication::new_with_manifest(malformed, &manifest, Some("{}".into()))
                .unwrap();
        rejected.update(rejected.button_message("apply").unwrap());
        assert!(rejected.take_effects().is_empty());
        assert_eq!(rejected.last_error(), Some("display layout is invalid"));
    }

    #[test]
    fn bundled_plugin_packages_validate_with_manifest_sample_data() {
        for name in ["hello-panel", "nickel-default"] {
            let directory = format!("{}/../../assets/plugins/{name}", env!("CARGO_MANIFEST_DIR"));
            let package = PluginPackage::load(directory).unwrap();
            PluginPanelApplication::validate_package(&package)
                .unwrap_or_else(|error| panic!("{name} validation failed: {error}"));
        }
    }

    #[test]
    fn native_surface_authority_is_ordered_deduplicated_and_mount_scoped() {
        let source = "globalThis.observedSurface=null; function App(){observedSurface=useSurface();return h(Panel,{id:'main'},h(Text,null,'surface'));}";
        let mut left = PluginPanelApplication::new_with_manifest_for_surface_and_scope(
            source,
            manifest(),
            Some(
                serde_json::json!({"surface":{"id":"main","kind":"panel","width":320,"height":48}})
                    .to_string(),
            ),
            Some("main"),
            None,
            "left",
        )
        .unwrap();
        let mut right = PluginPanelApplication::new_with_manifest_for_surface_and_scope(
            source,
            manifest(),
            Some(
                serde_json::json!({"surface":{"id":"main","kind":"panel","width":320,"height":48}})
                    .to_string(),
            ),
            Some("main"),
            Some(left.shared_runtime()),
            "right",
        )
        .unwrap();

        assert!(
            left.sync_surface_geometry(
                Some("DP-1"),
                Some((1920.0, 1032.0)),
                Some(1.25),
                Some(true)
            )
            .unwrap()
        );
        assert!(left.sync_surface_focus(true).unwrap());
        assert!(!left.sync_surface_focus(true).unwrap());
        left.runtime.borrow_mut().select_surface("left").unwrap();
        assert!(left.runtime.borrow_mut().eval_json::<bool>("__surfaceStore.snapshot.output === 'DP-1' && __surfaceStore.snapshot.availableSize.height === 1032 && __surfaceStore.snapshot.scaleFactor === 1.25 && __surfaceStore.snapshot.focused === true && __surfaceStore.generation === 3").unwrap());

        right.runtime.borrow_mut().select_surface("right").unwrap();
        assert!(right.runtime.borrow_mut().eval_json::<bool>("__surfaceStore.snapshot.output === null && __surfaceStore.snapshot.focused === null").unwrap());
        assert!(
            right
                .sync_surface_geometry(Some("DP-2"), Some((1280.0, 680.0)), Some(2.0), Some(true))
                .unwrap()
        );
        left.runtime.borrow_mut().select_surface("left").unwrap();
        assert!(left.runtime.borrow_mut().eval_json::<bool>("__surfaceStore.snapshot.output === 'DP-1' && __surfaceStore.snapshot.focused === true").unwrap());
    }

    #[test]
    fn external_surface_validation_uses_manifest_sample_data() {
        let mut package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap();
        package.source = package.source.replace(
            "h(Text, null, \"Window plugin\")",
            "h(Text, null, nickel.data.sampleTitle.toUpperCase())",
        );
        assert!(PluginPanelApplication::validate_package(&package).is_err());

        package.manifest.validation_data.insert(
            "main".into(),
            serde_json::json!({"sampleTitle": "Window plugin"}),
        );
        PluginPanelApplication::validate_package(&package).unwrap();
    }

    #[test]
    fn jsx_menu_items_preserve_submenus_disabled_reasons_and_shortcuts() {
        let source = r#"function App() { return h(Panel, {height: 80},
            h(Menu, {id: 'actions', anchor: 'root', open: true},
                h(MenuItem, {id: 'view', label: 'View', separatorBefore: true},
                    h(MenuItem, {id: 'show', shortcut: 'Ctrl+D', onClick: () => nickel.request('show-launcher')}, 'Show'),
                    h(MenuItem, {id: 'paste', disabledReason: 'Clipboard is empty'}, 'Paste')))); }"#;
        let application = PluginPanelApplication::new(source).unwrap();
        let Some(PanelNode::Menu { items, .. }) = application.accepted.node().menu("actions")
        else {
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

    fn fixed_window_test_manifest() -> PluginManifest {
        PluginManifest::from_json(r#"{"api_version":1,"id":"org.example.fixed-window","name":"Fixed Window","author":"Test","version":"0.1.0","entry":"main.js","surfaces":[{"id":"main","kind":"panel","width":1920,"height":56,"output":"all","reserve_work_area":true}],"capabilities":["launcher-show"]}"#).unwrap()
    }

    #[test]
    fn output_copies_share_one_runtime_with_independent_hook_state() {
        let source = "function App() { const [count, setCount] = useState(0); return h(FixedWindow, {width: '100%', height: 56, output: 'all', edge: 'bottom', reserveWorkArea: true}, h(Button, {id: 'advance', onClick: () => setCount(count + 1)}, `${nickel.data.label}:${count}`)); }";
        let mut left = PluginPanelApplication::new_with_manifest_for_surface_and_scope(
            source,
            &fixed_window_test_manifest(),
            Some(serde_json::json!({"label": "left"}).to_string()),
            None,
            None,
            "taskbar-output:left",
        )
        .unwrap();
        let mut right = PluginPanelApplication::new_with_manifest_for_surface_and_scope(
            source,
            &fixed_window_test_manifest(),
            Some(serde_json::json!({"label": "right"}).to_string()),
            None,
            Some(left.shared_runtime()),
            "taskbar-output:right",
        )
        .unwrap();
        assert!(std::rc::Rc::ptr_eq(
            &left.shared_runtime(),
            &right.shared_runtime()
        ));
        left.update(left.button_message("advance").unwrap());
        right
            .sync_data(&serde_json::json!({"label": "right refreshed"}))
            .unwrap();
        assert!(format!("{:?}", left.accepted.node()).contains("left:1"));
        assert!(format!("{:?}", right.accepted.node()).contains("right refreshed:0"));
        left.retire_surface().unwrap();
        right.update(right.button_message("advance").unwrap());
        assert!(format!("{:?}", right.accepted.node()).contains("right refreshed:1"));
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
    fn tray_artwork_paints_and_disappears_when_the_feed_is_cleared() {
        let mut app = PluginPanelApplication::new("function App() { return h(Panel, {}, h(Button, {id:'tray-test', icon:'tray:test', accessibilityLabel:'Tray test', onClick:()=>nickel.request('show-launcher')}, '?')); }").unwrap();
        let images = PluginImages::from([(
            "tray:test".into(),
            (
                0x7100,
                Arc::new(image::RgbaImage::from_pixel(
                    24,
                    24,
                    image::Rgba([55, 200, 255, 255]),
                )),
            ),
        )]);
        assert!(app.sync_application_images(images));
        let mut host = nickel_ui::UiHost::new(app, 300, 180);
        assert!(host.commands().iter().any(|command| matches!(command, nickel_ui::backend::PaintCommand::Image { id, .. } if *id == 0x7100)));
        assert!(
            host.application_mut()
                .sync_application_images(Default::default())
        );
        host.step(nickel_ui::HostBatch {
            application_changed: true,
            ..Default::default()
        });
        assert!(!host.commands().iter().any(|command| matches!(command, nickel_ui::backend::PaintCommand::Image { id, .. } if *id == 0x7100)));
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
    fn shared_window_preview_uses_public_snapshot_and_typed_revision_request() {
        with_package_runtime_stack(|| {
            let package = crate::bundled_plugin_assets::load_package("nickel-default").unwrap();
            let surface = package
                .manifest
                .surfaces
                .iter()
                .find(|s| s.id == "window-preview")
                .unwrap();
            let mut app = PluginPanelApplication::from_package_surface(
                &package,
                &Default::default(),
                surface,
            )
            .unwrap();
            app.sync_host_data_field(
                "windowPreviews",
                &serde_json::json!({"available":true,"revision":"r1","windows":[
                    {"id":"71","title":"Document","image":"window:71","canClose":true},
                    {"id":"72","title":"Mail","image":"window:72","canClose":true}
                ]}),
            )
            .unwrap();
            let mut host = nickel_ui::UiHost::new(app, 600, 214);
            let selector = |name: &str| nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            };
            let first = host.query_unique(&selector("Document")).unwrap();
            let second = host.query_unique(&selector("Mail")).unwrap();
            assert!(second.bounds.origin.x > first.bounds.origin.x);
            assert!(second.bounds.origin.x + second.bounds.size.width <= 600.0);
            host.perform_semantic_action(
                first.id,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert_eq!(
                host.application_mut().take_effects(),
                vec![PluginEffect::WindowPreviewRequest {
                    plugin_id: "nickel-default".into(),
                    revision: "r1".into(),
                    action: PreviewAction::Activate(crate::model::WindowId(71))
                }]
            );
        });
    }

    #[test]
    fn independent_jsx_sliders_dispatch_their_own_values() {
        let source = "function App() { const [hue, setHue] = useState(0.2); const [intensity, setIntensity] = useState(0.6); return h(Panel, {}, h(Slider, {value: hue, accessibilityLabel: 'Hue', className: 'hue', onChange: setHue}), h(Slider, {value: intensity, accessibilityLabel: 'Intensity', onChange: setIntensity})); }";
        let mut application = PluginPanelApplication::new(source).unwrap();
        application.stylesheet =
            StyleSheet::compile("slider { width: 180px; height: 24px; }").unwrap();
        let mut host = nickel_ui::UiHost::new(application, 440, 220);
        let hue = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Slider,
                name: "Hue".into(),
            })
            .unwrap();
        let intensity = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Slider,
                name: "Intensity".into(),
            })
            .unwrap();
        assert_ne!(hue.id, intensity.id);
        host.perform_semantic_action(
            hue.id,
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Number(0.75)),
        );
        let PanelNode::Surface { children, .. } = &host.application_mut().accepted.node() else {
            panic!("plugin root is not a Window");
        };
        let PanelNode::Row { children, .. } = &children[0] else {
            panic!("legacy Panel children are not composed into a Row");
        };
        assert!(
            matches!(&children[0], PanelNode::Slider { value, .. } if (*value - 0.75).abs() < 0.001)
        );
        assert!(
            matches!(&children[1], PanelNode::Slider { value, .. } if (*value - 0.6).abs() < 0.001)
        );
        host.perform_semantic_action(
            intensity.id,
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Number(0.35)),
        );
        let PanelNode::Surface { children, .. } = &host.application_mut().accepted.node() else {
            panic!("plugin root is not a Window");
        };
        let PanelNode::Row { children, .. } = &children[0] else {
            panic!("legacy Panel children are not composed into a Row");
        };
        assert!(
            matches!(&children[0], PanelNode::Slider { value, .. } if (*value - 0.75).abs() < 0.001)
        );
        assert!(
            matches!(&children[1], PanelNode::Slider { value, .. } if (*value - 0.35).abs() < 0.001)
        );
        let hue = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Slider,
                name: "Hue".into(),
            })
            .unwrap();
        let pointer = Point {
            x: hue.bounds.origin.x + hue.bounds.size.width * 0.2,
            y: hue.bounds.origin.y + hue.bounds.size.height / 2.0,
        };
        host.handle_event(nickel_ui::UiEvent::PointerPressed(pointer));
        host.handle_event(nickel_ui::UiEvent::PointerReleased(pointer));
        let PanelNode::Surface { children, .. } = &host.application_mut().accepted.node() else {
            panic!("plugin root is not a Window");
        };
        let PanelNode::Row { children, .. } = &children[0] else {
            panic!("legacy Panel children are not composed into a Row");
        };
        assert!(
            matches!(&children[0], PanelNode::Slider { value, .. } if (*value - 0.2).abs() < 0.02),
            "{:?} bounds={:?} pointer={pointer:?}",
            children[0],
            hue.bounds
        );
    }

    #[test]
    fn jsx_slider_requires_a_bounded_value_label_and_handler() {
        for props in [
            "value: -0.1, accessibilityLabel: 'Hue', onChange: value => {}",
            "value: 1.1, accessibilityLabel: 'Hue', onChange: value => {}",
            "value: 0.5, onChange: value => {}",
            "value: 0.5, accessibilityLabel: 'Hue'",
        ] {
            let source =
                format!("function App() {{ return h(Panel, {{}}, h(Slider, {{{props}}})); }}");
            assert!(PluginPanelApplication::new(&source).is_err(), "{props}");
        }
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
        assert!(format!("{:?}", panel.accepted.node()).contains("Count: 0"));
        panel.update(PluginMessage::Click(0));
        assert!(format!("{:?}", panel.accepted.node()).contains("Count: 1"));
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
        assert!(format!("{:?}", panel.accepted.node()).contains("Count: 0"));

        let valid = panel.button_message("valid").unwrap();
        panel.update(valid);
        assert!(panel.last_error().is_none());
        assert!(format!("{:?}", panel.accepted.node()).contains("Count: 2"));
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
        assert!(format!("{:?}", panel.accepted.node()).contains("Count: 0"));

        let valid = panel.button_message("valid").unwrap();
        panel.update(valid);
        assert!(panel.last_error().is_none());
        assert!(format!("{:?}", panel.accepted.node()).contains("Count: 2"));
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
        assert!(format!("{:?}", panel.accepted.node()).contains("Count: 0"));

        let valid = panel.button_message("valid").unwrap();
        panel.update(valid);
        assert!(panel.last_error().is_none());
        assert!(format!("{:?}", panel.accepted.node()).contains("Count: 2"));
    }

    #[test]
    fn bundled_panel_dialog_opens_and_cancels() {
        let mut panel = PluginPanelApplication::bundled().expect("bundled plugin loads");
        panel.update(PluginMessage::Click(1));
        assert!(panel.pending_transient.is_some());
        assert!(matches!(
            panel.accepted.node().dialog("launcher-dialog"),
            Some(PanelNode::Dialog { open: true, .. })
        ));
        panel.pending_transient.take();
        let Some(PanelNode::Dialog {
            close_action: Some(cancel),
            ..
        }) = panel.accepted.node().dialog("launcher-dialog")
        else {
            panic!("open dialog has no close action");
        };
        panel.update(PluginMessage::Click(*cancel));
        assert!(matches!(
            panel.accepted.node().dialog("launcher-dialog"),
            Some(PanelNode::Dialog { open: false, .. })
        ));
        assert!(panel.take_effects().is_empty());
        assert!(panel.last_error().is_none());
    }

    #[test]
    fn packaged_panel_aligns_at_bottom_and_dispatches_dialog_action() {
        with_package_runtime_stack(|| {
            let mut external_manifest = manifest().clone();
            external_manifest.id = "org.example.tall-panel".into();
            external_manifest.surfaces[0].height = 400;
            let package = PluginPackage {
            modules: Vec::new(),
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
            let app = PluginPanelApplication::from_package_surface(
                &package,
                &Default::default(),
                &package.manifest.surfaces[0],
            )
            .unwrap();
            let mut host = nickel_ui::UiHost::new(app, 440, 400);
            let open = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Button,
                    name: "Open dialog".into(),
                })
                .unwrap();
            assert!(matches!(
                &host.application().accepted.node(),
                PanelNode::Surface {
                    width: Length::Percent(1.0),
                    height: Length::Percent(1.0),
                    ..
                }
            ));
            assert!(open.bounds.origin.y > 250.0);
            assert!(open.bounds.origin.y + open.bounds.size.height <= 400.0);
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
        });
    }

    #[test]
    fn plugin_css_styles_controls_without_changing_actions_or_text_editing() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.css-controls".into();
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: "button.primary { background-color: #345678; padding: 8px; border: 2px solid #abc; border-radius: 6px; color: #fff; font-size: 18px; } button.primary:focus { background-color: #123abc; } text-field.entry { background-color: rgba(10, 20, 30, 0.5); padding: 4px; line-height: 24px; } text-field.entry:focus { background-color: #3479ab; }".into(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%'}, h(Button, {id: 'go', className: 'primary', onClick: () => nickel.request('show-launcher')}, 'Go'), h(TextField, {id: 'name', className: 'entry', value: '', onChange: value => {}})); }".into(),
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
                nickel_ui::UiEvent::AccessibilityActivate(button.id.clone()),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowLauncher]
        );
        let field = host
            .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
            .unwrap();
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityFocus(button.id),
            )],
            ..Default::default()
        });
        assert!(host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill {
                color: 0xff123abc,
                ..
            } | nickel_ui::backend::PaintCommand::Fill {
                color: 0xff123abc,
                ..
            }
        )));
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityFocus(field.id),
            )],
            ..Default::default()
        });
        assert!(host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill {
                color: 0xff3479ab,
                ..
            } | nickel_ui::backend::PaintCommand::Fill {
                color: 0xff3479ab,
                ..
            }
        )));
    }

    #[test]
    fn plugin_css_inherits_text_properties_through_layout_nodes() {
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: manifest().clone(),
            images: Default::default(),
            stylesheet: "window.parent { color: #123456; font-size: 20px; line-height: 28px; } div.nested { color: #abcdef; } button.override { color: #fedcba; } text.own { color: #aabbcc; }".into(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%', className: 'parent'}, h(Text, {}, 'Root text'), h('div', {className: 'nested'}, h(Text, {}, 'Nested text'), h(Text, {className: 'own', color: 0xff112233}, 'Own text'), h(Button, {id: 'nested-button', onClick: () => {}}, 'Nested button'), h(TextField, {id: 'nested-field', value: '', placeholder: 'Enter', onChange: () => {}})), h(Button, {id: 'override', className: 'override', onClick: () => {}}, 'Override')); }".into(),
        };
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package(&package).unwrap(),
            500,
            300,
        );
        let painted = |label: &str| {
            host.commands()
                .iter()
                .find_map(|command| match command {
                    nickel_ui::backend::PaintCommand::Text {
                        bounds,
                        text,
                        scale,
                        color,
                        ..
                    } if text == label => Some((*scale, *color, bounds.size.height)),
                    _ => None,
                })
                .unwrap_or_else(|| panic!("missing painted text {label:?}"))
        };
        for (label, color) in [
            ("Root text", 0xff123456),
            ("Nested text", 0xffabcdef),
            ("Own text", 0xffaabbcc),
            ("Nested button", 0xffabcdef),
            ("Enter", 0xffabcdef),
            ("Override", 0xfffedcba),
        ] {
            let (scale, painted_color, height) = painted(label);
            assert_eq!((scale, painted_color), (-20.0, color), "{label}");
            assert!(height >= 28.0, "{label} lost its inherited line height");
        }
    }

    #[test]
    fn badge_css_styles_the_taskbar_extension_visual() {
        let mut app = PluginPanelApplication::new(
            "function App() { return h(Panel, null, h(Badge, {count: 5, label: 'Mail', color: 0xffc9354c, className: 'task-badge'})); }",
        )
        .unwrap();
        app.stylesheet = StyleSheet::compile(
            "badge.task-badge { background: #123456; border-radius: 7px; color: #abcdef; font-size: 12px; }",
        )
        .unwrap();
        let host = nickel_ui::UiHost::new(app, 120, 56);
        assert!(host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill { color: 0xff123456, radius, .. } if *radius == 7.0
        )));
        assert!(host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Text {
                color: 0xffabcdef,
                ..
            }
        )));
        assert!(
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Status,
                name: "Mail: 5".into(),
            })
            .is_ok()
        );
    }

    #[test]
    fn generic_div_uses_css_grid_tracks_for_plugin_controls() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.css-grid".into();
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: ".grid { display: grid; grid-template-columns: 100px 100px; gap: 10px; width: 210px; } button { width: 100px; }".into(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%'}, h(Div, {className: 'grid'}, h(Button, {id: 'left', onClick: () => nickel.request('show-launcher')}, 'Left'), h(Button, {id: 'right', onClick: () => nickel.request('show-launcher')}, 'Right'))); }".into(),
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
    fn generic_div_grid_applies_css_justification_and_alignment() {
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: manifest().clone(),
            images: Default::default(),
            stylesheet: ".grid { display: grid; grid-template-columns: 40px 60px; width: 200px; height: 80px; justify-content: center; align-items: center; } button#short { height: 20px; } button#tall { height: 40px; }".into(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%'}, h('div', {className: 'grid'}, h(Button, {id: 'short', onClick: () => {}}, 'Short'), h(Button, {id: 'tall', onClick: () => {}}, 'Tall'))); }".into(),
        };
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package(&package).unwrap(),
            400,
            120,
        );
        let button = |name: &str| {
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            })
            .unwrap()
            .bounds
        };
        let short = button("Short");
        let tall = button("Tall");
        assert!((short.origin.x - 50.0).abs() < 1.0);
        assert!((tall.origin.x - short.origin.x - 40.0).abs() < 1.0);
        assert!((short.origin.y - tall.origin.y - 10.0).abs() < 1.0);
    }

    #[test]
    fn generic_div_flex_uses_explicit_css_dimensions_for_alignment() {
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: manifest().clone(),
            images: Default::default(),
            stylesheet: "div.toolbar { display: flex; width: 100%; height: 80px; align-items: center; justify-content: space-between; } button { width: 50px; height: 20px; }".into(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%'}, h('div', {className: 'toolbar'}, h(Button, {id: 'left', onClick: () => {}}, 'Left'), h(Button, {id: 'right', onClick: () => {}}, 'Right'))); }".into(),
        };
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::from_package(&package).unwrap(),
            400,
            220,
        );
        let button = |name: &str| {
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            })
            .unwrap()
            .bounds
        };
        let left = button("Left");
        let right = button("Right");
        assert!(left.origin.y > 20.0);
        assert!(right.origin.x > left.origin.x + 250.0);
    }

    #[test]
    fn spacer_css_width_stays_fixed_and_unstyled_spacer_grows() {
        let host = |spacer_class: &str, stylesheet: &str| {
            let package = PluginPackage {
                modules: Vec::new(),
                manifest: manifest().clone(),
                images: Default::default(),
                stylesheet: stylesheet.into(),
                source: format!(
                    "function App() {{ return h(FixedWindow, {{width: '100%', height: '100%'}}, h(Row, {{className: 'bar'}}, h(Button, {{id: 'left', onClick: () => {{}}}}, 'Left'), h(Spacer, {{className: '{spacer_class}'}}), h(Button, {{id: 'right', onClick: () => {{}}}}, 'Right'))); }}"
                ),
            };
            nickel_ui::UiHost::new(
                PluginPanelApplication::from_package(&package).unwrap(),
                300,
                80,
            )
        };
        let bounds = |host: &nickel_ui::UiHost<PluginPanelApplication>, name: &str| {
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: name.into(),
            })
            .unwrap()
            .bounds
        };
        let base_css = "row.bar { width: 100%; gap: 0px; } button { width: 40px; height: 20px; }";
        let fixed = host(
            "fixed",
            &format!("{base_css} spacer.fixed {{ width: 24px; }}"),
        );
        let left = bounds(&fixed, "Left");
        let right = bounds(&fixed, "Right");
        assert!((right.origin.x - left.origin.x - left.size.width - 24.0).abs() < 0.5);

        let spaced = host(
            "fixed",
            &format!("{base_css} spacer.fixed {{ width: 24px; margin: 0px 8px; }}"),
        );
        let left = bounds(&spaced, "Left");
        let right = bounds(&spaced, "Right");
        assert!((right.origin.x - left.origin.x - left.size.width - 40.0).abs() < 0.5);

        let flexible = host("flexible", base_css);
        let right = bounds(&flexible, "Right");
        assert!(right.origin.x > 250.0);
    }

    #[test]
    fn row_and_column_classes_apply_css_flex_alignment() {
        with_package_runtime_stack(|| {
            let package = PluginPackage {
            modules: Vec::new(),
            manifest: manifest().clone(),
            images: Default::default(),
            stylesheet: "row.toolbar { width: 100%; height: 80px; align-items: center; justify-content: space-between; } column.stack { width: 100%; height: 100px; align-items: flex-end; justify-content: space-between; } button { width: 50px; height: 20px; }".into(),
            source: r#"
                function App() {
                    return h(FixedWindow, {width: '100%', height: '100%'},
                        h(Column, {},
                            h(Row, {className: 'toolbar'},
                                h(Button, {id: 'left', onClick: () => {}}, 'Left'),
                                h(Button, {id: 'right', onClick: () => {}}, 'Right')),
                            h(Column, {className: 'stack'},
                                h(Button, {id: 'top', onClick: () => {}}, 'Top'),
                                h(Button, {id: 'bottom', onClick: () => {}}, 'Bottom'))));
                }
            "#.into(),
        };
            let host = nickel_ui::UiHost::new(
                PluginPanelApplication::from_package(&package).unwrap(),
                400,
                220,
            );
            let button = |name: &str| {
                host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: name.into(),
                })
                .unwrap()
                .bounds
            };
            let left = button("Left");
            let right = button("Right");
            let top = button("Top");
            let bottom = button("Bottom");
            assert!(right.origin.x > left.origin.x + 250.0);
            assert!(
                left.origin.y > 20.0,
                "left={left:?} right={right:?} top={top:?} bottom={bottom:?}"
            );
            assert!(top.origin.x > 300.0);
            assert!(bottom.origin.y > top.origin.y + 50.0);
        });
    }

    #[test]
    fn fixed_window_jsx_helper_uses_one_manifest_checked_window_root() {
        let mut granted = fixed_window_test_manifest();
        granted.id = "org.example.fixed-window".into();
        let source = "function App() { return h(FixedWindow, {id: 'main', width: '100%', height: 56, output: 'all', edge: 'bottom', reserveWorkArea: true, className: 'bar'}, h(Button, {id: 'open', onClick: () => nickel.request('show-launcher')}, 'Open')); }";
        let mut package = PluginPackage {
            modules: Vec::new(),
            manifest: granted,
            images: Default::default(),
            stylesheet: "window.bar { width: 100%; background: rgba(20, 30, 40, 0.8); }".into(),
            source: source.into(),
        };
        PluginPanelApplication::validate_package(&package).unwrap();
        let app = PluginPanelApplication::from_package(&package).unwrap();
        assert!(matches!(
            app.accepted.node(),
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

        package.source = source.replace("width: '100%', height: 56, ", "");
        package.stylesheet =
            "window.bar { width: 100%; height: 56px; background: rgba(20, 30, 40, 0.8); }".into();
        let css_sized = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(
            css_sized
                .resolved_surface(&package.manifest.surfaces[0])
                .unwrap()
                .height,
            56
        );
        let css_host = nickel_ui::UiHost::new(css_sized, 1366, 100);
        let snapshot = css_host.layout_snapshot();
        assert!(
            snapshot.contains("allocated=0.00,0.00,1366.00,56.00"),
            "{snapshot}"
        );
        package.stylesheet = "window.bar { width: 100%; height: 64px; }".into();
        assert!(PluginPanelApplication::from_package(&package).is_err());
        package.stylesheet =
            "window.bar { width: 100%; background: rgba(20, 30, 40, 0.8); }".into();

        package.source = source.replace("reserveWorkArea: true", "reserveWorkArea: false");
        let app = PluginPanelApplication::from_package(&package).unwrap();
        assert!(
            !app.resolved_surface(&package.manifest.surfaces[0])
                .unwrap()
                .reserve_work_area
        );
        package.source = source.replace("output: 'all'", "output: 'primary'");
        let app = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(
            app.resolved_surface(&package.manifest.surfaces[0])
                .unwrap()
                .output,
            nickel_core::plugins::PluginOutputScope::Primary
        );

        for invalid in [
            source.replace("id: 'main'", "id: 'other'"),
            source.replace("height: 56", "height: 64"),
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
        let ordinary_source = package.source.clone();
        package.source = ordinary_source.replace("height: 340", "height: 340, output: 'active'");
        let active = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(
            active
                .resolved_surface(&package.manifest.surfaces[0])
                .unwrap()
                .output,
            nickel_core::plugins::PluginOutputScope::Active
        );
        package.source = ordinary_source.clone();
        package.source = "function App() { return h(Window, {className: 'settings'}, h(Button, {id: 'save', onClick: () => {}}, 'Save')); }".into();
        package.stylesheet = "window.settings { width: 520px; height: 340px; }".into();
        let css_sized = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(
            css_sized
                .resolved_surface(&package.manifest.surfaces[0])
                .unwrap()
                .width,
            520
        );
        let css_host = nickel_ui::UiHost::new(css_sized, 600, 400);
        let snapshot = css_host.layout_snapshot();
        assert!(snapshot.contains("preferred=520.00,340.00"), "{snapshot}");
        package.stylesheet = "window.settings { width: 520px; height: 360px; }".into();
        assert!(PluginPanelApplication::from_package(&package).is_err());
        package.stylesheet =
            "window.bar { width: 100%; background: rgba(20, 30, 40, 0.8); }".into();
        for invalid in [
            ordinary_source.replace("height: 340", "height: 340, output: 'all'"),
            ordinary_source.replace("height: 340", "height: 340, reserveWorkArea: true"),
        ] {
            package.source = invalid;
            assert!(PluginPanelApplication::from_package(&package).is_err());
        }
        package.source = ordinary_source;
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
    fn max_content_window_resolves_to_intrinsic_child_width() {
        let manifest = fixed_window_test_manifest();
        let source = "function App() { return h(FixedWindow, {id: 'main', width: 'max-content', height: 56, output: 'all', edge: 'bottom'}, h(Row, {}, h(Button, {id: 'one', width: 58, height: 40, onClick: () => {}}, 'One'), h(Button, {id: 'two', width: 70, height: 40, onClick: () => {}}, 'Two'))); }";
        let mut package = PluginPackage {
            modules: Vec::new(),
            manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: source.into(),
        };
        package.manifest.surfaces[0].reserve_work_area = false;
        let application = PluginPanelApplication::from_package(&package).unwrap();
        let resolved = application
            .resolved_surface(&package.manifest.surfaces[0])
            .unwrap();
        assert_eq!(resolved.width, 128);
    }

    #[test]
    fn fixed_window_resolves_dock_distance_within_manifest_bound() {
        let mut package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/hello-panel"
        ))
        .unwrap();
        package.source = package
            .source
            .replace("bottomOffset: 24", "bottomOffset: 12");
        let application = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(
            application
                .resolved_surface(&package.manifest.surfaces[0])
                .unwrap()
                .bottom_offset,
            12
        );

        package.source = package
            .source
            .replace("bottomOffset: 12", "bottomOffset: 25");
        assert!(PluginPanelApplication::from_package(&package).is_err());

        package.source = package.source.replace("bottomOffset: 25, ", "");
        package
            .stylesheet
            .push_str("\nwindow.hello-panel { bottom: 12px; }");
        let application = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(
            application
                .resolved_surface(&package.manifest.surfaces[0])
                .unwrap()
                .bottom_offset,
            12
        );
        package.stylesheet = package.stylesheet.replace("bottom: 12px", "bottom: 25px");
        assert!(PluginPanelApplication::from_package(&package).is_err());
        package.source = package
            .source
            .replace("edge: \"bottom\", ", "edge: \"bottom\", bottomOffset: 24, ");
        let application = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(
            application
                .resolved_surface(&package.manifest.surfaces[0])
                .unwrap()
                .bottom_offset,
            24
        );
    }

    #[test]
    fn top_anchored_overlay_resolves_css_top_within_manifest_bound() {
        let mut package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-overlay"
        ))
        .unwrap();
        let notice = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "notice")
            .unwrap()
            .clone();
        package
            .stylesheet
            .push_str("\nwindow#notice { top: 12px; }");
        PluginPanelApplication::validate_package(&package).unwrap();
        let application =
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &notice)
                .unwrap();
        assert_eq!(application.resolved_surface(&notice).unwrap().offset_y, 12);

        package.stylesheet = package.stylesheet.replace("top: 12px", "top: 25px");
        assert!(PluginPanelApplication::validate_package(&package).is_err());
        package.stylesheet = package
            .stylesheet
            .replace("top: 25px", "top: 12px; bottom: 0px");
        assert!(PluginPanelApplication::validate_package(&package).is_err());
    }

    #[test]
    fn top_center_jsx_root_uses_the_matching_manifest_grant() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-overlay"
        ))
        .unwrap()
        .manifest;
        let notice = manifest
            .surfaces
            .iter_mut()
            .find(|surface| surface.id == "notice")
            .unwrap();
        notice.anchor = nickel_core::plugins::PluginSurfaceAnchor::TopCenter;
        let source = "function App() { return h(Window, {id:'notice',placement:'fixed',width:300,height:120,edge:'top',anchor:'top-center'}, h(Text, null, 'Notice')); }";
        let app = PluginPanelApplication::new_with_manifest_for_surface(
            source,
            &manifest,
            Some(r#"{"surface":{"id":"notice"}}"#.into()),
            Some("notice"),
            None,
        )
        .unwrap();
        let grant = manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "notice")
            .unwrap();
        assert_eq!(app.resolved_surface(grant).unwrap().anchor, grant.anchor);
    }

    #[test]
    fn native_text_revisions_follow_accepted_content_and_presentation_lifetimes() {
        with_package_runtime_stack(native_text_revision_lifecycle_inner);
    }

    fn native_text_revision_lifecycle_inner() {
        let source = "function App() { return h(Panel, {}, h(Text, {}, nickel.data.label)); }";
        let make = |label: &str| {
            PluginPanelApplication::new_with_manifest(
                source,
                manifest(),
                Some(serde_json::json!({"label":label}).to_string()),
            )
            .unwrap()
        };
        let mut host = nickel_ui::UiHost::new(make("Before"), 320, 100);
        let initial = host.application().native_text_revision.get().unwrap();
        assert!(initial.2.is_some());
        for _ in 0..4 {
            let outcome = host.step(nickel_ui::HostBatch {
                application_changed: true,
                ..Default::default()
            });
            assert_eq!(host.application().native_text_revision.get(), Some(initial));
            assert_eq!(outcome.telemetry.nodes_measured, 0);
        }
        let other = nickel_ui::UiHost::new(make("Before"), 320, 100);
        assert_ne!(
            other.application().native_text_revision.get().unwrap().2,
            initial.2
        );
        host.application_mut()
            .sync_data(&serde_json::json!({"label":"A longer changed label"}))
            .unwrap();
        host.step(nickel_ui::HostBatch {
            application_changed: true,
            ..Default::default()
        });
        let changed = host.application().native_text_revision.get().unwrap();
        assert_ne!(changed.2, initial.2);
        let cold = nickel_ui::UiHost::new(make("A longer changed label"), 320, 100);
        assert_eq!(host.commands(), cold.commands());
        assert_eq!(host.layout_snapshot(), cold.layout_snapshot());
        host.application_mut()
            .sync_reading_direction(nickel_ui::ReadingDirection::RightToLeft);
        host.step(nickel_ui::HostBatch {
            application_changed: true,
            ..Default::default()
        });
        assert_ne!(
            host.application().native_text_revision.get().unwrap().2,
            changed.2
        );

        let exhausted = AtomicU64::new(u64::MAX - 1);
        assert_eq!(
            allocate_native_text_revision(&exhausted),
            Some(u64::MAX - 1)
        );
        assert_eq!(allocate_native_text_revision(&exhausted), None);
        assert_eq!(allocate_native_text_revision(&exhausted), None);
        assert_eq!(exhausted.load(Ordering::Relaxed), u64::MAX);
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
    fn unstyled_window_root_does_not_paint_opaque_black() {
        let source = "function App() { return h(Window, {placement: 'fixed', width: 440, height: 220}, h(Text, {}, 'Visible')); }";
        let host = nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 440, 220);
        assert!(!host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Fill {
                color: 0xff000000,
                ..
            } | nickel_ui::backend::PaintCommand::RoundedFill {
                color: 0xff000000,
                ..
            }
        )));
    }

    #[test]
    fn unstyled_box_does_not_paint_opaque_black() {
        let source =
            "function App() { return h(Panel, {}, h(Box, {x: 0, y: 0, width: 80, height: 40})); }";
        let host = nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 440, 220);
        assert!(!host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Fill {
                color: 0xff000000,
                ..
            } | nickel_ui::backend::PaintCommand::RoundedFill {
                color: 0xff000000,
                ..
            }
        )));
    }

    #[test]
    fn progress_paint_comes_from_css() {
        let source = "function App() { return h(Panel, {}, h(Progress, {className: 'meter', percent: 50, width: 100, height: 8})); }";
        let mut application = PluginPanelApplication::new(source).unwrap();
        let unstyled = nickel_ui::UiHost::new(application, 440, 220);
        assert!(!unstyled.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::Fill {
                color: 0xffaaaaaa | 0xff555555,
                ..
            } | nickel_ui::backend::PaintCommand::RoundedFill {
                color: 0xffaaaaaa | 0xff555555,
                ..
            }
        )));

        application = PluginPanelApplication::new(source).unwrap();
        application.stylesheet = StyleSheet::compile(
            "progress.meter { background: #112233; border-radius: 4px; } progress-fill.meter { background: #aabbcc; }",
        )
        .unwrap();
        let styled = nickel_ui::UiHost::new(application, 440, 220);
        for color in [0xff112233, 0xffaabbcc] {
            assert!(styled.commands().iter().any(|command| matches!(
                command,
                nickel_ui::backend::PaintCommand::Fill { color: painted, .. }
                    | nickel_ui::backend::PaintCommand::RoundedFill { color: painted, .. }
                    if *painted == color
            )));
        }
    }

    #[test]
    fn shared_window_root_tracks_resize_and_keeps_css_content_inset() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.window-resize".into();
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: "window { background: #112233; } div.content { width: 100%; height: 100%; padding: 20px; }".into(),
            source: "function App() { return h(Window, {id: 'main', placement: 'fixed', width: '100%', height: '100%'}, h('div', {className: 'content'}, h(Button, {id: 'open', onClick: () => nickel.request('show-launcher')}, 'Open'))); }".into(),
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
    fn managed_window_declared_size_is_initial_geometry_not_a_fixed_viewport() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.managed-window-resize".into();
        external_manifest.surfaces[0].kind = PluginSurfaceKind::Window;
        external_manifest.surfaces[0].width = 900;
        external_manifest.surfaces[0].height = 600;
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: "function App() { return h(Window, {id: 'main', accessibilityLabel: 'Managed window', width: 900, height: 600}, h(Button, {id: 'open', onClick: () => {}}, 'Open')); }".into(),
        };
        let app = PluginPanelApplication::from_package(&package).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 900, 600);
        for (width, height) in [(900, 600), (1100, 720), (640, 480)] {
            host.step(nickel_ui::HostBatch {
                surface_size: Some((width, height)),
                ..Default::default()
            });
            let root = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::ApplicationPresentation,
                    name: "Managed window".into(),
                })
                .unwrap();
            assert_eq!(root.bounds.size.width, width as f32);
            assert_eq!(root.bounds.size.height, height as f32);
        }
    }

    #[test]
    fn state_update_changes_resolved_window_root_size() {
        let manifest = PluginManifest::from_json(
            r#"{"api_version":1,"id":"org.example.window-resize","name":"Resize","entry":"main.js","surfaces":[{"id":"main","kind":"window","width":400,"height":240}]}"#,
        )
        .unwrap();
        let surface = manifest.surfaces[0].clone();
        let package = PluginPackage {
            modules: Vec::new(),
            manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: "function App(){const [compact,setCompact]=useState(false);return h(Window,{width:compact?300:360,height:220},h(Button,{id:'resize',onClick:()=>setCompact(true)},'Resize'));}".into(),
        };
        let mut application =
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &surface)
                .unwrap();
        assert_eq!(application.resolved_surface(&surface).unwrap().width, 360);
        application.update(application.button_message("resize").unwrap());
        assert_eq!(application.last_error(), None);
        assert_eq!(application.resolved_surface(&surface).unwrap().width, 300);
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
                    .unwrap_or_else(|error| panic!("missing {name}: {error:?}"));
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
                    vec![PluginEffect::ShowSettings(None)]
                );
            } else {
                assert!(host.application_mut().take_effects().is_empty());
                assert!(host.application_mut().last_error().is_some());
            }
        }
    }

    #[test]
    fn external_plugin_can_open_a_granted_settings_screen() {
        let manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        for (screen, granted) in [("plugins", true), ("unknown", true), ("plugins", false)] {
            let mut manifest = manifest.clone();
            if !granted {
                manifest.capabilities.clear();
            }
            let source = format!(
                "function App() {{ return h(Window, {{id:'main',width:520,height:340}}, h(Button, {{id:'settings',onClick:()=>nickel.request({{type:'show-settings',screen:'{screen}'}})}}, 'Settings')); }}"
            );
            let mut app =
                PluginPanelApplication::new_with_manifest(&source, &manifest, None).unwrap();
            app.update(app.button_message("settings").unwrap());
            if screen == "plugins" && granted {
                assert_eq!(
                    app.take_effects(),
                    vec![PluginEffect::ShowSettings(Some("plugins".into()))]
                );
            } else {
                assert!(app.take_effects().is_empty());
                assert!(app.last_error().is_some());
            }
        }
    }

    #[test]
    fn plugin_window_placement_request_is_typed_and_bounded() {
        let manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        for (surface_id, anchor, offset_x, expected) in [
            (
                "main",
                "top-right",
                -24,
                Some(nickel_core::plugins::PluginSurfaceAnchor::TopRight),
            ),
            (
                "main",
                "bottom-center",
                0,
                Some(nickel_core::plugins::PluginSurfaceAnchor::BottomCenter),
            ),
            ("other", "top-right", -24, None),
            ("main", "top-right", 8193, None),
        ] {
            let source = format!(
                "function App() {{ return h(Window, {{id:'main',width:520,height:340}}, h(Button, {{id:'move',onClick:()=>nickel.request({{type:'surface.setPlacement',id:'{surface_id}',anchor:'{anchor}',offsetX:{offset_x},offsetY:24}})}}, 'Move')); }}"
            );
            let mut app =
                PluginPanelApplication::new_with_manifest(&source, &manifest, None).unwrap();
            app.update(app.button_message("move").unwrap());
            if let Some(anchor) = expected {
                assert_eq!(
                    app.take_effects(),
                    vec![PluginEffect::SetPluginSurfacePlacement {
                        plugin_id: manifest.id.clone(),
                        surface_id: "main".into(),
                        anchor,
                        offset_x,
                        offset_y: 24,
                    }]
                );
            } else {
                assert!(app.take_effects().is_empty());
                assert!(app.last_error().is_some());
            }
        }
    }

    #[test]
    fn invalid_dynamic_root_css_rolls_back_the_event() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.surfaces[0].anchor = nickel_core::plugins::PluginSurfaceAnchor::TopLeft;
        manifest.surfaces[0].offset_y = 20;
        let source = "function App() { const [bad, setBad] = useState(false); return h(Window, {id:'main',width:520,height:340,className:bad?'bad':'good'}, h(Button, {id:'toggle',onClick:()=>setBad(true)}, 'Toggle')); }";
        let mut app = PluginPanelApplication::new_with_manifest_for_surface(
            source,
            &manifest,
            None,
            Some("main"),
            None,
        )
        .unwrap();
        app.stylesheet = StyleSheet::compile("window.bad { top: 30px; }").unwrap();
        let generation = app.accepted.generation();
        let nodes = app.accepted.nodes().clone();
        let handler_slots = app.accepted.handler_slots().clone();
        let source = app.accepted.source().clone();
        let candidate_generation = app.next_generation;
        app.update(app.button_message("toggle").unwrap());
        assert!(app.last_error().unwrap().contains("exceeds its grant"));
        assert!(app.take_effects().is_empty());
        assert_eq!(app.accepted.generation(), generation);
        assert_eq!(app.accepted.nodes(), &nodes);
        assert_eq!(app.accepted.handler_slots(), &handler_slots);
        assert_eq!(app.accepted.source(), &source);
        assert_eq!(app.next_generation, candidate_generation + 1);
        assert!(matches!(
            &app.accepted.node(),
            PanelNode::Surface { class_name: Some(class_name), .. } if class_name == "good"
        ));
    }

    #[test]
    fn rejected_effect_preserves_the_full_accepted_wrapper() {
        let source = "function App() { const [count,setCount]=useState(0); return h(Panel, {}, h(Button, {id:'reject',onClick:()=>{setCount(1);nickel.request({type:'not.granted'})}}, String(count))); }";
        let mut app = PluginPanelApplication::new(source).unwrap();
        let accepted_generation = app.accepted.generation();
        let accepted_source = app.accepted.source().clone();
        let accepted_nodes = app.accepted.nodes().clone();
        let accepted_slots = app.accepted.handler_slots().clone();

        app.update(app.button_message("reject").unwrap());

        assert!(app.last_error().is_some());
        assert!(app.take_effects().is_empty());
        assert_eq!(app.accepted.generation(), accepted_generation);
        assert_eq!(app.accepted.source(), &accepted_source);
        assert_eq!(app.accepted.nodes(), &accepted_nodes);
        assert_eq!(app.accepted.handler_slots(), &accepted_slots);
        assert!(format!("{:?}", app.accepted.node()).contains("\"0\""));
    }

    #[test]
    fn invalid_host_data_root_keeps_the_last_valid_window() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.surfaces[0].anchor = nickel_core::plugins::PluginSurfaceAnchor::TopLeft;
        manifest.surfaces[0].offset_y = 20;
        let source = "function App() { return h(Window, {id:'main',width:520,height:340,className:nickel.data.invalid?'bad':'good'}, h(Text, null, nickel.data.label)); }";
        let mut app = PluginPanelApplication::new_with_manifest_for_surface(
            source,
            &manifest,
            Some(r#"{"invalid":false,"label":"Before"}"#.into()),
            Some("main"),
            None,
        )
        .unwrap();
        app.stylesheet = StyleSheet::compile("window.bad { top: 30px; }").unwrap();
        assert!(
            !app.sync_data(&serde_json::json!({"invalid": true, "label": "Rejected"}))
                .unwrap()
        );
        assert!(app.last_error().unwrap().contains("exceeds its grant"));
        assert!(format!("{:?}", app.accepted.node()).contains("Before"));
        assert!(
            app.sync_data(&serde_json::json!({"invalid": false, "label": "After"}))
                .unwrap()
        );
        assert!(app.last_error().is_none());
        assert!(format!("{:?}", app.accepted.node()).contains("After"));
    }

    #[test]
    fn audio_host_data_requires_audio_read_and_updates_the_tree() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        let source = "function App() { return h(Window, {id:'main',width:520,height:340}, h(Text, null, nickel.data.audio.percent)); }";
        let mut app = PluginPanelApplication::new_with_manifest_for_surface(
            source,
            &manifest,
            Some(r#"{"audio":{"percent":10}}"#.into()),
            Some("main"),
            None,
        )
        .unwrap();
        let next = serde_json::json!({"percent": 65});
        assert!(app.sync_host_data_field("audio", &next).is_err());

        manifest.capabilities.push(PluginCapability::AudioRead);
        let mut app = PluginPanelApplication::new_with_manifest_for_surface(
            source,
            &manifest,
            Some(r#"{"audio":{"percent":10}}"#.into()),
            Some("main"),
            None,
        )
        .unwrap();
        assert!(app.sync_host_data_field("audio", &next).unwrap());
        assert!(!app.sync_host_data_field("audio", &next).unwrap());
        assert!(format!("{:?}", app.accepted.node()).contains("65"));
    }

    #[test]
    fn appearance_clients_require_domain_read_and_control_grants() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        for (resource, read, control, snapshot, call) in [
            (
                "appearance",
                PluginCapability::AppearanceRead,
                PluginCapability::AppearanceControl,
                serde_json::json!({"available":true,"writable":true,"generation":1,"configured":{"theme":"system","accent_hue":null,"accent_intensity":null,"reduce_transparency":false,"animations":"normal"}}),
                "nickel.appearance.set({theme:'dark',accent_hue:271,accent_intensity:63,reduce_transparency:true,animations:'reduced'})",
            ),
            (
                "wallpaper",
                PluginCapability::WallpaperRead,
                PluginCapability::WallpaperControl,
                serde_json::json!({"available":true,"writable":true,"generation":1,"configured":{"custom_image_configured":false,"position":"fill"},"images":[{"id":"approved"}]}),
                "nickel.wallpaper.selectImage('approved')",
            ),
            (
                "wallpaper",
                PluginCapability::WallpaperRead,
                PluginCapability::WallpaperControl,
                serde_json::json!({"available":true,"writable":true,"generation":1,"configured":{"custom_image_configured":false,"position":"fill"},"images":[],"chooser":{"available":true,"pending":false}}),
                "nickel.wallpaper.chooseImage()",
            ),
        ] {
            let source = format!(
                "function App() {{ return h(Window, {{id:'main',width:520,height:340}}, h(Button, {{id:'apply',onClick:()=>{call}}}, 'Apply')); }}"
            );
            let mut data = serde_json::json!({});
            data[resource] = snapshot.clone();
            for grants in [vec![], vec![read], vec![control], vec![read, control]] {
                manifest.capabilities = grants.clone();
                let mut app = PluginPanelApplication::new_with_manifest(
                    &source,
                    &manifest,
                    Some(data.to_string()),
                )
                .unwrap();
                assert_eq!(
                    app.sync_host_data_field(resource, &snapshot).is_ok(),
                    grants.contains(&read)
                );
                app.update(app.button_message("apply").unwrap());
                assert_eq!(
                    matches!(
                        app.take_effects().as_slice(),
                        [PluginEffect::Appearance { .. }]
                    ),
                    grants.contains(&read) && grants.contains(&control)
                );
            }
            manifest.capabilities = vec![read, control];
            let stale_source = "function App() { return h(Window, {id:'main',width:520,height:340}, h(Button, {id:'apply',onClick:()=>nickel.request({type:'wallpaper.change',transaction:{generation:0,prior:{custom_image_configured:false,position:'fill'},change:{kind:'reset_custom_image'}}})}, 'Apply')); }";
            let mut app = PluginPanelApplication::new_with_manifest(
                stale_source,
                &manifest,
                Some(data.to_string()),
            )
            .unwrap();
            app.update(app.button_message("apply").unwrap());
            assert!(app.take_effects().is_empty());
        }
    }

    #[test]
    fn file_artwork_controls_have_visible_native_bounds_and_dispatch_choices() {
        with_package_runtime_stack(|| {
            let mut package = PluginPackage::load(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/example-window"
            ))
            .unwrap();
            package.manifest.surfaces[0].width = 809;
            package.manifest.surfaces[0].height = 529;
            package.manifest.capabilities = vec![
                PluginCapability::PreferencesRead,
                PluginCapability::PreferencesControl,
            ];
            package.source =
                include_str!("../../../assets/plugins/nickel-default/src/Preferences.js")
                    .lines()
                    .filter(|line| {
                        !line.starts_with("import ") && !line.starts_with("registerSettingsPage(")
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
                    .replace("export function", "function");
            package.source.push_str("\nfunction App() { return h(Window, {id:'main',width:809,height:529}, h(FileArtwork)); }");
            package.stylesheet = format!(
                "{}\n{}",
                include_str!("../../../assets/plugins/nickel-default/src/styles/controls.css"),
                include_str!("../../../assets/plugins/nickel-default/src/styles/preferences.css")
            );
            let snapshot = serde_json::json!({"available":true,"writable":true,"revision":"0123456789abcdef",
                "configured":{"barOnAllDisplays":true,"allWindowsOnEveryBar":true,"desktopCount":4,"preferredTerminal":null,"preferredFileManager":null,"fileIconProvider":"system","fileIconTheme":"breeze-dark","idleDimSeconds":300,"idleLockSeconds":900,"idleSuspendSeconds":null},
                "iconThemes":["breeze-dark","Papirus"],"unavailableSelections":{}});
            let mut app = PluginPanelApplication::new_with_manifest(
                &package.source,
                &package.manifest,
                Some(serde_json::json!({"preferences":snapshot}).to_string()),
            )
            .unwrap();
            app.stylesheet = StyleSheet::compile(&package.stylesheet).unwrap();
            let mut host = nickel_ui::UiHost::new(app, 809, 529);
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    application_changed: true,
                    ..Default::default()
                },
            )
            .unwrap();
            for label in [
                "Nickel icons",
                "System icons",
                "Use system default icon theme",
                "breeze-dark",
                "Papirus",
            ] {
                let target = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: nickel_ui::SemanticRole::Button,
                        name: label.into(),
                    })
                    .unwrap();
                assert!(
                    target.bounds.size.width > 20.0,
                    "{label} has no usable width"
                );
                assert!(
                    target.bounds.size.height >= 36.0,
                    "{label} has no usable height"
                );
                assert!(
                    target.bounds.origin.y + target.bounds.size.height <= 529.0,
                    "{label} is clipped"
                );
            }
            let app = host.application_mut();
            app.update(
                app.button_message("preferences-file-theme-choice-1")
                    .unwrap(),
            );
            assert!(matches!(
                app.take_effects().as_slice(),
                [PluginEffect::Preferences { .. }]
            ));
        });
    }

    #[test]
    fn preferences_clients_enforce_read_control_grants_and_typed_fields() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities.clear();
        let snapshot = serde_json::json!({"available":true,"writable":true,"revision":"0123456789abcdef","configured":{"barOnAllDisplays":true,"allWindowsOnEveryBar":true,"desktopCount":4,"preferredTerminal":null,"preferredFileManager":null,"fileIconProvider":"nickel","fileIconTheme":null,"idleDimSeconds":300,"idleLockSeconds":900,"idleSuspendSeconds":null}});
        let source = "function App() { return h(Window, {id:'main',width:520,height:340}, h(Button, {id:'change',onClick:()=>nickel.preferences.set({desktopCount:6})}, 'Change')); }";
        let data = serde_json::json!({"preferences":snapshot}).to_string();
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        assert!(
            denied
                .sync_host_data_field("preferences", &snapshot)
                .is_err()
        );
        denied.update(denied.button_message("change").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest
            .capabilities
            .push(PluginCapability::PreferencesRead);
        let mut readonly =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        assert!(
            readonly
                .sync_host_data_field("preferences", &snapshot)
                .is_ok()
        );
        readonly.update(readonly.button_message("change").unwrap());
        assert!(readonly.take_effects().is_empty());
        manifest
            .capabilities
            .push(PluginCapability::PreferencesControl);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        granted.update(granted.button_message("change").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::Preferences { .. }]
        ));
        let invalid = source.replace("desktopCount:6", "desktopCount:99");
        let mut invalid =
            PluginPanelApplication::new_with_manifest(&invalid, &manifest, Some(data)).unwrap();
        invalid.update(invalid.button_message("change").unwrap());
        assert!(invalid.take_effects().is_empty());
    }

    #[test]
    fn plugins_clients_require_both_grants_and_current_inventory() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities.clear();
        let snapshot = serde_json::json!({"available":true,"writable":true,"revision":"7","plugins":[{"id":"example","enabled":true}]});
        let source = "function App() { return h(Window,{id:'main',width:520,height:340},h(Button,{id:'change',onClick:()=>nickel.plugins.disable('example','7')},'Disable')); }";
        let data = serde_json::json!({"plugins":snapshot}).to_string();
        for capabilities in [
            vec![],
            vec![PluginCapability::PluginsRead],
            vec![PluginCapability::PluginsControl],
        ] {
            manifest.capabilities = capabilities;
            let mut denied =
                PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                    .unwrap();
            denied.update(denied.button_message("change").unwrap());
            assert!(denied.take_effects().is_empty());
            assert_eq!(
                denied.sync_host_data_field("plugins", &snapshot).is_ok(),
                manifest
                    .capabilities
                    .contains(&PluginCapability::PluginsRead)
            );
        }
        manifest.capabilities = vec![
            PluginCapability::PluginsRead,
            PluginCapability::PluginsControl,
        ];
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data)).unwrap();
        granted.update(granted.button_message("change").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::Plugins { .. }]
        ));
        let mut stale = snapshot;
        stale["revision"] = "8".into();
        granted.sync_host_data_field("plugins", &stale).unwrap();
        granted.update(granted.button_message("change").unwrap());
        assert!(granted.take_effects().is_empty());
    }

    #[test]
    fn plugin_metadata_and_preview_effects_dispatch_with_native_grants() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        for (operation, preview) in [
            ("nickel.plugins.setSetting('example','count',3,'7')", false),
            ("nickel.plugins.selectShell('example','7')", false),
            ("nickel.plugins.confirmShell('11','7')", true),
            ("nickel.plugins.revertShell('11','7')", true),
        ] {
            let snapshot = serde_json::json!({"available":true,"writable":true,"revision":"7",
                "shellPreview":if preview { serde_json::json!({"token":"11","canConfirm":true,"canRevert":true}) } else {Value::Null},
                "plugins":[{"id":"example","shell":true,"enabled":true,
                    "settings":[{"id":"count","kind":{"kind":"integer","min":1,"max":4},"value":2}]}]
            });
            let source = format!(
                "function App() {{ return h(Window,{{id:'main',width:520,height:340}},h(Button,{{id:'change',onClick:()=>{operation}}},'Change')); }}"
            );
            for capabilities in [
                vec![],
                vec![PluginCapability::PluginsRead],
                vec![PluginCapability::PluginsControl],
                vec![
                    PluginCapability::PluginsRead,
                    PluginCapability::PluginsControl,
                ],
            ] {
                let admitted = capabilities.len() == 2;
                manifest.capabilities = capabilities;
                let mut application = PluginPanelApplication::new_with_manifest(
                    &source,
                    &manifest,
                    Some(serde_json::json!({"plugins":snapshot}).to_string()),
                )
                .unwrap();
                application.update(application.button_message("change").unwrap());
                let effects = application.take_effects();
                if admitted {
                    assert!(
                        application.last_error().is_none(),
                        "{operation}: {:?}",
                        application.last_error()
                    );
                    assert!(
                        matches!(
                            (operation, effects.as_slice()),
                            (
                                "nickel.plugins.setSetting('example','count',3,'7')",
                                [PluginEffect::PluginsSetting { .. }]
                            ) | (
                                "nickel.plugins.selectShell('example','7')",
                                [PluginEffect::ShellSelection { .. }]
                            ) | (
                                "nickel.plugins.confirmShell('11','7')"
                                    | "nickel.plugins.revertShell('11','7')",
                                [PluginEffect::ShellPreviewDecision { .. }]
                            )
                        ),
                        "{operation}: {effects:?}"
                    );
                    let mut stale = snapshot.clone();
                    stale["revision"] = "8".into();
                    application.sync_host_data_field("plugins", &stale).unwrap();
                    application.update(application.button_message("change").unwrap());
                    assert!(application.take_effects().is_empty(), "stale {operation}");
                } else {
                    assert!(effects.is_empty(), "ungranted {operation}: {effects:?}");
                }
            }
        }
    }

    #[test]
    fn associations_clients_require_grants_and_validate_effect_identity() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities.clear();
        let snapshot = serde_json::json!({"available":true,"revision":"7","targets":[{"id":"mime:text/plain","canSetDefault":true,"protected":false,"effectiveHandlerId":"old.desktop","handlers":[{"id":"new.desktop","protected":false}]}]});
        let source = "function App() { return h(Window, {id:'main',width:520,height:340}, h(Button, {id:'change',onClick:()=>nickel.associations.setDefault('mime:text/plain','new.desktop','7')}, 'Change')); }";
        let data = serde_json::json!({"associations":snapshot}).to_string();
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        assert!(
            denied
                .sync_host_data_field("associations", &snapshot)
                .is_err()
        );
        denied.update(denied.button_message("change").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest
            .capabilities
            .push(PluginCapability::AssociationsRead);
        let mut readonly =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        assert!(
            readonly
                .sync_host_data_field("associations", &snapshot)
                .is_ok()
        );
        readonly.update(readonly.button_message("change").unwrap());
        assert!(readonly.take_effects().is_empty());
        manifest
            .capabilities
            .push(PluginCapability::AssociationsControl);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data)).unwrap();
        granted.update(granted.button_message("change").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::Associations { .. }]
        ));
        let mut stale = snapshot;
        stale["revision"] = "8".into();
        granted
            .sync_host_data_field("associations", &stale)
            .unwrap();
        granted.update(granted.button_message("change").unwrap());
        assert!(granted.take_effects().is_empty());

        let source = "function App() { return h(Window, {id:'main',width:520,height:340}, h(Button, {id:'open',onClick:()=>nickel.associations.openSystemSettings()}, 'Open')); }";
        manifest.capabilities.clear();
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        denied.update(denied.button_message("open").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest
            .capabilities
            .push(PluginCapability::AssociationsControl);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        granted.update(granted.button_message("open").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::Associations {
                effect: crate::associations_capabilities::AssociationsEffect::OpenSystemSettings,
                ..
            }]
        ));
    }

    #[test]
    fn public_window_operations_require_context_grants_and_preserve_destination_identity() {
        let mut manifest = manifest().clone();
        manifest.capabilities.clear();
        let source = "function App(){return h(Panel,{},h(Button,{id:'move',onClick:()=>nickel.windows.moveToWorkspace('71','18446744073709551615')},'Move'));}";
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        assert!(
            denied
                .sync_host_data_field("windowMenu", &serde_json::json!({"targetId":"71"}))
                .is_err()
        );
        denied.update(denied.button_message("move").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest.capabilities.push(PluginCapability::WindowsContext);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        granted.update(granted.button_message("move").unwrap());
        assert!(
            matches!(granted.take_effects().as_slice(), [PluginEffect::WindowOperation { operation, window: Some(crate::model::WindowId(71)), destination: Some(destination), .. }] if operation == "windows.moveToWorkspace" && destination == "18446744073709551615")
        );
    }

    #[test]
    fn public_window_menu_dismissal_validates_focus_option_and_grant() {
        for (options, permitted, expected) in [
            ("", true, Some(true)),
            ("{restoreFocus:false}", true, Some(false)),
            ("{restoreFocus:'false'}", true, None),
            ("{restoreFocus:false}", false, None),
        ] {
            let mut manifest = manifest().clone();
            manifest.capabilities.clear();
            if permitted {
                manifest.capabilities.push(PluginCapability::WindowsContext);
            }
            let source = format!(
                "function App(){{return h(Panel,{{}},h(Button,{{id:'dismiss',onClick:()=>nickel.windows.dismissMenu({options})}},'Dismiss'));}}"
            );
            let mut application =
                PluginPanelApplication::new_with_manifest(&source, &manifest, Some("{}".into()))
                    .unwrap();
            application.update(application.button_message("dismiss").unwrap());
            let effects = application.take_effects();
            match expected {
                Some(expected) => assert!(
                    matches!(effects.as_slice(), [PluginEffect::WindowOperation{operation,restore_focus,..}] if operation == "windows.dismissMenu" && *restore_focus == expected)
                ),
                None => assert!(effects.is_empty()),
            }
        }
    }

    #[test]
    fn connectivity_clients_require_read_and_control_grants_and_reject_stale_identity() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities.clear();
        let snapshot =
            crate::connectivity_capabilities::wifi_snapshot(&crate::platform::NetworkStatus {
                available: true,
                enabled: true,
                networks: vec![crate::platform::WifiNetworkStatus {
                    id: "stable-profile".into(),
                    saved: true,
                    ..Default::default()
                }],
                ..Default::default()
            });
        let source = "function App() { return h(Window, {id:'main',width:520,height:340}, h(Button, {id:'connect',onClick:()=>nickel.wifi.connect('stable-profile')}, 'Connect')); }";
        let data = serde_json::json!({"wifi": snapshot}).to_string();
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        assert!(denied.sync_host_data_field("wifi", &snapshot).is_err());
        denied.update(denied.button_message("connect").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest.capabilities.extend([
            PluginCapability::NetworkRead,
            PluginCapability::NetworkControl,
        ]);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data)).unwrap();
        granted.update(granted.button_message("connect").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::Connectivity { .. }]
        ));
        let mut stale = snapshot;
        stale["networks"] = serde_json::json!([]);
        granted.sync_host_data_field("wifi", &stale).unwrap();
        granted.update(granted.button_message("connect").unwrap());
        assert!(granted.take_effects().is_empty());
        assert!(granted.last_error().is_some());
    }

    #[test]
    fn public_workspace_requests_require_grants_and_reject_retired_private_control_actions() {
        let mut manifest = manifest().clone();
        manifest.capabilities.clear();
        let snapshot = crate::workspace_capabilities::snapshot(
            &[
                crate::platform::WorkspaceSummary {
                    id: 9,
                    active: true,
                },
                crate::platform::WorkspaceSummary {
                    id: 44,
                    active: false,
                },
            ],
            true,
        );
        let data = serde_json::json!({"workspaces":snapshot}).to_string();
        let source = "function App(){return h(Panel,{},h(Button,{id:'go',onClick:()=>nickel.workspaces.switch('44')},'Go'));}";
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        assert!(
            denied
                .sync_host_data_field("workspaces", &snapshot)
                .is_err()
        );
        denied.update(denied.button_message("go").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest.capabilities.extend([
            PluginCapability::WorkspacesRead,
            PluginCapability::WorkspacesSwitch,
        ]);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data)).unwrap();
        granted.update(granted.button_message("go").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::Workspace { .. }]
        ));
        let obsolete = "function App(){return h(Panel,{},h(Button,{id:'go',onClick:()=>nickel.request({type:'control-action',action:'workspace-switch',value:44})},'Go'));}";
        let mut retired =
            PluginPanelApplication::new_with_manifest(obsolete, &manifest, Some("{}".into()))
                .unwrap();
        retired.update(retired.button_message("go").unwrap());
        assert!(retired.take_effects().is_empty());
    }

    #[test]
    fn optional_feature_effects_require_grants_and_the_current_native_snapshot() {
        let mut manifest = manifest().clone();
        manifest.capabilities.clear();
        let features = crate::feature_capabilities::snapshot(
            &nickel_core::optional_features::OptionalFeatureSettings::default(),
            &nickel_core::optional_features::OptionalFeatureRuntime::default(),
            None,
            false,
            nickel_core::optional_features::FeaturePolicy::Editable,
            true,
        );
        let data = serde_json::json!({"features":features}).to_string();
        let source = "function App(){return h(Panel,{},h(Button,{id:'set',onClick:()=>nickel.features.setKeyboardMode('disabled')},'Set'));}";
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        assert!(denied.sync_host_data_field("features", &features).is_err());
        denied.update(denied.button_message("set").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest.capabilities.extend([
            PluginCapability::FeaturesRead,
            PluginCapability::FeaturesControl,
        ]);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data)).unwrap();
        granted.update(granted.button_message("set").unwrap());
        assert!(matches!(
            granted.take_effects().as_slice(),
            [PluginEffect::Feature { .. }]
        ));
    }

    #[test]
    fn application_search_requires_read_grants_and_keeps_package_identity() {
        let mut manifest = manifest().clone();
        manifest.id = "search-owner".into();
        manifest.capabilities.clear();
        let source = "function App() { return h(Panel, {}, h(Button, {id:'search',onClick:()=>nickel.applications.search('editor')}, 'Search')); }";
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        assert!(
            denied
                .sync_host_data_field("applicationSearch", &serde_json::json!({"results":[]}))
                .is_err()
        );
        denied.update(denied.button_message("search").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest
            .capabilities
            .push(PluginCapability::ApplicationsRead);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some("{}".into()))
                .unwrap();
        granted.update(granted.button_message("search").unwrap());
        assert_eq!(
            granted.take_effects(),
            vec![PluginEffect::SearchApplications {
                plugin_id: "search-owner".into(),
                query: "editor".into()
            }]
        );
    }

    #[test]
    fn external_window_actions_require_the_matching_capability() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        for (action, capability, expected) in [
            (
                "activate",
                PluginCapability::WindowsFocus,
                PluginEffect::ActivateWindow(crate::model::WindowId(71)),
            ),
            (
                "close",
                PluginCapability::WindowsContext,
                PluginEffect::CloseWindow(crate::model::WindowId(71)),
            ),
        ] {
            let request = if action == "activate" {
                "windows.focus"
            } else {
                "windows.close"
            };
            let source = format!(
                "function App() {{ return h(Window, {{id:'main',width:520,height:340}}, h(Button, {{id:'action',onClick:()=>nickel.request({{type:'{request}',id:'71'}})}}, 'Act')); }}"
            );
            manifest.capabilities.clear();
            let mut denied =
                PluginPanelApplication::new_with_manifest(&source, &manifest, None).unwrap();
            denied.update(denied.button_message("action").unwrap());
            assert!(denied.take_effects().is_empty());
            assert!(denied.last_error().is_some());

            manifest.capabilities.push(capability);
            let mut granted =
                PluginPanelApplication::new_with_manifest(&source, &manifest, None).unwrap();
            granted.update(granted.button_message("action").unwrap());
            assert_eq!(granted.take_effects(), vec![expected]);
        }
    }

    #[test]
    fn public_run_client_requires_grant_and_keeps_effect_owner() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities.clear();
        let source = "function App(){return h(Window,{width:320,height:180},h(Button,{id:'execute',onClick:()=>nickel.run.execute('editor')},'Run'));}";
        let data = serde_json::json!({"run":{"available":true,"revision":"owner:1","status":null}})
            .to_string();
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data.clone()))
                .unwrap();
        denied.update(denied.button_message("execute").unwrap());
        assert!(denied.take_effects().is_empty());
        assert!(denied.last_error().is_some());
        manifest.capabilities.push(PluginCapability::RunCommand);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, Some(data)).unwrap();
        granted.update(granted.button_message("execute").unwrap());
        assert_eq!(
            granted.take_effects(),
            vec![PluginEffect::RunExecute {
                plugin_id: manifest.id,
                execute: crate::run_capabilities::Execute {
                    command: "editor".into(),
                    revision: "owner:1".into()
                }
            }]
        );
    }

    #[test]
    fn one_ui_transition_dispatches_all_jsx_handlers_before_rerendering() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities = vec![PluginCapability::LauncherShow];
        let source = r#"
            function App() {
                const [show, setShow] = useState(true);
                return h(Window, {width: 320, height: 180},
                    h(Button, {id: 'hide', onClick: () => setShow(false)}, 'Hide'),
                    show ? h(Button, {id: 'open', onClick: () => nickel.request('show-launcher')}, 'Open') : null);
            }
        "#;
        let mut app = PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        let hide = app.button_message("hide").unwrap();
        let open = app.button_message("open").unwrap();
        app.update_messages(vec![hide, open]);
        assert_eq!(app.take_effects(), vec![PluginEffect::ShowLauncher]);
        assert!(app.button_message("open").is_none());
        assert!(app.last_error().is_none());
    }

    #[test]
    fn native_window_focus_batches_control_and_window_callbacks_once() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities = vec![PluginCapability::ProjectsMenuShow];
        let source = r#"function App() {
            const [show,setShow] = useState(true);
            const [activated,setActivated] = useState(false);
            return h(Window,{id:'main',width:320,height:180,onFocus:()=>{setActivated(true);nickel.projects.show();},onBlur:()=>nickel.projects.toggle()},
                show ? h(TextField,{id:'field',placeholder:'Field',value:'',autoFocus:true,onChange:()=>{},onBlur:()=>{if(activated){setShow(false);nickel.projects.show();}}}) : null);
        }"#;
        let app = PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 320, 180);
        host.step(nickel_ui::HostBatch {
            window_focused: Some(false),
            ..Default::default()
        });
        // Initial loss may precede first compositor activation; it does not dismiss the window.
        assert!(host.application_mut().take_effects().is_empty());
        host.step(nickel_ui::HostBatch {
            window_focused: Some(true),
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ProjectsVisibility {
                plugin_id: manifest.id.clone(),
                toggle: false
            }]
        );
        host.step(nickel_ui::HostBatch {
            window_focused: Some(true),
            ..Default::default()
        });
        assert!(host.application_mut().take_effects().is_empty());
        let field = host
            .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
            .unwrap();
        host.request_focus(field.id);
        assert!(
            host.application_mut().take_effects().is_empty(),
            "control focus does not activate the Window callback"
        );
        // A focused child callback can rebuild; the root callback still belongs to the old batch.
        host.step(nickel_ui::HostBatch {
            window_focused: Some(false),
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![
                PluginEffect::ProjectsVisibility {
                    plugin_id: manifest.id.clone(),
                    toggle: false
                },
                PluginEffect::ProjectsVisibility {
                    plugin_id: manifest.id.clone(),
                    toggle: true
                }
            ]
        );
        host.step(nickel_ui::HostBatch {
            window_focused: Some(false),
            ..Default::default()
        });
        assert!(host.application_mut().take_effects().is_empty());

        manifest.capabilities.clear();
        let app = PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        let mut denied = nickel_ui::UiHost::new(app, 320, 180);
        denied.step(nickel_ui::HostBatch {
            window_focused: Some(true),
            ..Default::default()
        });
        assert!(denied.application_mut().take_effects().is_empty());
        assert!(denied.application().last_error().is_some());
    }

    #[test]
    fn jsx_text_fields_keep_explicit_accessible_names_when_values_change() {
        let source = "function App(){const [value,setValue]=useState(''); return h(Column,{}, h(TextField,{id:'search',value,placeholder:'Type here…',accessibilityLabel:'Search applications',onChange:setValue}), h(TextField,{id:'legacy',value:'',placeholder:'Legacy search',onChange:()=>{}}));}";
        let mut host =
            nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 400, 160);
        let selector = nickel_ui::SemanticSelector::RoleAndName {
            role: SemanticRole::TextField,
            name: "Search applications".into(),
        };
        let input = host.query_unique(&selector).unwrap();
        host.perform_semantic_action(
            input.id,
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(
                "terminal".into(),
            )),
        );
        assert!(host.query_unique(&selector).is_ok());
        assert!(
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::TextField,
                name: "Legacy search".into()
            })
            .is_ok()
        );
    }

    #[test]
    fn jsx_text_fields_dispatch_focus_and_blur_callbacks() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities = vec![
            PluginCapability::LauncherShow,
            PluginCapability::ControlCenterShow,
        ];
        let source = r#"
            function App() {
                return h(Window, {width: 320, height: 180},
                    h(TextField, {id: 'first', placeholder: 'First', value: '', onChange: () => {},
                        onFocus: () => nickel.request('show-launcher'),
                        onBlur: () => nickel.request({type: 'toggle-control-center'})}),
                    h(TextField, {id: 'second', placeholder: 'Second', value: '', onChange: () => {},
                        onFocus: () => nickel.request('show-launcher')}),
                    h(Button, {id: 'focus-button', onClick: () => {},
                        onFocus: () => nickel.request({type: 'toggle-control-center'})}, 'Focus button'));
            }
        "#;
        let app = PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 320, 180);
        let first = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::TextField,
                name: "First".into(),
            })
            .unwrap();
        let second = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::TextField,
                name: "Second".into(),
            })
            .unwrap();
        let button = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Focus button".into(),
            })
            .unwrap();
        host.handle_event(nickel_ui::UiEvent::AccessibilityFocus(first.id));
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowLauncher]
        );
        host.handle_event(nickel_ui::UiEvent::AccessibilityFocus(second.id));
        assert_eq!(
            host.application_mut().take_effects(),
            vec![
                PluginEffect::ToggleControlCenter,
                PluginEffect::ShowLauncher
            ]
        );
        host.handle_event(nickel_ui::UiEvent::AccessibilityFocus(button.id));
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ToggleControlCenter]
        );
    }

    #[test]
    fn jsx_removing_focused_field_dispatches_its_old_blur_handler_once() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities = vec![PluginCapability::ControlCenterShow];
        let source = r#"
            function App() {
                const [show, setShow] = useState(true);
                return h(Window, {width: 320, height: 180},
                    show ? h(TextField, {id: 'field', placeholder: 'Field', value: '',
                        onChange: () => {},
                        onBlur: () => nickel.request({type: 'toggle-control-center'})}) : null,
                    h(Button, {id: 'remove', onClick: () => setShow(false)}, 'Remove'));
            }
        "#;
        let app = PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        let mut host = nickel_ui::UiHost::new(app, 320, 180);
        let field = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::TextField,
                name: "Field".into(),
            })
            .unwrap();
        host.handle_event(nickel_ui::UiEvent::AccessibilityFocus(field.id));
        assert!(host.application_mut().take_effects().is_empty());
        let remove = host.application().button_message("remove").unwrap();
        host.application_mut().update(remove);
        host.step(nickel_ui::HostBatch {
            application_changed: true,
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ToggleControlCenter]
        );
        host.step(nickel_ui::HostBatch {
            application_changed: true,
            ..Default::default()
        });
        assert!(host.application_mut().take_effects().is_empty());
    }

    #[test]
    fn external_application_catalog_and_launch_action_follow_capabilities() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities.clear();
        let source = r#"
            function App() {
                return h(Window, {width: 320, height: 180},
                    h(Text, {}, nickel.data.applications?.[0]?.name || 'None'),
                    h(Button, {id: 'launch', onClick: () => nickel.request({type: 'applications.launch', id: 'org.example.Editor'})}, 'Launch'));
            }
        "#;
        let applications =
            serde_json::json!([{"id":"org.example.Editor","name":"Editor","pinned":false}]);
        let mut denied = PluginPanelApplication::new_with_manifest(
            source,
            &manifest,
            Some(r#"{"applications":[]}"#.into()),
        )
        .unwrap();
        assert!(
            denied
                .sync_host_data_field("applications", &applications)
                .is_err()
        );
        denied.update(denied.button_message("launch").unwrap());
        assert!(denied.take_effects().is_empty());

        manifest
            .capabilities
            .push(PluginCapability::ApplicationsRead);
        let mut read_only = PluginPanelApplication::new_with_manifest(
            source,
            &manifest,
            Some(r#"{"applications":[]}"#.into()),
        )
        .unwrap();
        assert!(
            read_only
                .sync_host_data_field("applications", &applications)
                .unwrap()
        );
        assert!(format!("{:?}", read_only.accepted.node()).contains("Editor"));
        read_only.update(read_only.button_message("launch").unwrap());
        assert!(read_only.take_effects().is_empty());

        manifest
            .capabilities
            .push(PluginCapability::ApplicationsLaunch);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        granted.update(granted.button_message("launch").unwrap());
        assert_eq!(
            granted.take_effects(),
            vec![PluginEffect::LaunchApplication {
                id: "org.example.Editor".into()
            }]
        );
    }

    #[test]
    fn stock_launcher_settings_requests_show_then_focus() {
        with_package_runtime_stack(|| {
            let package = crate::bundled_plugin_assets::load_package("nickel-default").unwrap();
            let surface = package
                .manifest
                .surfaces
                .iter()
                .find(|surface| surface.id == "launcher")
                .unwrap();
            let mut application = PluginPanelApplication::from_package_surface(
                &package,
                &Default::default(),
                surface,
            )
            .unwrap();
            for _ in 0..2 {
                application.update(application.button_message("launcher-settings").unwrap());
                assert_eq!(
                    application.take_effects(),
                    vec![
                        PluginEffect::ShowPluginSurface {
                            plugin_id: "nickel-default".into(),
                            surface_id: "settings".into()
                        },
                        PluginEffect::FocusPluginSurface {
                            plugin_id: "nickel-default".into(),
                            surface_id: "settings".into()
                        },
                    ]
                );
            }
        });
    }

    #[test]
    fn stock_taskbar_formats_public_clock_without_native_presentation_data() {
        let package = crate::bundled_plugin_assets::load_package("nickel-default").unwrap();
        let surface = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "taskbar")
            .unwrap();
        let mut application =
            PluginPanelApplication::from_package_surface(&package, &Default::default(), surface)
                .unwrap();
        application
            .sync_host_data_field(
                "clock",
                &serde_json::json!({"unixMilliseconds":47_100_000,"utcOffsetMinutes":0}),
            )
            .unwrap();
        assert!(format!("{:?}", application.accepted.node()).contains("1:05 PM"));
    }

    #[test]
    fn public_native_ui_clients_follow_host_owned_manifest_grants() {
        let mut manifest = PluginManifest::from_json(include_str!(
            "../../../assets/plugins/example-window/plugin.json"
        ))
        .unwrap();
        manifest.capabilities.clear();
        let source = "function App() { return h(Window, {width:320,height:180}, h(Button, {id:'projects',onClick:() => nickel.projects.show()}, 'Projects'), h(Button, {id:'keyboard',onClick:() => nickel.keyboard.toggle()}, 'Keyboard')); }";
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        denied.update(denied.button_message("projects").unwrap());
        denied.update(denied.button_message("keyboard").unwrap());
        assert!(denied.take_effects().is_empty());
        manifest.capabilities.extend([
            PluginCapability::ProjectsMenuShow,
            PluginCapability::OnScreenKeyboardShow,
        ]);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        granted.update(granted.button_message("projects").unwrap());
        granted.update(granted.button_message("keyboard").unwrap());
        assert_eq!(
            granted.take_effects(),
            vec![
                PluginEffect::ProjectsVisibility {
                    plugin_id: manifest.id.clone(),
                    toggle: false
                },
                PluginEffect::ToggleOnScreenKeyboard {
                    plugin_id: manifest.id
                }
            ]
        );
    }

    #[test]
    fn external_notification_data_and_actions_follow_capabilities() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities.clear();
        let source = r#"
            function App() {
                return h(Window, {width: 320, height: 180},
                    h(Text, {}, nickel.data.notifications?.notification?.summary || 'None'),
                    h(Button, {id: 'dismiss', onClick: () => nickel.notifications.dismiss(71)}, 'Dismiss'));
            }
        "#;
        let projection = serde_json::json!({
            "notification": {"id":71,"appName":"Mail","summary":"New mail","body":"Hello","actions":[]},
            "history":[]
        });
        let mut denied = PluginPanelApplication::new_with_manifest(
            source,
            &manifest,
            Some(r#"{"notifications":null}"#.into()),
        )
        .unwrap();
        assert!(
            denied
                .sync_host_data_field("notifications", &projection)
                .is_err()
        );
        denied.update(denied.button_message("dismiss").unwrap());
        assert!(denied.take_effects().is_empty());

        manifest
            .capabilities
            .push(PluginCapability::NotificationsRead);
        let mut read_only = PluginPanelApplication::new_with_manifest(
            source,
            &manifest,
            Some(r#"{"notifications":null}"#.into()),
        )
        .unwrap();
        assert!(
            read_only
                .sync_host_data_field("notifications", &projection)
                .unwrap()
        );
        assert!(format!("{:?}", read_only.accepted.node()).contains("New mail"));
        read_only.update(read_only.button_message("dismiss").unwrap());
        assert!(read_only.take_effects().is_empty());

        manifest
            .capabilities
            .push(PluginCapability::NotificationsAct);
        let mut granted = PluginPanelApplication::new_with_manifest(
            source,
            &manifest,
            Some(r#"{"notifications":null}"#.into()),
        )
        .unwrap();
        granted.update(granted.button_message("dismiss").unwrap());
        assert_eq!(
            granted.take_effects(),
            vec![PluginEffect::DismissNotification {
                plugin_id: manifest.id.clone(),
                id: 71
            }]
        );
    }

    #[test]
    fn component_window_example_opens_dialog_and_requests_settings() {
        with_package_runtime_stack(|| {
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
            assert!(matches!(
                host.application().accepted.node(),
                PanelNode::Surface {
                    window_request: Some(_),
                    ..
                }
            ));
            let open = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Button,
                    name: "Open dialog".into(),
                })
                .unwrap();
            assert!(open.bounds.size.width > 0.0);
            assert!(open.bounds.size.height > 0.0);
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
            assert!(settings.bounds.size.width > 0.0);
            assert!(settings.bounds.size.height > 0.0);
            host.step(nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(
                    nickel_ui::UiEvent::AccessibilityActivate(settings.id),
                )],
                ..Default::default()
            });
            assert_eq!(
                host.application_mut().take_effects(),
                vec![PluginEffect::ShowSettings(None)]
            );
            assert!(host.application_mut().last_error().is_none());
        });
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
        let focus = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Focus details".into(),
            })
            .unwrap();
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::AccessibilityActivate(focus.id),
            )],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::FocusPluginSurface {
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
    fn jsx_window_size_is_resolved_within_manifest_bounds() {
        let directory = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-two-windows"
        );
        let mut package = PluginPackage::load(directory).unwrap();
        let home = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "home")
            .unwrap()
            .clone();
        package.source = package
            .source
            .replace("home ? 400 : 450", "home ? 360 : 450")
            .replace("home ? 240 : 260", "home ? 220 : 260");
        let application =
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &home)
                .unwrap();
        let resolved = application.resolved_surface(&home).unwrap();
        assert_eq!((resolved.width, resolved.height), (360, 220));
        assert_eq!((home.width, home.height), (400, 240));

        package.source = package
            .source
            .replace("home ? 360 : 450", "home ? 401 : 450");
        assert!(
            PluginPanelApplication::from_package_surface(&package, &Default::default(), &home)
                .is_err()
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
        assert!(matches!(
            home.application().accepted.node(),
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        let open = home
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Open dialog".into(),
            })
            .unwrap();
        assert!(open.bounds.size.width > 0.0);
        assert!(open.bounds.size.height > 0.0);
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
        assert!(matches!(
            dialog.application().accepted.node(),
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        let dismiss = dialog
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Dismiss".into(),
            })
            .unwrap();
        assert!(dismiss.bounds.size.width > 0.0);
        assert!(dismiss.bounds.size.height > 0.0);
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
        assert!(matches!(
            home.application().accepted.node(),
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        let show = home
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Show overlay".into(),
            })
            .unwrap();
        assert!(show.bounds.size.width > 0.0);
        assert!(show.bounds.size.height > 0.0);
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
        assert!(matches!(
            overlay.application().accepted.node(),
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        assert!(overlay.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill {
                color: 0xb0202830,
                ..
            }
        )));
        let hide = overlay
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: nickel_ui::SemanticRole::Button,
                name: "Close overlay".into(),
            })
            .unwrap();
        assert!(hide.bounds.size.width > 0.0);
        assert!(hide.bounds.size.height > 0.0);
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
            modules: Vec::new(),
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
                host.application_mut().accepted.node().dialog("confirm"),
                Some(PanelNode::Dialog { open: true, .. })
            ));
            host.step(nickel_ui::HostBatch {
                events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Dismiss)],
                ..Default::default()
            });
            assert!(host.inspect().open_overlay.is_none());
            assert!(matches!(
                host.application_mut().accepted.node().dialog("confirm"),
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
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: "function App() { return h(Panel, {}, h(Text, {}, nickel.data.settings['show-count'] ? 'Shown' : 'Hidden')); }".into(),
        };
        let settings =
            std::collections::BTreeMap::from([("show-count".to_owned(), serde_json::json!(false))]);
        let app = PluginPanelApplication::from_package_with_settings(&package, &settings).unwrap();
        assert!(format!("{:?}", app.accepted.node()).contains("Hidden"));
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
        assert!(format!("{:?}", app.accepted.node()).contains("nickel-icon"));
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
    fn external_window_shortcuts_dispatch_jsx_handlers() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.window-shortcuts".into();
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%', onEscape: () => nickel.request('show-launcher'), onSubmit: () => nickel.request('show-launcher')}, h(Text, {}, 'Ready')); }".into(),
        };
        let mut panel = PluginPanelApplication::from_package(&package).unwrap();
        for shortcut in [Shortcut::Escape, Shortcut::Submit] {
            assert_eq!(
                panel.shortcut_outcome(shortcut).disposition,
                nickel_ui::EventDisposition::Handled
            );
            assert_eq!(panel.take_effects(), vec![PluginEffect::ShowLauncher]);
        }
    }

    #[test]
    fn external_control_center_toggle_requires_capability() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.control-toggle".into();
        let source = "function App() { return h(FixedWindow, {width: '100%', height: '100%', onEscape: () => nickel.request({type: 'toggle-control-center'})}); }";
        let mut package = PluginPackage {
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: source.into(),
        };
        let mut denied = PluginPanelApplication::from_package(&package).unwrap();
        denied.shortcut_outcome(Shortcut::Escape);
        assert!(denied.take_effects().is_empty());
        package
            .manifest
            .capabilities
            .push(PluginCapability::ControlCenterShow);
        let mut granted = PluginPanelApplication::from_package(&package).unwrap();
        granted.shortcut_outcome(Shortcut::Escape);
        assert_eq!(
            granted.take_effects(),
            vec![PluginEffect::ToggleControlCenter]
        );
    }

    #[test]
    fn external_preview_actions_use_grants_instead_of_plugin_identity() {
        let (request, capability, expected) = (
            "{type: 'windowPreviews.action', action: 'activate', window: '71', revision: 'r1'}",
            PluginCapability::WindowsFocus,
            PluginEffect::WindowPreviewRequest {
                plugin_id: "org.example.desktop-controls".into(),
                revision: "r1".into(),
                action: PreviewAction::Activate(crate::model::WindowId(71)),
            },
        );
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.desktop-controls".into();
        external_manifest.capabilities.clear();
        let mut package = PluginPackage {
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: format!(
                "function App() {{ return h(FixedWindow, {{width: '100%', height: '100%', onEscape: () => nickel.request({request})}}); }}"
            ),
        };
        let mut denied = PluginPanelApplication::from_package(&package).unwrap();
        denied.shortcut_outcome(Shortcut::Escape);
        assert!(denied.take_effects().is_empty());
        package.manifest.capabilities.push(capability);
        let mut missing_read = PluginPanelApplication::from_package(&package).unwrap();
        missing_read.shortcut_outcome(Shortcut::Escape);
        assert!(missing_read.take_effects().is_empty());
        package
            .manifest
            .capabilities
            .push(PluginCapability::WindowsRead);
        let mut granted = PluginPanelApplication::from_package(&package).unwrap();
        granted.shortcut_outcome(Shortcut::Escape);
        assert_eq!(granted.take_effects(), vec![expected]);
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
    fn application_pin_save_retry_requires_pin_capability() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.pin-retry".into();
        external_manifest.capabilities.clear();
        let mut package = PluginPackage {
            modules: Vec::new(),
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%', onEscape: () => nickel.request({type: 'applications-retry-pin-save'})}); }".into(),
        };
        let mut denied = PluginPanelApplication::from_package(&package).unwrap();
        denied.shortcut_outcome(Shortcut::Escape);
        assert!(denied.take_effects().is_empty());
        package
            .manifest
            .capabilities
            .push(PluginCapability::ApplicationsPin);
        let mut granted = PluginPanelApplication::from_package(&package).unwrap();
        granted.shortcut_outcome(Shortcut::Escape);
        assert_eq!(
            granted.take_effects(),
            vec![PluginEffect::RetryApplicationPinSave]
        );
    }

    #[test]
    fn plugin_stylesheet_tracks_host_palette_without_restarting_js() {
        let package = PluginPackage {
            modules: Vec::new(),
            manifest: manifest().clone(),
            images: Default::default(),
            stylesheet: "window { background: var(--nickel-panel); } text { color: var(--nickel-text); }".into(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%'}, h(Text, {}, 'Ready')); }".into(),
        };
        let mut app = PluginPanelApplication::from_package(&package).unwrap();
        let dark = nickel_core::theme::ThemePalette::from_appearance(
            nickel_core::theme::Appearance::default(),
        );
        let light =
            nickel_core::theme::ThemePalette::from_appearance(nickel_core::theme::Appearance {
                mode: nickel_core::theme::ThemeMode::Light,
                ..nickel_core::theme::Appearance::default()
            });
        assert_eq!(
            app.stylesheet.resolve("window", None, None).background,
            Some(0xff00_0000 | dark.panel)
        );
        assert!(app.sync_theme_palette(light).unwrap());
        assert_eq!(
            app.stylesheet.resolve("window", None, None).background,
            Some(0xff00_0000 | light.panel)
        );
        assert!(!app.sync_theme_palette(light).unwrap());
    }

    #[test]
    fn plugin_rows_and_grids_mirror_for_right_to_left_layout() {
        with_package_runtime_stack(|| {
            let package = PluginPackage {
            modules: Vec::new(),
            manifest: manifest().clone(),
            images: Default::default(),
            stylesheet: ".grid { display: grid; grid-template-columns: 80px 80px; } button { width: 70px; height: 30px; }".into(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%'}, h(Column, {}, h(Row, {}, h(Button, {id: 'row-first', onClick: () => {}}, 'Row first'), h(Button, {id: 'row-second', onClick: () => {}}, 'Row second')), h('div', {className: 'grid'}, h(Button, {id: 'grid-first', onClick: () => {}}, 'Grid first'), h(Button, {id: 'grid-second', onClick: () => {}}, 'Grid second')))); }".into(),
        };
            let ltr = nickel_ui::UiHost::new(
                PluginPanelApplication::from_package(&package).unwrap(),
                240,
                120,
            );
            let mut rtl_app = PluginPanelApplication::from_package(&package).unwrap();
            assert!(rtl_app.sync_reading_direction(nickel_ui::ReadingDirection::RightToLeft));
            let rtl = nickel_ui::UiHost::new(rtl_app, 240, 120);
            let x = |host: &nickel_ui::UiHost<PluginPanelApplication>, name: &str| {
                host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: name.into(),
                })
                .unwrap()
                .bounds
                .origin
                .x
            };
            assert!(x(&ltr, "Row first") < x(&ltr, "Row second"));
            assert!(x(&rtl, "Row first") > x(&rtl, "Row second"));
            assert!(x(&ltr, "Grid first") < x(&ltr, "Grid second"));
            assert!(x(&rtl, "Grid first") > x(&rtl, "Grid second"));
        });
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
        assert!(format!("{:?}", panel.accepted.node()).contains("second:1"));
        panel.update(PluginMessage::Click(0));
        assert!(panel.last_error().is_none());
        let without_first = format!("{:?}", panel.accepted.node());
        assert!(!without_first.contains("first:0"));
        assert!(without_first.contains("second:1"));
        panel.update(PluginMessage::Click(0));
        let restored = format!("{:?}", panel.accepted.node());
        assert!(restored.contains("first:0"));
        assert!(restored.contains("second:1"));
    }

    #[test]
    fn typed_patch_exposes_only_mount_and_generation_correlation() {
        let source = "function App(){const [count,setCount]=useState(0);return h(Panel,null,h(Button,{id:'next',onClick:()=>setCount(count+1)},String(count)));}";
        let mut panel = PluginPanelApplication::new(source).unwrap();
        let mount = panel.diagnostic_mount;
        panel.update(PluginMessage::Click(0));
        let correlation = nickel_ui::Application::take_frame_correlation(&mut panel).unwrap();
        assert_eq!(correlation.mount, mount);
        assert_eq!(correlation.generation, panel.accepted.generation());
        assert!(nickel_ui::Application::take_frame_correlation(&mut panel).is_none());
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

    #[test]
    fn gc_only_idle_maintenance_preserves_the_accepted_panel_generation() {
        let mut panel =
            PluginPanelApplication::new("function App(){return h(Panel,null,h(Text,null,'idle'))}")
                .unwrap();
        let generation = panel.accepted.generation();
        for _ in 0..16 {
            panel.runtime.borrow_mut().eval(
                "globalThis.temporaryAllocations=Array.from({length:16384},(_,i)=>({i}));temporaryAllocations=null"
            ).unwrap();
            assert!(!panel.service_idle_platform_tasks().unwrap());
            assert_eq!(panel.accepted.generation(), generation);
            assert!(panel.take_effects().is_empty());
        }
        panel.runtime.borrow_mut().begin_transaction().unwrap();
        assert!(panel.service_idle_platform_tasks().is_err());
        panel
            .runtime
            .borrow_mut()
            .finish_transaction(false)
            .unwrap();
        assert!(!panel.service_idle_platform_tasks().unwrap());
    }

    #[test]
    fn empty_validated_dispatch_preserves_native_tree_on_rejection() {
        let mut panel = PluginPanelApplication::new(
            "function App(){const [invalid,setInvalid]=useState(false);globalThis.invalidateIdle=setInvalid;return h(Panel,null,invalid?h('not-a-component'):h(Text,null,'accepted'));}",
        )
        .unwrap();
        let original = format!("{:?}", panel.accepted.node());
        let generation = panel.accepted.generation();
        panel
            .runtime
            .borrow_mut()
            .eval("invalidateIdle(true)")
            .unwrap();
        panel.dispatch_validated_events(Vec::new());
        assert_eq!(format!("{:?}", panel.accepted.node()), original);
        assert_eq!(panel.accepted.generation(), generation);
        assert!(panel.last_error().is_some());
        assert!(panel.take_effects().is_empty());
    }

    #[test]
    fn empty_validated_dispatch_keeps_effect_capability_checks() {
        for granted in [false, true] {
            let mut manifest = PluginPackage::load(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../assets/plugins/example-window"
            ))
            .unwrap()
            .manifest;
            manifest.capabilities = if granted {
                vec![PluginCapability::LauncherShow]
            } else {
                Vec::new()
            };
            let mut panel = PluginPanelApplication::new_with_manifest(
                "function App(){return h(Window,{width:320,height:180},h(Text,null,'accepted'))}",
                &manifest,
                None,
            )
            .unwrap();
            let generation = panel.accepted.generation();
            panel
                .runtime
                .borrow_mut()
                .eval("nickel.request('show-launcher')")
                .unwrap();
            panel.dispatch_validated_events(Vec::new());
            assert_eq!(panel.accepted.generation(), generation);
            if granted {
                assert_eq!(panel.take_effects(), vec![PluginEffect::ShowLauncher]);
                assert!(panel.last_error().is_none());
            } else {
                assert!(panel.take_effects().is_empty());
                assert!(panel.last_error().is_some());
            }
        }
    }

    #[test]
    fn idle_reconciliation_admits_dirty_state_without_fabricated_input() {
        let mut panel = PluginPanelApplication::new(
            "function App(){const [count,setCount]=useState(0);globalThis.setIdleCount=setCount;return h(Panel,null,h(Text,null,'count:'+count));}",
        )
        .unwrap();
        let original_generation = panel.accepted.generation();
        assert!(!panel.reconcile_idle_work().unwrap());
        assert_eq!(panel.accepted.generation(), original_generation);
        panel.runtime.borrow_mut().eval("setIdleCount(1)").unwrap();
        assert!(panel.reconcile_idle_work().unwrap());
        assert!(format!("{:?}", panel.accepted.node()).contains("count:1"));
        assert!(panel.accepted.generation() > original_generation);
        assert!(panel.last_error().is_none());
        assert!(panel.take_effects().is_empty());
        let accepted_generation = panel.accepted.generation();
        assert!(!panel.reconcile_idle_work().unwrap());
        assert_eq!(panel.accepted.generation(), accepted_generation);
    }

    #[test]
    fn host_poll_reconciles_pending_state_without_idle_timer_or_clean_redraw() {
        let panel = PluginPanelApplication::new(
            "function App(){const [count,setCount]=useState(0);globalThis.pollSetter=setCount;return h(Panel,null,h(Text,null,'count:'+count));}"
        ).unwrap();
        let runtime = panel.runtime.clone();
        let mut host = nickel_ui::UiHost::new(panel, 320, 80);
        let generation = host.resolved_frame_generation();
        assert!(host.next_deadline().is_none());
        assert!(!host.poll());
        assert_eq!(host.resolved_frame_generation(), generation);
        runtime.borrow_mut().eval("pollSetter(1)").unwrap();
        assert!(host.poll());
        assert!(format!("{:?}", host.application().accepted.node()).contains("count:1"));
        let generation = host.resolved_frame_generation();
        assert!(!host.poll());
        assert_eq!(host.resolved_frame_generation(), generation);
        assert!(host.next_deadline().is_none());
        assert!(host.application().last_error().is_none());
        runtime
            .borrow_mut()
            .eval("nickel.request('show-launcher')")
            .unwrap();
        assert!(!host.poll());
        assert_eq!(host.resolved_frame_generation(), generation);
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ShowLauncher]
        );
    }

    #[test]
    fn passive_effect_state_is_reconciled_after_mount() {
        let panel = PluginPanelApplication::new(
            r#"
                function App() {
                    const [count, setCount] = useState(0);
                    useEffect(() => { setCount(1); }, []);
                    return h(Panel, null, h(Text, null, 'count:' + count));
                }
            "#,
        )
        .unwrap();
        assert!(format!("{:?}", panel.accepted.node()).contains("count:1"));
        assert!(panel.last_error().is_none());
    }

    #[test]
    fn passive_effect_without_native_changes_preserves_generation() {
        let panel = PluginPanelApplication::new(
            r#"
                function App() {
                    const [ready, setReady] = useState(false);
                    useEffect(() => { setReady(true); }, []);
                    return h(Panel, null, h(Text, null, 'unchanged'));
                }
            "#,
        )
        .unwrap();
        assert!(panel.last_error().is_none());
        assert_eq!(panel.accepted.generation(), 1);
        assert_eq!(
            panel
                .runtime
                .borrow_mut()
                .eval_json::<u64>("__runtimeCounters.renders")
                .unwrap(),
            2
        );
    }

    #[test]
    fn passive_effect_updates_are_batched_into_one_generation() {
        let panel = PluginPanelApplication::new(
            r#"
                function App() {
                    const [left, setLeft] = useState(0);
                    const [right, setRight] = useState(0);
                    useEffect(() => { setLeft(1); setRight(2); }, []);
                    return h(Panel, null, h(Text, null, left + ':' + right));
                }
            "#,
        )
        .unwrap();
        assert!(format!("{:?}", panel.accepted.node()).contains("1:2"));
        assert_eq!(panel.next_generation, 3);
    }

    #[test]
    fn passive_effect_cleanup_updates_are_reconciled_after_dependency_change() {
        let mut panel = PluginPanelApplication::new(
            r#"
                function App() {
                    const [dependency, setDependency] = useState(0);
                    const [cleaned, setCleaned] = useState('none');
                    useEffect(() => () => setCleaned('cleanup:' + dependency), [dependency]);
                    return h(Panel, null,
                        h(Button, {onClick: () => setDependency(1)}, 'change'),
                        h(Text, null, cleaned));
                }
            "#,
        )
        .unwrap();
        panel.update(PluginMessage::Click(0));
        assert!(format!("{:?}", panel.accepted.node()).contains("cleanup:0"));
    }

    #[test]
    fn rejected_passive_effect_render_preserves_the_accepted_tree() {
        let panel = PluginPanelApplication::new(
            r#"
                function App() {
                    const [invalid, setInvalid] = useState(false);
                    useEffect(() => setInvalid(true), []);
                    return h(Panel, null, invalid ? h('not-a-component') : h(Text, null, 'accepted'));
                }
            "#,
        )
        .unwrap();
        assert!(format!("{:?}", panel.accepted.node()).contains("accepted"));
        assert!(panel.last_error().is_some());
    }

    #[test]
    fn clean_mount_does_not_schedule_idle_reconciliation() {
        let mut panel = PluginPanelApplication::new(
            "function App() { return h(Panel, null, h(Text, null, 'idle')); }",
        )
        .unwrap();
        let generation = panel.next_generation;
        assert!(!panel.reconcile_passive_effects().unwrap());
        assert_eq!(panel.next_generation, generation);
    }

    #[test]
    fn passive_effect_loops_are_bounded_and_diagnosed() {
        let panel = PluginPanelApplication::new(
            r#"
                function App() {
                    const [count, setCount] = useState(0);
                    useEffect(() => { setCount(count + 1); });
                    return h(Panel, null, h(Text, null, String(count)));
                }
            "#,
        )
        .unwrap();
        assert!(
            panel
                .last_error()
                .is_some_and(|error| error.contains("passive effect reconciliation exceeded"))
        );
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct SettingsAdmissionWork {
        cold_tree_bytes: u64,
        patch_bytes: u64,
        patch_operations: u64,
        patch_nodes_visited: u64,
        patch_nodes_mutated: u64,
        patch_local_materializations: u64,
        patch_expansion_nodes: u64,
        patch_complete_tree_bytes: u64,
        component_executions: u64,
        typed_apply_attempts: u64,
        typed_apply_rejections: u64,
        mounts_open: usize,
        mounts_closed: usize,
    }

    #[derive(Default)]
    struct SettingsAdmissionSamples {
        open: Vec<std::time::Duration>,
        page_switch: Vec<std::time::Duration>,
        control_edit: Vec<std::time::Duration>,
        close: Vec<std::time::Duration>,
        total: Vec<std::time::Duration>,
        exact: Option<SettingsAdmissionWork>,
    }

    fn settings_admission_distribution(samples: &[std::time::Duration]) -> Value {
        let mut samples = samples.to_vec();
        samples.sort_unstable();
        let at = |percentile: usize| {
            let rank = samples.len().saturating_mul(percentile).div_ceil(100);
            samples[rank.saturating_sub(1).min(samples.len() - 1)].as_nanos() as u64
        };
        serde_json::json!({
            "p50_ns": at(50), "p95_ns": at(95), "p99_ns": at(99),
            "max_ns": samples.last().unwrap().as_nanos() as u64,
        })
    }

    fn canonicalize_settings_actions(value: &mut Value) {
        match value {
            Value::Array(values) => values.iter_mut().for_each(canonicalize_settings_actions),
            Value::Object(object) => {
                let slots = object
                    .get("__handlerSlots")
                    .and_then(Value::as_object)
                    .map(|slots| {
                        slots
                            .iter()
                            .filter_map(|(property, slot)| {
                                slot.as_str()
                                    .map(|slot| (property.clone(), slot.to_owned()))
                            })
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_default();
                for (property, slot) in slots {
                    object.insert(property, Value::String(slot));
                }
                object.values_mut().for_each(canonicalize_settings_actions);
            }
            _ => {}
        }
    }

    fn find_settings_source<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
        if value.get("id").and_then(Value::as_str) == Some(id) {
            return Some(value);
        }
        match value {
            Value::Array(values) => values
                .iter()
                .find_map(|value| find_settings_source(value, id)),
            Value::Object(object) => object
                .values()
                .find_map(|value| find_settings_source(value, id)),
            _ => None,
        }
    }

    type AdmissionRuntime = (
        std::collections::BTreeMap<String, PluginPackage>,
        PluginSurface,
        std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>,
    );

    fn settings_admission_runtime() -> Result<AdmissionRuntime, String> {
        let package = crate::bundled_plugin_assets::load_package("nickel-default")?;
        settings_admission_runtime_with_package(package)
    }

    fn settings_admission_runtime_with_package(
        package: PluginPackage,
    ) -> Result<AdmissionRuntime, String> {
        let surface = package
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "settings")
            .ok_or("default Settings surface is missing")?
            .clone();
        let catalog = std::collections::BTreeMap::from([(package.manifest.id.clone(), package)]);
        let host = std::rc::Rc::new(std::cell::RefCell::new(ShellCompositionRuntime::new(
            &catalog,
            "nickel-default",
            &Default::default(),
        )?));
        let owners = host
            .borrow()
            .participating_owners()
            .cloned()
            .collect::<Vec<_>>();
        let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
        for owner in &owners {
            host.borrow()
                .shared_owner_runtime(owner)?
                .borrow_mut()
                .publish_settings(&mut registry, &owner.id)?;
        }
        for owner in &owners {
            host.borrow()
                .shared_owner_runtime(owner)?
                .borrow_mut()
                .set_settings_registry(&registry)?;
        }
        Ok((catalog, surface, host))
    }

    fn settings_admission_application_with_runtime(
        destination: &str,
        catalog: &std::collections::BTreeMap<String, PluginPackage>,
        surface: &PluginSurface,
        host: std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>,
    ) -> Result<PluginPanelApplication, String> {
        let owners = host
            .borrow()
            .participating_owners()
            .cloned()
            .collect::<Vec<_>>();
        let snapshots = owners
            .iter()
            .map(|owner| {
                let package = &catalog[&owner.id];
                let settings = package
                    .manifest
                    .settings
                    .iter()
                    .map(|setting| (setting.id.clone(), setting.kind.default_value()))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let mut data = serde_json::json!({
                    "settings": settings,
                    "windows": [],
                    "applications": [],
                    "notifications": initial_notifications_data(&package.manifest),
                    "surface": {"id":"settings","kind":"window","width":1100,"height":800},
                    "navigation": {"revision":"admission-1","destination":destination},
                    "appearance": {"available":true,"writable":true,"generation":1,
                        "configured":{"theme":"system","accent_hue":null,"accent_intensity":null,
                            "reduce_transparency":false,"animations":"normal"},
                        "resolved":{"theme":"dark","hue":200,"intensity":65,"accent":[55,145,255]}},
                    "wallpaper": {"available":true,"writable":true,"generation":1,
                        "configured":{"custom_image_configured":false,"position":"fill"},
                        "images":[],"chooser":{"available":true,"pending":false,"result":null}},
                });
                if let Some(projection) = validation_surface_projection(package, surface) {
                    data.as_object_mut()
                        .unwrap()
                        .extend(projection.as_object().unwrap().clone());
                }
                (owner.clone(), data)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let application = PluginPanelApplication::from_composed_surface(
            catalog,
            "nickel-default",
            &snapshots,
            surface,
            Some(host.clone()),
        )?;
        Ok(application)
    }

    fn settings_admission_application(
        destination: &str,
    ) -> Result<
        (
            PluginPanelApplication,
            std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>,
        ),
        String,
    > {
        let (catalog, surface, host) = settings_admission_runtime()?;
        let application = settings_admission_application_with_runtime(
            destination,
            &catalog,
            &surface,
            host.clone(),
        )?;
        Ok((application, host))
    }

    #[test]
    #[ignore = "headless Settings resize timing"]
    fn production_settings_resize_benchmark() {
        with_package_runtime_stack(|| {
            let (application, _composition) =
                settings_admission_application("nickel-default/appearance").unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            let mut samples = Vec::new();
            let mut view_samples = Vec::new();
            let mut layout_samples = Vec::new();
            for index in 0..60 {
                let started = std::time::Instant::now();
                let outcome = host.step(nickel_ui::HostBatch {
                    surface_size: Some((1000 + index * 2, 740 + index % 20)),
                    ..Default::default()
                });
                assert_eq!(outcome.telemetry.view_calls, 0);
                samples.push(started.elapsed().as_secs_f64() * 1000.0);
                view_samples.push(outcome.telemetry.paint_list_us as f64 / 1000.0);
                layout_samples.push(outcome.telemetry.layout_us as f64 / 1000.0);
            }
            samples.sort_by(f64::total_cmp);
            view_samples.sort_by(f64::total_cmp);
            layout_samples.sort_by(f64::total_cmp);
            println!(
                "settings resize: median={:.3}ms p95={:.3}ms view={:.3}ms layout={:.3}ms",
                samples[30], samples[57], view_samples[30], layout_samples[30]
            );
        });
    }

    #[test]
    fn appearance_theme_picker_previews_shell_and_exposes_keep_and_revert() {
        with_package_runtime_stack(|| {
            let (mut application, _composition) =
                settings_admission_application("nickel-default/appearance").unwrap();
            let mut catalog = serde_json::json!({"available":true,"writable":true,"revision":"7","shellPreview":null,
                "plugins":[{"id":"nickel-default","name":"Default","shell":true,"selected":true,"enabled":true},
                    {"id":"nickel-cupertino-dock","name":"Cupertino","shell":true,"selected":false,"enabled":true}]});
            application
                .sync_host_data_field("plugins", &catalog)
                .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 1000);
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            for _ in 0..32 {
                if !host.application().virtual_work_pending {
                    break;
                }
                let now = host.next_deadline();
                crate::live_shell::step_plugin_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        now,
                        ..Default::default()
                    },
                )
                .unwrap();
            }
            let activate = |host: &mut nickel_ui::UiHost<PluginPanelApplication>, name: &str| {
                let target = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: name.into(),
                    })
                    .unwrap();
                host.perform_semantic_action(
                    target.id,
                    nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                );
                assert!(host.application_mut().last_error().is_none());
            };
            activate(&mut host, "Preview Cupertino");
            assert!(
                matches!(host.application_mut().take_effects().as_slice(), [PluginEffect::ShellSelection { effect, .. }] if effect.id == "nickel-cupertino-dock" && effect.revision == 7)
            );
            catalog["shellPreview"] = serde_json::json!({"token":"11","previousShell":"nickel-default","selectedShell":"nickel-cupertino-dock","canConfirm":true,"canRevert":true});
            host.application_mut()
                .sync_host_data_field("plugins", &catalog)
                .unwrap();
            crate::live_shell::step_plugin_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    application_changed: true,
                    ..Default::default()
                },
            )
            .unwrap();
            activate(&mut host, "Keep theme");
            assert!(
                matches!(host.application_mut().take_effects().as_slice(), [PluginEffect::ShellPreviewDecision { effect, .. }] if effect.confirm)
            );
            activate(&mut host, "Restore previous theme");
            assert!(
                matches!(host.application_mut().take_effects().as_slice(), [PluginEffect::ShellPreviewDecision { effect, .. }] if !effect.confirm)
            );
        });
    }

    #[test]
    fn appearance_sliders_commit_once_on_release_and_cancel_without_writing() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let (application, _composition) =
                    settings_admission_application("nickel-default/appearance").unwrap();
                let mut host = nickel_ui::UiHost::new(application, 1100, 1400);
                for name in ["Interface hue", "Color intensity"] {
                    let slider = host
                        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                            role: SemanticRole::Slider,
                            name: name.into(),
                        })
                        .unwrap();
                    let point = |fraction| Point {
                        x: slider.bounds.origin.x + slider.bounds.size.width * fraction,
                        y: slider.bounds.origin.y + slider.bounds.size.height / 2.0,
                    };
                    let send = |host: &mut nickel_ui::UiHost<PluginPanelApplication>, event| {
                        crate::live_shell::step_plugin_host(
                            host,
                            None,
                            nickel_ui::HostBatch {
                                events: vec![nickel_ui::HostEvent::Ui(event)],
                                ..Default::default()
                            },
                        )
                        .unwrap();
                        assert!(host.application_mut().last_error().is_none());
                    };
                    host.application_mut().take_effects();
                    send(&mut host, nickel_ui::UiEvent::PointerMoved(point(0.2)));
                    send(&mut host, nickel_ui::UiEvent::PointerPressed(point(0.2)));
                    for fraction in [0.3, 0.5, 0.7] {
                        send(&mut host, nickel_ui::UiEvent::PointerMoved(point(fraction)));
                        assert!(host.application_mut().take_effects().is_empty());
                    }
                    send(&mut host, nickel_ui::UiEvent::PointerReleased(point(0.7)));
                    assert!(matches!(
                        host.application_mut().take_effects().as_slice(),
                        [PluginEffect::Appearance { .. }]
                    ));
                    send(&mut host, nickel_ui::UiEvent::PointerPressed(point(0.1)));
                    send(&mut host, nickel_ui::UiEvent::PointerMoved(point(0.4)));
                    send(&mut host, nickel_ui::UiEvent::PointerCancelled);
                    assert!(host.application_mut().take_effects().is_empty());
                    host.perform_semantic_action(
                        slider.id,
                        nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Number(
                            80.0,
                        )),
                    );
                    assert!(matches!(
                        host.application_mut().take_effects().as_slice(),
                        [PluginEffect::Appearance { .. }]
                    ));
                }
            })
            .unwrap()
            .join()
            .unwrap();
    }

    fn settings_profile_totals(diagnostics: &Value) -> [u64; 7] {
        diagnostics["profiles"]
            .as_array()
            .unwrap()
            .iter()
            .fold([0; 7], |mut totals, profile| {
                for (index, field) in [
                    "coldTreeTransportBytes",
                    "patchEnvelopeTransportBytes",
                    "patchOperations",
                    "patchNodesVisited",
                    "componentExecutions",
                    "typedPatchApplyAttempts",
                    "typedPatchApplyRejections",
                ]
                .iter()
                .enumerate()
                {
                    totals[index] += profile[*field].as_u64().unwrap_or(0);
                }
                totals
            })
    }

    fn exercise_production_settings_admission() -> (SettingsAdmissionWork, [std::time::Duration; 5])
    {
        let total_started = std::time::Instant::now();
        // The production shell retains one composition runtime for all of its
        // surfaces. Opening Settings mounts into that already-live authority.
        let (catalog, surface, composition) = settings_admission_runtime().unwrap();
        let open_started = std::time::Instant::now();
        let application = settings_admission_application_with_runtime(
            "nickel-default/plugins",
            &catalog,
            &surface,
            composition.clone(),
        )
        .unwrap();
        let mut host = nickel_ui::UiHost::new(application, 1100, 800);
        let open = open_started.elapsed();
        let mount_count = composition.borrow().mount_count();
        assert!(mount_count > 0);
        assert!(!host.commands().is_empty());
        let runtime = host.application_mut().shared_runtime();
        let before = settings_profile_totals(&runtime.borrow_mut().runtime_diagnostics().unwrap());
        let navigation = find_settings_source(
            host.application_mut().accepted.source(),
            "settings-navigation/destination/nickel-default/appearance",
        )
        .unwrap();
        let navigation_native = navigation["__nativeId"].clone();
        let navigation_handler = navigation["__handlerSlots"]["action"].clone();

        let page_started = std::time::Instant::now();
        let appearance = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Appearance".into(),
            })
            .unwrap();
        assert_eq!(
            host.application()
                .accepted
                .node()
                .button_action("settings-navigation/destination/nickel-default/appearance"),
            Some(6)
        );
        host.perform_semantic_action(
            appearance.id,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
        );
        let page_switch = page_started.elapsed();
        assert!(host.application_mut().last_error().is_none());
        let navigation = find_settings_source(
            host.application_mut().accepted.source(),
            "settings-navigation/destination/nickel-default/appearance",
        )
        .unwrap();
        assert_eq!(navigation["__nativeId"], navigation_native);
        assert_eq!(navigation["__handlerSlots"]["action"], navigation_handler);

        let control = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Slider,
                name: "Interface hue".into(),
            })
            .unwrap();
        let control_source =
            find_settings_source(host.application_mut().accepted.source(), "appearance-hue")
                .unwrap();
        let control_native = control_source["__nativeId"].clone();
        let control_handler = control_source["__handlerSlots"]["action"].clone();
        let edit_started = std::time::Instant::now();
        host.perform_semantic_action(
            control.id,
            nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Number(210.0)),
        );
        let control_edit = edit_started.elapsed();
        assert!(host.application_mut().last_error().is_none());
        let control_source =
            find_settings_source(host.application_mut().accepted.source(), "appearance-hue")
                .unwrap();
        assert_eq!(control_source["__nativeId"], control_native);
        assert_eq!(control_source["__handlerSlots"]["action"], control_handler);
        assert!(matches!(
            host.application_mut().take_effects().as_slice(),
            [PluginEffect::Appearance { .. }]
        ));

        let mut accepted = host.application_mut().accepted.source().clone();
        let (oracle, oracle_composition) =
            settings_admission_application("nickel-default/appearance").unwrap();
        let mut oracle = oracle.accepted.source().clone();
        canonicalize_settings_actions(&mut accepted);
        canonicalize_settings_actions(&mut oracle);
        assert_eq!(
            accepted, oracle,
            "incremental Settings diverged from cold admission"
        );
        drop(oracle_composition);

        let after = settings_profile_totals(&runtime.borrow_mut().runtime_diagnostics().unwrap());
        let patch_operations = host.application_mut().diagnostic_patch_operations;
        let patch_transport_bytes = host.application_mut().diagnostic_patch_transport_bytes;
        let patch_counters = host.application_mut().diagnostic_patch_counters;
        let close_started = std::time::Instant::now();
        host.application_mut().retire_surface().unwrap();
        let close = close_started.elapsed();
        assert_eq!(composition.borrow().mount_count(), 0);
        let work = SettingsAdmissionWork {
            cold_tree_bytes: before[0],
            patch_bytes: patch_transport_bytes,
            patch_operations,
            patch_nodes_visited: patch_counters.nodes_visited,
            patch_nodes_mutated: patch_counters.nodes_mutated,
            patch_local_materializations: patch_counters.local_materializations,
            patch_expansion_nodes: patch_counters.expansion_nodes,
            patch_complete_tree_bytes: patch_counters.tree_bytes,
            component_executions: after[4] - before[4],
            typed_apply_attempts: after[5] - before[5],
            typed_apply_rejections: after[6] - before[6],
            mounts_open: mount_count,
            mounts_closed: composition.borrow().mount_count(),
        };
        let total = total_started.elapsed();
        (work, [open, page_switch, control_edit, close, total])
    }

    #[test]
    fn settings_navigation_survives_surface_authority_reconciliation() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(settings_navigation_survives_surface_authority_reconciliation_inner)
            .unwrap()
            .join()
            .unwrap();
    }

    fn settings_navigation_survives_surface_authority_reconciliation_inner() {
        let (catalog, surface, composition) = settings_admission_runtime().unwrap();
        let application = settings_admission_application_with_runtime(
            "nickel-default/plugins",
            &catalog,
            &surface,
            composition,
        )
        .unwrap();
        let mut host = nickel_ui::UiHost::new(application, 900, 600);
        let navigation = serde_json::json!({
            "revision": "live",
            "destination": "nickel-default/plugins",
        });
        assert!(
            host.application_mut()
                .sync_host_data_fields(&[("navigation", &navigation)])
                .unwrap()
        );
        assert!(
            host.application_mut()
                .sync_surface_geometry(Some("nested"), Some((780.0, 580.0)), Some(1.0), Some(true))
                .unwrap()
        );
        assert!(
            host.application_mut()
                .reconcile_surface_authority()
                .unwrap()
        );
        assert!(host.application_mut().sync_surface_focus(true).unwrap());
        host.application_mut()
            .reconcile_surface_focus_authority()
            .unwrap();
        for tick in 0..64 {
            let clock = serde_json::json!({"tick": tick});
            assert!(
                host.application_mut()
                    .sync_host_data_fields(&[("clock", &clock)])
                    .unwrap()
            );
        }
        // Native owned-value publication and the legacy serialized entry point
        // must share the same committed snapshot and unchanged-update behavior.
        let snapshot = host.application().projection_value.clone().unwrap();
        let serialized = snapshot.to_string();
        assert_eq!(
            host.application().projection_data.as_deref(),
            Some(serialized.as_str())
        );
        assert!(
            !host
                .application_mut()
                .sync_serialized_data(serialized)
                .unwrap()
        );
        assert!(!host.application_mut().sync_data(&snapshot).unwrap());
        assert!(
            !host
                .application_mut()
                .sync_host_data_fields(&[
                    ("navigation", &navigation),
                    ("clock", &snapshot["clock"])
                ])
                .unwrap()
        );
        assert!(
            host.application_mut()
                .sync_host_data_fields(&[("not-a-host-field", &serde_json::Value::Null)])
                .is_err()
        );
        assert_eq!(
            host.application().projection_value.as_ref(),
            Some(&snapshot)
        );
        assert!(
            host.application_mut().refresh_settings_render().unwrap(),
            "forced sibling refresh must not take the unchanged fast path"
        );
        let appearance = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Appearance".into(),
            })
            .unwrap();
        let point = nickel_ui::Point {
            x: appearance.bounds.origin.x + appearance.bounds.size.width / 2.0,
            y: appearance.bounds.origin.y + appearance.bounds.size.height / 2.0,
        };
        host.step(nickel_ui::HostBatch {
            events: vec![
                nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::PointerMoved(point)),
                nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::PointerPressed(point)),
            ],
            ..Default::default()
        });
        let clock = serde_json::json!({"tick": 65});
        assert!(
            host.application_mut()
                .sync_host_data_fields(&[("clock", &clock)])
                .unwrap()
        );
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Ui(
                nickel_ui::UiEvent::PointerReleased(point),
            )],
            ..Default::default()
        });
        assert!(host.application_mut().last_error().is_none());
        assert!(
            host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Slider,
                name: "Interface hue".into(),
            })
            .is_ok()
        );
    }

    #[test]
    #[ignore = "production-sized release-profile Settings admission workload"]
    fn production_settings_lifecycle_emits_release_distribution() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                let warmup = std::env::var("NICKEL_SETTINGS_ADMISSION_WARMUP")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(3);
                let iterations = std::env::var("NICKEL_SETTINGS_ADMISSION_ITERATIONS")
                    .ok()
                    .and_then(|value| value.parse().ok())
                    .unwrap_or(20);
                assert!(iterations > 0);
                for _ in 0..warmup {
                    exercise_production_settings_admission();
                }
                let mut samples = SettingsAdmissionSamples::default();
                for _ in 0..iterations {
                    let (work, timings) = exercise_production_settings_admission();
                    if let Some(expected) = samples.exact {
                        assert_eq!(work, expected, "deterministic Settings work changed");
                    } else {
                        samples.exact = Some(work);
                    }
                    samples.open.push(timings[0]);
                    samples.page_switch.push(timings[1]);
                    samples.control_edit.push(timings[2]);
                    samples.close.push(timings[3]);
                    samples.total.push(timings[4]);
                }
                let work = samples.exact.unwrap();
                let report = serde_json::json!({
                    "schema": 1,
                    "suite": "jsx_incremental",
                    "workload": "production_settings_lifecycle",
                    "metadata": {"iterations":iterations,"warmupIterations":warmup,"surface":"nickel-default/settings","page":"nickel-default/appearance"},
                    "work": {
                        "coldTreeTransportBytesPerIteration":work.cold_tree_bytes,
                        "patchEnvelopeTransportBytesPerIteration":work.patch_bytes,
                        "patchOperationsPerIteration":work.patch_operations,
                        "patchNodesVisitedPerIteration":work.patch_nodes_visited,
                        "patchNodesMutatedPerIteration":work.patch_nodes_mutated,
                        "patchLocalMaterializationsPerIteration":work.patch_local_materializations,
                        "patchExpansionNodesPerIteration":work.patch_expansion_nodes,
                        "patchCompleteTreeBytesPerIteration":work.patch_complete_tree_bytes,
                        "componentExecutionsPerIteration":work.component_executions,
                        "typedPatchApplyAttemptsPerIteration":work.typed_apply_attempts,
                        "typedPatchApplyRejectionsPerIteration":work.typed_apply_rejections,
                        "mountsWhileOpen":work.mounts_open,"mountsAfterClose":work.mounts_closed,
                    },
                    "timings": {
                        "open":settings_admission_distribution(&samples.open),
                        "pageSwitch":settings_admission_distribution(&samples.page_switch),
                        "controlEdit":settings_admission_distribution(&samples.control_edit),
                        "close":settings_admission_distribution(&samples.close),
                        "total":settings_admission_distribution(&samples.total),
                    }
                });
                eprintln!("nickel_release_admission={report}");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    #[ignore = "release-profile real Settings same-target pointer workload"]
    fn production_settings_pointer_motion_emits_release_distribution() {
        std::thread::Builder::new()
            .stack_size(32 * 1024 * 1024)
            .spawn(|| {
                for (page, wallpaper_count, target) in [
                    ("keyboard-shortcuts", 0, "Keyboard shortcuts"),
                    ("appearance", 0, "Light"),
                    ("appearance", 128, "Light"),
                ] {
                    let (mut application, _composition) =
                        settings_admission_application(&format!("nickel-default/{page}")).unwrap();
                    let wallpaper = serde_json::json!({
                        "available":true,"writable":true,"generation":2,
                        "configured":{"custom_image_configured":false,"position":"fill"},
                        "images":(0..wallpaper_count).map(|index| serde_json::json!({
                            "id":format!("fixture-{index}"),"label":format!("Wallpaper {index}"),
                            "configured":false,
                        })).collect::<Vec<_>>(),
                        "chooser":{"available":true,"pending":false,"result":null},
                    });
                    application.sync_host_data_fields(&[
                        ("wallpaper", &wallpaper),
                        ("shortcuts", &crate::shortcut_capabilities::snapshot(false, None)),
                    ]).unwrap();
                    let mut host = nickel_ui::UiHost::new(application, 1100, 800);
                    let target = host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button, name: target.into(),
                    }).unwrap();
                    let center = Point {
                        x: target.bounds.origin.x + target.bounds.size.width / 2.0,
                        y: target.bounds.origin.y + target.bounds.size.height / 2.0,
                    };
                    let motion = |host: &mut nickel_ui::UiHost<PluginPanelApplication>, x| {
                        crate::live_shell::step_plugin_host(host, None, nickel_ui::HostBatch {
                            events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::PointerMoved(Point { x, ..center }))],
                            ..Default::default()
                        }).unwrap()
                    };
                    for _ in 0..16 { motion(&mut host, center.x); }
                    assert_eq!(host.inspect().pointer_hover.as_ref(), Some(&target.id));
                    let generation = host.resolved_frame_generation();
                    let runtime = host.application_mut().shared_runtime();
                    let before = settings_profile_totals(&runtime.borrow_mut().runtime_diagnostics().unwrap());
                    let mut samples = Vec::with_capacity(512);
                    let mut allocation_operations = 0;
                    let mut requested_bytes = 0;
                    let mut bytes = None;
                    let started = Instant::now();
                    for index in 0..512 {
                        let allocations_before = crate::allocation_counter::thread_allocation_operations();
                        let bytes_before = crate::allocation_counter::thread_requested_bytes();
                        let started = Instant::now();
                        let (outcome, retained) = motion(&mut host, center.x + (index % 2) as f32);
                        samples.push(started.elapsed());
                        allocation_operations += crate::allocation_counter::thread_allocation_operations() - allocations_before;
                        requested_bytes += crate::allocation_counter::thread_requested_bytes() - bytes_before;
                        assert!(!outcome.changed);
                        assert_eq!(outcome.telemetry.view_calls, 0);
                        assert_eq!(outcome.telemetry.nodes_measured, 0);
                        assert_eq!(outcome.telemetry.nodes_placed, 0);
                        assert_eq!(outcome.telemetry.semantic_nodes_rebuilt, 0);
                        assert_eq!(outcome.telemetry.paint_commands_emitted, 0);
                        assert_eq!(outcome.telemetry.retained_paint_refreshes, 0);
                        assert_eq!(host.resolved_frame_generation(), generation);
                        if let Some(bytes) = bytes { assert_eq!(retained, bytes); }
                        bytes = Some(retained);
                    }
                    let duration = started.elapsed();
                    let after = settings_profile_totals(&runtime.borrow_mut().runtime_diagnostics().unwrap());
                    assert_eq!(after, before, "mouse movement executed JSX or patch transport");
                    assert!(host.application().last_error().is_none());
                    let report = serde_json::json!({
                        "schema":1,"suite":"settings_interaction","workload":"same_target_motion",
                        "metadata":{"page":page,"wallpapers":wallpaper_count,"previewAssets":0,
                            "viewport":[1100,800],"scale":1,"samples":samples.len(),"warmup":16,
                            "backend":"native-host-headless","release":!cfg!(debug_assertions),
                            "sampleDurationNs":duration.as_nanos() as u64},
                        "work":{"viewCalls":0,"nodesMeasured":0,"nodesPlaced":0,
                            "semanticNodesRebuilt":0,"paintCommandsEmitted":0,
                            "jsxProfileDelta":after.iter().zip(before).map(|(a,b)|a-b).collect::<Vec<_>>()},
                        "retainedFrameAndImageBytes":bytes,
                        "allocationVolume":{"scope":"thread Rust System requests; includes event transport, excludes V8 allocator and live memory",
                            "operations":allocation_operations,"requestedBytes":requested_bytes},
                        "timings":settings_admission_distribution(&samples),
                    });
                    eprintln!("nickel_release_admission={report}");
                    let mut samples = Vec::with_capacity(128);
                    let mut allocation_operations = 0;
                    let mut requested_bytes = 0;
                    let mut max_fragments = 0;
                    let mut max_commands = 0;
                    for index in 0..128 {
                        let allocations_before = crate::allocation_counter::thread_allocation_operations();
                        let bytes_before = crate::allocation_counter::thread_requested_bytes();
                        let started = Instant::now();
                        let (outcome, _) = motion(&mut host, if index % 2 == 0 { -1.0 } else { center.x });
                        samples.push(started.elapsed());
                        allocation_operations += crate::allocation_counter::thread_allocation_operations() - allocations_before;
                        requested_bytes += crate::allocation_counter::thread_requested_bytes() - bytes_before;
                        assert!(outcome.changed);
                        assert_eq!(outcome.telemetry.view_calls, 0, "{page}: hover rebuilt application");
                        assert_eq!(outcome.telemetry.nodes_measured, 0);
                        assert_eq!(outcome.telemetry.nodes_placed, 0);
                        assert_eq!(outcome.telemetry.semantic_nodes_rebuilt, 0);
                        assert_eq!(outcome.telemetry.retained_paint_refreshes, 1);
                        assert_eq!(outcome.telemetry.paint_interaction_records_saved, 2);
                        max_fragments = max_fragments.max(outcome.telemetry.paint_fragments_rebuilt);
                        max_commands = max_commands.max(outcome.telemetry.paint_commands_emitted);
                    }
                    assert_eq!(settings_profile_totals(&runtime.borrow_mut().runtime_diagnostics().unwrap()), before);
                    let report = serde_json::json!({
                        "schema":1,"suite":"settings_interaction","workload":"hover_entry_exit",
                        "metadata":{"page":page,"wallpapers":wallpaper_count,"previewAssets":0,
                            "viewport":[1100,800],"scale":1,"samples":samples.len(),
                            "backend":"native-host-headless","release":!cfg!(debug_assertions)},
                        "work":{"viewCalls":0,"nodesMeasured":0,"nodesPlaced":0,"semanticNodesRebuilt":0,
                            "maxPaintFragmentsRebuilt":max_fragments,"maxPaintCommandsEmitted":max_commands,
                            "interactionRecordsSavedPerTransition":2},
                        "allocationVolume":{"scope":"thread Rust System requests; includes event transport, excludes V8 allocator and live memory",
                            "operations":allocation_operations,"requestedBytes":requested_bytes},
                        "timings":settings_admission_distribution(&samples),
                    });
                    eprintln!("nickel_release_admission={report}");
                }
            }).unwrap().join().unwrap();
    }

    #[test]
    #[ignore = "release-profile long-lived Settings virtual traversal workload"]
    fn production_settings_traversal_reports_retained_lifetimes() {
        with_package_runtime_stack(|| {
            fn settle_previews(
                host: &mut nickel_ui::UiHost<PluginPanelApplication>,
                previews: &mut crate::wallpaper_previews::WallpaperPreviews,
                key: &nickel_core::plugins::PluginSurfaceKey,
                outcome: &mut nickel_ui::HostEventOutcome,
                retained: &mut u64,
            ) {
                let timeout = Instant::now() + std::time::Duration::from_secs(30);
                loop {
                    assert!(
                        Instant::now() < timeout,
                        "preview/geometry convergence timed out"
                    );
                    let size = host.render_frame().logical_size;
                    let viewport = nickel_ui::Rect::new(0.0, 0.0, size.0 as f32, size.1 as f32);
                    let demand = host
                        .application()
                        .wallpaper_preview_demand(host.resolved_layout(), viewport);
                    previews.sync_surfaces(std::iter::once((key, demand)), Instant::now());
                    while previews.next_deadline().is_some() {
                        assert!(Instant::now() < timeout, "preview decoding timed out");
                        previews.poll(Instant::now());
                        if previews.next_deadline().is_some() {
                            std::thread::sleep(std::time::Duration::from_millis(1));
                        }
                    }
                    if !host
                        .application_mut()
                        .sync_application_images(previews.images().clone())
                    {
                        break;
                    }
                    let (next, bytes) = step_host(
                        host,
                        None,
                        nickel_ui::HostBatch {
                            application_changed: true,
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    outcome.merge(next);
                    *retained = bytes;
                }
                // This fixture's 800-pixel viewport can intersect at most ten
                // 90-pixel image rectangles. This is an assertion, not admission.
                assert!(previews.images().len() <= 10);
            }
            fn native_actions(value: &Value, actions: &mut std::collections::BTreeSet<u64>) {
                match value {
                    Value::Object(object) => {
                        if let Some(slots) = object.get("__handlerSlots").and_then(Value::as_object)
                        {
                            for name in slots.keys() {
                                actions.insert(object[name].as_u64().unwrap());
                            }
                        }
                        for value in object.values() {
                            native_actions(value, actions);
                        }
                    }
                    Value::Array(values) => {
                        for value in values {
                            native_actions(value, actions);
                        }
                    }
                    _ => {}
                }
            }
            let (mut application, composition) =
                settings_admission_application("nickel-default/appearance").unwrap();
            let previews_enabled =
                std::env::var("NICKEL_SETTINGS_TRAVERSAL_PREVIEWS").is_ok_and(|value| value == "1");
            let fixture_directory = previews_enabled.then(|| tempfile::tempdir().unwrap());
            let preview_catalog = fixture_directory.as_ref().map(|directory| {
                for index in 0..128u8 {
                    image::RgbaImage::from_pixel(800, 450, image::Rgba([index, 80, 160, 255]))
                        .save(directory.path().join(format!("wallpaper-{index:03}.png")))
                        .unwrap();
                }
                crate::wallpaper_selection::Catalog::discover_fixture(directory.path())
            });
            let wallpaper_ids: Vec<String> = preview_catalog.as_ref().map_or_else(
                || (0..128).map(|index| format!("fixture-{index}")).collect(),
                |catalog| {
                    catalog
                        .choices(None)
                        .into_iter()
                        .map(|choice| choice.id)
                        .collect()
                },
            );
            let mut previews = crate::wallpaper_previews::WallpaperPreviews::default();
            if let Some(catalog) = preview_catalog {
                previews.set_catalog(catalog);
            }
            let preview_key = nickel_core::plugins::PluginSurfaceKey {
                plugin_id: "nickel-default".into(),
                surface_id: "settings".into(),
            };
            let wallpaper = serde_json::json!({
                "available":true,"writable":true,"generation":2,
                "configured":{"custom_image_configured":false,"position":"fill"},
                "images":wallpaper_ids.iter().enumerate().map(|(index, id)| {
                    let mut image = serde_json::json!({"id":id,"label":format!("Wallpaper {index}"),"configured":false});
                    if previews_enabled { image["previewAsset"] = format!("wallpaper:{id}").into(); }
                    image
                }).collect::<Vec<_>>(),
                "chooser":{"available":true,"pending":false,"result":null}
            });
            application
                .sync_host_data_fields(&[
                    ("wallpaper", &wallpaper),
                    (
                        "shortcuts",
                        &crate::shortcut_capabilities::snapshot(false, None),
                    ),
                ])
                .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            let mut reports = Vec::new();
            let cycles = std::env::var("NICKEL_SETTINGS_TRAVERSAL_CYCLES")
                .map(|value| {
                    value
                        .parse::<usize>()
                        .expect("integer traversal cycle count")
                })
                .unwrap_or(24);
            assert!(
                (24..=8192).contains(&cycles),
                "traversal fixture cycles must be 24..=8192"
            );
            let snapshot_directory = std::env::var_os("NICKEL_SETTINGS_TRAVERSAL_HEAP_SNAPSHOTS")
                .map(std::path::PathBuf::from);
            let pump_platform_tasks = std::env::var_os("NICKEL_SETTINGS_TRAVERSAL_PUMP_TASKS")
                .is_some_and(|value| value == "1");
            let host_maintenance = std::env::var_os("NICKEL_SETTINGS_TRAVERSAL_HOST_MAINTENANCE")
                .is_some_and(|value| value == "1");
            assert!(!(pump_platform_tasks && host_maintenance));
            let exit_before_teardown =
                std::env::var_os("NICKEL_SETTINGS_TRAVERSAL_EXIT_BEFORE_TEARDOWN")
                    .is_some_and(|value| value == "1");
            if exit_before_teardown {
                assert!(
                    std::env::args().any(|argument| argument == "--exact"),
                    "pre-teardown attribution requires an isolated exact test invocation"
                );
            }
            if let Some(directory) = &snapshot_directory {
                assert!(
                    directory.is_dir(),
                    "heap snapshot directory must already exist"
                );
                assert!(
                    cycles >= 256,
                    "heap attribution needs a warm baseline and later snapshot"
                );
            }
            let mut steady_compilations = None;
            for cycle in 0..cycles {
                let mut phases = Vec::with_capacity(6);
                let started = Instant::now();
                let allocations_before = crate::allocation_counter::thread_allocation_operations();
                let bytes_before = crate::allocation_counter::thread_requested_bytes();
                let mut peak_frame = 0;
                let mut peak_preview_pixels = 0usize;
                for delta in [100_000.0, -100_000.0, 100_000.0, -100_000.0] {
                    let phase_started = Instant::now();
                    let phase_operations =
                        crate::allocation_counter::thread_allocation_operations();
                    let phase_bytes = crate::allocation_counter::thread_requested_bytes();
                    let (mut outcome, mut retained) = step_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                                point: Point { x: 700.0, y: 600.0 },
                                delta_y: delta,
                            })],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    if previews_enabled {
                        settle_previews(
                            &mut host,
                            &mut previews,
                            &preview_key,
                            &mut outcome,
                            &mut retained,
                        );
                        peak_preview_pixels = peak_preview_pixels.max(
                            previews
                                .images()
                                .values()
                                .map(|(_, image)| image.len())
                                .sum(),
                        );
                    }
                    let elapsed = phase_started.elapsed();
                    let operations = crate::allocation_counter::thread_allocation_operations()
                        - phase_operations;
                    let requested =
                        crate::allocation_counter::thread_requested_bytes() - phase_bytes;
                    phases.push(serde_json::json!({"action":if delta>0.0 {"scroll-end"} else {"scroll-start"},
                        "elapsedNs":elapsed.as_nanos() as u64,"allocationOperations":operations,"requestedBytes":requested,
                        "viewCalls":outcome.telemetry.view_calls,"nodesMeasured":outcome.telemetry.nodes_measured,
                        "nodesPlaced":outcome.telemetry.nodes_placed,"paintCommands":outcome.telemetry.paint_commands_emitted}));
                    assert!(!host.application().virtual_work_pending);
                    peak_frame = peak_frame.max(retained);
                    let (sources, rows, _) = host.application().accepted.virtual_source_usage();
                    assert_eq!((sources, rows), (1, 128));
                    assert!(host.resolved_layout().nodes().len() < 500);
                    if delta > 0.0 {
                        assert!(
                            host.application()
                                .button_message(&format!(
                                    "appearance-wallpaper-{}",
                                    wallpaper_ids[127]
                                ))
                                .is_some()
                        );
                        assert!(
                            host.application()
                                .button_message(&format!(
                                    "appearance-wallpaper-{}",
                                    wallpaper_ids[0]
                                ))
                                .is_none()
                        );
                    }
                }
                let mut checkpoints = Vec::new();
                for page in ["Keyboard shortcuts", "Appearance"] {
                    let destination = if page == "Appearance" {
                        "appearance"
                    } else {
                        "keyboard-shortcuts"
                    };
                    let token = host
                        .application()
                        .accepted
                        .node()
                        .button_action(&format!(
                            "settings-navigation/destination/nickel-default/{destination}"
                        ))
                        .unwrap();
                    let binding = host
                        .application()
                        .composition
                        .as_ref()
                        .unwrap()
                        .events
                        .get(&(token as u64));
                    assert!(
                        binding.is_some(),
                        "cycle {cycle} {page}: live navigation token {token} has no callback authority"
                    );
                    let navigation = host
                        .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                            role: SemanticRole::Button,
                            name: page.into(),
                        })
                        .unwrap();
                    let phase_started = Instant::now();
                    let phase_operations =
                        crate::allocation_counter::thread_allocation_operations();
                    let phase_bytes = crate::allocation_counter::thread_requested_bytes();
                    let mut action = host.perform_semantic_action(
                        navigation.id,
                        nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                    );
                    assert!(
                        action.semantic_failures.is_empty(),
                        "cycle {cycle} {page}: {:?}",
                        action.semantic_failures
                    );
                    assert!(
                        action.changed,
                        "cycle {cycle} {page}: navigation did not change the host"
                    );
                    let (followup, mut retained) =
                        step_host(&mut host, None, Default::default()).unwrap();
                    action.merge(followup);
                    if previews_enabled {
                        settle_previews(
                            &mut host,
                            &mut previews,
                            &preview_key,
                            &mut action,
                            &mut retained,
                        );
                    }
                    let elapsed = phase_started.elapsed();
                    let operations = crate::allocation_counter::thread_allocation_operations()
                        - phase_operations;
                    let requested =
                        crate::allocation_counter::thread_requested_bytes() - phase_bytes;
                    phases.push(serde_json::json!({"action":page,"elapsedNs":elapsed.as_nanos() as u64,
                        "allocationOperations":operations,"requestedBytes":requested,"viewCalls":action.telemetry.view_calls,
                        "nodesMeasured":action.telemetry.nodes_measured,"nodesPlaced":action.telemetry.nodes_placed,
                        "paintCommands":action.telemetry.paint_commands_emitted}));
                    assert!(
                        host.application().last_error().is_none(),
                        "cycle {cycle} page {page}: {:?}",
                        host.application().last_error()
                    );
                    assert!(!host.application().virtual_work_pending);
                    let catalog = host.application().accepted.virtual_source_usage();
                    let mut live_actions = std::collections::BTreeSet::new();
                    native_actions(host.application().accepted.source(), &mut live_actions);
                    let admitted_actions = host
                        .application()
                        .composition
                        .as_ref()
                        .unwrap()
                        .events
                        .keys()
                        .copied()
                        .collect();
                    assert_eq!(
                        live_actions, admitted_actions,
                        "cycle {cycle} {page}: callback table differs from live native actions"
                    );
                    if page == "Keyboard shortcuts" {
                        assert_eq!(catalog, (0, 0, 0));
                        assert!(previews.images().is_empty());
                        assert!(previews.next_deadline().is_none());
                        assert!(!host.application().has_wallpaper_images());
                    } else {
                        assert_eq!(
                            (catalog.0, catalog.1),
                            (1, 128),
                            "cycle {cycle}: Appearance navigation did not restore its source"
                        );
                    }
                    checkpoints.push(serde_json::json!({"page":page,"frameAndImageBytes":retained,
                        "previewImages":previews.images().len(),
                        "previewPixelBytes":previews.images().values().map(|(_, image)| image.len()).sum::<usize>(),
                        "catalogSources":catalog.0,"catalogRows":catalog.1,"catalogPayloadBytes":catalog.2,
                        "mounts":composition.borrow().mount_count(),
                        "eventBindings":host.application().composition.as_ref().unwrap().events.len()}));
                }
                let elapsed = started.elapsed();
                let operations =
                    crate::allocation_counter::thread_allocation_operations() - allocations_before;
                let requested = crate::allocation_counter::thread_requested_bytes() - bytes_before;
                let runtime = host.application_mut().shared_runtime();
                let (pumped_tasks, platform_task_ns) = if host_maintenance {
                    let before = runtime.borrow_mut().runtime_diagnostics().unwrap()["engine"]
                        ["platformTasksServiced"].as_u64().unwrap();
                    let task_started = Instant::now();
                    assert!(
                        !host
                            .application_mut()
                            .service_idle_platform_tasks()
                            .unwrap(),
                        "GC-only maintenance must not change the accepted UI"
                    );
                    let elapsed = task_started.elapsed().as_nanos() as u64;
                    let after = runtime.borrow_mut().runtime_diagnostics().unwrap()["engine"]
                        ["platformTasksServiced"].as_u64().unwrap();
                    ((after - before) as usize, elapsed)
                } else if pump_platform_tasks {
                    let task_started = Instant::now();
                    let count = runtime
                        .borrow_mut()
                        .diagnostic_pump_platform_tasks()
                        .unwrap();
                    (count, task_started.elapsed().as_nanos() as u64)
                } else {
                    (0, 0)
                };
                let diagnostics = runtime.borrow_mut().runtime_diagnostics().unwrap();
                if cycle >= 4 {
                    let compilations = diagnostics["engine"]["scriptCompilations"]
                        .as_u64()
                        .unwrap();
                    assert_eq!(
                        *steady_compilations.get_or_insert(compilations),
                        compilations,
                        "unchanged loaded packages must not compile call wrappers during traversal"
                    );
                }
                // Sample outside the measured cycle. This includes the entire
                // test process (V8, allocator retention and report storage),
                // unlike the native frame resource accounting above.
                #[cfg(target_os = "linux")]
                let process_memory = {
                    // Observation only: do not trim or force collection to
                    // manufacture a plateau. Include allocator live/free space
                    // so RSS growth alone is not mislabeled as object leakage.
                    let memory = crate::process_memory::trim_snapshot();
                    serde_json::json!({"rssBytes":memory.process_rss_bytes,
                        "anonymousRssBytes":memory.process_anonymous_bytes,
                        "privateBytes":memory.process_private_bytes,
                        "allocatorArenaBytes":memory.allocator_arena_bytes,
                        "allocatorMmapBytes":memory.allocator_mmap_bytes,
                        "allocatorLiveBytes":memory.allocator_live_bytes,
                        "allocatorFreeBytes":memory.allocator_free_bytes,
                        "allocatorReleasableBytes":memory.allocator_releasable_bytes})
                };
                #[cfg(not(target_os = "linux"))]
                let process_memory = Value::Null;
                let rust_live_requested = crate::allocation_counter::live_requested_bytes();
                let report = serde_json::json!({"cycle":cycle,"warmup":cycle<4,"elapsedNs":elapsed.as_nanos() as u64,
                    "rustLiveRequestedBytes":rust_live_requested,
                    "rustLiveAccounting":true,
                    "diagnosticHeapSnapshots":snapshot_directory.is_some(),
                    "diagnosticPlatformTasks":pump_platform_tasks,"pumpedPlatformTasks":pumped_tasks,
                    "diagnosticHostMaintenance":host_maintenance,
                    "platformTaskNs":platform_task_ns,
                    "peakPreviewPixelBytes":peak_preview_pixels,
                    "allocationOperations":operations,"requestedBytes":requested,"peakFrameAndImageBytes":peak_frame,
                    "checkpoints":checkpoints,"phases":phases,"engine":diagnostics["engine"],
                    "runtimeRetention":diagnostics["retained"],"runtimeCounters":diagnostics["counters"],"processMemory":process_memory});
                if cycles > 24 {
                    // Long memory runs stream outside the measured interval so
                    // retaining benchmark reports cannot create a false leak.
                    eprintln!("nickel_settings_traversal_cycle={report}");
                } else {
                    reports.push(report);
                }
                if let Some(directory) = &snapshot_directory
                    && (cycle == 128 || cycle + 1 == cycles)
                {
                    use std::io::Write;
                    // Synthetic fixture only. V8 may collect during snapshots;
                    // keep this explicitly separate from normal plateau runs.
                    let snapshot = runtime.borrow_mut().diagnostic_heap_snapshot().unwrap();
                    let path = directory.join(format!("cycle-{cycle:05}.heapsnapshot"));
                    let mut file = std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        .unwrap();
                    file.write_all(&snapshot).unwrap();
                    eprintln!("diagnostic_heap_snapshot={}", path.display());
                }
            }
            eprintln!(
                "nickel_settings_traversal={}",
                serde_json::json!({
                    "backend":"native-host-headless","viewport":[1100,800],"scale":1,
                    "wallpapers":128,"previewAssets":if previews_enabled {128} else {0},"release":!cfg!(debug_assertions),
                    "previewSourcePixels":if previews_enabled {serde_json::json!([800,450])} else {Value::Null},
                    "previewTimingScope":"headless phases await native worker completion; includes deadline waits, not compositor latency; thread allocations exclude the worker",
                    "cycleCount":cycles,"streamedCycles":cycles>24,"cycles":reports,
                    "diagnosticHeapSnapshots":snapshot_directory.is_some(),
                    "diagnosticPlatformTasks":pump_platform_tasks,
                    "diagnosticHostMaintenance":host_maintenance,
                    "diagnosticExitBeforeTeardown":exit_before_teardown,
                    "rustLiveAccounting":true
                })
            );
            if exit_before_teardown {
                // Profiler attribution only: retain the live host/isolate so
                // process-exit allocation stacks describe this boundary rather
                // than V8 teardown caches. This intentionally bypasses the test
                // harness result and is never a test-pass or plateau claim.
                // Remove only the fixture's own temporary image directory.
                drop(fixture_directory);
                eprintln!(
                    "diagnostic_exit_before_teardown=true; test completion intentionally bypassed"
                );
                std::process::exit(0);
            }
            host.application_mut().retire_surface().unwrap();
            assert_eq!(composition.borrow().mount_count(), 0);
        });
    }

    #[test]
    fn production_settings_initial_size_matches_manifest_before_output_observation() {
        with_package_runtime_stack(|| {
            let (mut application, _composition) =
                settings_admission_application("nickel-default/appearance").unwrap();
            let manifest = crate::bundled_plugin_assets::load_package("nickel-default")
                .unwrap()
                .manifest;
            let surface = manifest
                .surfaces
                .iter()
                .find(|surface| surface.id == "settings")
                .unwrap();
            application
                .sync_host_data_fields(&[(
                    "displays",
                    &serde_json::json!({"available":false,"outputs":[]}),
                )])
                .unwrap();
            let window = find_settings_source(application.accepted.source(), "settings").unwrap();
            assert_eq!(window["width"], surface.width);
            assert_eq!(window["height"], surface.height);
            application
                .sync_host_data_fields(&[(
                    "displays",
                    &serde_json::json!({"available":true,"outputs":[{
                        "primary":true,"geometry":{"x":0,"y":0,"width":1200,"height":768},
                        "work_area":{"x":0,"y":0,"width":1200,"height":712}
                    }]}),
                )])
                .unwrap();
            let window = find_settings_source(application.accepted.source(), "settings").unwrap();
            assert_eq!(window["width"], 1100);
            assert_eq!(window["height"], 656);
        });
    }

    #[test]
    fn production_settings_associations_bound_cards_and_handler_rows() {
        with_package_runtime_stack(|| {
            let settle = |host: &mut nickel_ui::UiHost<PluginPanelApplication>| {
                for _ in 0..16 {
                    if !host.application().virtual_work_pending {
                        return;
                    }
                    let now = host
                        .next_deadline()
                        .expect("virtual continuation must schedule its native deadline");
                    crate::live_shell::step_plugin_host(
                        host,
                        None,
                        nickel_ui::HostBatch {
                            now: Some(now),
                            events: vec![nickel_ui::HostEvent::Poll],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    assert!(host.resolved_layout().nodes().len() < 600);
                }
                assert!(
                    !host.application().virtual_work_pending,
                    "nested sources failed to converge"
                );
            };
            let (mut application, _composition) =
                settings_admission_application("nickel-default/default-apps").unwrap();
            let catalog = serde_json::json!({"available":true,"writable":true,"revision":"7",
                "operations":{"setDefault":true,"openSystemSettings":true},
                "targets":(0..128).map(|target| serde_json::json!({
                    "id":format!("mime:text/x-{target}"),"family":if target % 2 == 0 {"Documents"} else {"Links"},"canSetDefault":true,
                    "effectiveHandlerId":"handler-0","protected":false,
                    "handlers":(0..128).map(|handler| serde_json::json!({
                        "id":format!("handler-{handler}"),"name":format!("Handler {target}/{handler}"),"protected":false,
                    })).collect::<Vec<_>>()
                })).collect::<Vec<_>>()
            });
            application
                .sync_host_data_fields(&[("associations", &catalog)])
                .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            settle(&mut host);
            assert!(!host.application().virtual_work_pending);
            assert_eq!(
                logical_source(host.application().accepted.node())
                    .unwrap()
                    .len(),
                128
            );
            let bounded = |host: &nickel_ui::UiHost<PluginPanelApplication>| {
                let rows = host
                    .application()
                    .accepted
                    .node()
                    .virtual_collection_measurements(host.resolved_layout())
                    .unwrap();
                assert!(rows.len() <= 4, "too many association cards materialized");
                assert!(
                    rows.iter().map(|source| source.rows.len()).sum::<usize>() < 50,
                    "offscreen handler rows materialized"
                );
                assert!(host.resolved_layout().nodes().len() < 600);
            };
            bounded(&host);
            let last = nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Handler 127/127".into(),
            };
            assert!(host.query_unique(&last).is_err());
            crate::live_shell::step_plugin_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                        point: Point { x: 700.0, y: 600.0 },
                        delta_y: 1_000_000.0,
                    })],
                    ..Default::default()
                },
            )
            .unwrap();
            settle(&mut host);
            assert!(!host.application().virtual_work_pending);
            bounded(&host);
            let last = host
                .query_unique(&last)
                .expect("last handler in last association reachable");
            host.perform_semantic_action(
                last.id,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert!(
                host.application().last_error().is_none(),
                "{:?}",
                host.application().last_error()
            );
            let effects = host.application_mut().take_effects();
            let expected = crate::associations_capabilities::AssociationsEffect::parse(&serde_json::json!({
                "type":"associations.setDefault","targetId":"mime:text/x-127","handlerId":"handler-127",
                "revision":"7","expectedHandlerId":"handler-0",
            })).unwrap();
            assert!(
                matches!(effects.as_slice(),[PluginEffect::Associations{effect,..}] if effect == &expected),
                "{effects:?}"
            );
            let search = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::TextField,
                    name: "Search default applications".into(),
                })
                .unwrap();
            host.request_focus(search.id.clone());
            for (query, count) in [("Handler 37/", 1), ("no matching handler", 0), ("", 128)] {
                host.perform_semantic_action(
                    search.id.clone(),
                    nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(
                        query.into(),
                    )),
                );
                crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
                settle(&mut host);
                assert_eq!(
                    logical_source(host.application().accepted.node())
                        .unwrap()
                        .len(),
                    count
                );
                bounded(&host);
                assert!(host.application().last_error().is_none());
            }
            let family = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Documents".into(),
                })
                .unwrap();
            host.perform_semantic_action(
                family.id,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            settle(&mut host);
            assert_eq!(
                logical_source(host.application().accepted.node())
                    .unwrap()
                    .len(),
                64
            );
            bounded(&host);
        });
    }

    #[test]
    fn production_settings_plugins_virtualize_and_preserve_editor_drafts() {
        with_package_runtime_stack(|| {
            let settle = |host: &mut nickel_ui::UiHost<PluginPanelApplication>, phase: &str| {
                for _ in 0..32 {
                    if !host.application().virtual_work_pending {
                        return;
                    }
                    let now = host.next_deadline().expect("native virtual continuation");
                    step_host(
                        host,
                        None,
                        nickel_ui::HostBatch {
                            now: Some(now),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
                panic!(
                    "plugin collections did not converge during {phase}: {:?}; {:?}",
                    host.application().last_error(),
                    host.application()
                        .accepted
                        .node()
                        .virtual_collection_measurements(host.resolved_layout())
                        .unwrap()
                );
            };
            let snapshot = serde_json::json!({
                "available":true,"writable":true,"revision":"7",
                "plugins":(0..256).map(|index| serde_json::json!({
                    "id":format!("fixture-{index}"),"name":format!("Plugin {index}"),
                    "enabled":true,"health":{"state":"running"},"grants":[],"surfaces":[],
                    "composition":[],"memory":{"jsHeapBytes":null,"nativeUiBytes":null,
                        "textureBytes":null,"trackedPeakBytes":null,"timers":0,"subscriptions":0},
                    "settings":[{"id":"value","label":format!("Value {index}"),
                        "kind":{"kind":"text","max_length":1024},"value":format!("Stored {index}")}]
                })).collect::<Vec<_>>()
            });
            let (mut application, _) =
                settings_admission_application("nickel-default/plugins").unwrap();
            application
                .sync_host_data_fields(&[("plugins", &snapshot)])
                .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            step_host(&mut host, None, Default::default()).unwrap();
            settle(&mut host, "initial mount");
            let editor = |index| nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::TextField,
                name: format!("Value {index}"),
            };
            let first = host.query_unique(&editor(0)).unwrap();
            assert!(host.query_unique(&editor(255)).is_err());
            host.perform_semantic_action(
                first.id,
                nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(
                    "Unapplied draft".into(),
                )),
            );
            step_host(&mut host, None, Default::default()).unwrap();
            settle(&mut host, "draft edit");
            assert!(host.application_mut().take_effects().is_empty());
            for (delta_y, visible, absent) in [(1_000_000.0, 255, 0), (-1_000_000.0, 0, 255)] {
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                            point: Point { x: 700.0, y: 600.0 },
                            delta_y,
                        })],
                        ..Default::default()
                    },
                )
                .unwrap();
                settle(&mut host, "scroll");
                assert!(host.query_unique(&editor(visible)).is_ok());
                assert!(host.query_unique(&editor(absent)).is_err());
                assert!(host.application().last_error().is_none());
                let rows = host
                    .application()
                    .accepted
                    .node()
                    .virtual_collection_measurements(host.resolved_layout())
                    .unwrap();
                assert!(rows.iter().map(|batch| batch.rows.len()).sum::<usize>() < 16);
                assert!(host.application().accepted.virtual_source_usage().1 >= 256);
                assert!(host.application_mut().take_effects().is_empty());
            }
            assert_eq!(
                find_settings_source(
                    host.application().accepted.source(),
                    "plugin-setting/fixture-0/value"
                )
                .unwrap()["value"],
                "Unapplied draft"
            );
            let apply = host
                .query(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Apply".into(),
                })
                .into_iter()
                .find(|node| {
                    node.id
                        .as_str()
                        .ends_with("plugin-setting/fixture-0/value/apply")
                })
                .unwrap();
            host.perform_semantic_action(
                apply.id,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            let expected =
                crate::plugins_capabilities::PluginsSettingEffect::parse(&serde_json::json!({
                    "type":"plugins.setSetting","id":"fixture-0","key":"value","revision":"7",
                    "priorValue":"Stored 0","value":"Unapplied draft"
                }))
                .unwrap();
            let effects = host.application_mut().take_effects();
            assert!(
                matches!(effects.as_slice(),
                [PluginEffect::PluginsSetting{effect,..}] if effect == &expected),
                "{effects:?}; {:?}",
                host.application().last_error()
            );
            // A single card can itself contain a long settings list. Keeping
            // cards bounded alone must not eagerly construct those editors.
            let mut nested = snapshot.clone();
            nested["plugins"].as_array_mut().unwrap().truncate(1);
            nested["plugins"][0]["settings"] =
                serde_json::json!((0..128).map(|index| serde_json::json!({
                "id":if index == 0 { "value".to_owned() } else { format!("value-{index}") },
                "label":format!("Value {index}"),"kind":{"kind":"text","max_length":1024},
                "value":format!("Stored {index}")
            })).collect::<Vec<_>>());
            host.application_mut()
                .sync_host_data_fields(&[("plugins", &nested)])
                .unwrap();
            // Live shell projection updates the admitted tree before stepping
            // the native host. Its previous layout still contains the old rows.
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    application_changed: true,
                    ..Default::default()
                },
            )
            .unwrap();
            settle(&mut host, "nested settings mount");
            assert!(host.query_unique(&editor(0)).is_ok());
            assert!(host.query_unique(&editor(127)).is_err());
            for (delta_y, visible, absent) in [(1_000_000.0, 127, 0), (-1_000_000.0, 0, 127)] {
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                            point: Point { x: 700.0, y: 600.0 },
                            delta_y,
                        })],
                        ..Default::default()
                    },
                )
                .unwrap();
                settle(&mut host, "nested settings scroll");
                assert!(host.query_unique(&editor(visible)).is_ok());
                assert!(host.query_unique(&editor(absent)).is_err());
                let rows = host
                    .application()
                    .accepted
                    .node()
                    .virtual_collection_measurements(host.resolved_layout())
                    .unwrap();
                assert!(rows.iter().map(|batch| batch.rows.len()).sum::<usize>() < 24);
                assert_eq!(host.application().accepted.virtual_source_usage().1, 129);
                assert!(host.application_mut().take_effects().is_empty());
            }
            assert_eq!(
                find_settings_source(
                    host.application().accepted.source(),
                    "plugin-setting/fixture-0/value"
                )
                .unwrap()["value"],
                "Unapplied draft"
            );
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                        point: Point { x: 700.0, y: 600.0 },
                        delta_y: 1_000_000.0,
                    })],
                    // Native callers include viewport metadata on ordinary
                    // input. Equal dimensions must not undo the scroll anchor.
                    surface_size: Some((1100, 800)),
                    scale_factor: Some(1.0),
                    ..Default::default()
                },
            )
            .unwrap();
            settle(&mut host, "before offscreen schema replacement");
            assert!(host.query_unique(&editor(0)).is_err());
            for (revision, kind, value) in [
                (
                    "8",
                    serde_json::json!({"kind":"integer","min":0,"max":99}),
                    serde_json::json!(2),
                ),
                (
                    "9",
                    serde_json::json!({"kind":"text","max_length":1024}),
                    serde_json::json!("Stored 0"),
                ),
            ] {
                nested["revision"] = revision.into();
                nested["plugins"][0]["settings"][0]["kind"] = kind;
                nested["plugins"][0]["settings"][0]["value"] = value;
                host.application_mut()
                    .sync_host_data_fields(&[("plugins", &nested)])
                    .unwrap();
                step_host(&mut host, None, Default::default()).unwrap();
                settle(&mut host, "offscreen schema replacement");
                assert!(host.query_unique(&editor(0)).is_err());
            }
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                        point: Point { x: 700.0, y: 600.0 },
                        delta_y: -1_000_000.0,
                    })],
                    ..Default::default()
                },
            )
            .unwrap();
            settle(&mut host, "after offscreen schema replacement");
            assert_eq!(
                find_settings_source(
                    host.application().accepted.source(),
                    "plugin-setting/fixture-0/value"
                )
                .unwrap()["value"],
                "Stored 0",
                "restoring an editor type must not resurrect a retired draft"
            );
            assert!(host.application_mut().take_effects().is_empty());
        });
    }

    #[test]
    fn production_display_drag_updates_box_geometry_without_failing_shell() {
        with_package_runtime_stack(|| {
            let outputs = (0..2).map(|index| serde_json::json!({
                "name":format!("DP-{}", index + 1),"model":format!("Fixture {}", index + 1),
                "physical_width_mm":500,"physical_height_mm":300,
                "enabled":true,"primary":index == 0,"scale_120":120,"transform":"normal",
                "geometry":{"x":index * 1920,"y":0,"width":1920,"height":1080},
                "work_area":{"x":index * 1920,"y":0,"width":1920,"height":1080},
                "current_mode":{"width":1920,"height":1080,"refresh_millihz":60000},"modes":[]
            })).collect::<Vec<_>>();
            let (mut application, _composition) =
                settings_admission_application("nickel-default/displays").unwrap();
            application.sync_host_data_field("displays", &serde_json::json!({"available":true,"revision":"0123456789abcdef","operations":{"identify":true},"outputs":outputs})).unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            let card = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Fixture 2 display, DP-2".into(),
                })
                .unwrap();
            let start = Point {
                x: card.bounds.origin.x + 40.0,
                y: card.bounds.origin.y + 40.0,
            };
            let end = Point {
                x: start.x + 120.0,
                y: start.y + 30.0,
            };
            let before = host.application().accepted.source().clone();
            for event in [
                nickel_ui::UiEvent::PointerMoved(start),
                nickel_ui::UiEvent::PointerPressed(start),
                nickel_ui::UiEvent::PointerMoved(end),
                nickel_ui::UiEvent::PointerReleased(end),
            ] {
                crate::live_shell::step_plugin_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(event)],
                        ..Default::default()
                    },
                )
                .unwrap();
                assert!(host.application_mut().last_error().is_none());
            }
            assert_ne!(host.application().accepted.source(), &before);
            assert!(host.application_mut().take_effects().is_empty());
            assert!(
                host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Fixture 2 display, DP-2".into()
                })
                .is_ok()
            );
        });
    }

    #[test]
    fn production_settings_display_modes_virtualize_without_losing_draft() {
        with_package_runtime_stack(|| {
            let settle = |host: &mut nickel_ui::UiHost<PluginPanelApplication>| {
                for _ in 0..16 {
                    if !host.application().virtual_work_pending {
                        return;
                    }
                    let now = host.next_deadline().expect("native virtual continuation");
                    step_host(
                        host,
                        None,
                        nickel_ui::HostBatch {
                            now: Some(now),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
                panic!("display mode range did not converge");
            };
            let modes = (0..128)
                .map(|index| {
                    serde_json::json!({
                        "width":1920,"height":1080,"refresh_millihz":60_000 + index * 1000
                    })
                })
                .collect::<Vec<_>>();
            let snapshot = serde_json::json!({
                "available":true,"revision":"0123456789abcdef","operations":{},
                "outputs":[{"name":"DP-1","model":"Fixture display","enabled":true,
                    "primary":true,"scale_120":120,"transform":"normal",
                    "physical_width_mm":500,"physical_height_mm":300,
                    "geometry":{"x":0,"y":0,"width":1920,"height":1080},
                    "work_area":{"x":0,"y":0,"width":1920,"height":1080},
                    "current_mode":modes[0],"modes":modes}]
            });
            let (mut application, _) =
                settings_admission_application("nickel-default/displays").unwrap();
            application
                .sync_host_data_fields(&[("displays", &snapshot)])
                .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            step_host(&mut host, None, Default::default()).unwrap();
            settle(&mut host);
            let mode = |index: usize| nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: format!("1920 × 1080 · {}.00 Hz", 60 + index),
            };
            let assert_top_text = |host: &nickel_ui::UiHost<PluginPanelApplication>| {
                for label in [
                    "Arrange displays",
                    "Drag displays to match their physical positions. Apply to preview your changes.",
                    "Fixture display · Primary",
                ] {
                    assert!(
                        host.commands().iter().any(|command| matches!(
                            command,
                            nickel_ui::backend::PaintCommand::Text { text, .. } if text == label
                        )),
                        "missing display text {label:?}"
                    );
                }
            };
            assert_top_text(&host);
            assert!(host.query_unique(&mode(0)).is_ok());
            assert!(host.query_unique(&mode(127)).is_err());
            for (visit, (delta_y, visible, absent)) in [
                (100_000.0, 127, 0),
                (-100_000.0, 0, 127),
                (100_000.0, 127, 0),
            ]
            .into_iter()
            .enumerate()
            {
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                            point: Point { x: 700.0, y: 600.0 },
                            delta_y,
                        })],
                        ..Default::default()
                    },
                )
                .unwrap();
                settle(&mut host);
                assert!(!host.application().virtual_work_pending);
                let button = host.query_unique(&mode(visible)).unwrap();
                assert!(host.query_unique(&mode(absent)).is_err());
                if visit == 0 {
                    host.perform_semantic_action(
                        button.id,
                        nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                    );
                    step_host(&mut host, None, Default::default()).unwrap();
                    settle(&mut host);
                }
                if visible == 127 {
                    assert_eq!(
                        find_settings_source(
                            host.application().accepted.source(),
                            "display-mode-127"
                        )
                        .unwrap()["state"],
                        "selected"
                    );
                } else {
                    assert_top_text(&host);
                    assert_eq!(
                        find_settings_source(
                            host.application().accepted.source(),
                            "display-mode-0"
                        )
                        .unwrap()["state"],
                        "unselected"
                    );
                }
                assert!(host.application().last_error().is_none());
                let rows = host
                    .application()
                    .accepted
                    .node()
                    .virtual_collection_measurements(host.resolved_layout())
                    .unwrap();
                assert!(rows.iter().map(|batch| batch.rows.len()).sum::<usize>() < 24);
                assert_eq!(host.application().accepted.virtual_source_usage().1, 128);
                assert!(
                    host.application_mut().take_effects().is_empty(),
                    "choosing a mode edits the draft; only Apply may change displays"
                );
            }
            let discard = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Discard draft".into(),
                })
                .unwrap();
            host.perform_semantic_action(
                discard.id,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            step_host(&mut host, None, Default::default()).unwrap();
            settle(&mut host);
            assert_eq!(
                find_settings_source(host.application().accepted.source(), "display-mode-127")
                    .unwrap()["state"],
                "unselected"
            );
            assert!(host.application_mut().take_effects().is_empty());
        });
    }

    #[test]
    fn production_settings_adapter_inventory_uses_native_virtual_rows() {
        with_package_runtime_stack(|| {
            let snapshot =
                crate::connectivity_capabilities::wifi_snapshot(&crate::platform::NetworkStatus {
                    adapters_available: true,
                    adapters: (0..256)
                        .map(|index| crate::platform::NetworkAdapterStatus {
                            id: format!("adapter-{index}"),
                            name: format!("Adapter {index}"),
                            description: "A variable-height adapter description "
                                .repeat(index % 4 + 1),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                });
            let (mut application, _) =
                settings_admission_application("nickel-default/wifi").unwrap();
            application
                .sync_host_data_fields(&[("wifi", &snapshot)])
                .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            step_host(&mut host, None, Default::default()).unwrap();
            let text = |name: String| nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Text,
                name,
            };
            assert!(host.query_unique(&text("Adapter 0".into())).is_ok());
            assert!(host.query_unique(&text("Adapter 255".into())).is_err());
            for (delta_y, visible, absent) in [(100_000.0, 255, 0), (-100_000.0, 0, 255)] {
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                            point: Point { x: 700.0, y: 600.0 },
                            delta_y,
                        })],
                        ..Default::default()
                    },
                )
                .unwrap();
                assert!(!host.application().virtual_work_pending);
                assert!(
                    host.query_unique(&text(format!("Adapter {visible}")))
                        .is_ok()
                );
                assert!(
                    host.query_unique(&text(format!("Adapter {absent}")))
                        .is_err()
                );
                let rows = host
                    .application()
                    .accepted
                    .node()
                    .virtual_collection_measurements(host.resolved_layout())
                    .unwrap();
                assert!(rows.iter().map(|batch| batch.rows.len()).sum::<usize>() < 20);
                assert_eq!(host.application().accepted.virtual_source_usage().1, 256);
            }
            assert!(host.application_mut().take_effects().is_empty());
        });
    }

    #[test]
    fn production_settings_connectivity_lists_use_native_virtual_windows() {
        with_package_runtime_stack(|| {
            let long_network = "é".repeat(256);
            let long_device = "d".repeat(512);
            let last_wifi = format!("settings-wifi-connect/{long_network}");
            let last_bluetooth = format!("settings-bluetooth-pair/{long_device}");
            let wifi =
                crate::connectivity_capabilities::wifi_snapshot(&crate::platform::NetworkStatus {
                    available: true,
                    enabled: true,
                    networks: (0..256)
                        .map(|index| crate::platform::WifiNetworkStatus {
                            id: if index == 255 {
                                long_network.clone()
                            } else {
                                format!("network-{index}")
                            },
                            name: format!("Network {index}"),
                            saved: true,
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                });
            let bluetooth = crate::connectivity_capabilities::bluetooth_snapshot(
                &crate::platform::BluetoothStatus {
                    available: true,
                    powered: true,
                    devices: (0..256)
                        .map(|index| crate::platform::BluetoothDeviceStatus {
                            id: if index == 255 {
                                long_device.clone()
                            } else {
                                format!("device-{index}")
                            },
                            name: format!("Device {index}"),
                            battery_percent: (index % 2 == 0).then_some(80),
                            kind: (index % 3 == 0).then(|| "Headphones".into()),
                            ..Default::default()
                        })
                        .collect(),
                    ..Default::default()
                },
            );
            for (page, snapshot, first, last) in [
                (
                    "wifi",
                    &wifi,
                    "settings-wifi-connect/network-0",
                    last_wifi.as_str(),
                ),
                (
                    "bluetooth",
                    &bluetooth,
                    "settings-bluetooth-pair/device-0",
                    last_bluetooth.as_str(),
                ),
            ] {
                let (mut application, _composition) =
                    settings_admission_application(&format!("nickel-default/{page}")).unwrap();
                application
                    .sync_host_data_fields(&[(page, snapshot)])
                    .unwrap();
                let mut host = nickel_ui::UiHost::new(application, 1100, 800);
                crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
                assert!(!host.application().virtual_work_pending);
                let source = logical_source(host.application().accepted.node())
                    .unwrap()
                    .clone();
                assert_eq!(source.len(), 256);
                assert!(host.application().button_message(first).is_some());
                assert!(host.application().button_message(last).is_none());
                for delta_y in [100_000.0, -100_000.0, 100_000.0] {
                    crate::live_shell::step_plugin_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                                point: Point { x: 700.0, y: 600.0 },
                                delta_y,
                            })],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    assert!(!host.application().virtual_work_pending);
                    let rows = host
                        .application()
                        .accepted
                        .node()
                        .virtual_collection_measurements(host.resolved_layout())
                        .unwrap();
                    assert_eq!(rows.len(), 1);
                    assert!(
                        rows[0].rows.len() < 20,
                        "{page} materialized offscreen inventory"
                    );
                    assert_eq!(
                        logical_source(host.application().accepted.node())
                            .unwrap()
                            .revision(),
                        source.revision()
                    );
                }
                assert!(host.application().button_message(first).is_none());
                let action = host
                    .application()
                    .button_message(last)
                    .expect("last device reachable");
                host.application_mut().update(action);
                assert!(
                    host.application().last_error().is_none(),
                    "{page}: {:?}",
                    host.application().last_error()
                );
                let effects = host.application_mut().take_effects();
                let expected = crate::connectivity_capabilities::ConnectivityEffect::parse(
                    &serde_json::json!({
                        "type":if page == "wifi" {"wifi.connect"} else {"bluetooth.pair"},
                        "revision":snapshot["revision"],
                    "id":if page == "wifi" {&long_network} else {&long_device},
                    }),
                )
                .unwrap();
                assert!(
                    matches!(effects.as_slice(),[PluginEffect::Connectivity {effect,..}] if effect == &expected),
                    "{effects:?}"
                );
                let mut empty = snapshot.clone();
                empty[if page == "wifi" {
                    "networks"
                } else {
                    "devices"
                }] = serde_json::json!([]);
                host.application_mut()
                    .sync_host_data_fields(&[(page, &empty)])
                    .unwrap();
                crate::live_shell::step_plugin_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        application_changed: true,
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(
                    logical_source(host.application().accepted.node())
                        .unwrap()
                        .len(),
                    0
                );
                assert!(host.application().button_message(last).is_none());
                assert!(!host.application().virtual_work_pending);
            }
        });
    }

    #[test]
    fn production_repeated_settings_preserve_drafts_across_virtual_unmount() {
        with_package_runtime_stack(|| {
            fn settle(host: &mut nickel_ui::UiHost<PluginPanelApplication>) {
                for _ in 0..16 {
                    if !host.application().virtual_work_pending {
                        return;
                    }
                    for message in host
                        .application()
                        .accepted
                        .node()
                        .virtual_collection_feedback(
                            host.resolved_layout(),
                            nickel_ui::Rect::new(0.0, 0.0, 1100.0, 800.0),
                        )
                        .unwrap()
                    {
                        if let PluginMessage::Text(action, _) = message {
                            assert!(
                                host.application()
                                    .composition
                                    .as_ref()
                                    .unwrap()
                                    .events
                                    .contains_key(&(action as u64)),
                                "native virtual feedback lost its callback authority"
                            );
                        }
                    }
                    step_host(
                        host,
                        None,
                        nickel_ui::HostBatch {
                            now: host.next_deadline(),
                            events: vec![nickel_ui::HostEvent::Poll],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                }
                assert!(
                    !host.application().virtual_work_pending,
                    "editable rows did not settle"
                );
            }
            for nesting in ["plain", "group", "repeated"] {
                eprintln!("editable fixture: {nesting}");
                let field_path = match nesting {
                    "group" => "fixture/row-0/details/name",
                    "repeated" => "fixture/row-0/children/row-0/name",
                    _ => "fixture/row-0/name",
                };
                let last_field_path = field_path.replacen("row-0", "row-127", 1);
                let mut package =
                    crate::bundled_plugin_assets::load_package("nickel-default").unwrap();
                let fixture = r#"
                function RepeatedFixture() {
                    const nesting='__NESTING__';
                    const [rows,setRows]=useState(()=>Array.from({length:128},(_,i)=>({name:'Stored '+i,details:{name:'Stored '+i},children:[{name:'Stored '+i}]})));
                    const leaf={id:'name',type:'text',label:'Name',maxLength:1024};
                    const fields=nesting==='group'?[{id:'details',type:'group',label:'Details',fields:[leaf]}]
                        :nesting==='repeated'?[{id:'children',type:'repeated',label:'Children',maxItems:4,fields:[leaf]}]:[leaf];
                    const saved=nesting==='group'?rows[0].details.name:nesting==='repeated'?rows[0].children[0].name:rows[0].name;
                    return h(Column,{},h(Text,{id:'saved-first'},saved),
                        h(SettingControl,{controlId:'fixture',setting:{id:'fixture',providerPackage:'nickel-default',
                            type:'repeated',maxItems:256,fields,
                            value:rows,onChange:setRows}}));
                }
                registerSettingsPage({id:'repeated-fixture',group:'Tests',label:'Repeated fixture',component:RepeatedFixture});
            "#.replace("__NESTING__", nesting);
                package.source.push_str(&fixture);
                package
                    .modules
                    .iter_mut()
                    .find(|module| module.path == "src/Shell.tsx")
                    .unwrap()
                    .source
                    .push_str(&fixture);
                let (catalog, surface, composition) =
                    settings_admission_runtime_with_package(package).unwrap();
                let application = settings_admission_application_with_runtime(
                    "nickel-default/repeated-fixture",
                    &catalog,
                    &surface,
                    composition,
                )
                .unwrap();
                let mut host = nickel_ui::UiHost::new(application, 1100, 800);
                step_host(&mut host, None, Default::default())
                    .unwrap_or_else(|error| panic!("{nesting}: {error}"));
                settle(&mut host);
                let field = host
                    .query(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::TextField,
                        name: "Name".into(),
                    })
                    .into_iter()
                    .find(|node| node.id.as_str().ends_with(field_path))
                    .unwrap_or_else(|| {
                        panic!(
                            "repeated inputs missing: {:?}; {:?}",
                            host.query(&nickel_ui::SemanticSelector::RoleAndName {
                                role: SemanticRole::TextField,
                                name: "Name".into()
                            }),
                            host.application().last_error()
                        )
                    });
                host.perform_semantic_action(
                    field.id.clone(),
                    nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Text(
                        "Unapplied draft".into(),
                    )),
                );
                step_host(&mut host, None, Default::default()).unwrap();
                settle(&mut host);
                assert!(
                    find_settings_source(host.application().accepted.source(), "saved-first")
                        .unwrap()
                        .to_string()
                        .contains("Stored 0"),
                    "editing a draft applied the setting"
                );
                host.request_focus(field.id.clone());
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::ImePreedit(
                            "未確定".into(),
                        ))],
                        ..Default::default()
                    },
                )
                .unwrap();
                assert!(host.input_context().text_focused);
                assert!(
                    host.commands().iter().any(|command| matches!(
                        command,
                        nickel_ui::backend::PaintCommand::Text { text, .. }
                            if text.contains("未確定")
                    )),
                    "focused editor did not display its IME preedit"
                );
                assert_eq!(
                    find_settings_source(host.application().accepted.source(), field_path).unwrap()
                        ["value"],
                    "Unapplied draft",
                    "IME preedit must not commit the draft"
                );
                for delta in [100_000.0, -100_000.0] {
                    step_host(
                        &mut host,
                        None,
                        nickel_ui::HostBatch {
                            events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                                point: Point { x: 700.0, y: 600.0 },
                                delta_y: delta,
                            })],
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    settle(&mut host);
                    assert!(!host.application().virtual_work_pending);
                    let source = logical_source(host.application().accepted.node()).unwrap();
                    assert_eq!(source.len(), 128);
                    let rows = host
                        .application()
                        .accepted
                        .node()
                        .virtual_collection_measurements(host.resolved_layout())
                        .unwrap();
                    assert!(rows[0].rows.len() < 20);
                    if delta > 0.0 {
                        assert!(!host.input_context().text_focused);
                        assert_ne!(host.inspect().keyboard_focus.as_ref(), Some(&field.id));
                        step_host(
                            &mut host,
                            None,
                            nickel_ui::HostBatch {
                                events: vec![nickel_ui::HostEvent::Ui(
                                    nickel_ui::UiEvent::TextInput("late IME commit".into()),
                                )],
                                ..Default::default()
                            },
                        )
                        .unwrap();
                        assert!(
                            find_settings_source(host.application().accepted.source(), field_path)
                                .is_none()
                        );
                        assert!(
                            find_settings_source(
                                host.application().accepted.source(),
                                &last_field_path
                            )
                            .is_some()
                        );
                        assert_eq!(
                            find_settings_source(
                                host.application().accepted.source(),
                                &last_field_path
                            )
                            .unwrap()["value"],
                            "Stored 127",
                            "retired editor's text must not reach a recycled row"
                        );
                    }
                }
                assert_eq!(
                    find_settings_source(host.application().accepted.source(), field_path).unwrap()
                        ["value"],
                    "Unapplied draft"
                );
                assert!(
                    !host.commands().iter().any(|command| matches!(
                        command,
                        nickel_ui::backend::PaintCommand::Text { text, .. }
                            if text.contains("未確定") || text.contains("late IME commit")
                    )),
                    "retired composition reappeared after rematerialization"
                );
                let apply = host
                    .query(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: "Apply".into(),
                    })
                    .into_iter()
                    .find(|node| node.id.as_str().ends_with(&format!("{field_path}/apply")))
                    .unwrap();
                host.perform_semantic_action(
                    apply.id,
                    nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                );
                step_host(&mut host, None, Default::default()).unwrap();
                let saved =
                    find_settings_source(host.application().accepted.source(), "saved-first")
                        .unwrap();
                assert!(saved.to_string().contains("Unapplied draft"));
                assert!(host.application_mut().take_effects().is_empty());
            }
        });
    }

    #[test]
    fn production_settings_wallpaper_previews_follow_visible_rows_and_arrive_without_effects() {
        with_package_runtime_stack(|| {
            let (mut application, _composition) =
                settings_admission_application("nickel-default/appearance").unwrap();
            let wallpaper = serde_json::json!({
                "available":true,"writable":true,"generation":2,
                "configured":{"custom_image_configured":false,"position":"fill"},
                "images":(0..128).map(|index| serde_json::json!({
                    "id":format!("fixture-{index}"),"label":format!("Wallpaper {index}"),
                    "configured":false,"previewAsset":format!("wallpaper:fixture-{index}"),
                })).collect::<Vec<_>>(),
                "chooser":{"available":true,"pending":false,"result":null},
            });
            application
                .sync_host_data_fields(&[("wallpaper", &wallpaper)])
                .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            step_host(&mut host, None, Default::default()).unwrap();
            let viewport = nickel_ui::Rect::new(0.0, 0.0, 1100.0, 800.0);
            let first = host
                .application()
                .wallpaper_preview_demand(host.resolved_layout(), viewport);
            assert!(first.len() <= 8);
            assert!(!first.iter().any(|asset| asset == "wallpaper:fixture-127"));
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                        point: Point { x: 700.0, y: 600.0 },
                        delta_y: 100_000.0,
                    })],
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(!host.application().virtual_work_pending);
            let demand = host
                .application()
                .wallpaper_preview_demand(host.resolved_layout(), viewport);
            assert!(!demand.is_empty() && demand.len() <= 8);
            assert!(demand.iter().any(|asset| asset == "wallpaper:fixture-127"));
            assert!(!demand.iter().any(|asset| asset == "wallpaper:fixture-0"));
            let last = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Wallpaper 127".into(),
                })
                .unwrap();
            let unfocused_paint = host.commands().to_vec();
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(
                        nickel_ui::UiEvent::AccessibilityFocus(last.id.clone()),
                    )],
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(
                host.commands() != unfocused_paint.as_slice(),
                "keyboard focus on a wallpaper must have a visible paint affordance"
            );
            let images = demand
                .iter()
                .enumerate()
                .map(|(index, asset)| {
                    (
                        asset.clone(),
                        (
                            64000 + index as u16,
                            Arc::new(image::RgbaImage::from_pixel(
                                160,
                                90,
                                image::Rgba([20, 40, 60, 255]),
                            )),
                        ),
                    )
                })
                .collect();
            assert!(host.application_mut().sync_application_images(images));
            step_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    application_changed: true,
                    ..Default::default()
                },
            )
            .unwrap();
            assert_eq!(host.inspect().keyboard_focus, Some(last.id.clone()));
            let bounds = host.resolved_layout().find(&last.id).unwrap().allocated;
            assert!(bounds.origin.y >= 0.0 && bounds.origin.y + bounds.size.height <= 800.0);
            assert!(host.commands().iter().any(|command| matches!(command, nickel_ui::backend::PaintCommand::Image {id,..} if *id >= 64000 && *id < 64008)));
            assert!(host.resolved_layout().nodes().len() < 500);
            assert!(host.application_mut().take_effects().is_empty());
        });
    }

    #[test]
    fn production_settings_wallpapers_materialize_only_the_native_window() {
        with_package_runtime_stack(|| {
            let (mut application, _composition) =
                settings_admission_application("nickel-default/appearance").unwrap();
            let wallpaper = serde_json::json!({
                "available":true,"writable":true,"generation":2,
                "configured":{"custom_image_configured":false,"position":"fill"},
                "images":(0..128).map(|index| serde_json::json!({
                    "id":format!("fixture-{index}"),"label":format!("Wallpaper {index}"),
                    "configured":false,
                })).collect::<Vec<_>>(),
                "chooser":{"available":true,"pending":false,"result":null},
            });
            application
                .sync_host_data_fields(&[("wallpaper", &wallpaper)])
                .unwrap();
            let mut host = nickel_ui::UiHost::new(application, 1100, 800);
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            assert!(!host.application().virtual_work_pending);
            let source = logical_source(host.application().accepted.node())
                .unwrap()
                .clone();
            assert_eq!(source.len(), 128);
            let rows = host
                .application()
                .accepted
                .node()
                .virtual_collection_measurements(host.resolved_layout())
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert!(
                rows[0].rows.len() < 30,
                "offscreen wallpapers were materialized"
            );
            assert!(
                host.application()
                    .button_message("appearance-wallpaper-fixture-127")
                    .is_none()
            );
            crate::live_shell::step_plugin_host(
                &mut host,
                None,
                nickel_ui::HostBatch {
                    events: vec![nickel_ui::HostEvent::Ui(nickel_ui::UiEvent::Scroll {
                        point: Point { x: 700.0, y: 600.0 },
                        delta_y: 100_000.0,
                    })],
                    ..Default::default()
                },
            )
            .unwrap();
            assert!(!host.application().virtual_work_pending);
            assert_eq!(
                logical_source(host.application().accepted.node())
                    .unwrap()
                    .revision(),
                source.revision(),
                "scrolling replaced the logical source"
            );
            let rows = host
                .application()
                .accepted
                .node()
                .virtual_collection_measurements(host.resolved_layout())
                .unwrap();
            assert!(rows[0].rows.len() < 30);
            assert!(
                host.application()
                    .button_message("appearance-wallpaper-fixture-127")
                    .is_some(),
                "last wallpaper is unreachable through the outer Settings scroller"
            );
            assert!(
                host.application()
                    .button_message("appearance-wallpaper-fixture-0")
                    .is_none(),
                "retired offscreen wallpaper kept its callback"
            );
            let last = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Wallpaper 127".into(),
                })
                .unwrap();
            assert!(last.bounds.origin.y < 800.0);
            host.perform_semantic_action(
                last.id,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            assert!(host.application().last_error().is_none());
            let effects = host.application_mut().take_effects();
            assert!(
                matches!(effects.as_slice(), [PluginEffect::Appearance { .. }]),
                "{effects:?}"
            );
            let chooser = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Choose image…".into(),
                })
                .unwrap();
            host.request_focus(chooser.id);
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            let mut peak_javascript_handlers = 0;
            for (index, event) in (0..128)
                .map(|index| (index, nickel_ui::UiEvent::FocusNext))
                .chain(
                    (0..127)
                        .rev()
                        .map(|index| (index, nickel_ui::UiEvent::FocusPrevious)),
                )
            {
                crate::live_shell::step_plugin_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: vec![nickel_ui::HostEvent::Ui(event)],
                        ..Default::default()
                    },
                )
                .unwrap();
                let expected = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: format!("Wallpaper {index}"),
                    })
                    .unwrap_or_else(|error| {
                        panic!("keyboard did not materialize wallpaper {index}: {error:?}")
                    });
                assert_eq!(
                    host.inspect().keyboard_focus.as_ref(),
                    Some(&expected.id),
                    "wallpaper {index}"
                );
                assert!(
                    expected.bounds.origin.y >= 0.0
                        && expected.bounds.origin.y + expected.bounds.size.height <= 800.0
                );
                let rows = host
                    .application()
                    .accepted
                    .node()
                    .virtual_collection_measurements(host.resolved_layout())
                    .unwrap();
                assert!(
                    rows[0].rows.len() < 30,
                    "keyboard traversal accumulated offscreen rows"
                );
                let runtime = host.application_mut().shared_runtime();
                let diagnostics = runtime.borrow_mut().runtime_diagnostics().unwrap();
                let retained = &diagnostics["retained"];
                let handlers = retained["handlerEntries"].as_u64().unwrap();
                peak_javascript_handlers = peak_javascript_handlers.max(handlers);
                // This fixture has fewer than 30 admitted wallpaper rows plus
                // the fixed Settings controls. Walking all 128 logical rows
                // must not retain a JavaScript callback for each visited row.
                assert!(handlers < 100, "wallpaper {index}: {retained}");
                assert!(retained["handlerSlots"].as_u64().unwrap() < 100);
                assert!(retained["previousHandlerEntries"].as_u64().unwrap() < 100);
                assert_eq!(
                    logical_source(host.application().accepted.node())
                        .unwrap()
                        .revision(),
                    source.revision()
                );
            }
            eprintln!(
                "production wallpaper traversal peak JavaScript handlers: {peak_javascript_handlers}"
            );
            // OS input may arrive in one batch. Each focus step must reveal
            // the next logical row before the following event is interpreted.
            for (steps, expected_index, forward) in [
                (32, 32, true),
                (32, 64, true),
                (32, 96, true),
                (31, 127, true),
                (64, 63, false),
                (63, 0, false),
            ] {
                crate::live_shell::step_plugin_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        events: (0..steps)
                            .map(|_| {
                                nickel_ui::HostEvent::Ui(if forward {
                                    nickel_ui::UiEvent::FocusNext
                                } else {
                                    nickel_ui::UiEvent::FocusPrevious
                                })
                            })
                            .collect(),
                        ..Default::default()
                    },
                )
                .unwrap();
                let expected = host
                    .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                        role: SemanticRole::Button,
                        name: format!("Wallpaper {expected_index}"),
                    })
                    .unwrap();
                assert_eq!(
                    host.inspect().keyboard_focus.as_ref(),
                    Some(&expected.id),
                    "batched traversal to {expected_index}"
                );
            }
            let focused = host.inspect().keyboard_focus.unwrap();
            let mut updated_wallpaper = wallpaper.clone();
            updated_wallpaper["generation"] = serde_json::json!(3);
            updated_wallpaper["images"]
                .as_array_mut()
                .unwrap()
                .reverse();
            let mut updated_snapshot = host.application().projection_value.clone().unwrap();
            updated_snapshot["wallpaper"] = updated_wallpaper;
            step_host(
                &mut host,
                Some(updated_snapshot.to_string()),
                Default::default(),
            )
            .unwrap();
            assert_eq!(host.inspect().keyboard_focus, Some(focused.clone()));
            let reordered = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Wallpaper 0".into(),
                })
                .unwrap();
            assert_eq!(reordered.id, focused);
            assert!(
                reordered.bounds.origin.y >= 0.0
                    && reordered.bounds.origin.y + reordered.bounds.size.height <= 800.01
            );
            assert!(host.resolved_layout().nodes().len() < 500);
            assert!(host.application().last_error().is_none());
            assert!(host.application_mut().take_effects().is_empty());
            for height in [400, 800] {
                step_host(
                    &mut host,
                    None,
                    nickel_ui::HostBatch {
                        surface_size: Some((1100, height)),
                        ..Default::default()
                    },
                )
                .unwrap();
                assert_eq!(host.render_frame().logical_size, (1100, height));
                assert_eq!(host.inspect().keyboard_focus, Some(focused.clone()));
                let bounds = host.resolved_layout().find(&focused).unwrap().allocated;
                assert!(
                    bounds.origin.y >= 0.0
                        && bounds.origin.y + bounds.size.height <= height as f32 + 0.01,
                    "focused wallpaper escaped resized viewport: {bounds:?}, height={height}"
                );
                assert!(!host.application().virtual_work_pending);
                assert!(host.application_mut().take_effects().is_empty());
            }
            let navigation = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: "Keyboard shortcuts".into(),
                })
                .unwrap();
            host.perform_semantic_action(
                navigation.id,
                nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
            );
            crate::live_shell::step_plugin_host(&mut host, None, Default::default()).unwrap();
            assert!(
                logical_source(host.application().accepted.node()).is_none(),
                "inactive Appearance retained its logical wallpaper source"
            );
            assert!(
                host.application()
                    .button_message("appearance-wallpaper-fixture-127")
                    .is_none()
            );
            assert!(!host.application().virtual_work_pending);
        });
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    struct TwoOutputShellWork {
        left: [u64; 10],
        right: [u64; 10],
        mounts_open: usize,
        mounts_closed: usize,
    }

    fn taskbar_admission_setup(active: &str) -> Result<AdmissionRuntime, String> {
        let default = crate::bundled_plugin_assets::load_package("nickel-default")?;
        let mut catalog =
            std::collections::BTreeMap::from([(default.manifest.id.clone(), default)]);
        if active == "nickel-cupertino-dock" {
            let package = crate::bundled_plugin_assets::load_package(active)?;
            catalog.insert(package.manifest.id.clone(), package);
        }
        let surface = catalog[active]
            .manifest
            .surfaces
            .iter()
            .find(|surface| surface.id == "taskbar")
            .ok_or("taskbar surface is missing")?
            .clone();
        let host = std::rc::Rc::new(std::cell::RefCell::new(ShellCompositionRuntime::new(
            &catalog,
            active,
            &Default::default(),
        )?));
        Ok((catalog, surface, host))
    }

    fn taskbar_admission_application(
        active: &str,
        catalog: &std::collections::BTreeMap<String, PluginPackage>,
        surface: &PluginSurface,
        host: std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>,
        windows: &Value,
    ) -> Result<PluginPanelApplication, String> {
        let owners = host
            .borrow()
            .participating_owners()
            .cloned()
            .collect::<Vec<_>>();
        let snapshots = owners
            .iter()
            .map(|owner| {
                let package = &catalog[&owner.id];
                let settings = package
                    .manifest
                    .settings
                    .iter()
                    .map(|setting| (setting.id.clone(), setting.kind.default_value()))
                    .collect::<std::collections::BTreeMap<_, _>>();
                let mut data = serde_json::json!({
                    "settings": settings,
                    "windows": windows,
                    "applications": [],
                    "notifications": initial_notifications_data(&package.manifest),
                    "surface": {"id":"taskbar","kind":surface.kind.as_str(),"width":surface.width,"height":surface.height},
                });
                if let Some(projection) = validation_surface_projection(package, surface) {
                    data.as_object_mut().unwrap().extend(projection.as_object().unwrap().clone());
                }
                (owner.clone(), data)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        PluginPanelApplication::from_composed_surface(
            catalog,
            active,
            &snapshots,
            surface,
            Some(host),
        )
    }

    fn subtract_profile(after: [u64; 7], before: [u64; 7]) -> [u64; 7] {
        std::array::from_fn(|index| after[index] - before[index])
    }

    fn two_output_patch_work(profile: [u64; 7], counters: NativePatchCounters) -> [u64; 10] {
        [
            profile[0],
            profile[1],
            profile[2],
            profile[3],
            profile[4],
            profile[5],
            profile[6],
            counters.local_materializations,
            counters.expansion_nodes,
            counters.tree_bytes,
        ]
    }

    fn assert_incremental_two_output_work(active: &str, output: &str, work: [u64; 10]) {
        assert_eq!(work[0], 0, "{active} {output} rebuilt a complete tree");
        assert!(work[1] > 0, "{active} {output} omitted typed transport");
        assert_eq!(work[2], 1, "{active} {output} patch was not localized");
        assert_eq!(work[3], 2, "{active} {output} traversed unexpected nodes");
        assert!(
            work[4] <= 3,
            "{active} {output} executed unrelated components"
        );
        assert_eq!(work[5], 1, "{active} {output} skipped native admission");
        assert_eq!(work[6], 0, "{active} {output} rejected its typed patch");
        assert_eq!(work[7], 0, "{active} {output} materialized a local tree");
        assert_eq!(work[8], 0, "{active} {output} re-expanded composition");
        assert_eq!(work[9], 0, "{active} {output} transported tree bytes");
    }

    fn canonicalize_composition_assets(value: &mut Value) {
        match value {
            Value::String(asset) if asset.starts_with("composition.asset.") => {
                *asset = "composition.asset.<generation>".into();
            }
            Value::Array(values) => values.iter_mut().for_each(canonicalize_composition_assets),
            Value::Object(object) => object
                .values_mut()
                .for_each(canonicalize_composition_assets),
            _ => {}
        }
    }

    fn composition_profile_totals(
        composition: &std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>,
    ) -> [u64; 7] {
        let owners = composition
            .borrow()
            .participating_owners()
            .cloned()
            .collect::<Vec<_>>();
        owners.into_iter().fold([0; 7], |mut total, owner| {
            let runtime = composition.borrow().shared_owner_runtime(&owner).unwrap();
            let owner_total =
                settings_profile_totals(&runtime.borrow_mut().runtime_diagnostics().unwrap());
            for index in 0..7 {
                total[index] += owner_total[index];
            }
            total
        })
    }

    fn exercise_two_output_shell_admission(
        active: &str,
    ) -> (TwoOutputShellWork, [std::time::Duration; 4]) {
        let total_started = std::time::Instant::now();
        let (catalog, surface, composition) = taskbar_admission_setup(active).unwrap();
        let initial = serde_json::json!([]);
        let open_started = std::time::Instant::now();
        let mut left = taskbar_admission_application(
            active,
            &catalog,
            &surface,
            composition.clone(),
            &initial,
        )
        .unwrap();
        left.sync_surface_authority(
            Some("DP-1"),
            Some((1920.0, 1032.0)),
            Some(1.0),
            Some(true),
            Some(true),
        )
        .unwrap();
        let mut right = taskbar_admission_application(
            active,
            &catalog,
            &surface,
            composition.clone(),
            &initial,
        )
        .unwrap();
        right
            .sync_surface_authority(
                Some("HDMI-A-1"),
                Some((2560.0, 1392.0)),
                Some(1.25),
                Some(false),
                Some(true),
            )
            .unwrap();
        let open = open_started.elapsed();
        let mounts_open = composition.borrow().mount_count();
        assert_eq!(
            mounts_open, 4,
            "each output owns its root and taskbar mounts"
        );

        let control_id = if active == "nickel-default" {
            "taskbar-launcher"
        } else {
            "cupertino-dock-launcher"
        };
        let left_control = find_settings_source(left.accepted.source(), control_id).unwrap();
        let left_identity = (
            left_control["__nativeId"].clone(),
            left_control["__handlerSlots"].clone(),
        );
        let right_control = find_settings_source(right.accepted.source(), control_id).unwrap();
        let right_identity = (
            right_control["__nativeId"].clone(),
            right_control["__handlerSlots"].clone(),
        );
        // Native ids are mount-local paths and may intentionally be identical;
        // the two applications and composition mounts provide their namespace.

        let before = composition_profile_totals(&composition);
        let changed = serde_json::json!([{"id":"admission-window","title":"Admission","applicationId":"admission.app","active":true}]);
        let update_started = std::time::Instant::now();
        assert!(
            left.sync_host_data_fields(&[("windows", &changed)])
                .unwrap()
        );
        let middle = composition_profile_totals(&composition);
        let left_patch_counters = left.diagnostic_patch_counters;
        assert!(right.refresh_composition_snapshots().unwrap());
        let after = composition_profile_totals(&composition);
        let right_patch_counters = right.diagnostic_patch_counters;
        let update = update_started.elapsed();
        let left_after = find_settings_source(left.accepted.source(), control_id).unwrap();
        let right_after = find_settings_source(right.accepted.source(), control_id).unwrap();
        assert_eq!(
            (
                left_after["__nativeId"].clone(),
                left_after["__handlerSlots"].clone()
            ),
            left_identity
        );
        assert_eq!(
            (
                right_after["__nativeId"].clone(),
                right_after["__handlerSlots"].clone()
            ),
            right_identity
        );

        let (oracle_catalog, oracle_surface, oracle_runtime) =
            taskbar_admission_setup(active).unwrap();
        let mut oracle_left = taskbar_admission_application(
            active,
            &oracle_catalog,
            &oracle_surface,
            oracle_runtime.clone(),
            &changed,
        )
        .unwrap();
        oracle_left
            .sync_surface_authority(
                Some("DP-1"),
                Some((1920.0, 1032.0)),
                Some(1.0),
                Some(true),
                Some(true),
            )
            .unwrap();
        let mut oracle_right = taskbar_admission_application(
            active,
            &oracle_catalog,
            &oracle_surface,
            oracle_runtime.clone(),
            &changed,
        )
        .unwrap();
        oracle_right
            .sync_surface_authority(
                Some("HDMI-A-1"),
                Some((2560.0, 1392.0)),
                Some(1.25),
                Some(false),
                Some(true),
            )
            .unwrap();
        let mut actual_left = left.accepted.source().clone();
        let mut actual_right = right.accepted.source().clone();
        let mut cold_left = oracle_left.accepted.source().clone();
        let mut cold_right = oracle_right.accepted.source().clone();
        for value in [
            &mut actual_left,
            &mut actual_right,
            &mut cold_left,
            &mut cold_right,
        ] {
            canonicalize_settings_actions(value);
            canonicalize_composition_assets(value);
        }
        assert_eq!(
            actual_left, cold_left,
            "left output diverged from cold composition"
        );
        assert_eq!(
            actual_right, cold_right,
            "right output diverged from cold composition"
        );
        oracle_left.retire_surface().unwrap();
        oracle_right.retire_surface().unwrap();

        let close_started = std::time::Instant::now();
        left.retire_surface().unwrap();
        right.retire_surface().unwrap();
        let close = close_started.elapsed();
        let work = TwoOutputShellWork {
            left: two_output_patch_work(subtract_profile(middle, before), left_patch_counters),
            right: two_output_patch_work(subtract_profile(after, middle), right_patch_counters),
            mounts_open,
            mounts_closed: composition.borrow().mount_count(),
        };
        assert_incremental_two_output_work(active, "DP-1", work.left);
        assert_incremental_two_output_work(active, "HDMI-A-1", work.right);
        assert_eq!(work.mounts_closed, 0);
        (work, [open, update, close, total_started.elapsed()])
    }

    #[test]
    #[ignore = "production-sized two-output default/Cupertino admission workload"]
    fn production_two_output_shells_emit_release_distribution() {
        std::thread::Builder::new().stack_size(32 * 1024 * 1024).spawn(|| {
            let warmup = std::env::var("NICKEL_TWO_OUTPUT_ADMISSION_WARMUP").ok().and_then(|v| v.parse().ok()).unwrap_or(2);
            let iterations = std::env::var("NICKEL_TWO_OUTPUT_ADMISSION_ITERATIONS").ok().and_then(|v| v.parse().ok()).unwrap_or(10);
            assert!(iterations > 0);
            for active in ["nickel-default", "nickel-cupertino-dock"] {
                for _ in 0..warmup { exercise_two_output_shell_admission(active); }
                let mut timings: [Vec<std::time::Duration>; 4] = Default::default();
                let mut exact = None;
                for _ in 0..iterations {
                    let (work, measured) = exercise_two_output_shell_admission(active);
                    if let Some(expected) = exact { assert_eq!(work, expected, "deterministic two-output work changed for {active}"); } else { exact = Some(work); }
                    for (samples, duration) in timings.iter_mut().zip(measured) { samples.push(duration); }
                }
                let work = exact.unwrap();
                let fields = |values: [u64; 10]| serde_json::json!({
                    "coldTreeTransportBytes":values[0], "patchEnvelopeTransportBytes":values[1],
                    "patchOperations":values[2], "patchNodesVisited":values[3],
                    "componentExecutions":values[4], "typedPatchApplyAttempts":values[5],
                    "typedPatchApplyRejections":values[6],
                    "patchLocalMaterializations":values[7], "patchExpansionNodes":values[8],
                    "patchCompleteTreeBytes":values[9]
                });
                let report = serde_json::json!({
                    "schema":1,"suite":"jsx_incremental","workload":"production_two_output_shell_lifecycle",
                    "metadata":{"iterations":iterations,"warmupIterations":warmup,"theme":active,"outputs":["DP-1","HDMI-A-1"],"experimental":active == "nickel-cupertino-dock"},
                    "work":{"DP-1":fields(work.left),"HDMI-A-1":fields(work.right),"mountsWhileOpen":work.mounts_open,"mountsAfterClose":work.mounts_closed},
                    "timings":{"open":settings_admission_distribution(&timings[0]),"steadyUpdate":settings_admission_distribution(&timings[1]),"close":settings_admission_distribution(&timings[2]),"total":settings_admission_distribution(&timings[3])}
                });
                eprintln!("nickel_release_admission={report}");
            }
        }).unwrap().join().unwrap();
    }
}
