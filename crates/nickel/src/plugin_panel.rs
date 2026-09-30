//! Experimental JavaScript panel host. The bundled example uses the same small
//! component vocabulary as an external plugin; native surfaces remain shell-owned.

use std::{
    borrow::Cow,
    collections::HashSet,
    sync::{Arc, OnceLock},
};

use nickel_core::plugins::{
    PluginCapability, PluginManifest, PluginPackage, PluginSurface, PluginSurfaceKind,
};
pub use nickel_plugin_presentation::components::{
    DesktopPluginWidget, PluginImages, PluginMessage, PluginSectionContribution,
    TaskbarPluginAction,
};
use nickel_plugin_presentation::components::{PanelNode, render_panel};
use nickel_plugin_runtime::JsxRuntime;
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
    overlay_open: bool,
    dispatch_removed_focus: bool,
    images: PluginImages,
    stylesheet: StyleSheet,
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
    SetLauncherQuery(String),
    SetLauncherPage {
        dashboard: bool,
        page: usize,
    },
    ActivateLauncherResult {
        index: usize,
        id: String,
    },
    LaunchApplication {
        id: String,
    },
    SetLauncherView(LauncherView),
    ToggleApplicationPin {
        id: String,
    },
    RetryApplicationPinSave,
    DismissLauncher,
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
    pub pin_save_failed: bool,
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
pub struct VolumeOsdPluginProjection {
    pub label: String,
    pub percent: u8,
}

