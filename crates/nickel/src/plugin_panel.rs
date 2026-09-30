//! Experimental JavaScript panel host. The bundled example uses the same small
//! component vocabulary as an external plugin; native surfaces remain shell-owned.

use std::{
    borrow::Cow,
    collections::HashSet,
    sync::{Arc, OnceLock},
};

use nickel_core::package_composition::PackageIdentity;
use nickel_core::plugins::{
    PluginCapability, PluginManifest, PluginPackage, PluginSurface, PluginSurfaceKind,
};
use nickel_plugin_presentation::components::parse_panel_for_manifest;
use nickel_plugin_presentation::components::{PanelNode, render_panel, render_panel_validated};
pub use nickel_plugin_presentation::components::{
    PluginActionContribution, PluginImages, PluginMessage, PluginSectionContribution,
    PluginWidgetContribution,
};
use nickel_plugin_runtime::composition_runtime::{
    ComponentEventHandle, ComponentMount, ShellCompositionRuntime,
};
use nickel_plugin_runtime::{JsxModuleGraph, JsxRuntime, ModuleSource};

struct CompositionPanelState {
    host: std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>,
    mount: ComponentMount,
    events: std::collections::BTreeMap<u64, ComponentEventHandle>,
    manifests: std::collections::BTreeMap<PackageIdentity, PluginManifest>,
    snapshots: std::collections::BTreeMap<PackageIdentity, Value>,
}
use nickel_ui::{
    AnyView, Column, DragPhase, FrameOverlay, OverlayAnchor, OverlayId, OverlayMenu, OverlayStyle,
    Row, Shortcut, Size, Spacer, TransientSurface, UiId, ViewContext,
};
#[cfg(test)]
use nickel_ui::{Length, Point, SemanticRole};
use serde_json::Value;

use crate::control_view::ControlAction;
use crate::platform::SessionAction;
use nickel_codex_ui::{ProjectMenuProjection, ProjectMenuRevision};
use nickel_core::display_projection::ProjectionMode;
use nickel_plugin_presentation::css::StyleSheet;

use crate::launcher::TaskbarApplication;
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

pub fn surface_key() -> nickel_core::plugins::PluginSurfaceKey {
    nickel_core::plugins::PluginSurfaceKey {
        plugin_id: manifest().id.clone(),
        surface_id: surface().id.clone(),
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

pub fn taskbar_enabled() -> bool {
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

pub struct PluginPanelApplication {
    runtime: std::rc::Rc<std::cell::RefCell<JsxRuntime>>,
    node: PanelNode,
    effects: Vec<PluginEffect>,
    pending_transient: Option<(OverlayId, UiId)>,
    last_error: Option<String>,
    runtime_failure: Option<String>,
    manifest: PluginManifest,
    expected_surface_id: Option<String>,
    runtime_surface_id: String,
    projection_data: Option<String>,
    overlay_open: bool,
    dispatch_removed_focus: bool,
    images: PluginImages,
    stylesheet: StyleSheet,
    composition: Option<CompositionPanelState>,
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
    RunSubmit(String),
    RunDismiss,
    ToggleLauncher,
    ShowControlCenter,
    ActivateWindow(crate::model::WindowId),
    CloseWindow(crate::model::WindowId),
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
    ToggleCodexProjects,
    CodexProjectRefresh,
    CodexProjectClose,
    CodexProjectOpen {
        token: String,
        revision: ProjectMenuRevision,
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
    CloseTaskbarMenuWindows,
    InvokePluginSlotAction {
        target_plugin: String,
        slot_id: String,
        plugin_id: String,
        id: String,
        item: Option<String>,
    },
    InvokePluginSlotSection {
        target_plugin: String,
        slot_id: String,
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
pub struct TaskbarWindowMenuPluginProjection {
    pub root: Vec<(String, Option<&'static str>)>,
    pub workspaces: Vec<(String, Option<&'static str>)>,
    pub displays: Vec<(String, Option<&'static str>)>,
}

impl TaskbarWindowMenuPluginProjection {
    pub(crate) fn to_json(&self) -> String {
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

    pub(crate) fn to_json(&self) -> String {
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
            "historyVisible": false,
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
    pub(crate) fn invoke_registered_setting(
        &mut self,
        id: &str,
        value: &Value,
    ) -> Result<Vec<PluginEffect>, String> {
        let provider =
            serde_json::to_string(&self.manifest.id).map_err(|error| error.to_string())?;
        let id = serde_json::to_string(id).map_err(|error| error.to_string())?;
        let action: usize = {
            let mut runtime = self.runtime.borrow_mut();
            runtime.select_surface(&self.runtime_surface_id)?;
            runtime.eval_json(&format!("JSON.stringify(__handlers.push(() => __nickelInvokeSetting({provider}, {id}, {value})) - 1)"))?
        };
        nickel_ui::Application::update(self, PluginMessage::Click(action));
        if let Some(error) = &self.last_error {
            return Err(error.clone());
        }
        Ok(self.take_effects())
    }

    pub fn resolved_surface(&self, grant: &PluginSurface) -> Result<PluginSurface, String> {
        self.node
            .requested_surface(grant, &self.stylesheet)
            .map(|surface| surface.unwrap_or_else(|| grant.clone()))
    }

    fn bundled_application(
        manifest: &PluginManifest,
        entry: &str,
        source: &'static str,
        stylesheet: Option<&'static str>,
        data: String,
    ) -> Result<Self, String> {
        let source = bundled_source(manifest, entry, source)?;
        let mut application = Self::new_with_manifest(source.as_ref(), manifest, Some(data))?;
        if let Some(stylesheet) = stylesheet {
            application.stylesheet = bundled_stylesheet(manifest, stylesheet)?;
        }
        if let [surface] = manifest.surfaces.as_slice() {
            application.resolved_surface(surface)?;
        }
        Ok(application)
    }

    /// Bundled source selection is packaging; rendering and data refresh use
    /// the same path as an installed plugin after the asset has been selected.
    pub(crate) fn bundled_with_data(
        manifest: &PluginManifest,
        entry: &str,
        data: String,
    ) -> Result<Self, String> {
        let (source, stylesheet) = crate::bundled_plugin_assets::resolve(&manifest.id, entry)?;
        Self::bundled_application(manifest, entry, source, Some(stylesheet), data)
    }

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
        let data =
            serde_json::json!({ "settings": settings, "slots": {}, "windows": [] }).to_string();
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
            "slots": {},
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
                .resolution()
                .inheritance_chain
                .iter()
                .map(|identity| (identity.clone(), catalog[&identity.id].manifest.clone()))
                .collect();
            let css = host_ref
                .resolution()
                .inheritance_chain
                .iter()
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
        let rendered =
            host.borrow_mut()
                .render_expanded(&mount, &serde_json::json!({}), |value| {
                    let node = parse_panel_for_manifest(value, &manifest, Some(&surface.id))?;
                    node.requested_surface(surface, &stylesheet)?;
                    Ok(())
                })?;
        let node = parse_panel_for_manifest(&rendered.node, &manifest, Some(&surface.id))?;
        let snapshots = {
            let host = host.borrow();
            host.resolution()
                .inheritance_chain
                .iter()
                .map(|owner| Ok((owner.clone(), host.snapshot(owner)?.clone())))
                .collect::<Result<std::collections::BTreeMap<_, _>, String>>()?
        };
        let projection_data = Some(
            serde_json::to_string(&snapshots[&host.borrow().resolution().active])
                .map_err(|error| error.to_string())?,
        );
        Ok(Self {
            runtime,
            node,
            effects: Vec::new(),
            pending_transient: None,
            last_error: None,
            runtime_failure: None,
            manifest,
            expected_surface_id: Some(surface.id.clone()),
            runtime_surface_id: surface.id.clone(),
            projection_data,
            overlay_open: false,
            dispatch_removed_focus: false,
            images: PluginImages::new(),
            stylesheet,
            composition: Some(CompositionPanelState {
                host,
                mount,
                events: rendered.events,
                manifests,
                snapshots,
            }),
        })
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

    pub(crate) fn refresh_composition_snapshots(&mut self) -> Result<bool, String> {
        let Some(state) = &mut self.composition else {
            return Ok(false);
        };
        let host = state.host.borrow();
        for (owner, data) in &mut state.snapshots {
            *data = host.snapshot(owner)?.clone();
        }
        let active = host.resolution().active.clone();
        let serialized =
            serde_json::to_string(&state.snapshots[&active]).map_err(|error| error.to_string())?;
        drop(host);
        self.sync_serialized_data_inner(serialized, true)
    }

    pub(crate) fn shared_composition_runtime(
        &self,
    ) -> Option<std::rc::Rc<std::cell::RefCell<ShellCompositionRuntime>>> {
        self.composition.as_ref().map(|state| state.host.clone())
    }

    pub fn validate_package(package: &PluginPackage) -> Result<(), String> {
        package_stylesheet(package)?;
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

    pub fn badge_contributions(&self) -> Result<Vec<(String, String, u16, u32)>, String> {
        let mut badges = Vec::new();
        self.node.collect_badges(&mut badges)?;
        if badges.is_empty() {
            return Err("badge extension did not return a badge".into());
        }
        Ok(badges)
    }

    pub fn widget_contributions(&self) -> Result<Vec<PluginWidgetContribution>, String> {
        let mut widgets = Vec::new();
        self.node.collect_widgets(&mut widgets)?;
        if widgets.is_empty() {
            return Err("widget extension did not return a widget".into());
        }
        Ok(widgets)
    }

    pub fn action_contributions(&self) -> Result<Vec<PluginActionContribution>, String> {
        let mut actions = Vec::new();
        self.node.collect_actions(&mut actions)?;
        if actions.is_empty() {
            return Err("action extension did not return an action".into());
        }
        Ok(actions)
    }

    pub fn retained_contribution_bytes(&self) -> u64 {
        self.node.contribution_bytes() + self.stylesheet.estimated_retained_bytes()
    }

    pub fn section_contributions(&self) -> Result<Vec<PluginSectionContribution>, String> {
        let mut sections = Vec::new();
        self.node.collect_sections(&mut sections)?;
        if sections.is_empty() {
            return Err("section extension did not return a section".into());
        }
        Ok(sections)
    }

    pub fn activate_section(&mut self, id: &str) -> bool {
        fn find(node: &PanelNode, id: &str) -> Option<usize> {
            match node {
                PanelNode::Section {
                    id: section_id,
                    action,
                    ..
                } if section_id == id => Some(*action),
                _ => node
                    .container_children()?
                    .iter()
                    .find_map(|child| find(child, id)),
            }
        }
        let Some(action) = find(&self.node, id) else {
            return false;
        };
        nickel_ui::Application::update(self, PluginMessage::Click(action));
        self.last_error.is_none()
    }

    pub fn activate_action(&mut self, id: &str, item_id: &str) -> bool {
        let Some(action) = self.find_action(id, item_id) else {
            return false;
        };
        nickel_ui::Application::update(self, PluginMessage::Text(action, item_id.to_owned()));
        self.last_error.is_none()
    }

    fn find_action(&self, id: &str, item_id: &str) -> Option<usize> {
        fn find(node: &PanelNode, id: &str, item_id: &str) -> Option<usize> {
            match node {
                PanelNode::Action {
                    id: action_id,
                    item,
                    action,
                    ..
                } if action_id == id && item.as_deref().is_none_or(|item| item == item_id) => {
                    Some(*action)
                }
                _ => node
                    .container_children()?
                    .iter()
                    .find_map(|child| find(child, id, item_id)),
            }
        }
        find(&self.node, id, item_id)
    }

    pub fn validate_contribution(&self) -> Result<(), String> {
        use nickel_core::plugins::PluginSlotContract;
        let [contribution] = self.manifest.contributes.as_slice() else {
            return Err("extension needs exactly one contribution".into());
        };
        match contribution.contract {
            PluginSlotContract::Badge => self.badge_contributions().map(|_| ()),
            PluginSlotContract::Widget => self.widget_contributions().map(|_| ()),
            PluginSlotContract::Action => self.action_contributions().map(|_| ()),
            PluginSlotContract::Section => self.section_contributions().map(|_| ()),
        }
    }

    #[cfg(test)]
    pub(crate) fn taskbar_with_test_source(
        source: &str,
        projection: &TaskbarPluginProjection,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, taskbar_manifest(), Some(projection.to_json()))
    }

    #[cfg(test)]
    pub(crate) fn volume_osd_with_test_source(source: &str, data: &Value) -> Result<Self, String> {
        Self::new_with_manifest(source, volume_osd_manifest(), Some(data.to_string()))
    }

    #[cfg(test)]
    pub(crate) fn control_center_with_test_source(
        source: &str,
        data: &Value,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, control_center_manifest(), Some(data.to_string()))
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

    /// Refresh the plugin's host-owned data using the same render transaction
    /// regardless of which surface or first-party plugin consumes it.
    pub fn sync_data(&mut self, data: &Value) -> Result<bool, String> {
        let serialized = data.to_string();
        self.sync_serialized_data(serialized)
    }

    /// Refreshes every sibling surface after shared Settings snapshots change.
    pub(crate) fn refresh_settings_render(&mut self) -> Result<bool, String> {
        let serialized = self.projection_data.clone().unwrap_or_else(|| "{}".into());
        self.sync_serialized_data_inner(serialized, true)
    }

    pub(crate) fn sync_serialized_data(&mut self, serialized: String) -> Result<bool, String> {
        self.sync_serialized_data_inner(serialized, false)
    }

    fn sync_serialized_data_inner(
        &mut self,
        serialized: String,
        force: bool,
    ) -> Result<bool, String> {
        if !force && self.projection_data.as_deref() == Some(serialized.as_str()) {
            return Ok(false);
        }
        if let Some(state) = &mut self.composition {
            let data: Value =
                serde_json::from_str(&serialized).map_err(|error| error.to_string())?;
            let mut host = state.host.borrow_mut();
            let owner = host.resolution().active.clone();
            state.snapshots.insert(owner.clone(), data.clone());
            host.update_snapshot(&owner, &data)?;
            let rendered = host.render_expanded(&state.mount, &serde_json::json!({}), |value| {
                parse_panel_for_manifest(value, &self.manifest, self.expected_surface_id.as_deref())
                    .map(|_| ())
            })?;
            self.node = parse_panel_for_manifest(
                &rendered.node,
                &self.manifest,
                self.expected_surface_id.as_deref(),
            )?;
            state.events = rendered.events;
            self.projection_data = Some(serialized);
            return Ok(true);
        }
        let previous_data = self
            .projection_data
            .as_deref()
            .unwrap_or("{\"query\":\"\",\"results\":[]}")
            .to_owned();
        let mut validation_rejected = false;
        let node = {
            let mut runtime = self.runtime.borrow_mut();
            runtime.select_surface(&self.runtime_surface_id)?;
            runtime.set_data(&serialized)?;
            let result = render_panel_validated(
                &mut runtime,
                &self.manifest,
                self.expected_surface_id.as_deref(),
                "__nickelRender()",
                &self.stylesheet,
                &mut validation_rejected,
            );
            if result.is_err() {
                runtime.set_data(&previous_data)?;
            }
            match result {
                Ok(node) => node,
                Err(error) if validation_rejected => {
                    self.last_error = Some(error);
                    return Ok(false);
                }
                Err(error) => return Err(error),
            }
        };
        self.node = node;
        self.projection_data = Some(serialized);
        self.last_error = None;
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn window_preview_with_test_source(
        source: &str,
        data: &Value,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, window_preview_manifest(), Some(data.to_string()))
    }

    #[cfg(test)]
    pub(crate) fn run_with_test_source(source: &str) -> Result<Self, String> {
        Self::new_with_manifest(
            source,
            run_manifest(),
            Some(serde_json::json!({ "status": null }).to_string()),
        )
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
        let node = {
            let mut runtime_ref = runtime.borrow_mut();
            runtime_ref.select_surface(runtime_surface_id)?;
            if let Some(data) = data.as_deref() {
                runtime_ref.set_data(data)?;
            }
            render_panel(
                &mut runtime_ref,
                manifest,
                expected_surface_id,
                "__nickelRender()",
            )?
        };
        Ok(Self {
            runtime,
            node,
            effects: Vec::new(),
            pending_transient: None,
            last_error: None,
            runtime_failure: None,
            manifest: manifest.clone(),
            expected_surface_id: expected_surface_id.map(str::to_owned),
            runtime_surface_id: runtime_surface_id.to_owned(),
            projection_data: data,
            overlay_open: false,
            dispatch_removed_focus: false,
            images: PluginImages::new(),
            stylesheet: StyleSheet::default(),
            composition: None,
        })
    }

    pub fn sync_theme_palette(
        &mut self,
        palette: nickel_core::theme::ThemePalette,
    ) -> Result<bool, String> {
        self.stylesheet.set_palette(palette)
    }

    pub fn sync_reading_direction(&mut self, direction: nickel_ui::ReadingDirection) -> bool {
        self.stylesheet.set_reading_direction(direction)
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

    pub(crate) fn sync_application_images(&mut self, images: PluginImages) -> bool {
        let mut combined = self.images.clone();
        combined
            .retain(|key, _| !key.starts_with("application:") && !key.starts_with("wallpaper:"));
        combined.extend(images);
        self.sync_images(combined)
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
        self.sync_host_data_field("slots", slots)
    }

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
        let Some(data) = self.projection_data.as_deref() else {
            return Err("plugin has no external projection".into());
        };
        let mut data: Value = serde_json::from_str(data)
            .map_err(|error| format!("invalid external plugin projection: {error}"))?;
        let object = data
            .as_object_mut()
            .ok_or("external plugin projection must be an object")?;
        for (field, value) in fields {
            object.insert((*field).into(), (*value).clone());
        }
        self.sync_data(&data)
    }

    fn validate_host_fields(
        manifest: &PluginManifest,
        fields: &[(&str, &Value)],
    ) -> Result<(), String> {
        if fields.iter().any(|(field, _)| {
            !matches!(
                *field,
                "slots"
                    | "windows"
                    | "applications"
                    | "applicationSearch"
                    | "features"
                    | "shortcuts"
                    | "notifications"
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
        if fields.iter().any(|(field, _)| *field == "windows")
            && !manifest
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
        let Some(state) = &self.composition else {
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
            let mut data = host.snapshot(owner)?.clone();
            let object = data
                .as_object_mut()
                .ok_or("package snapshot must be an object")?;
            for (name, value) in fields {
                object.insert((*name).into(), value.clone());
            }
            if host.snapshot(owner)? != &data {
                host.update_snapshot(owner, &data)?;
                changed = true;
            }
        }
        drop(host);
        if changed {
            self.refresh_composition_snapshots()?;
        }
        Ok(changed)
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
}

impl nickel_ui::Application for PluginPanelApplication {
    type Message = PluginMessage;

    fn shortcut_outcome(&mut self, shortcut: Shortcut) -> nickel_ui::ShortcutOutcome {
        if self.overlay_open && shortcut == Shortcut::Escape {
            return nickel_ui::ShortcutOutcome::from_changed(false);
        }
        if self.overlay_open && shortcut == Shortcut::Submit {
            return nickel_ui::ShortcutOutcome::from_changed(false);
        }
        if let Some(action) = self.node.window_shortcut_action(shortcut) {
            self.update(PluginMessage::Click(action));
            return nickel_ui::ShortcutOutcome::handled(true);
        }
        nickel_ui::ShortcutOutcome::from_changed(false)
    }

    fn update(&mut self, message: Self::Message) {
        self.update_messages(vec![message]);
    }

    fn update_removed_focus(&mut self, message: Self::Message) {
        self.dispatch_removed_focus = true;
        self.update_messages(vec![message]);
        self.dispatch_removed_focus = false;
    }

    fn update_messages(&mut self, messages: Vec<Self::Message>) {
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
        let mut validation_rejected = false;
        let (rendered, effects) = if let Some(state) = &mut self.composition {
            let result = (|| {
                if events.len() != 1 {
                    return Err(
                        "composed surface event batches require owner-safe batch dispatch"
                            .to_owned(),
                    );
                }
                let event = &events[0];
                let handle = state
                    .events
                    .get(&event[0].as_u64().ok_or("invalid host action")?)
                    .ok_or("stale host action")?
                    .clone();
                let mut host = state.host.borrow_mut();

                let rendered = host.dispatch_expanded(
                    &state.mount,
                    &handle,
                    event.get(1).unwrap_or(&Value::Null),
                    |value| {
                        let node = parse_panel_for_manifest(
                            value,
                            &self.manifest,
                            self.expected_surface_id.as_deref(),
                        )?;
                        if let Some(id) = &self.expected_surface_id {
                            let surface = self
                                .manifest
                                .surfaces
                                .iter()
                                .find(|surface| &surface.id == id)
                                .ok_or("composed surface grant is missing")?;
                            node.requested_surface(surface, &self.stylesheet)?;
                        }
                        Ok(())
                    },
                )?;
                let node = parse_panel_for_manifest(
                    &rendered.node,
                    &self.manifest,
                    self.expected_surface_id.as_deref(),
                )?;
                state.events = rendered.events;
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
                Ok((node, effects))
            })();
            match result {
                Ok((node, effects)) => (Ok(node), Ok(effects)),
                Err(error) => (Err(error), Ok(Vec::new())),
            }
        } else {
            let expression = if self.dispatch_removed_focus {
                format!("__nickelDispatchRemovedFocus({})", events[0][0])
            } else {
                format!(
                    "__nickelDispatchBatch({})",
                    serde_json::Value::Array(events)
                )
            };
            {
                let mut runtime = self.runtime.borrow_mut();
                if let Err(error) = runtime.select_surface(&self.runtime_surface_id) {
                    self.runtime_failure = Some(error.clone());
                    self.last_error = Some(error);
                    return;
                }
                let rendered = render_panel_validated(
                    &mut runtime,
                    &self.manifest,
                    self.expected_surface_id.as_deref(),
                    &expression,
                    &self.stylesheet,
                    &mut validation_rejected,
                );
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
        (|| match (rendered, effects) {
            (Ok(node), Ok(effects)) => {
                let mut approved = Vec::new();
                let mut requested_dialog = None;
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
                        _ if effect.get("type").and_then(Value::as_str) == Some("run-submit")
                            && effect_manifest
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
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::RunCommand) =>
                        {
                            approved.push(PluginEffect::RunDismiss);
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
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("toggle-on-screen-keyboard")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::OnScreenKeyboardShow) =>
                        {
                            approved.push(PluginEffect::ToggleOnScreenKeyboard);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("toggle-projects-menu")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::ProjectsMenuShow) =>
                        {
                            approved.push(PluginEffect::ToggleCodexProjects);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-activate-item")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::WindowsFocus)
                            && effect_manifest
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
                            && effect_manifest
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
                            && effect_manifest
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
                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|kind| {
                                matches!(kind, "taskbar-activate-tray" | "tray.activate")
                            })
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
                            .is_some_and(|kind| {
                                matches!(kind, "taskbar-context-tray" | "tray.contextMenu")
                            })
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
                            == Some("taskbar-menu-close-all")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::WindowsContext) =>
                        {
                            approved.push(PluginEffect::CloseTaskbarMenuWindows);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("taskbar-window-menu-action")
                            && effect_manifest
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
                            let item = effect.get("item").and_then(Value::as_str);
                            let valid = slot_id.is_some_and(|value| {
                                effect_manifest.provides_slots.iter().any(|slot| {
                                    slot.id == value
                                        && slot.contract
                                            == nickel_core::plugins::PluginSlotContract::Action
                                })
                            }) && plugin_id
                                .is_some_and(|value| !value.is_empty() && value.len() <= 128)
                                && id.is_some_and(|value| !value.is_empty() && value.len() <= 64);
                            if !valid
                                || effect
                                    .get("item")
                                    .is_some_and(|value| !value.is_null() && !value.is_string())
                                || !item.is_none_or(|value| !value.is_empty() && value.len() <= 256)
                            {
                                self.last_error = Some("plugin slot action is invalid".into());
                                return;
                            }
                            let projected = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok())
                                .and_then(|data| {
                                    if data
                                        .get("slotContext")
                                        .and_then(|context| context.get("item"))
                                        .is_some_and(|context| context.as_str() != item)
                                    {
                                        return None;
                                    }
                                    data.get("slots")?.get(slot_id?)?.as_array().cloned()
                                })
                                .is_some_and(|actions| {
                                    actions.iter().any(|action| {
                                        action.get("pluginId").and_then(Value::as_str) == plugin_id
                                            && action.get("id").and_then(Value::as_str) == id
                                            && action["item"]
                                                .as_str()
                                                .is_none_or(|target| Some(target) == item)
                                    })
                                });
                            if !projected {
                                self.last_error = Some("plugin slot action is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::InvokePluginSlotAction {
                                target_plugin: effect_manifest.id.clone(),
                                slot_id: slot_id.unwrap().to_owned(),
                                plugin_id: plugin_id.unwrap().to_owned(),
                                id: id.unwrap().to_owned(),
                                item: item.map(str::to_owned),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("preview-action") =>
                        {
                            match preview_request(&effect) {
                                Ok((action, capability))
                                    if effect_manifest.capabilities.contains(&capability) =>
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
                            == Some("invoke-plugin-slot-section") =>
                        {
                            let slot_id = effect.get("slot").and_then(Value::as_str);
                            let plugin_id = effect.get("pluginId").and_then(Value::as_str);
                            let id = effect.get("id").and_then(Value::as_str);
                            if !slot_id.is_some_and(|value| {
                                effect_manifest.provides_slots.iter().any(|slot| {
                                    slot.id == value
                                        && slot.contract
                                            == nickel_core::plugins::PluginSlotContract::Section
                                })
                            }) || !plugin_id
                                .is_some_and(|value| !value.is_empty() && value.len() <= 128)
                                || !id.is_some_and(|value| !value.is_empty() && value.len() <= 64)
                            {
                                self.last_error = Some("plugin slot section is invalid".into());
                                return;
                            }
                            let projected = self
                                .projection_data
                                .as_deref()
                                .and_then(|data| serde_json::from_str::<Value>(data).ok())
                                .and_then(|data| {
                                    data.get("slots")?.get(slot_id?)?.as_array().cloned()
                                })
                                .is_some_and(|sections| {
                                    sections.iter().any(|section| {
                                        section.get("pluginId").and_then(Value::as_str) == plugin_id
                                            && section.get("id").and_then(Value::as_str) == id
                                    })
                                });
                            if !projected {
                                self.last_error = Some("plugin slot section is stale".into());
                                return;
                            }
                            approved.push(PluginEffect::InvokePluginSlotSection {
                                target_plugin: effect_manifest.id.clone(),
                                slot_id: slot_id.unwrap().to_owned(),
                                plugin_id: plugin_id.unwrap().to_owned(),
                                id: id.unwrap().to_owned(),
                            });
                        }
                        _ if effect["type"] == "preferences.set" => {
                            let request =
                                crate::preferences_capabilities::PreferencesEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !self
                                            .manifest
                                            .capabilities
                                            .contains(&request.capability())
                                            || !self
                                                .manifest
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
                                    plugin_id: self.manifest.id.clone(),
                                    effect,
                                }),
                                Err(error) => {
                                    self.last_error = Some(error);
                                    return;
                                }
                            }
                        }
                        Some("plugins.selectShell") => {
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
                        Some("plugins.setSetting") => {
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
                                        if !self
                                            .manifest
                                            .capabilities
                                            .contains(&PluginCapability::PluginsRead)
                                            || !self
                                                .manifest
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
                                    plugin_id: self.manifest.id.clone(),
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
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("control-action") =>
                        {
                            match control_request(&effect) {
                                Ok((action, capability))
                                    if effect_manifest.capabilities.contains(&capability) =>
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
                            && effect_manifest.id == codex_projects_manifest().id
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::ProjectsRead) =>
                        {
                            approved.push(PluginEffect::CodexProjectRefresh);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("codex-project-close")
                            && effect_manifest.id == codex_projects_manifest().id =>
                        {
                            approved.push(PluginEffect::CodexProjectClose);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("codex-project-open")
                            && effect_manifest.id == codex_projects_manifest().id
                            && effect_manifest
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
                        _ if effect_manifest
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
                        _ if effect
                            .get("type")
                            .and_then(Value::as_str)
                            .is_some_and(|operation| operation.starts_with("features.")) =>
                        {
                            let request =
                                crate::feature_capabilities::FeatureEffect::parse(&effect)
                                    .and_then(|request| {
                                        if !self
                                            .manifest
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
                            == Some("notification-invoke")
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
                                id,
                                key: key.to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("notification-dismiss")
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
                            approved.push(PluginEffect::DismissNotification { id });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("notification-close-history")
                            && effect_manifest
                                .capabilities
                                .contains(&PluginCapability::NotificationsAct) =>
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
                if !validation_rejected {
                    self.runtime_failure = Some(error.clone());
                }
                self.last_error = Some(error);
            }
        })();
        if let Some(state) = &self.composition {
            if self.last_error.is_some() {
                // Mixed-context renders cannot use the single-context rollback
                // path. Retire participants before any denied effect can execute.
                let owners = state.manifests.keys().cloned().collect::<Vec<_>>();
                for owner in owners {
                    state.host.borrow_mut().retire(&owner);
                }
                self.runtime_failure = self.last_error.clone();
            }
            return;
        }
        if let Err(error) = self
            .runtime
            .borrow_mut()
            .finish_event(self.last_error.is_none())
        {
            self.runtime_failure = Some(error.clone());
            self.last_error = Some(error);
        }
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        if matches!(&self.node, PanelNode::Surface { .. }) {
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
                    overlays.push(FrameOverlay::Menu(self.node.style_overlay_menu_for(
                        id,
                        menu,
                        &self.stylesheet,
                    )));
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
        self.node.window_title().unwrap_or(&self.manifest.name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_ui::Application;

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
    fn embedded_default_shell_surfaces_share_runtime_and_settings_registry() {
        let package = crate::bundled_plugin_assets::load_package("nickel-default").unwrap();
        assert_eq!(package.manifest.entry, "src/Shell.js");
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
            "./src/Plugins.js#Plugins"
        );
        assert!(
            package
                .modules
                .iter()
                .any(|module| module.path == "src/Shell.jsx")
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
        assert_eq!(applications.len(), 5);
        // Constructing additional surfaces must not initialize registration modules again.
        runtime
            .borrow_mut()
            .publish_settings(&mut registry, &package.manifest.id)
            .unwrap();
        assert_eq!(registry.settings_pages_snapshot().pages, pages.pages);
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
            let base = package(
                "base-shell",
                "globalThis.origin = 'base';\nexport function Shell() { return h(Window, {id:'main',placement:'fixed',width:440,height:220,edge:'bottom',bottomOffset:24,output:'all'}, h(nickel.component('shell.taskbar'), {onChange:()=>nickel.request('show-launcher')}, h(Button,{id:'owned-child',onClick:()=>nickel.request('show-launcher')},'Owned child')), nickel.contributions('taskbar.items').map(entry => h(entry.component, {key:entry.key}))); }\nexport function Taskbar() { return h(Button, {id:'base',onClick:()=>nickel.request('show-launcher')}, origin); }\nexport default Shell;",
                None,
                vec![
                    PluginCapability::LauncherShow,
                    PluginCapability::WindowsRead,
                ],
            );
            let mut child = package(
                "derived-shell",
                "globalThis.origin = 'derived';\nexport function Taskbar(props) { const [count,setCount] = useState(0); return h(Column,null,h(Button,{id:'callback-control',onClick:()=>props.onChange('changed')},'Callback'),h(Button, {id:'replacement',onClick:()=>{setCount(count+1); nickel.request('show-launcher');}}, origin + count),...props.children); }\nexport function Widget() { return h(Button, {id:'contribution',onClick:()=>nickel.request('show-launcher')}, 'Owned contribution'); }\nexport default Taskbar;",
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
            } else {
                assert!(application.take_effects().is_empty());
                assert!(application.last_error().is_some());
                // The base grant does not authorize a replacement's callback.
                assert!(application.take_runtime_failure().is_some());
            }
        }
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
        for name in [
            "hello-panel",
            "taskbar",
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
                })
                .collect(),
            tray: Vec::new(),
            clock: "12:00".into(),
            keyboard_enabled: false,
            codex_available: false,
        };
        let app = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::taskbar_manifest(),
            "main.js",
            projection.to_json(),
        )
        .unwrap();
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
    fn desktop_actions_follow_capabilities_instead_of_plugin_identity() {
        let source = r#"function App() { return h(FixedWindow, {width: '100%', height: 56, output: 'all', edge: 'bottom', reserveWorkArea: true},
            h(Button, {id: 'activate', onClick: () => nickel.request({type: 'taskbar-activate-item', index: 0, id: 'app.example'})}, 'Activate')) }"#;
        let mut manifest = taskbar_manifest().clone();
        manifest.id = "org.example.custom-bar".into();
        let mut application =
            PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        application.update(application.button_message("activate").unwrap());
        assert_eq!(
            application.take_effects(),
            vec![PluginEffect::ActivateTaskbarItem {
                index: 0,
                id: "app.example".into(),
            }]
        );

        manifest
            .capabilities
            .retain(|capability| *capability != PluginCapability::WindowsFocus);
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        denied.update(denied.button_message("activate").unwrap());
        assert!(denied.take_effects().is_empty());
        assert!(denied.last_error().is_some());
    }

    #[test]
    fn output_copies_share_one_runtime_with_independent_hook_state() {
        let source = "function App() { const [count, setCount] = useState(0); return h(FixedWindow, {width: '100%', height: 56, output: 'all', edge: 'bottom', reserveWorkArea: true}, h(Button, {id: 'advance', onClick: () => setCount(count + 1)}, `${nickel.data.label}:${count}`)); }";
        let mut left = PluginPanelApplication::new_with_manifest_for_surface_and_scope(
            source,
            taskbar_manifest(),
            Some(serde_json::json!({"label": "left"}).to_string()),
            None,
            None,
            "taskbar-output:left",
        )
        .unwrap();
        let mut right = PluginPanelApplication::new_with_manifest_for_surface_and_scope(
            source,
            taskbar_manifest(),
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
        assert!(format!("{:?}", left.node).contains("left:1"));
        assert!(format!("{:?}", right.node).contains("right refreshed:0"));
        left.retire_surface().unwrap();
        right.update(right.button_message("advance").unwrap());
        assert!(format!("{:?}", right.node).contains("right refreshed:1"));
    }

    #[test]
    fn bundled_taskbar_visual_snapshot() {
        let projection = TaskbarPluginProjection {
            items: ["Files", "Browser", "Editor"]
                .into_iter()
                .enumerate()
                .map(|(index, name)| TaskbarPluginItem {
                    index,
                    id: name.to_lowercase(),
                    name: name.into(),
                    active: index == 1,
                    pinned: true,
                    icon: false,
                })
                .collect(),
            tray: vec![TaskbarPluginTrayItem {
                id: "network".into(),
                title: "Network".into(),
                icon: false,
            }],
            clock: "12:45".into(),
            keyboard_enabled: true,
            codex_available: true,
        };
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::taskbar_manifest(),
                "main.js",
                projection.to_json(),
            )
            .unwrap(),
            960,
            56,
        );
        let editor = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Editor".into(),
            })
            .unwrap();
        let keyboard = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "On-screen keyboard".into(),
            })
            .unwrap();
        assert!(
            keyboard.bounds.origin.x > editor.bounds.origin.x + editor.bounds.size.width + 100.0
        );
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(960, 56, 1.0);
        host.render_software(&mut renderer);
        let clear_area = renderer.pixels()[(2 * 960 + 600) as usize];
        assert!(
            (1..255).contains(&clear_area.a),
            "taskbar background lost its CSS alpha: {clear_area:?}"
        );
        let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(960, 56, |x, y| {
            let pixel = renderer.pixels()[(y * 960 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        });
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/taskbar-shared.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
    }

    #[test]
    fn bundled_volume_osd_visual_snapshot() {
        let projection = serde_json::json!({
            "audio": {"label": "Speakers · 65%", "percent": 65}
        });
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::volume_osd_manifest(),
                "main.js",
                projection.to_string(),
            )
            .unwrap(),
            420,
            96,
        );
        assert!(matches!(
            host.application().node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(420, 96, 1.0);
        host.render_software(&mut renderer);
        let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(420, 96, |x, y| {
            let pixel = renderer.pixels()[(y * 420 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        });
        assert_eq!(image.get_pixel(20, 47).0, [123, 166, 255, 255]);
        assert_eq!(image.get_pixel(300, 47).0, [74, 82, 98, 255]);
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/volume-osd-shared.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
    }

    #[test]
    fn bundled_control_center_uses_shared_window_and_keeps_controls() {
        let package = PluginPackage::load(format!(
            "{}/../../assets/plugins/control-center",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let data = validation_surface_projection(&package, control_center_surface()).unwrap();
        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::control_center_manifest(),
                "main.js",
                data.to_string(),
            )
            .unwrap(),
            420,
            600,
        );
        assert!(matches!(
            host.application().node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        for name in ["Mute", "Show desktop", "Lock", "Restart Nickel"] {
            assert!(
                host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: name.into(),
                })
                .is_ok()
            );
        }
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(420, 600, 1.0);
        host.render_software(&mut renderer);
        let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(420, 600, |x, y| {
            let pixel = renderer.pixels()[(y * 420 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        });
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/control-center-shared.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
        host.step(nickel_ui::HostBatch {
            events: vec![nickel_ui::HostEvent::Shortcut(Shortcut::Escape)],
            ..Default::default()
        });
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::ToggleControlCenter]
        );
    }

    #[test]
    fn bundled_keyboard_uses_shared_window_and_keeps_key_actions() {
        let package = PluginPackage::load(format!(
            "{}/../../assets/plugins/on-screen-keyboard",
            env!("CARGO_MANIFEST_DIR")
        ))
        .unwrap();
        let data = validation_surface_projection(&package, &package.manifest.surfaces[0]).unwrap();
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::on_screen_keyboard_manifest(),
                "main.js",
                data.to_string(),
            )
            .unwrap(),
            1056,
            368,
        );
        assert!(matches!(
            host.application().node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        for name in ["Hold modifiers", "Hide", "Smaller"] {
            assert!(
                host.query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: name.into(),
                })
                .is_ok()
            );
        }
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(1056, 368, 1.0);
        host.render_software(&mut renderer);
        let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(1056, 368, |x, y| {
            let pixel = renderer.pixels()[(y * 1056 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        });
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/keyboard-shared.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
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
    fn taskbar_action_contribution_dispatches_its_own_granted_callback() {
        let package = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-task-action"
        ))
        .unwrap();
        PluginPanelApplication::validate_package(&package).unwrap();
        let mut application = PluginPanelApplication::from_package(&package).unwrap();
        assert_eq!(
            application.action_contributions().unwrap()[0].id,
            "find-apps"
        );
        assert!(application.activate_action("find-apps", "org.nickel.mail"));
        assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
        assert!(!application.activate_action("missing", "org.nickel.mail"));
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
        assert_eq!(
            application.section_contributions().unwrap()[0].id,
            "find-apps"
        );
        assert!(application.activate_section("find-apps"));
        assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);
        assert!(!application.activate_section("missing"));
        assert!(application.take_effects().is_empty());
    }

    #[test]
    fn contribution_callbacks_work_inside_generic_component_containers() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins");
        let mut action = PluginPackage::load(format!("{root}/example-task-action")).unwrap();
        let plain_bytes = PluginPanelApplication::from_package(&action)
            .unwrap()
            .retained_contribution_bytes();
        action.source = r#"
            function App() {
                return h(Div, {className: 'contribution'},
                    h(Box, {x: 0, y: 0, width: 80, height: 32},
                        h(Action, {id: 'find-apps', label: 'Find apps',
                            onClick: () => nickel.request('show-launcher')})));
            }
        "#
        .into();
        let mut application = PluginPanelApplication::from_package(&action).unwrap();
        assert!(application.retained_contribution_bytes() > plain_bytes);
        assert_eq!(
            application.action_contributions().unwrap()[0].id,
            "find-apps"
        );
        assert!(application.activate_action("find-apps", "org.nickel.mail"));
        assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);

        let mut section = PluginPackage::load(format!("{root}/example-control-section")).unwrap();
        section.source = r#"
            function App() {
                return h(Div, {}, h(Column, {},
                    h(Section, {id: 'find-apps', label: 'Applications', value: 'Search',
                        onClick: () => nickel.request('show-launcher')})));
            }
        "#
        .into();
        let mut application = PluginPanelApplication::from_package(&section).unwrap();
        assert_eq!(
            application.section_contributions().unwrap()[0].id,
            "find-apps"
        );
        assert!(application.activate_section("find-apps"));
        assert_eq!(application.take_effects(), vec![PluginEffect::ShowLauncher]);

        let mut badge = PluginPackage::load(format!("{root}/example-task-badge")).unwrap();
        badge.source = r#"
            function App() {
                return h(Div, {}, h(Badge,
                    {item: 'org.example.mail', label: 'Unread mail', count: 3}));
            }
        "#
        .into();
        let application = PluginPanelApplication::from_package(&badge).unwrap();
        assert_eq!(application.badge_contributions().unwrap()[0].2, 3);

        let mut widget = PluginPackage::load(format!("{root}/example-widget-contributor")).unwrap();
        widget.source = r#"
            function App() {
                return h(Div, {}, h(Widget,
                    {label: 'Unread mail', value: '3', percent: 50}));
            }
        "#
        .into();
        let application = PluginPanelApplication::from_package(&widget).unwrap();
        assert_eq!(
            application.widget_contributions().unwrap()[0].label,
            "Unread mail"
        );
    }

    #[test]
    fn installed_action_slot_dispatches_only_a_projected_contributors_action() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins");
        let provider = PluginPackage::load(format!("{root}/example-widget-host")).unwrap();
        let contributor =
            PluginPackage::load(format!("{root}/example-action-contributor")).unwrap();
        PluginPanelApplication::validate_package(&provider).unwrap();
        PluginPanelApplication::validate_package(&contributor).unwrap();
        let mut scoped = contributor.clone();
        scoped.source = scoped.source.replace(
            "label: \"Open launcher\"",
            "item: \"mail\", label: \"Open launcher\"",
        );
        let scoped = PluginPanelApplication::from_package(&scoped).unwrap();
        scoped.validate_contribution().unwrap();
        assert_eq!(
            scoped.action_contributions().unwrap()[0].item.as_deref(),
            Some("mail")
        );
        let mut contributor = PluginPanelApplication::from_package(&contributor).unwrap();
        assert_eq!(
            contributor.action_contributions().unwrap()[0].id,
            "open-launcher"
        );
        assert!(contributor.activate_action("open-launcher", ""));
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
        assert!(button.bounds.size.width > 0.0);
        assert!(button.bounds.size.height > 0.0);
        let app = host.application_mut();
        app.update(stale_click.clone());
        assert_eq!(
            app.take_effects(),
            vec![PluginEffect::InvokePluginSlotAction {
                target_plugin: provider.manifest.id.clone(),
                slot_id: "commands".into(),
                plugin_id: "org.example.action-contributor".into(),
                id: "open-launcher".into(),
                item: None,
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
    fn item_scoped_slot_action_requires_the_projected_item() {
        let root = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/plugins");
        let provider = PluginPackage::load(format!("{root}/example-widget-host")).unwrap();
        let source = r#"
            function App() {
                return h(Window, {width: 420, height: 280},
                    h(Button, {id: 'invoke', onClick: () => nickel.request({
                        type: 'invoke-plugin-slot-action', slot: 'commands',
                        pluginId: 'org.example.action-contributor', id: 'open-launcher',
                        item: 'mail'
                    })}, 'Invoke'));
            }
        "#;
        let projection = serde_json::json!({
            "slotContext": {"item": "mail"},
            "slots": {"commands": [{
                "pluginId": "org.example.action-contributor",
                "id": "open-launcher", "label": "Open launcher", "item": "mail"
            }]}
        });
        let mut app = PluginPanelApplication::new_with_manifest(
            source,
            &provider.manifest,
            Some(projection.to_string()),
        )
        .unwrap();
        app.update(app.button_message("invoke").unwrap());
        assert_eq!(
            app.take_effects(),
            vec![PluginEffect::InvokePluginSlotAction {
                target_plugin: provider.manifest.id.clone(),
                slot_id: "commands".into(),
                plugin_id: "org.example.action-contributor".into(),
                id: "open-launcher".into(),
                item: Some("mail".into()),
            }]
        );
        let mut forged = PluginPanelApplication::new_with_manifest(
            &source.replace("item: 'mail'", "item: 'other'"),
            &provider.manifest,
            Some(projection.to_string()),
        )
        .unwrap();
        forged.update(forged.button_message("invoke").unwrap());
        assert!(forged.take_effects().is_empty());
        assert_eq!(forged.last_error(), Some("plugin slot action is stale"));
        app.sync_external_slots(&serde_json::json!({"commands": [{
            "pluginId": "org.example.action-contributor",
            "id": "open-launcher", "label": "Open launcher", "item": "other"
        }]}))
        .unwrap();
        app.update(app.button_message("invoke").unwrap());
        assert!(app.take_effects().is_empty());
        assert_eq!(app.last_error(), Some("plugin slot action is stale"));
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
        let app = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::window_preview_manifest(),
            "main.js",
            data.to_string(),
        )
        .unwrap();
        assert!(matches!(
            &app.node,
            PanelNode::Surface {
                window_request: Some(_),
                width: Length::Percent(1.0),
                height: Length::Percent(1.0),
                ..
            }
        ));
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

        let two_windows = serde_json::json!({"windows": [
            {"id":"71","title":"Document","accessibleName":"Document","closable":true,"imageWidth":244,"selected":false},
            {"id":"72","title":"Mail","accessibleName":"Mail","closable":true,"imageWidth":244,"selected":false}
        ]});
        let wide = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::window_preview_manifest(),
                "main.js",
                two_windows.to_string(),
            )
            .unwrap(),
            600,
            214,
        );
        let first = wide
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Document".into(),
            })
            .unwrap();
        let second = wide
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Mail".into(),
            })
            .unwrap();
        assert!(second.bounds.origin.x > first.bounds.origin.x);
        assert!(second.bounds.origin.x + second.bounds.size.width <= 600.0);
    }

    #[test]
    fn independent_jsx_sliders_dispatch_their_own_values() {
        let source = "function App() { const [hue, setHue] = useState(0.2); const [intensity, setIntensity] = useState(0.6); return h(Panel, {}, h(Slider, {value: hue, accessibilityLabel: 'Hue', className: 'hue', onChange: setHue}), h(Slider, {value: intensity, accessibilityLabel: 'Intensity', onChange: setIntensity})); }";
        let mut host =
            nickel_ui::UiHost::new(PluginPanelApplication::new(source).unwrap(), 440, 220);
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
        let PanelNode::Surface { children, .. } = &host.application_mut().node else {
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
        let PanelNode::Surface { children, .. } = &host.application_mut().node else {
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
        let PanelNode::Surface { children, .. } = &host.application_mut().node else {
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
    fn packaged_panel_aligns_at_bottom_and_dispatches_dialog_action() {
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
            &host.application().node,
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
    }

    #[test]
    fn fixed_window_jsx_helper_uses_one_manifest_checked_window_root() {
        let mut granted = taskbar_manifest().clone();
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
            "progress.meter { background: #112233; color: #aabbcc; border-radius: 4px; }",
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
        app.update(app.button_message("toggle").unwrap());
        assert!(app.last_error().unwrap().contains("exceeds its grant"));
        assert!(app.take_effects().is_empty());
        assert!(matches!(
            &app.node,
            PanelNode::Surface { class_name: Some(class_name), .. } if class_name == "good"
        ));
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
        assert!(format!("{:?}", app.node).contains("Before"));
        assert!(
            app.sync_data(&serde_json::json!({"invalid": false, "label": "After"}))
                .unwrap()
        );
        assert!(app.last_error().is_none());
        assert!(format!("{:?}", app.node).contains("After"));
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
        assert!(format!("{:?}", app.node).contains("65"));
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
    fn external_run_command_uses_capability_instead_of_plugin_identity() {
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
                    h(Button, {id: 'run', onClick: () => nickel.request({type: 'run-submit', command: 'nickel-test'})}, 'Run'));
            }
        "#;
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        denied.update(denied.button_message("run").unwrap());
        assert!(denied.take_effects().is_empty());
        assert!(denied.last_error().is_some());

        manifest.capabilities.push(PluginCapability::RunCommand);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, None).unwrap();
        granted.update(granted.button_message("run").unwrap());
        assert_eq!(
            granted.take_effects(),
            vec![PluginEffect::RunSubmit("nickel-test".into())]
        );

        let dismiss_source = "function App() { return h(Window, {width: 320, height: 180, onEscape: () => nickel.request({type: 'run-dismiss'})}); }";
        manifest.capabilities.clear();
        let mut denied =
            PluginPanelApplication::new_with_manifest(dismiss_source, &manifest, None).unwrap();
        denied.shortcut_outcome(Shortcut::Escape);
        assert!(denied.take_effects().is_empty());
        assert!(denied.last_error().is_some());
        manifest.capabilities.push(PluginCapability::RunCommand);
        let mut granted =
            PluginPanelApplication::new_with_manifest(dismiss_source, &manifest, None).unwrap();
        granted.shortcut_outcome(Shortcut::Escape);
        assert_eq!(granted.take_effects(), vec![PluginEffect::RunDismiss]);
    }

    #[test]
    fn external_keyboard_action_uses_capability_and_current_projection() {
        let mut manifest = PluginPackage::load(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../assets/plugins/example-window"
        ))
        .unwrap()
        .manifest;
        manifest.capabilities.clear();
        let source = "function App() { return h(Window, {width: 320, height: 180, onEscape: () => nickel.request({type: 'keyboard-hide', generation: 7})}); }";
        let data = Some(r#"{"generation":7,"rows":[]}"#.to_owned());
        let mut denied =
            PluginPanelApplication::new_with_manifest(source, &manifest, data.clone()).unwrap();
        denied.shortcut_outcome(Shortcut::Escape);
        assert!(denied.take_effects().is_empty());
        assert!(denied.last_error().is_some());

        manifest
            .capabilities
            .push(PluginCapability::OnScreenKeyboardInput);
        let mut granted =
            PluginPanelApplication::new_with_manifest(source, &manifest, data).unwrap();
        granted.shortcut_outcome(Shortcut::Escape);
        assert_eq!(
            granted.take_effects(),
            vec![PluginEffect::KeyboardHide { generation: 7 }]
        );
        granted
            .sync_data(&serde_json::json!({"generation": 8, "rows": []}))
            .unwrap();
        granted.shortcut_outcome(Shortcut::Escape);
        assert!(granted.take_effects().is_empty());
        assert_eq!(granted.last_error(), Some("keyboard request is stale"));
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
        assert!(format!("{:?}", read_only.node).contains("Editor"));
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
                    h(Button, {id: 'dismiss', onClick: () => nickel.request({type: 'notification-dismiss', id: 71})}, 'Dismiss'));
            }
        "#;
        let projection = serde_json::json!({
            "notification": {"id":71,"appName":"Mail","summary":"New mail","body":"Hello","actions":[]},
            "history":[],"historyVisible":false
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
        assert!(format!("{:?}", read_only.node).contains("New mail"));
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
            vec![PluginEffect::DismissNotification { id: 71 }]
        );
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
        assert!(matches!(
            host.application().node,
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
            home.application().node,
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
            dialog.application().node,
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
            home.application().node,
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
            overlay.application().node,
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
            modules: Vec::new(),
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
        let mut panel = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::run_manifest(),
            "main.js",
            serde_json::json!({ "status": null }).to_string(),
        )
        .unwrap();
        assert!(matches!(
            &panel.node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        panel.update(PluginMessage::Text(0, "  nickel-test  ".into()));
        let submit = panel.node.button_action("run-submit").unwrap();
        panel.update(PluginMessage::Click(submit));
        assert_eq!(
            panel.take_effects(),
            vec![PluginEffect::RunSubmit("nickel-test".into())]
        );
        assert!(
            panel
                .sync_data(&serde_json::json!({ "status": "Could not run command: missing" }))
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
        let mut panel = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::run_manifest(),
            "main.js",
            serde_json::json!({ "status": null }).to_string(),
        )
        .unwrap();
        assert_eq!(
            panel.shortcut_outcome(Shortcut::Escape).disposition,
            nickel_ui::EventDisposition::Handled
        );
        assert_eq!(panel.take_effects(), vec![PluginEffect::RunDismiss]);
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
    fn external_shell_toggles_use_capabilities_instead_of_taskbar_identity() {
        for (action, capability, expected) in [
            (
                "show-control-center",
                PluginCapability::ControlCenterShow,
                PluginEffect::ShowControlCenter,
            ),
            (
                "toggle-launcher",
                PluginCapability::LauncherShow,
                PluginEffect::ToggleLauncher,
            ),
            (
                "toggle-on-screen-keyboard",
                PluginCapability::OnScreenKeyboardShow,
                PluginEffect::ToggleOnScreenKeyboard,
            ),
            (
                "toggle-projects-menu",
                PluginCapability::ProjectsMenuShow,
                PluginEffect::ToggleCodexProjects,
            ),
        ] {
            let mut external_manifest = manifest().clone();
            external_manifest.id = format!("org.example.{action}");
            external_manifest.capabilities.clear();
            let mut package = PluginPackage {
                modules: Vec::new(),
                manifest: external_manifest,
                images: Default::default(),
                stylesheet: String::new(),
                source: format!(
                    "function App() {{ return h(FixedWindow, {{width: '100%', height: '100%', onEscape: () => nickel.request({{type: '{action}'}})}}); }}"
                ),
            };
            let mut denied = PluginPanelApplication::from_package(&package).unwrap();
            denied.shortcut_outcome(Shortcut::Escape);
            assert!(denied.take_effects().is_empty(), "{action}");
            package.manifest.capabilities.push(capability);
            let mut granted = PluginPanelApplication::from_package(&package).unwrap();
            granted.shortcut_outcome(Shortcut::Escape);
            assert_eq!(granted.take_effects(), vec![expected], "{action}");
        }
    }

    #[test]
    fn external_preview_and_control_actions_use_grants_instead_of_plugin_identity() {
        for (request, capability, expected) in [
            (
                "{type: 'preview-action', action: 'activate', window: '71'}",
                PluginCapability::WindowsFocus,
                PluginEffect::Preview(PreviewAction::Activate(crate::model::WindowId(71))),
            ),
            (
                "{type: 'control-action', action: 'audio-volume', value: 25}",
                PluginCapability::AudioControl,
                PluginEffect::Control(ControlAction::SetAudioVolume(25)),
            ),
        ] {
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
            let mut granted = PluginPanelApplication::from_package(&package).unwrap();
            granted.shortcut_outcome(Shortcut::Escape);
            assert_eq!(granted.take_effects(), vec![expected]);
        }
    }

    #[test]
    fn run_plugin_host_accepts_text_and_submit_from_focused_field() {
        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::run_manifest(),
                "main.js",
                serde_json::json!({ "status": null }).to_string(),
            )
            .unwrap(),
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
        let mut panel = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::codex_projects_manifest(),
            "main.js",
            serde_json::to_string(&projection).unwrap(),
        )
        .unwrap();
        assert!(matches!(
            panel.node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        let rendered = format!("{:?}", panel.node);
        assert!(rendered.contains("Example project"));
        assert!(!rendered.contains("/private/work"));
        assert!(
            !panel
                .sync_serialized_data(serde_json::to_string(&projection).unwrap())
                .unwrap()
        );
        let mut disconnected = projection.clone();
        disconnected.status = "disconnected";
        disconnected.projects.clear();
        assert!(
            panel
                .sync_serialized_data(serde_json::to_string(&disconnected).unwrap())
                .unwrap()
        );
        assert!(!format!("{:?}", panel.node).contains("Example project"));

        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::codex_projects_manifest(),
                "main.js",
                serde_json::to_string(&projection).unwrap(),
            )
            .unwrap(),
            520,
            680,
        );
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(520, 680, 1.0);
        host.render_software(&mut renderer);
        let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(520, 680, |x, y| {
            let pixel = renderer.pixels()[(y * 520 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        });
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/codex-projects-shared.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
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
        let mut plugin = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::on_screen_keyboard_manifest(),
            "main.js",
            data.to_string(),
        )
        .unwrap();
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
        assert_eq!(
            plugin.shortcut_outcome(Shortcut::Escape).disposition,
            nickel_ui::EventDisposition::Handled
        );
        assert_eq!(
            plugin.take_effects(),
            vec![PluginEffect::KeyboardHide { generation: 7 }]
        );
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
        assert!(plugin.sync_data(&disabled).unwrap());
        let message = plugin.button_message("osk-char-113").unwrap();
        plugin.update(message);
        assert!(plugin.take_effects().is_empty());
        plugin.shortcut_outcome(Shortcut::Escape);
        assert_eq!(
            plugin.take_effects(),
            vec![PluginEffect::KeyboardHide { generation: 8 }]
        );
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