impl VolumeOsdPluginProjection {
    pub(crate) fn to_json(&self) -> String {
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

    pub(crate) fn to_json(&self) -> String {
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
            pin_save_failed: false,
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
        self.pin_save_failed = status
            .as_deref()
            .is_some_and(|status| status.starts_with("Launcher preferences could not be saved:"));
        self.status = status.map(|status| status.chars().take(160).collect());
        self
    }

    pub(crate) fn to_json(&self) -> String {
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
        serde_json::json!({"query": self.query, "status": self.status, "pinSaveFailed": self.pin_save_failed, "dashboardVisible": self.dashboard_visible, "view": view,
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

impl PluginPanelApplication {
    pub fn resolved_surface(&self, grant: &PluginSurface) -> PluginSurface {
        let mut surface = grant.clone();
        if let Some((width, height)) = self.node.requested_window_size(grant) {
            surface.width = width;
            surface.height = height;
        }
        surface
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
        Ok(application)
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
        let data =
            serde_json::json!({ "settings": settings, "slots": {}, "windows": [] }).to_string();
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
            "windows": [],
            "applications": [],
            "notifications": initial_notifications_data(&package.manifest),
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

    pub fn badge_contributions(&self) -> Result<Vec<(String, String, u16, u32)>, String> {
        let mut badges = Vec::new();
        self.node.collect_badges(&mut badges)?;
        if badges.is_empty() {
            return Err("badge extension did not return a badge".into());
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
                _ => node
                    .container_children()?
                    .iter()
                    .find_map(|child| find(child, id, application_id)),
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
            PluginSlotContract::Badge => self.badge_contributions().map(|_| ()),
            PluginSlotContract::Widget => self.desktop_widgets().map(|_| ()),
            PluginSlotContract::Action => self.taskbar_actions().map(|_| ()),
            PluginSlotContract::Section => self.section_contributions().map(|_| ()),
        }
    }

    #[cfg(test)]
    pub(crate) fn launcher_with_test_source(
        source: &str,
        projection: &LauncherPluginProjection,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, launcher_manifest(), Some(projection.to_json()))
    }

    #[cfg(test)]
    pub(crate) fn taskbar_with_test_source(
        source: &str,
        projection: &TaskbarPluginProjection,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, taskbar_manifest(), Some(projection.to_json()))
    }

    #[cfg(test)]
    pub(crate) fn notification_with_test_source(
        source: &str,
        projection: &NotificationPluginProjection,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, notification_manifest(), Some(projection.to_json()))
    }

    #[cfg(test)]
    pub(crate) fn volume_osd_with_test_source(
        source: &str,
        projection: &VolumeOsdPluginProjection,
    ) -> Result<Self, String> {
        Self::new_with_manifest(source, volume_osd_manifest(), Some(projection.to_json()))
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

    pub(crate) fn sync_serialized_data(&mut self, serialized: String) -> Result<bool, String> {
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
            overlay_open: false,
            dispatch_removed_focus: false,
            images: PluginImages::new(),
            stylesheet: StyleSheet::default(),
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
        if fields.iter().any(|(field, _)| {
            !matches!(
                *field,
                "slots" | "windows" | "applications" | "notifications"
            )
        }) {
            return Err("unknown host data field".into());
        }
        if fields.iter().any(|(field, _)| *field == "notifications")
            && !self
                .manifest
                .capabilities
                .contains(&PluginCapability::NotificationsRead)
        {
            return Err("notification data requires notifications.read".into());
        }
        if fields.iter().any(|(field, _)| *field == "applications")
            && !self
                .manifest
                .capabilities
                .contains(&PluginCapability::ApplicationsRead)
        {
            return Err("application data requires applications-read".into());
        }
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

    pub fn set_overlay_open(&mut self, open: bool) {
        self.overlay_open = open;
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
                PluginMessage::Scroll => None,
            })
            .collect::<Vec<_>>();
        if events.is_empty() {
            return;
        }
        let expression = if self.dispatch_removed_focus {
            format!("__nickelDispatchRemovedFocus({})", events[0][0])
        } else {
            format!(
                "__nickelDispatchBatch({})",
                serde_json::Value::Array(events)
            )
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
                            == Some("dismiss-launcher")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::LauncherShow) =>
                        {
                            approved.push(PluginEffect::DismissLauncher);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("show-settings")
                            && self
                                .manifest
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
                        _ if effect.get("type").and_then(Value::as_str) == Some("run-submit")
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
                            == Some("toggle-launcher")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::LauncherShow) =>
                        {
                            approved.push(PluginEffect::ToggleLauncher);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("toggle-control-center")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ControlCenterShow) =>
                        {
                            approved.push(PluginEffect::ToggleControlCenter);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("show-control-center")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ControlCenterShow) =>
                        {
                            approved.push(PluginEffect::ShowControlCenter);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("window-action") =>
                        {
                            let action = effect.get("action").and_then(Value::as_str);
                            let window = effect
                                .get("window")
                                .and_then(Value::as_str)
                                .and_then(|value| value.parse::<u64>().ok())
                                .filter(|id| *id != 0)
                                .map(crate::model::WindowId);
                            let requested = match (action, window) {
                                (Some("activate"), Some(window))
                                    if self
                                        .manifest
                                        .capabilities
                                        .contains(&PluginCapability::WindowsFocus) =>
                                {
                                    Some(PluginEffect::ActivateWindow(window))
                                }
                                (Some("close"), Some(window))
                                    if self
                                        .manifest
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
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::OnScreenKeyboardShow) =>
                        {
                            approved.push(PluginEffect::ToggleOnScreenKeyboard);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("toggle-projects-menu")
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
                            == Some("invoke-plugin-slot-action") =>
                        {
                            let slot_id = effect.get("slot").and_then(Value::as_str);
                            let plugin_id = effect.get("pluginId").and_then(Value::as_str);
                            let id = effect.get("id").and_then(Value::as_str);
                            let item = effect.get("item").and_then(Value::as_str);
                            let valid = slot_id.is_some_and(|value| {
                                self.manifest.provides_slots.iter().any(|slot| {
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
                                target_plugin: self.manifest.id.clone(),
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
                            == Some("invoke-plugin-slot-section") =>
                        {
                            let slot_id = effect.get("slot").and_then(Value::as_str);
                            let plugin_id = effect.get("pluginId").and_then(Value::as_str);
                            let id = effect.get("id").and_then(Value::as_str);
                            if !slot_id.is_some_and(|value| {
                                self.manifest.provides_slots.iter().any(|slot| {
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
                                target_plugin: self.manifest.id.clone(),
                                slot_id: slot_id.unwrap().to_owned(),
                                plugin_id: plugin_id.unwrap().to_owned(),
                                id: id.unwrap().to_owned(),
                            });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("control-action") =>
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
                            == Some("applications.launch")
                            && self
                                .manifest
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
                            == Some("launcher-set-view")
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
                            == Some("applications.togglePin")
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
                            approved.push(PluginEffect::ToggleApplicationPin { id: id.to_owned() });
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("applications-retry-pin-save")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ApplicationsPin) =>
                        {
                            approved.push(PluginEffect::RetryApplicationPinSave);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-open-project")
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
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::ProjectsRead) =>
                        {
                            approved.push(PluginEffect::LauncherSeeAllProjects);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("launcher-request-logout")
                            && self
                                .manifest
                                .capabilities
                                .contains(&PluginCapability::SessionLogoutRequest) =>
                        {
                            approved.push(PluginEffect::LauncherRequestLogout);
                        }
                        _ if effect.get("type").and_then(Value::as_str)
                            == Some("notification-invoke")
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
                            && self
                                .manifest
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
        self.node.window_title().unwrap_or(&self.manifest.name)
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
    fn bundled_plugin_packages_validate_with_manifest_sample_data() {
        for name in [
            "hello-panel",
            "taskbar",
            "launcher",
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
        let projection = VolumeOsdPluginProjection {
            label: "Speakers · 65%".into(),
            percent: 65,
        };
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::volume_osd_manifest(),
                "main.js",
                projection.to_json(),
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
    fn bundled_launcher_visual_snapshot() {
        let launcher = Launcher::default();
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                launcher_manifest(),
                "main.js",
                LauncherPluginProjection::from_launcher(&launcher).to_json(),
            )
            .unwrap(),
            920,
            680,
        );
        assert_eq!(host.application().title(), "Nickel Launcher");
        let firefox = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Firefox".into(),
            })
            .unwrap();
        let files = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Files".into(),
            })
            .unwrap();
        assert!(firefox.bounds.size.width >= 64.0);
        assert!(files.bounds.origin.x > firefox.bounds.origin.x + firefox.bounds.size.width);
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(920, 680, 1.0);
        host.render_software(&mut renderer);
        let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(920, 680, |x, y| {
            let pixel = renderer.pixels()[(y * 920 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        });
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/launcher-shared.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
    }

    #[test]
    fn bundled_launcher_css_tracks_light_and_dark_palettes() {
        let launcher = Launcher::default();
        let mut app = PluginPanelApplication::bundled_with_data(
            launcher_manifest(),
            "main.js",
            LauncherPluginProjection::from_launcher(&launcher).to_json(),
        )
        .unwrap();
        let dark = nickel_core::theme::ThemePalette::from_appearance(
            nickel_core::theme::Appearance::default(),
        );
        let light =
            nickel_core::theme::ThemePalette::from_appearance(nickel_core::theme::Appearance {
                mode: nickel_core::theme::ThemeMode::Light,
                ..nickel_core::theme::Appearance::default()
            });
        let background = |app: &PluginPanelApplication| {
            app.stylesheet
                .resolve("window", None, Some("launcher-window"))
                .background
        };
        let dark_background = background(&app).expect("dark window background");
        assert_eq!(
            app.stylesheet.resolve("text", None, None).color,
            Some(0xff00_0000 | dark.text)
        );
        assert!(app.sync_theme_palette(light).unwrap());
        let light_background = background(&app).expect("light window background");
        assert_ne!(dark_background, light_background);
        assert_eq!(
            app.stylesheet.resolve("text", None, None).color,
            Some(0xff00_0000 | light.text)
        );
        let host = nickel_ui::UiHost::new(app, 920, 680);
        let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(920, 680, 1.0);
        host.render_software(&mut renderer);
        let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(920, 680, |x, y| {
            let pixel = renderer.pixels()[(y * 920 + x) as usize];
            image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
        });
        let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../target/nickel-ui-snapshots/launcher-light.png");
        std::fs::create_dir_all(output.parent().unwrap()).unwrap();
        image.save(output).unwrap();
    }

    #[test]
    fn bundled_launcher_grid_reflows_with_a_narrow_host() {
        let host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                launcher_manifest(),
                "main.js",
                LauncherPluginProjection::from_launcher(&Launcher::default()).to_json(),
            )
            .unwrap(),
            600,
            600,
        );
        for name in ["Firefox", "Files", "Nickel Terminal", "Discover"] {
            let button = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: SemanticRole::Button,
                    name: name.into(),
                })
                .unwrap();
            assert!(
                button.bounds.origin.x + button.bounds.size.width <= 600.0,
                "{name} overflowed: {button:?}"
            );
        }
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
        assert_eq!(application.taskbar_actions().unwrap()[0].id, "find-apps");
        assert!(application.activate_taskbar_action("find-apps", "org.nickel.mail"));
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
            application.desktop_widgets().unwrap()[0].label,
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
            scoped.taskbar_actions().unwrap()[0].item.as_deref(),
            Some("mail")
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
    fn generic_div_uses_css_grid_tracks_for_plugin_controls() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.css-grid".into();
        let package = PluginPackage {
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
    fn generic_div_flex_uses_explicit_css_dimensions_for_alignment() {
        let package = PluginPackage {
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
    fn row_and_column_classes_apply_css_flex_alignment() {
        let package = PluginPackage {
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
    fn shared_window_root_tracks_resize_and_keeps_css_content_inset() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.window-resize".into();
        let package = PluginPackage {
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
            let source = format!(
                "function App() {{ return h(Window, {{id:'main',width:520,height:340}}, h(Button, {{id:'action',onClick:()=>nickel.request({{type:'window-action',action:'{action}',window:'71'}})}}, 'Act')); }}"
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
        let resolved = application.resolved_surface(&home);
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
    fn external_launcher_dismiss_requires_capability() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.launcher-dismiss".into();
        external_manifest.capabilities.clear();
        let mut package = PluginPackage {
            manifest: external_manifest,
            images: Default::default(),
            stylesheet: String::new(),
            source: "function App() { return h(FixedWindow, {width: '100%', height: '100%', onEscape: () => nickel.request({type: 'dismiss-launcher'})}); }".into(),
        };
        let mut denied = PluginPanelApplication::from_package(&package).unwrap();
        denied.shortcut_outcome(Shortcut::Escape);
        assert!(denied.take_effects().is_empty());
        package
            .manifest
            .capabilities
            .push(PluginCapability::LauncherShow);
        let mut granted = PluginPanelApplication::from_package(&package).unwrap();
        granted.shortcut_outcome(Shortcut::Escape);
        assert_eq!(granted.take_effects(), vec![PluginEffect::DismissLauncher]);
    }

    #[test]
    fn external_control_center_toggle_requires_capability() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.control-toggle".into();
        let source = "function App() { return h(FixedWindow, {width: '100%', height: '100%', onEscape: () => nickel.request({type: 'toggle-control-center'})}); }";
        let mut package = PluginPackage {
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
    fn launcher_plugin_submit_does_not_bypass_open_menu() {
        let mut launcher = Launcher::new(vec![crate::launcher::Application::new(
            "org.nickel.demo".into(),
            "Demo".into(),
            None,
            None,
            None,
        )]);
        launcher.set_query("demo");
        let mut panel = PluginPanelApplication::bundled_with_data(
            launcher_manifest(),
            "main.js",
            LauncherPluginProjection::from_launcher(&launcher).to_json(),
        )
        .unwrap();
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
        let mut panel = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::launcher_manifest(),
            "main.js",
            projection.to_json(),
        )
        .unwrap();
        assert!(format!("{:?}", panel.node).contains("Could not launch Demo"));
        assert!(
            panel
                .sync_serialized_data(LauncherPluginProjection::from_launcher(&launcher).to_json())
                .unwrap()
        );
        assert!(!format!("{:?}", panel.node).contains("Could not launch Demo"));
    }

    #[test]
    fn application_pin_save_retry_requires_pin_capability() {
        let mut external_manifest = manifest().clone();
        external_manifest.id = "org.example.pin-retry".into();
        external_manifest.capabilities.clear();
        let mut package = PluginPackage {
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
    fn external_plugins_use_launcher_actions_by_capability() {
        for (name, action, capability, expected) in [
            (
                "page",
                "{type: 'launcher-set-page', view: 'dashboard', page: 1}",
                PluginCapability::ApplicationsRead,
                PluginEffect::SetLauncherPage {
                    dashboard: true,
                    page: 1,
                },
            ),
            (
                "launch",
                "{type: 'applications.launch', id: 'org.example.app'}",
                PluginCapability::ApplicationsLaunch,
                PluginEffect::LaunchApplication {
                    id: "org.example.app".into(),
                },
            ),
            (
                "view",
                "{type: 'launcher-set-view', view: 'applications'}",
                PluginCapability::ApplicationsRead,
                PluginEffect::SetLauncherView(LauncherView::Applications),
            ),
            (
                "pin",
                "{type: 'applications.togglePin', id: 'org.example.app'}",
                PluginCapability::ApplicationsPin,
                PluginEffect::ToggleApplicationPin {
                    id: "org.example.app".into(),
                },
            ),
            (
                "project",
                "{type: 'launcher-open-project', id: 'project'}",
                PluginCapability::ProjectsOpen,
                PluginEffect::LauncherOpenProject {
                    id: "project".into(),
                },
            ),
            (
                "projects",
                "{type: 'launcher-see-all-projects'}",
                PluginCapability::ProjectsRead,
                PluginEffect::LauncherSeeAllProjects,
            ),
            (
                "logout",
                "{type: 'launcher-request-logout'}",
                PluginCapability::SessionLogoutRequest,
                PluginEffect::LauncherRequestLogout,
            ),
        ] {
            let mut external_manifest = manifest().clone();
            external_manifest.id = format!("org.example.launcher-action-{name}");
            external_manifest.capabilities.clear();
            let mut package = PluginPackage {
                manifest: external_manifest,
                images: Default::default(),
                stylesheet: String::new(),
                source: format!(
                    "function App() {{ return h(FixedWindow, {{width: '100%', height: '100%', onEscape: () => nickel.request({action})}}); }}"
                ),
            };
            let mut denied = PluginPanelApplication::from_package(&package).unwrap();
            denied.shortcut_outcome(Shortcut::Escape);
            assert!(denied.take_effects().is_empty(), "{name}");
            package.manifest.capabilities.push(capability);
            let mut granted = PluginPanelApplication::from_package(&package).unwrap();
            granted.shortcut_outcome(Shortcut::Escape);
            assert_eq!(granted.take_effects(), vec![expected], "{name}");
        }
    }

    #[test]
    fn failed_launcher_save_exposes_retry_in_jsx_menu() {
        let launcher = Launcher::default();
        let projection = LauncherPluginProjection::from_launcher(&launcher).with_status(Some(
            "Launcher preferences could not be saved: storage unavailable".into(),
        ));
        let mut host = nickel_ui::UiHost::new(
            PluginPanelApplication::bundled_with_data(
                launcher_manifest(),
                "main.js",
                projection.to_json(),
            )
            .unwrap(),
            920,
            680,
        );
        let app = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::Button,
                name: "Firefox".into(),
            })
            .unwrap();
        let outcome = host.perform_accessibility_action(
            app.id,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::ContextMenu),
        );
        assert!(outcome.failures.is_empty(), "{:#?}", outcome.failures);
        let retry = host
            .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                role: SemanticRole::MenuItem,
                name: "Retry saving favorites".into(),
            })
            .unwrap();
        host.perform_accessibility_action(
            retry.id,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
        );
        assert_eq!(
            host.application_mut().take_effects(),
            vec![PluginEffect::RetryApplicationPinSave]
        );
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
    fn launcher_plugin_uses_window_root_and_keeps_nested_controls_reachable() {
        let launcher = Launcher::new(Vec::new());
        let mut panel = PluginPanelApplication::bundled_with_data(
            launcher_manifest(),
            "main.js",
            LauncherPluginProjection::from_launcher(&launcher).to_json(),
        )
        .unwrap();
        assert!(!panel.shortcut_outcome(Shortcut::Submit).changed);
        assert!(panel.take_effects().is_empty());
        assert!(matches!(
            &panel.node,
            PanelNode::Surface {
                window_request: Some(_),
                ..
            }
        ));
        assert!(panel.node.button_action("launcher-settings").is_some());
        panel.update(panel.button_message("launcher-settings").unwrap());
        assert_eq!(
            panel.take_effects(),
            vec![PluginEffect::ShowSettings(Some("appearance".into()))]
        );
        panel.update(panel.button_message("launcher-account").unwrap());
        assert_eq!(panel.take_effects(), vec![PluginEffect::ShowControlCenter]);
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
            let panel = PluginPanelApplication::bundled_with_data(
                crate::plugin_panel::notification_manifest(),
                "main.js",
                projection.to_json(),
            )
            .unwrap();
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
        let app = PluginPanelApplication::bundled_with_data(
            launcher_manifest(),
            "main.js",
            LauncherPluginProjection::from_launcher(&launcher).to_json(),
        )
        .unwrap();
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
