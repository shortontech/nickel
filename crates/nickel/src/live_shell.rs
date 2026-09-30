mod preference_persistence;

use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

static INTERNAL_INGRESS_ORDER: AtomicU64 = AtomicU64::new(1);
static INTERNAL_INGRESS_EPOCH: OnceLock<Instant> = OnceLock::new();

pub(crate) fn internal_normalized_ingress(
    input: nickel_input::InputEvent,
    clipboard_text: Option<String>,
    owner: &'static str,
    recipient: nickel_ui::HostInspection,
    transform_generation: Option<u64>,
) -> (HostEvent, nickel_ui::NormalizedIngressAuthority) {
    let device = input.device();
    let device_generation = device.map_or(0, |device| device.0);
    let order = INTERNAL_INGRESS_ORDER.fetch_add(1, Ordering::Relaxed);
    let epoch = INTERNAL_INGRESS_EPOCH.get_or_init(Instant::now);
    let source = nickel_ui::NormalizedSourceBinding {
        seat: 0,
        backend_stream: if device.is_some() {
            format!("routed-native-input:{owner}")
        } else {
            format!("internal-lifecycle:{owner}")
        },
        stream_generation: 1,
        device_generation,
        identity_capability: if device.is_some() {
            "normalized-device-generation".into()
        } else {
            "system-event-without-device".into()
        },
        reconnect_generation: device_generation,
    };
    let recipient = nickel_ui::NormalizedRecipientBinding {
        // A focus-gained event is the authority that restores this host's input
        // lease. Rejecting it while the previous lease is zero would leave a
        // window unable to accept input after its first focus loss.
        lease: u64::from(
            recipient.window_focused
                || matches!(input, nickel_input::InputEvent::FocusGained { .. }),
        ),
        lifetime: recipient.frame_generation,
    };
    let authority = nickel_ui::NormalizedIngressAuthority {
        source: source.clone(),
        recipient,
        transfer_cutoff: None,
        host_connection_generation: recipient.lifetime,
        operation_epoch: None,
        role: owner.into(),
        transform_generation,
        text_transaction: None,
        composition_recipient_epoch: None,
        coordinate_meaning: "host-logical".into(),
    };
    let event = HostEvent::NormalizedIngress(nickel_ui::NormalizedInputEnvelope {
        input,
        clipboard_text,
        source,
        admission: nickel_ui::NormalizedAdmissionBinding {
            order,
            monotonic_micros: epoch.elapsed().as_micros() as u64,
        },
        recipient,
        operation: None,
        transform_generation,
        text_transaction: None,
        transfer_cutoff: None,
        broker_event_id: None,
        host_connection_generation: recipient.lifetime,
        operation_epoch: None,
        role: owner.into(),
        coordinate_meaning: "host-logical".into(),
        composition_recipient_epoch: None,
    });
    (event, authority)
}

fn normalized_input(event: &HostEvent) -> Option<&nickel_input::InputEvent> {
    match event {
        HostEvent::Normalized { input, .. } => Some(input),
        HostEvent::NormalizedIngress(envelope) => Some(&envelope.input),
        _ => None,
    }
}

use nickel_core::task_switcher::{SwitchWindow, TaskSwitchEffect, TaskSwitcher};
use nickel_core::{
    launcher_preferences::LauncherPreferences, shell_settings::ShellSettings, theme::ThemePalette,
    wallpaper_settings::WallpaperSettings,
};
use nickel_file::desktop::{DesktopOutput, Point as DesktopPoint};
use nickel_session_protocol::ShellRole;
#[cfg(any(target_os = "linux", test))]
use nickel_session_protocol::{
    AnchorSide, Geometry, PointerInteraction, PreviewTargetAction, ResolvedShellTarget,
    ShellPopoverAnchor, ShellSemanticTarget, WindowMenuTargetAction,
};
use nickel_ui::InternalSurfaceId;
use nickel_ui::Rect;
use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    Application as UiApplication, Column, Container, ControllerAction, HostBatch, HostChangeToken,
    HostEvent, Insets, Point, SemanticRole, Shortcut, Size, Spacer, Text, TextAlign, TextField,
    UiEvent, UiHostViewport, ViewContext,
};

#[cfg(any(target_os = "linux", target_os = "windows", test))]
use crate::notification::{NotificationAction, NotificationRequest};

use crate::{
    control_view::{ControlAction, ControlCenterApp, ControlCenterHost},
    file_window_host::{FileWindowHost, default_file_window_host},
    launcher::{DashboardAccount, DashboardProject, DashboardSection, Launcher, LauncherView},
    launcher_actions::{LauncherAction, LauncherShellEffect, reduce_launcher_action},
    launcher_icon_cache::LauncherIconCache,
    model::{Application, OpenWindow, TrayItem, WindowGroup},
    notification::DesktopNotification,
    notification_view::{NotificationApp, NotificationEffect, NotificationHost},
    platform::{
        self, AudioStatus, BluetoothStatus, FeedState, FeedStatus, NetworkStatus, NotificationFeed,
        NotificationSource, ShellCommand, TrayFeed, TraySource, WindowAction, WindowFeed,
    },
    screenshot::ScreenshotTool,
    session_host::{SessionHost, default_session_host},
    window_preview::{
        ApplicationMenuAction, ApplicationMenuTarget, MENU_WIDTH, MenuAction, PreviewAction,
        TaskbarPreviewAnchor, application_menu_entries, display_menu_entries, menu_height,
        menu_height_for_rows, preview_dimensions, semantic_theme_from_palette,
        task_switcher_dimensions, validated_application_close_targets,
        window_menu_action_is_current, window_menu_entries, window_menu_max_rows,
        workspace_menu_entries,
    },
    winit_shell::SurfaceRole,
};

use nickel_input::KeyCode;
#[cfg(not(target_os = "windows"))]
use zeroize::Zeroize;
use zeroize::Zeroizing;

fn bundled_surface_host(
    manifest: &nickel_core::plugins::PluginManifest,
    data: String,
    images: crate::plugin_panel::PluginImages,
) -> Result<
    (
        nickel_core::plugins::PluginSurfaceKey,
        nickel_core::plugins::PluginSurface,
        nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>,
    ),
    String,
> {
    let mut application = crate::plugin_panel::PluginPanelApplication::bundled_with_data(
        manifest,
        &manifest.entry,
        data,
    )?;
    application.sync_images(images);
    let surface = manifest
        .surfaces
        .first()
        .ok_or_else(|| format!("bundled plugin {} has no surface", manifest.id))?
        .clone();
    let key = nickel_core::plugins::PluginSurfaceKey {
        plugin_id: manifest.id.clone(),
        surface_id: surface.id.clone(),
    };
    let host = nickel_ui::UiHost::new(application, surface.width, surface.height);
    Ok((key, surface, host))
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CodexApprovalOwner {
    // Internal surfaces are owned by the Linux compositor. Windows retains
    // this variant in fixtures that verify cross-platform approval identity.
    #[cfg_attr(all(target_os = "windows", not(test)), allow(dead_code))]
    Internal(InternalSurfaceId),
    Winit(crate::winit_shell::SurfaceId),
}

#[cfg(any(test, target_os = "linux"))]
fn launcher_controller_host_event(action: ControllerAction, overlay_open: bool) -> HostEvent {
    if action == ControllerAction::Cancel && !overlay_open {
        HostEvent::Shortcut(Shortcut::Escape)
    } else {
        HostEvent::Controller(action)
    }
}

const PANEL_ITEM_WIDTH: f32 = 52.0;
#[cfg(test)]
const PANEL_CLOCK_WIDTH: f32 = 96.0;
#[cfg(test)]
const PANEL_CONTROL_GAP: f32 = 8.0;
#[cfg(test)]
const PANEL_TRAY_WIDTH: f32 = 28.0;
const PANEL_TRAY_ICON_SIZE: u32 = 18;
#[cfg(test)]
const PANEL_CODEX_WIDTH: f32 = 36.0;
#[cfg(test)]
const PANEL_CODEX_ICON_SIZE: f32 = 28.0;
const PREVIEW_LEAVE_DELAY: Duration = Duration::from_millis(500);
const PREVIEW_HOVER_DELAY: Duration = Duration::from_millis(300);
const PREVIEW_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "linux")]
const RECURRING_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(30);
const WALLPAPER_MAX_WIDTH: u32 = 7680;
const WALLPAPER_MAX_HEIGHT: u32 = 4320;
const PREVIEW_CACHE_CAPACITY: usize = 32;

pub(crate) mod remote_semantics;
#[path = "live_shell/taskbar.rs"]
mod taskbar;
pub use taskbar::TaskbarAction;
use taskbar::{
    TaskbarHover, normalize_tray_items, panel_clock_text, panel_tray_icons, tint_panel_icon,
};
#[cfg(test)]
use taskbar::{panel_status_layout, visible_tray_item};

#[path = "live_shell/keyboard.rs"]
mod keyboard;

#[derive(Clone, Debug, PartialEq)]
struct PendingPopoverAnchor {
    role: ShellRole,
    control: String,
    output: String,
    bounds: Rect,
}
#[path = "live_shell/desktop.rs"]
mod desktop;
#[allow(unused_imports)]
pub use desktop::{DesktopApplication, DesktopCommand, DesktopMessage};
#[cfg(test)]
use desktop::{SettingsDestination, retain_unchanged_desktop_icons};

#[derive(Clone, Debug, Eq, PartialEq)]
struct WallpaperSourceFingerprint {
    path: std::path::PathBuf,
    length: u64,
    modified: Option<std::time::SystemTime>,
}

fn wallpaper_source_fingerprint(path: &std::path::Path) -> Option<WallpaperSourceFingerprint> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(WallpaperSourceFingerprint {
        path: path.to_owned(),
        length: metadata.len(),
        modified: metadata.modified().ok(),
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LockMessage {
    Password(String),
}

enum LockEffect {
    Authenticate(Zeroizing<String>),
}

pub struct LockApplication {
    password: Zeroizing<String>,
    status: Option<String>,
    palette: ThemePalette,
    effects: Vec<LockEffect>,
}

#[cfg(any(test, feature = "workbench-fixtures"))]
impl LockApplication {
    #[allow(dead_code)] // Binary and fixture library compile this shared module separately.
    pub fn fixture(password: &str, status: Option<String>) -> Self {
        Self {
            password: Zeroizing::new(password.to_owned()),
            status,
            palette: ThemePalette::from_appearance(nickel_core::theme::Appearance::default()),
            effects: Vec::new(),
        }
    }
}

impl nickel_ui::Application for LockApplication {
    type Message = LockMessage;

    fn update(&mut self, message: Self::Message) {
        match message {
            LockMessage::Password(value) if value.len() <= 1024 => {
                self.password = Zeroizing::new(value);
                self.status = None;
            }
            LockMessage::Password(_) => {}
        }
    }

    fn shortcut_outcome(&mut self, shortcut: Shortcut) -> nickel_ui::ShortcutOutcome {
        if shortcut != Shortcut::Submit {
            return nickel_ui::ShortcutOutcome::from_changed(false);
        }
        self.effects
            .push(LockEffect::Authenticate(std::mem::take(&mut self.password)));
        nickel_ui::ShortcutOutcome::handled(true)
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        let username = std::env::var("USER").unwrap_or_else(|_| "Session locked".into());
        let password_color = if self.password.is_empty() {
            self.palette.muted
        } else {
            self.palette.text
        };
        let theme = semantic_theme_from_palette(self.palette);
        let mut content = Column::new()
            .width(width)
            .height(height)
            .child(Spacer::vertical(height * 0.38))
            .child(
                Text::new(nickel_i18n::system_text("ui-live-shell-nickel"))
                    .height(48.0)
                    .scale(30.0)
                    .color(self.palette.text)
                    .align(TextAlign::Center)
                    .bold(true),
            )
            .child(Spacer::vertical(8.0))
            .child(
                Text::new(username)
                    .height(32.0)
                    .scale(18.0)
                    .color(self.palette.muted)
                    .align(TextAlign::Center),
            )
            .child(Spacer::vertical(16.0))
            .child(
                Container::new()
                    .id("lock-password")
                    .accessibility_label("Password")
                    .width(340.0)
                    .height(46.0)
                    .align_self(nickel_ui::Align::Center)
                    .background(self.palette.surface)
                    .radius(10.0)
                    .padding(Insets {
                        top: 9.0,
                        right: 16.0,
                        bottom: 9.0,
                        left: 16.0,
                    })
                    .child(
                        TextField::on_change_masked_with_placeholder(
                            &self.password,
                            nickel_i18n::system_text("ui-live-shell-password"),
                            '•',
                            LockMessage::Password,
                        )
                        .id("lock-password-input")
                        .accessibility_label("Password")
                        .scale(18.0)
                        .single_line_height(28.0)
                        .color(password_color)
                        .background(self.palette.surface)
                        .focus_background_tint(theme.borders.focus)
                        .controller_focus_background_tint(theme.borders.controller_focus),
                    ),
            );
        if let Some(status) = &self.status {
            content = content.child(Spacer::vertical(14.0)).child(
                Text::new(status)
                    .height(28.0)
                    .scale(15.0)
                    .color(theme.text.danger)
                    .align(TextAlign::Center),
            );
        }
        Container::new()
            .id("lock-screen")
            .accessibility_label("Session locked")
            .background(self.palette.background)
            .width(width)
            .height(height)
            .child(content)
    }

    fn title(&self) -> &str {
        "Nickel Lock Screen"
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ShellImageCacheDiagnostics {
    pub launcher_icon_entries: usize,
    pub launcher_icon_bytes: usize,
    pub wallpaper_entries: usize,
    pub wallpaper_bytes: usize,
    pub tray_entries: usize,
    pub tray_bytes: usize,
    pub preview_entries: usize,
    pub preview_bytes: usize,
}

#[derive(Debug, Default, Eq, PartialEq)]
pub struct ShellDeadlineOutcome {
    pub redraw: Vec<SurfaceRole>,
    pub capture_screenshot: bool,
    pub visibility_changed: bool,
}

struct PanelTaskProjection {
    revision: Arc<()>,
    windows: Vec<OpenWindow>,
    groups: Arc<Vec<crate::launcher::TaskbarApplication>>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum CodexMenuRequest {
    Refresh,
    Open {
        token: String,
        revision: nickel_codex_ui::ProjectMenuRevision,
    },
}

struct DisplayPreview {
    owner: String,
    previous: nickel_session_protocol::OutputLayout,
    applied: nickel_session_protocol::OutputLayout,
    deadline: Instant,
}

fn output_layout_from_snapshot(
    outputs: &[nickel_session_protocol::OutputSnapshot],
) -> nickel_session_protocol::OutputLayout {
    let mut layout = nickel_session_protocol::OutputLayout {
        primary: outputs
            .iter()
            .find(|output| output.primary && output.enabled)
            .map(|output| output.name.clone())
            .unwrap_or_default(),
        placements: outputs
            .iter()
            .map(|output| nickel_session_protocol::OutputPlacement {
                name: output.name.clone(),
                x: output.geometry.x,
                y: output.geometry.y,
                enabled: output.enabled,
                scale_120: output.scale_120,
                mode: output.current_mode,
            })
            .collect(),
    };
    layout
        .placements
        .sort_by(|left, right| left.name.cmp(&right.name));
    layout
}

fn normalized_output_layout_with_modes(
    mut layout: nickel_session_protocol::OutputLayout,
    outputs: &[nickel_session_protocol::OutputSnapshot],
) -> nickel_session_protocol::OutputLayout {
    let minimum_x = layout
        .placements
        .iter()
        .map(|placement| placement.x)
        .min()
        .unwrap_or(0);
    let minimum_y = layout
        .placements
        .iter()
        .map(|placement| placement.y)
        .min()
        .unwrap_or(0);
    for placement in &mut layout.placements {
        placement.x -= minimum_x;
        placement.y -= minimum_y;
        if placement.mode.is_none() {
            placement.mode = outputs
                .iter()
                .find(|output| output.name == placement.name)
                .and_then(|output| output.current_mode);
        }
    }
    layout
        .placements
        .sort_by(|left, right| left.name.cmp(&right.name));
    layout
}

fn validate_plugin_display_layout(
    outputs: &[nickel_session_protocol::OutputSnapshot],
    layout: &nickel_session_protocol::OutputLayout,
) -> Result<(), &'static str> {
    if outputs.is_empty()
        || outputs.len() > nickel_session_protocol::MAX_OUTPUTS
        || layout.placements.len() != outputs.len()
    {
        return Err("display topology changed");
    }
    let current = outputs
        .iter()
        .map(|output| (output.name.as_str(), output))
        .collect::<HashMap<_, _>>();
    if current.len() != outputs.len() {
        return Err("ambiguous display topology");
    }
    let mut seen = HashSet::new();
    for placement in &layout.placements {
        let Some(output) = current.get(placement.name.as_str()) else {
            return Err("display topology changed");
        };
        if !seen.insert(placement.name.as_str())
            || !(60..=480).contains(&placement.scale_120)
            || !(-1_000_000..=1_000_000).contains(&placement.x)
            || !(-1_000_000..=1_000_000).contains(&placement.y)
            || placement
                .mode
                .is_some_and(|mode| !output.modes.contains(&mode))
        {
            return Err("invalid display placement");
        }
    }
    if !layout
        .placements
        .iter()
        .any(|placement| placement.name == layout.primary && placement.enabled)
    {
        return Err("primary display must be enabled");
    }
    Ok(())
}

pub struct LiveShell {
    appearance_capabilities: crate::appearance_capabilities::AppearanceCapabilities,
    preferences_capabilities: crate::preferences_capabilities::PreferencesCapabilities,
    preferences_commit_pending: Option<ShellSettings>,
    session_host: Arc<dyn SessionHost>,
    screenshot_capture_pending: bool,
    pub(crate) screenshot_output: Option<String>,
    #[cfg(target_os = "linux")]
    active_window_capture: Option<ActiveWindowCapture>,
    #[cfg(target_os = "linux")]
    active_window_capture_target: Option<(
        String,
        nickel_session_protocol::Geometry,
        nickel_session_protocol::Geometry,
    )>,
    host_runtime_samples: HostRuntimeSamples,
    launcher: Launcher,
    window_feed: WindowFeed,
    #[cfg(target_os = "linux")]
    internal_session_snapshot: Option<nickel_session_protocol::Snapshot>,
    #[cfg(target_os = "linux")]
    internal_workspaces: Option<Vec<platform::WorkspaceSummary>>,
    tray_feed: TrayFeed,
    notification_feed: NotificationFeed,
    windows: Vec<OpenWindow>,
    window_icons: HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    task_switcher: TaskSwitcher<crate::model::WindowId>,
    task_switcher_group: Option<WindowGroup>,
    workspaces: Vec<platform::WorkspaceSummary>,
    configured_desktop_count: u8,
    window_feed_status: FeedStatus,
    workspace_feed_status: FeedStatus,
    tray: Vec<TrayItem>,
    tray_icons: Vec<Arc<image::RgbaImage>>,
    notification: Option<DesktopNotification>,
    notification_history_visible: bool,
    remote_lease_notifications: HashMap<u32, nickel_session_protocol::RemotePendingLease>,
    remote_lease_submitting: HashSet<u32>,
    dismissed_remote_lease_notifications: HashSet<u32>,
    remote_lease_overflow_rejections: HashSet<(String, u64)>,
    codex_approval_notifications: HashMap<
        u32,
        (
            CodexApprovalOwner,
            nickel_codex_ui::CodexApprovalNotification,
        ),
    >,
    codex_approval_decisions: Vec<(
        CodexApprovalOwner,
        nickel_codex_ui::CodexApprovalNotification,
        nickel_codex_ui::CodexApprovalChoice,
    )>,
    codex_approval_reviews: Vec<CodexApprovalOwner>,
    codex_approval_delivery_updates: Vec<(
        CodexApprovalOwner,
        nickel_codex_ui::CodexApprovalNotification,
        bool,
    )>,
    dismissed_codex_approval_notifications: HashSet<u32>,
    codex_approval_overflow_outcomes:
        HashSet<(CodexApprovalOwner, u64, nickel_codex::ServerRequestId, u64)>,
    #[cfg(target_os = "windows")]
    remote_lease_decisions: Vec<(nickel_session_protocol::RemotePendingLease, bool)>,
    wallpaper_path: Option<std::path::PathBuf>,
    wallpaper_source_fingerprint: Option<WallpaperSourceFingerprint>,
    wallpaper_loaded_source_fingerprint: Option<WallpaperSourceFingerprint>,
    wallpaper: Option<Arc<image::RgbaImage>>,
    wallpaper_size: (u32, u32),
    desktop_host: nickel_ui::UiHost<DesktopApplication>,
    desktop_viewports: HashMap<String, DesktopSurfaceViewport>,
    desktop_active_viewport: String,
    desktop_change_token: HostChangeToken,
    desktop_deadline: Option<Instant>,
    desktop_application_dirty: bool,
    /// Button whose current press/release transaction belongs to a desktop
    /// overlay.  Application messages may close the overlay on release, so
    /// routing cannot be inferred independently from the menu's current
    /// visibility without risking a half transaction reaching the file plane.
    desktop_overlay_pointer_capture: Option<nickel_input::PointerButton>,
    #[cfg(target_os = "windows")]
    output_identification: Option<(String, u64, usize)>,
    panel_icon: Arc<image::RgbaImage>,
    codex_icon: Arc<image::RgbaImage>,
    palette: ThemePalette,
    network: NetworkStatus,
    bluetooth: BluetoothStatus,
    audio: AudioStatus,
    audio_status_observed: bool,
    associations_results: HashMap<String, serde_json::Value>,
    volume_osd_until: Option<Instant>,
    launcher_visible: bool,
    run_visible: bool,
    locked: bool,
    lock_host: nickel_ui::UiHost<LockApplication>,
    lock_change_token: HostChangeToken,
    lock_deadline: Option<Instant>,
    control_visible: bool,
    codex_project_menu_visible: bool,
    panel_hover: Option<TaskbarHover>,
    panel_hover_output: Option<String>,
    plugin_registry: nickel_core::plugins::PluginRegistry,
    package_settings_registry: nickel_core::settings_registry::SettingsRegistry,
    package_settings_runtimes: std::collections::BTreeMap<
        String,
        std::rc::Rc<std::cell::RefCell<nickel_plugin_runtime::JsxRuntime>>,
    >,
    package_settings_generation: u64,
    package_settings_values: nickel_plugin_runtime::settings::SettingsValueSnapshot,
    package_settings_value_revisions: std::collections::BTreeMap<String, u64>,
    package_settings_invoking: bool,
    plugin_settings:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>,
    external_plugin_packages:
        std::collections::BTreeMap<String, nickel_core::plugins::PluginPackageDescriptor>,
    primary_panel_key: nickel_core::plugins::PluginSurfaceKey,
    plugin_activation_generation: u64,
    #[cfg(target_os = "linux")]
    last_published_plugin_status: Option<nickel_session_protocol::PluginStatusSnapshot>,
    plugin_surface_hosts: std::collections::BTreeMap<
        nickel_core::plugins::PluginSurfaceKey,
        (
            nickel_core::plugins::PluginSurface,
            nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>,
        ),
    >,
    plugin_panel_memory: std::collections::BTreeMap<nickel_core::plugins::PluginSurfaceKey, u64>,
    plugin_window_placement_overrides: std::collections::BTreeMap<
        nickel_core::plugins::PluginSurfaceKey,
        (nickel_core::plugins::PluginSurfaceAnchor, i32, i32),
    >,
    #[cfg(target_os = "windows")]
    pending_plugin_surface_focus: Option<nickel_core::plugins::PluginSurfaceKey>,
    plugin_taskbar_host: Option<nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>>,
    plugin_slot_hosts: std::collections::BTreeMap<String, PluginSlotHost>,
    plugin_taskbar_hosts:
        HashMap<Option<String>, nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>>,
    plugin_taskbar_memory: HashMap<Option<String>, u64>,
    plugin_taskbar_menu_memory: u64,
    panel_projections: HashMap<Option<String>, PanelTaskProjection>,
    panel_change_token: HostChangeToken,
    panel_deadline: Option<Instant>,
    panel_pet_frame: u8,
    panel_pet_deadline: Option<Instant>,
    panel_output: Option<String>,
    pending_popover_anchor: Option<PendingPopoverAnchor>,
    all_windows_on_every_bar: bool,
    #[cfg(target_os = "windows")]
    idle_policy: nickel_core::idle::IdlePolicy,
    preview_group: Option<usize>,
    preview_pending: Option<(usize, Instant)>,
    preview_focus_requested: bool,
    preview_pointer_inside: bool,
    preview_leave_deadline: Option<Instant>,
    preview_hovered: Option<crate::model::WindowId>,
    preview_images: HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    preview_refresh_deadline: Option<Instant>,
    window_menu: Option<crate::model::WindowId>,
    window_menu_snapshot: Option<OpenWindow>,
    window_menu_generation: u64,
    window_menu_anchor_x: Option<i32>,
    window_menu_anchor_y: Option<i32>,
    window_menu_plugin_host: Option<nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>>,
    application_menu_target: Option<ApplicationMenuTarget>,
    application_menu_plugin_host:
        Option<nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>>,
    notification_host: NotificationHost,
    panel_origin_x: i32,
    panel_origin_y: i32,
    control_host: ControlCenterHost,
    control_change_token: HostChangeToken,
    control_deadline: Option<Instant>,
    projection_chooser: nickel_core::display_projection::ProjectionChooser,
    projection_rollback_deadline: Option<Instant>,
    display_preview: Option<DisplayPreview>,
    launcher_icons: LauncherIconCache,
    launcher_icon_revision: u64,
    launcher_plugin_result_page: usize,
    launcher_plugin_dashboard_page: usize,
    launcher_status: Option<String>,
    #[cfg(target_os = "windows")]
    launcher_catalog_generation: u64,
    launcher_preference_persistence: preference_persistence::PreferencePersistence,
    launcher_preference_deadline: Option<Instant>,
    shortcut_action_status: Option<String>,
    shortcut_capability_status: Option<String>,
    #[cfg(test)]
    launcher_preferences_path: Option<std::path::PathBuf>,
    #[cfg(test)]
    launcher_persistence_attempts: usize,
    secure_storage_override: Option<String>,
    secure_storage_state: platform::SecureStorageState,
    #[cfg(target_os = "linux")]
    secure_storage_query_error: Option<(platform::SessionRequestError, Instant)>,
    requested_codex_project: Option<String>,
    codex_menu_requests: Vec<CodexMenuRequest>,
    screenshot: ScreenshotTool,
    keyboard_host: nickel_ui::UiHost<nickel_ui::on_screen_keyboard::KeyboardApp>,
    keyboard_plugin_generation: u64,
    keyboard_visible: bool,
    keyboard_enabled: bool,
    #[cfg(target_os = "windows")]
    keyboard_generation: u64,
    #[cfg(target_os = "windows")]
    keyboard_touchscreen_present: bool,
    keyboard_dock_top: bool,
    keyboard_height: u32,
    keyboard_resize: Option<(nickel_input::DeviceId, Option<nickel_input::TouchId>, f64)>,
    keyboard_override: nickel_core::on_screen_keyboard::KeyboardOverride,
    keyboard_deadline: Instant,
    keyboard_gesture_leases: HashMap<(nickel_input::DeviceId, Option<nickel_input::TouchId>), u64>,
    keyboard_recipient: Option<nickel_session_protocol::OnScreenKeyboardSnapshot>,
}

#[cfg(target_os = "linux")]
#[derive(Clone, Copy)]
struct ActiveWindowCapture {
    output: nickel_session_protocol::Geometry,
    window: nickel_session_protocol::Geometry,
    copy_path: bool,
}

#[cfg(target_os = "linux")]
fn active_window_capture_target(
    snapshot: &nickel_session_protocol::Snapshot,
) -> Option<(
    String,
    nickel_session_protocol::Geometry,
    nickel_session_protocol::Geometry,
)> {
    let window = snapshot
        .windows
        .iter()
        .find(|window| window.active)?
        .geometry?;
    let rect = nickel_core::geometry::LogicalRect {
        x: window.x,
        y: window.y,
        width: window.width,
        height: window.height,
    };
    let output = snapshot
        .outputs
        .iter()
        .filter(|output| output.enabled)
        .max_by_key(|output| {
            rect.intersection_area(nickel_core::geometry::LogicalRect {
                x: output.geometry.x,
                y: output.geometry.y,
                width: output.geometry.width,
                height: output.geometry.height,
            })
        })?;
    (rect.intersection_area(nickel_core::geometry::LogicalRect {
        x: output.geometry.x,
        y: output.geometry.y,
        width: output.geometry.width,
        height: output.geometry.height,
    }) > 0)
        .then(|| (output.name.clone(), output.geometry, window))
}

struct DesktopSurfaceViewport {
    host: UiHostViewport<desktop::DesktopMessage>,
    application: desktop::DesktopViewportState,
    change_token: HostChangeToken,
    deadline: Option<Instant>,
    overlay_pointer_capture: Option<nickel_input::PointerButton>,
}

#[cfg(any(test, target_os = "windows"))]
fn output_identification_is_current(current: Option<u64>, requested: u64) -> bool {
    current == Some(requested)
}

#[derive(Default)]
struct HostRuntimeSamples {
    input_to_message_us: VecDeque<u64>,
    input_to_frame_us: VecDeque<u64>,
    layout_us: VecDeque<u64>,
    paint_list_us: VecDeque<u64>,
    scheduled_wakeups: u64,
}

impl HostRuntimeSamples {
    fn record(&mut self, telemetry: nickel_ui::HostTelemetry) {
        for (samples, value) in [
            (&mut self.input_to_message_us, telemetry.input_to_message_us),
            (&mut self.input_to_frame_us, telemetry.input_to_frame_us),
            (&mut self.layout_us, telemetry.layout_us),
            (&mut self.paint_list_us, telemetry.paint_list_us),
        ] {
            if samples.len() == nickel_session_protocol::MAX_RUNTIME_PERFORMANCE_SAMPLES {
                samples.pop_front();
            }
            samples.push_back(value);
        }
        self.scheduled_wakeups = self
            .scheduled_wakeups
            .saturating_add(telemetry.scheduled_wakeups as u64);
    }
}

fn preview_refresh_due(deadline: Option<Instant>, now: Instant) -> bool {
    deadline.is_none_or(|deadline| now >= deadline)
}

fn shortcut_capability_status(
    capability: &nickel_input::global::ShortcutCapability,
) -> Option<String> {
    use nickel_input::global::{ShortcutCapability, UnavailableReason};

    let reason = match capability {
        ShortcutCapability::Available => return None,
        ShortcutCapability::Unavailable(UnavailableReason::UnsupportedPlatform) => {
            "this platform is unsupported".to_owned()
        }
        ShortcutCapability::Unavailable(UnavailableReason::MissingRuntime) => {
            "the Nickel session runtime is missing".to_owned()
        }
        ShortcutCapability::Unavailable(UnavailableReason::PermissionDenied) => {
            "the session denied permission".to_owned()
        }
        ShortcutCapability::Unavailable(UnavailableReason::SessionLocked) => {
            "the session is locked".to_owned()
        }
        ShortcutCapability::Unavailable(UnavailableReason::Backend(reason)) => reason.clone(),
    };
    Some(format!("Global shortcuts unavailable: {reason}."))
}

struct TaskbarProjectionInput<'a> {
    groups: &'a [crate::launcher::TaskbarApplication],
    keyboard_enabled: bool,
    codex_available: bool,
    panel_icon: &'a Arc<image::RgbaImage>,
    codex_icon: &'a Arc<image::RgbaImage>,
    task_icons: &'a [Option<(u16, Arc<image::RgbaImage>)>],
    tray: &'a [crate::model::TrayItem],
    tray_icons: &'a [Arc<image::RgbaImage>],
}

fn taskbar_plugin_data(
    input: TaskbarProjectionInput<'_>,
    clock: &str,
) -> (
    crate::plugin_panel::TaskbarPluginProjection,
    crate::plugin_panel::PluginImages,
) {
    let mut projection =
        crate::plugin_panel::TaskbarPluginProjection::from_groups(input.groups, clock);
    projection.keyboard_enabled = input.keyboard_enabled;
    projection.codex_available = input.codex_available;
    let mut images = crate::plugin_panel::PluginImages::new();
    images.insert("logo".into(), (2, Arc::clone(input.panel_icon)));
    if input.codex_available {
        images.insert("codex".into(), (0x5000, Arc::clone(input.codex_icon)));
    }
    for item in &mut projection.items {
        if let Some((image_id, image)) = input.task_icons.get(item.index).and_then(Option::as_ref) {
            item.icon = true;
            images.insert(
                format!("task:{}", item.index),
                (*image_id, Arc::clone(image)),
            );
        }
    }
    let mut seen_tray = std::collections::HashSet::new();
    for (index, item) in input.tray.iter().rev().take(4).rev().enumerate() {
        if item.id.is_empty() || item.id.len() > 256 || !seen_tray.insert(item.id.clone()) {
            continue;
        }
        let icon = input.tray_icons.get(index);
        if let Some(icon) = icon {
            images.insert(
                format!("tray:{}", item.id),
                (0x6000 + index as u16, Arc::clone(icon)),
            );
        }
        projection
            .tray
            .push(crate::plugin_panel::TaskbarPluginTrayItem {
                id: item.id.clone(),
                title: item.title.chars().take(120).collect(),
                icon: icon.is_some(),
            });
    }
    (projection, images)
}

struct PluginSlotHost {
    target_plugin: String,
    target_slot: String,
    contract: nickel_core::plugins::PluginSlotContract,
    priority: i16,
    mode: nickel_core::plugins::PluginContributionMode,
    application: crate::plugin_panel::PluginPanelApplication,
}

fn should_auto_start_installed_plugin(desired_enabled: bool, safe_mode: bool) -> bool {
    desired_enabled && !safe_mode
}

fn validated_extension_contract(
    manifest: &nickel_core::plugins::PluginManifest,
    registry: &nickel_core::plugins::PluginRegistry,
) -> Result<
    (
        nickel_core::plugins::PluginSlotContract,
        i16,
        nickel_core::plugins::PluginContributionMode,
    ),
    String,
> {
    use nickel_core::plugins::PluginContributionMode;
    let [contribution] = manifest.contributes.as_slice() else {
        return Err("extension needs exactly one contribution".into());
    };
    if !manifest.surfaces.is_empty() {
        return Err("extension must not declare a surface".into());
    }
    let target = registry
        .get(&contribution.target_plugin)
        .ok_or("extension target plugin is not registered")?;
    let slot = target
        .manifest
        .provides_slots
        .iter()
        .find(|slot| slot.id == contribution.target_slot && slot.contract == contribution.contract)
        .ok_or("extension target does not provide the declared slot and contract")?;
    if contribution.mode == PluginContributionMode::Replace && !slot.replaceable {
        return Err("extension target does not allow replacement".into());
    }
    Ok((
        contribution.contract,
        contribution.priority,
        contribution.mode,
    ))
}

fn external_plugin_settings(
    manifest: &nickel_core::plugins::PluginManifest,
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, String> {
    if manifest.settings.is_empty() {
        return Ok(std::collections::BTreeMap::new());
    }
    #[cfg(test)]
    let stored = nickel_core::plugins::PluginPreferences::default();
    #[cfg(not(test))]
    let stored = nickel_core::plugins::PluginPreferences::load_default(manifest)
        .map_err(|error| format!("could not load plugin settings: {error}"))?;
    Ok(stored.effective(manifest))
}

fn taskbar_plugin_control_bounds(
    host: &nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>,
    control: &str,
) -> Option<nickel_ui::Rect> {
    let mut matches = host
        .accessibility_nodes()
        .iter()
        .filter(|node| node.interactive && node.id.as_str().rsplit('/').next() == Some(control));
    let bounds = matches.next()?.rect;
    matches.next().is_none().then_some(bounds)
}

pub(crate) fn launcher_placeholder_icon() -> (u16, Arc<image::RgbaImage>) {
    static ICON: OnceLock<Arc<image::RgbaImage>> = OnceLock::new();
    // The asynchronous app icon cache starts at 0x4000.
    (
        0x3fff,
        Arc::clone(ICON.get_or_init(|| {
            Arc::new(
                crate::icons::load_svg_bytes(
                    include_bytes!("../../../assets/icons/start-menu/applications.svg"),
                    32,
                )
                .expect("bundled application placeholder must render"),
            )
        })),
    )
}

fn launcher_plugin_images(
    launcher: &Launcher,
    icons: &mut LauncherIconCache,
    projection: &crate::plugin_panel::LauncherPluginProjection,
) -> crate::plugin_panel::PluginImages {
    let mut images = crate::plugin_panel::PluginImages::new();
    for (slot, items) in [
        ("search", projection.results.as_slice()),
        ("dashboard", projection.dashboard.as_slice()),
        ("place", projection.places.as_slice()),
    ] {
        for item in items {
            let application = if slot == "place" {
                launcher.place_applications().nth(item.index)
            } else {
                launcher.result_at(item.index)
            };
            let icon = application
                .filter(|application| application.id() == item.id)
                .and_then(|application| icons.resolve(application))
                .unwrap_or_else(launcher_placeholder_icon);
            images.insert(format!("{slot}:{}", item.index), icon);
        }
    }
    images
}

fn step_plugin_host(
    host: &mut nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>,
    data: Option<String>,
    mut batch: HostBatch,
) -> Result<(nickel_ui::HostEventOutcome, u64), String> {
    if let Some(data) = data {
        batch.application_changed |= host.application_mut().sync_serialized_data(data)?;
    }
    let outcome = host.step(batch);
    if let Some(error) = host.application_mut().take_runtime_failure() {
        return Err(error);
    }
    let retained_bytes = (outcome.telemetry.retained_frame_bytes as u64)
        .saturating_add(host.application().retained_image_bytes());
    Ok((outcome, retained_bytes))
}

fn render_plugin_host(
    host: &mut nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>,
    data: Option<String>,
    batch: HostBatch,
) -> Result<(Vec<PaintCommand>, u64), String> {
    let (_, retained_bytes) = step_plugin_host(host, data, batch)?;
    Ok((host.commands().to_vec(), retained_bytes))
}

fn audio_plugin_data(audio: &AudioStatus, locked: bool) -> serde_json::Value {
    let percent = audio.volume_percent.min(100);
    let output = if locked {
        Some("Audio output")
    } else {
        audio
            .devices
            .iter()
            .find(|device| device.is_default)
            .map(|device| device.name.as_str())
    };
    let mut label = if audio.muted {
        "Muted".to_owned()
    } else {
        format!("Volume {percent}%")
    };
    if let Some(output) = output {
        label.push_str(" · ");
        label.extend(output.chars().take(120));
    }
    serde_json::json!({
        "available": audio.available,
        "label": label,
        "percent": percent,
        "muted": audio.muted,
        "outputName": output.map(|name| name.chars().take(120).collect::<String>()),
    })
}

// This shared shell implementation includes the compositor-facing API. The
// Windows winit owner calls its own subset and leaves those Linux methods idle.
#[cfg_attr(target_os = "windows", allow(dead_code))]
impl LiveShell {
    #[cfg(target_os = "linux")]
    pub fn host_runtime_samples(&self) -> (Vec<u64>, Vec<u64>, Vec<u64>, Vec<u64>, u64) {
        (
            self.host_runtime_samples
                .input_to_message_us
                .iter()
                .copied()
                .collect(),
            self.host_runtime_samples
                .input_to_frame_us
                .iter()
                .copied()
                .collect(),
            self.host_runtime_samples
                .layout_us
                .iter()
                .copied()
                .collect(),
            self.host_runtime_samples
                .paint_list_us
                .iter()
                .copied()
                .collect(),
            self.host_runtime_samples.scheduled_wakeups,
        )
    }
    pub fn set_dashboard_projects(
        &mut self,
        projects: DashboardSection<Vec<DashboardProject>>,
    ) -> bool {
        self.launcher.set_dashboard_projects(projects)
    }

    pub fn apply_codex_projection(
        &mut self,
        projection: nickel_core::optional_features::CodexAvailabilityProjection,
    ) -> bool {
        if projection.presentation() == nickel_core::optional_features::CodexPresentation::Hidden {
            self.codex_project_menu_visible = false;
            self.requested_codex_project = None;
            self.codex_menu_requests.clear();
        }
        self.launcher.apply_codex_projection(projection)
    }

    pub(crate) fn codex_projection(
        &self,
    ) -> Option<&nickel_core::optional_features::CodexAvailabilityProjection> {
        self.launcher.codex_projection()
    }

    pub fn take_requested_codex_project(&mut self) -> Option<String> {
        if self.launcher.codex_available() {
            self.requested_codex_project.take()
        } else {
            self.requested_codex_project = None;
            None
        }
    }

    pub(crate) fn apply_codex_menu_projection(
        &mut self,
        projection: &nickel_codex_ui::ProjectMenuProjection,
    ) -> bool {
        let key = crate::plugin_panel::codex_projects_surface_key();
        let data = match serde_json::to_string(projection) {
            Ok(data) => data,
            Err(error) => {
                self.fail_plugin_panel_runtime(&key.plugin_id, error.to_string());
                return false;
            }
        };
        let Some((_, host)) = self.plugin_surface_hosts.get_mut(&key) else {
            return false;
        };
        let changed = match host.application_mut().sync_serialized_data(data) {
            Ok(changed) => changed,
            Err(error) => {
                self.fail_plugin_panel_runtime(&key.plugin_id, error);
                return false;
            }
        };
        if !changed {
            return false;
        }
        let outcome = host.step(HostBatch {
            application_changed: true,
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        let failure = host.application_mut().take_runtime_failure();
        if let Some(error) = failure {
            self.fail_plugin_panel_runtime(&key.plugin_id, error);
            return false;
        }
        outcome.changed
    }

    pub(crate) fn take_codex_menu_requests(&mut self) -> Vec<CodexMenuRequest> {
        if self.launcher.codex_available() {
            std::mem::take(&mut self.codex_menu_requests)
        } else {
            self.codex_menu_requests.clear();
            Vec::new()
        }
    }

    #[cfg(test)]
    pub(crate) fn codex_available(&self) -> bool {
        self.launcher.codex_available()
    }
    pub fn new() -> Result<Self, String> {
        Self::new_with_hosts(default_session_host(), default_file_window_host())
    }

    pub fn new_with_safe_mode(safe_mode: bool) -> Result<Self, String> {
        Self::new_with_hosts_and_transport(
            default_session_host(),
            default_file_window_host(),
            true,
            safe_mode,
        )
    }

    #[cfg(test)]
    pub(crate) fn new_with_session_host(
        session_host: Arc<dyn SessionHost>,
    ) -> Result<Self, String> {
        Self::new_with_hosts(session_host, default_file_window_host())
    }

    pub(crate) fn new_with_hosts(
        session_host: Arc<dyn SessionHost>,
        file_window_host: Arc<dyn FileWindowHost>,
    ) -> Result<Self, String> {
        Self::new_with_hosts_and_transport(session_host, file_window_host, true, false)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn new_with_internal_hosts_in_mode(
        session_host: Arc<dyn SessionHost>,
        file_window_host: Arc<dyn FileWindowHost>,
        safe_mode: bool,
    ) -> Result<Self, String> {
        Self::new_with_hosts_and_transport(session_host, file_window_host, false, safe_mode)
    }

    fn new_with_hosts_and_transport(
        session_host: Arc<dyn SessionHost>,
        file_window_host: Arc<dyn FileWindowHost>,
        external_session_transport: bool,
        safe_mode: bool,
    ) -> Result<Self, String> {
        if safe_mode {
            tracing::info!("plugin safe mode: installed plugins will not start automatically");
        }
        let shell_settings = ShellSettings::load_default();
        #[cfg(target_os = "windows")]
        let optional_feature_settings =
            nickel_core::optional_features::OptionalFeatureSettings::load_default();
        let keyboard_override = {
            let value = std::env::var(nickel_core::on_screen_keyboard::ENVIRONMENT_VARIABLE)
                .unwrap_or_else(|_| "auto".into());
            nickel_core::on_screen_keyboard::KeyboardOverride::parse(&value).unwrap_or_else(|| {
                eprintln!(
                    "Invalid NICKEL_ON_SCREEN_KEYBOARD value; using saved keyboard preference"
                );
                Default::default()
            })
        };
        #[cfg(target_os = "windows")]
        let keyboard_touchscreen_present = crate::platform::windows_touchscreen_present();
        #[cfg(target_os = "windows")]
        let keyboard_enabled = nickel_core::on_screen_keyboard::resolve_enablement(
            optional_feature_settings.on_screen_keyboard,
            keyboard_override,
            if keyboard_touchscreen_present {
                nickel_core::on_screen_keyboard::TouchscreenPresence::Present
            } else {
                nickel_core::on_screen_keyboard::TouchscreenPresence::Absent
            },
        )
        .enabled;
        #[cfg(not(target_os = "windows"))]
        let keyboard_enabled = false;
        let application_discovery = platform::application_discovery();
        let application_status = application_discovery_status_label(application_discovery.status());
        let mut launcher = Launcher::new(application_discovery.into_applications());
        launcher.set_places(crate::places::applications(
            shell_settings.preferred_file_manager.as_deref(),
        ));
        let launcher_preferences = match LauncherPreferences::load_default() {
            Ok(preferences) => preferences,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                LauncherPreferences::default()
            }
            Err(error) => {
                tracing::warn!(%error, "launcher preferences could not be loaded");
                LauncherPreferences::default()
            }
        };
        let launcher_preference_persistence =
            preference_persistence::PreferencePersistence::new(launcher_preferences.clone());
        launcher.set_preferences(launcher_preferences);
        let _ = launcher.set_dashboard_account(
            std::env::var("USER")
                .or_else(|_| std::env::var("USERNAME"))
                .ok()
                .filter(|name| !name.trim().is_empty())
                .map_or_else(
                    || DashboardSection::Unavailable("Local identity unavailable".into()),
                    |display_name| {
                        DashboardSection::Ready(DashboardAccount {
                            display_name,
                            supporting_text: "Local session".into(),
                        })
                    },
                ),
        );
        let wallpaper_settings = WallpaperSettings::load_default();
        let (wallpaper_path, wallpaper, wallpaper_size) =
            initial_wallpaper(wallpaper_settings.image, || {
                #[cfg(target_os = "windows")]
                return platform::wallpaper().image;
                #[cfg(not(target_os = "windows"))]
                None
            });
        let wallpaper_source_fingerprint = wallpaper_path
            .as_deref()
            .and_then(wallpaper_source_fingerprint);
        let wallpaper_loaded_source_fingerprint =
            wallpaper.as_ref().and(wallpaper_source_fingerprint.clone());
        let palette = ThemePalette::from_appearance(
            shell_settings.resolve_appearance(crate::appearance_capabilities::system_appearance()),
        );
        let panel_icon = crate::icons::load_svg_bytes(
            include_bytes!("../../../assets/icons/nickel-start.svg"),
            96,
        )
        .map(|icon| tint_panel_icon(icon, palette.text))
        .map(Arc::new)
        .expect("embedded Nickel start icon remains valid");
        let codex_icon = crate::icons::load_svg_bytes(
            include_bytes!("../../../assets/icons/nickel-chat.svg"),
            96,
        )
        .map(|icon| tint_panel_icon(icon, palette.text))
        .map(Arc::new)
        .expect("embedded Nickel chat icon remains valid");
        #[cfg(target_os = "linux")]
        let window_feed = if external_session_transport {
            WindowFeed::new()
        } else {
            WindowFeed::internal()
        };
        #[cfg(not(target_os = "linux"))]
        let window_feed = {
            let _ = external_session_transport;
            WindowFeed::new()
        };
        let tray_feed = TrayFeed::new();
        let notification_feed = NotificationFeed::new()?;
        let windows = Vec::new();
        let workspaces = Vec::new();
        let tray = normalize_tray_items(tray_feed.snapshot());
        let tray_icons = panel_tray_icons(&tray);
        let network = platform::network_status();
        let bluetooth = platform::bluetooth_status();
        let audio = platform::audio_status();
        #[cfg(target_os = "linux")]
        let (secure_storage_state, secure_storage_query_error) =
            match session_host.secure_storage_state() {
                Ok(state) => (state, None),
                Err(error) => {
                    tracing::warn!(%error, "secure-storage query failed during shell startup");
                    (
                        platform::SecureStorageState::ControlUnavailable,
                        Some((error, Instant::now())),
                    )
                }
            };
        #[cfg(not(target_os = "linux"))]
        let secure_storage_state = platform::SecureStorageState::Ready;
        let control_host = ControlCenterHost::new(
            ControlCenterApp::new(
                network.clone(),
                bluetooth.clone(),
                audio.clone(),
                workspaces.clone(),
            ),
            380,
            650,
        );
        let notification_host = NotificationHost::new(NotificationApp::new(palette), 420, 180);
        let desktop_host = nickel_ui::UiHost::new(
            DesktopApplication::new(wallpaper.clone(), palette, file_window_host.clone()),
            1920,
            1080,
        );
        let lock_host = nickel_ui::UiHost::new(
            LockApplication {
                password: Zeroizing::new(String::new()),
                status: None,
                palette,
                effects: Vec::new(),
            },
            1920,
            1080,
        );
        let mut launcher_icons = LauncherIconCache::new();
        let (clock, _) = panel_clock_text();
        let initial_taskbar_groups = launcher.taskbar_applications(&windows);
        let mut plugin_registry = nickel_core::plugins::PluginRegistry::default();
        plugin_registry.register(crate::plugin_panel::manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::launcher_manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::run_manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::taskbar_manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::notification_manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::volume_osd_manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::control_center_manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::codex_projects_manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::on_screen_keyboard_manifest().clone())?;
        plugin_registry.register(crate::plugin_panel::window_preview_manifest().clone())?;
        plugin_registry.register(crate::settings_plugin_report::manifest().clone())?;
        #[cfg(test)]
        let catalog = nickel_core::plugins::PluginCatalog::default();
        #[cfg(not(test))]
        let catalog =
            nickel_core::plugins::PluginCatalog::discover_default().unwrap_or_else(|error| {
                tracing::warn!(%error, "could not discover installed plugins");
                nickel_core::plugins::PluginCatalog::default()
            });
        for failure in &catalog.failures {
            tracing::warn!(plugin = %failure.directory, reason = %failure.reason, "invalid installed plugin");
        }
        let mut external_plugin_packages = std::collections::BTreeMap::new();
        for (id, descriptor) in catalog.packages {
            if descriptor.manifest.claims_native_shell_surface() {
                tracing::warn!(plugin = %id, "installed plugin cannot replace a native shell surface");
                continue;
            }
            if plugin_registry.entries().count() >= 64 {
                tracing::warn!(plugin = %id, "plugin status capacity reached");
                continue;
            }
            match plugin_registry.register(descriptor.manifest.clone()) {
                Ok(()) => {
                    external_plugin_packages.insert(id, descriptor);
                }
                Err(error) => {
                    tracing::warn!(plugin = %id, %error, "installed plugin was not registered")
                }
            }
        }
        let plugin_settings = plugin_registry
            .entries()
            .filter(|entry| !entry.manifest.settings.is_empty())
            .map(|entry| {
                let values = external_plugin_settings(&entry.manifest).unwrap_or_else(|error| {
                    tracing::warn!(plugin = %entry.manifest.id, %error, "could not load plugin settings");
                    nickel_core::plugins::PluginPreferences::default()
                        .effective(&entry.manifest)
                });
                (entry.manifest.id.clone(), values)
            })
            .collect();
        // Unit tests exercise activation in parallel; core storage tests cover
        // persistence without sharing the user's activation file.
        #[cfg(test)]
        let plugin_activation = nickel_core::plugins::PluginActivationSettings::default();
        #[cfg(not(test))]
        let plugin_activation = nickel_core::plugins::PluginActivationSettings::load_default()
            .unwrap_or_else(|error| {
                if error.kind() != std::io::ErrorKind::NotFound {
                    tracing::warn!(%error, "could not read plugin activation settings");
                }
                nickel_core::plugins::PluginActivationSettings::default()
            });
        if plugin_activation.desired_enabled(crate::settings_plugin_report::ID, true) {
            plugin_registry.set_enabled(crate::settings_plugin_report::ID, true)?;
        }
        let plugin_panel_host = if plugin_activation.desired_enabled(
            &crate::plugin_panel::manifest().id,
            crate::plugin_panel::enabled(),
        ) {
            let id = &crate::plugin_panel::manifest().id;
            plugin_registry.set_enabled(id, true)?;
            match crate::plugin_panel::PluginPanelApplication::bundled() {
                Ok(application) => {
                    plugin_registry.mark_running(id)?;
                    Some(nickel_ui::UiHost::new(
                        application,
                        crate::plugin_panel::surface().width,
                        crate::plugin_panel::surface().height,
                    ))
                }
                Err(error) => {
                    tracing::error!(plugin = id, %error, "plugin failed to start");
                    plugin_registry.mark_failed(id, error)?;
                    None
                }
            }
        } else {
            None
        };
        let plugin_launcher_host = if plugin_activation.desired_enabled(
            &crate::plugin_panel::launcher_manifest().id,
            crate::plugin_panel::launcher_enabled(),
        ) {
            let id = &crate::plugin_panel::launcher_manifest().id;
            plugin_registry.set_enabled(id, true)?;
            let projection =
                crate::plugin_panel::LauncherPluginProjection::from_launcher(&launcher)
                    .with_status(application_status.clone());
            let images = launcher_plugin_images(&launcher, &mut launcher_icons, &projection);
            match bundled_surface_host(
                crate::plugin_panel::launcher_manifest(),
                projection.to_json(),
                images,
            ) {
                Ok(started) => {
                    plugin_registry.mark_running(id)?;
                    Some(started)
                }
                Err(error) => {
                    tracing::error!(plugin = id, %error, "plugin failed to start");
                    plugin_registry.mark_failed(id, error)?;
                    None
                }
            }
        } else {
            None
        };
        let plugin_run_host = if plugin_activation.desired_enabled(
            &crate::plugin_panel::run_manifest().id,
            crate::plugin_panel::run_enabled(),
        ) {
            let id = &crate::plugin_panel::run_manifest().id;
            plugin_registry.set_enabled(id, true)?;
            match bundled_surface_host(
                crate::plugin_panel::run_manifest(),
                serde_json::json!({ "status": null }).to_string(),
                crate::plugin_panel::PluginImages::new(),
            ) {
                Ok(started) => {
                    plugin_registry.mark_running(id)?;
                    Some(started)
                }
                Err(error) => {
                    tracing::error!(plugin = id, %error, "plugin failed to start");
                    plugin_registry.mark_failed(id, error)?;
                    None
                }
            }
        } else {
            None
        };
        let plugin_taskbar_host = if plugin_activation.desired_enabled(
            &crate::plugin_panel::taskbar_manifest().id,
            crate::plugin_panel::taskbar_enabled(),
        ) {
            let id = &crate::plugin_panel::taskbar_manifest().id;
            plugin_registry.set_enabled(id, true)?;
            let (projection, images) = taskbar_plugin_data(
                TaskbarProjectionInput {
                    groups: &initial_taskbar_groups,
                    keyboard_enabled: false,
                    codex_available: launcher.codex_available(),
                    panel_icon: &panel_icon,
                    codex_icon: &codex_icon,
                    task_icons: &[],
                    tray: &tray,
                    tray_icons: &tray_icons,
                },
                &clock,
            );
            match bundled_surface_host(
                crate::plugin_panel::taskbar_manifest(),
                projection.to_json(),
                images,
            ) {
                Ok((_, _, host)) => {
                    plugin_registry.mark_running(id)?;
                    Some(host)
                }
                Err(error) => {
                    tracing::error!(plugin = id, %error, "plugin failed to start");
                    plugin_registry.mark_failed(id, error)?;
                    None
                }
            }
        } else {
            None
        };
        let plugin_notification_host = if plugin_activation.desired_enabled(
            &crate::plugin_panel::notification_manifest().id,
            crate::plugin_panel::notification_enabled(),
        ) {
            let id = &crate::plugin_panel::notification_manifest().id;
            plugin_registry.set_enabled(id, true)?;
            let projection =
                crate::plugin_panel::NotificationPluginProjection::from_feed(None, &[], false);
            match bundled_surface_host(
                crate::plugin_panel::notification_manifest(),
                projection.to_json(),
                crate::plugin_panel::PluginImages::new(),
            ) {
                Ok(started) => {
                    plugin_registry.mark_running(id)?;
                    Some(started)
                }
                Err(error) => {
                    tracing::error!(plugin = id, %error, "plugin failed to start");
                    plugin_registry.mark_failed(id, error)?;
                    None
                }
            }
        } else {
            None
        };
        let plugin_volume_osd_host = if plugin_activation
            .desired_enabled(&crate::plugin_panel::volume_osd_manifest().id, true)
        {
            let id = &crate::plugin_panel::volume_osd_manifest().id;
            plugin_registry.set_enabled(id, true)?;
            let data = serde_json::json!({"audio": audio_plugin_data(&audio, false)});
            match bundled_surface_host(
                crate::plugin_panel::volume_osd_manifest(),
                data.to_string(),
                crate::plugin_panel::PluginImages::new(),
            ) {
                Ok(started) => {
                    plugin_registry.mark_running(id)?;
                    Some(started)
                }
                Err(error) => {
                    tracing::error!(plugin = id, %error, "plugin failed to start");
                    plugin_registry.mark_failed(id, error)?;
                    None
                }
            }
        } else {
            None
        };
        let launcher_icon_revision = launcher_icons.revision();
        let mut shell = Self {
            appearance_capabilities: Default::default(),
            preferences_capabilities: Default::default(),
            preferences_commit_pending: None,
            session_host: session_host.clone(),
            screenshot_capture_pending: false,
            screenshot_output: None,
            #[cfg(target_os = "linux")]
            active_window_capture: None,
            #[cfg(target_os = "linux")]
            active_window_capture_target: None,
            host_runtime_samples: HostRuntimeSamples::default(),
            launcher,
            window_feed,
            #[cfg(target_os = "linux")]
            internal_session_snapshot: None,
            #[cfg(target_os = "linux")]
            internal_workspaces: None,
            tray_feed,
            notification_feed,
            windows,
            window_icons: HashMap::new(),
            task_switcher: TaskSwitcher::default(),
            task_switcher_group: None,
            workspaces,
            configured_desktop_count: shell_settings.desktop_count,
            window_feed_status: FeedStatus::Loading,
            workspace_feed_status: FeedStatus::Loading,
            tray,
            tray_icons,
            notification: None,
            notification_history_visible: false,
            remote_lease_notifications: HashMap::new(),
            remote_lease_submitting: HashSet::new(),
            dismissed_remote_lease_notifications: HashSet::new(),
            remote_lease_overflow_rejections: HashSet::new(),
            codex_approval_notifications: HashMap::new(),
            codex_approval_decisions: Vec::new(),
            codex_approval_reviews: Vec::new(),
            codex_approval_delivery_updates: Vec::new(),
            dismissed_codex_approval_notifications: HashSet::new(),
            codex_approval_overflow_outcomes: HashSet::new(),
            #[cfg(target_os = "windows")]
            remote_lease_decisions: Vec::new(),
            wallpaper_path,
            wallpaper_source_fingerprint,
            wallpaper_loaded_source_fingerprint,
            wallpaper,
            wallpaper_size,
            desktop_host,
            desktop_viewports: HashMap::new(),
            desktop_active_viewport: "primary".into(),
            desktop_change_token: HostChangeToken::default(),
            desktop_deadline: None,
            desktop_application_dirty: false,
            desktop_overlay_pointer_capture: None,
            #[cfg(target_os = "windows")]
            output_identification: None,
            panel_icon,
            codex_icon,
            palette,
            network,
            bluetooth,
            audio,
            volume_osd_until: None,
            audio_status_observed: false,
            associations_results: HashMap::new(),
            launcher_visible: false,
            run_visible: false,
            locked: false,
            lock_host,
            lock_change_token: HostChangeToken::default(),
            lock_deadline: None,
            control_visible: false,
            codex_project_menu_visible: false,
            panel_hover: None,
            panel_hover_output: None,
            plugin_registry,
            package_settings_registry: Default::default(),
            package_settings_runtimes: Default::default(),
            package_settings_generation: 0,
            package_settings_values: Default::default(),
            package_settings_value_revisions: Default::default(),
            package_settings_invoking: false,
            plugin_settings,
            external_plugin_packages,
            primary_panel_key: crate::plugin_panel::surface_key(),
            plugin_activation_generation: 1,
            #[cfg(target_os = "linux")]
            last_published_plugin_status: None,
            plugin_surface_hosts: {
                let mut hosts = std::collections::BTreeMap::new();
                if let Some(host) = plugin_panel_host {
                    hosts.insert(
                        crate::plugin_panel::surface_key(),
                        (crate::plugin_panel::surface().clone(), host),
                    );
                }
                hosts
            },
            plugin_panel_memory: std::collections::BTreeMap::new(),
            plugin_window_placement_overrides: std::collections::BTreeMap::new(),
            #[cfg(target_os = "windows")]
            pending_plugin_surface_focus: None,
            plugin_taskbar_host,
            plugin_slot_hosts: std::collections::BTreeMap::new(),
            plugin_taskbar_hosts: HashMap::new(),
            plugin_taskbar_memory: HashMap::new(),
            plugin_taskbar_menu_memory: 0,
            panel_projections: HashMap::new(),
            panel_change_token: HostChangeToken::default(),
            panel_deadline: None,
            panel_pet_frame: 0,
            panel_pet_deadline: None,
            panel_output: None,
            pending_popover_anchor: None,
            all_windows_on_every_bar: shell_settings.all_windows_on_every_bar,
            #[cfg(target_os = "windows")]
            idle_policy: nickel_core::idle::IdlePolicy::from_seconds(
                shell_settings.idle_dim_seconds,
                shell_settings.idle_lock_seconds,
                shell_settings.idle_suspend_seconds,
            ),
            preview_group: None,
            preview_pending: None,
            preview_focus_requested: false,
            preview_pointer_inside: false,
            preview_leave_deadline: None,
            preview_hovered: None,
            preview_images: HashMap::new(),
            preview_refresh_deadline: None,
            window_menu: None,
            window_menu_snapshot: None,
            window_menu_generation: 0,
            window_menu_anchor_x: None,
            window_menu_anchor_y: None,
            window_menu_plugin_host: None,
            application_menu_target: None,
            application_menu_plugin_host: None,
            notification_host,
            panel_origin_x: 0,
            panel_origin_y: 0,
            control_host,
            control_change_token: HostChangeToken::default(),
            control_deadline: Some(Instant::now()),
            projection_chooser: Default::default(),
            projection_rollback_deadline: None,
            display_preview: None,
            launcher_icons,
            launcher_icon_revision,
            launcher_plugin_result_page: 0,
            launcher_plugin_dashboard_page: 0,
            launcher_status: application_status,
            #[cfg(target_os = "windows")]
            launcher_catalog_generation: 1,
            launcher_preference_persistence,
            launcher_preference_deadline: None,
            shortcut_action_status: None,
            shortcut_capability_status: None,
            #[cfg(test)]
            launcher_preferences_path: None,
            #[cfg(test)]
            launcher_persistence_attempts: 0,
            secure_storage_override: None,
            secure_storage_state,
            #[cfg(target_os = "linux")]
            secure_storage_query_error,
            requested_codex_project: None,
            codex_menu_requests: Vec::new(),
            screenshot: ScreenshotTool::default().with_session_host(session_host),
            keyboard_host: nickel_ui::UiHost::new(
                nickel_ui::on_screen_keyboard::KeyboardApp::new(palette),
                1280,
                nickel_core::on_screen_keyboard::KEYBOARD_HEIGHT,
            ),
            keyboard_plugin_generation: 1,
            keyboard_visible: false,
            keyboard_enabled,
            #[cfg(target_os = "windows")]
            keyboard_generation: optional_feature_settings.on_screen_keyboard_generation,
            #[cfg(target_os = "windows")]
            keyboard_touchscreen_present,
            keyboard_dock_top: false,
            keyboard_height: nickel_core::on_screen_keyboard::KEYBOARD_HEIGHT,
            keyboard_resize: None,
            keyboard_deadline: Instant::now(),
            keyboard_gesture_leases: HashMap::new(),
            keyboard_override,
            keyboard_recipient: None,
        };
        for (key, surface, host) in [
            plugin_volume_osd_host,
            plugin_run_host,
            plugin_launcher_host,
            plugin_notification_host,
        ]
        .into_iter()
        .flatten()
        {
            shell.plugin_surface_hosts.insert(key, (surface, host));
        }
        if plugin_activation
            .desired_enabled(&crate::plugin_panel::control_center_manifest().id, true)
        {
            let data = shell.control_plugin_data(720);
            shell.start_initial_bundled_surface(
                crate::plugin_panel::control_center_manifest(),
                data.to_string(),
            )?;
        }
        if plugin_activation
            .desired_enabled(&crate::plugin_panel::codex_projects_manifest().id, true)
        {
            let id = &crate::plugin_panel::codex_projects_manifest().id;
            shell.plugin_registry.set_enabled(id, true)?;
            let projection = nickel_codex_ui::ProjectMenuProjection::from_state(
                &nickel_codex_ui::ChatState::default(),
            );
            match serde_json::to_string(&projection)
                .map_err(|error| error.to_string())
                .and_then(|data| {
                    shell.start_initial_bundled_surface(
                        crate::plugin_panel::codex_projects_manifest(),
                        data,
                    )
                }) {
                Ok(()) => {}
                Err(error) => {
                    tracing::error!(plugin = id, %error, "plugin failed to start");
                    shell.plugin_registry.mark_failed(id, error)?;
                }
            }
        }
        if plugin_activation
            .desired_enabled(&crate::plugin_panel::on_screen_keyboard_manifest().id, true)
        {
            let data = shell.keyboard_plugin_data();
            shell.start_initial_bundled_surface(
                crate::plugin_panel::on_screen_keyboard_manifest(),
                data.to_string(),
            )?;
        }
        if plugin_activation
            .desired_enabled(&crate::plugin_panel::window_preview_manifest().id, true)
        {
            let data = serde_json::json!({"windows": []});
            shell.start_initial_bundled_surface(
                crate::plugin_panel::window_preview_manifest(),
                data.to_string(),
            )?;
        }
        #[cfg(not(test))]
        for id in shell
            .external_plugin_packages
            .keys()
            .cloned()
            .collect::<Vec<_>>()
        {
            if should_auto_start_installed_plugin(
                plugin_activation.desired_enabled(&id, false),
                safe_mode,
            ) {
                if let Some(descriptor) = shell.external_plugin_packages.get(&id)
                    && !plugin_activation
                        .approval_current(&descriptor.manifest, &descriptor.source_digest)
                {
                    let reason =
                        "Plugin package changed; review access in Settings before enabling";
                    shell.plugin_registry.mark_failed(&id, reason.into())?;
                    continue;
                }
                if let Err(error) = shell.set_plugin_enabled(&id, true) {
                    tracing::warn!(plugin = %id, %error, "installed plugin could not start");
                }
            }
        }
        shell.maybe_publish_plugin_status();
        Ok(shell)
    }

    pub fn refresh(&mut self) -> bool {
        let fast = self.refresh_fast();
        let system = self.refresh_system();
        fast || system
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn apply_internal_session_snapshot(
        &mut self,
        snapshot: nickel_session_protocol::Snapshot,
    ) {
        self.internal_workspaces = Some(
            snapshot
                .workspaces
                .ordered
                .iter()
                .map(|workspace| platform::WorkspaceSummary {
                    id: workspace.id.0,
                    active: workspace.id == snapshot.workspaces.active,
                })
                .collect(),
        );
        self.active_window_capture_target = active_window_capture_target(&snapshot);
        self.internal_session_snapshot = Some(snapshot);
    }

    #[cfg(target_os = "linux")]
    fn request_active_window_capture(&mut self, copy_path: bool) -> Result<(), String> {
        let (output_name, output, window) = self
            .active_window_capture_target
            .clone()
            .ok_or_else(|| "Nickel has no focused capturable application window".to_owned())?;
        self.screenshot_output = Some(output_name);
        self.active_window_capture = Some(ActiveWindowCapture {
            output,
            window,
            copy_path,
        });
        if copy_path {
            self.screenshot.request_capture_to_file();
        } else {
            self.screenshot.request_capture();
        }
        Ok(())
    }

    pub fn image_cache_diagnostics(&self) -> ShellImageCacheDiagnostics {
        self.image_cache_diagnostics_for_previews(|_| true)
    }

    pub(crate) fn image_cache_diagnostics_for_previews(
        &self,
        allow_preview: impl Fn(crate::model::WindowId) -> bool,
    ) -> ShellImageCacheDiagnostics {
        let launcher = self.launcher_icons.diagnostics();
        let (preview_entries, preview_bytes) = self
            .preview_images
            .iter()
            .filter(|(id, _)| allow_preview(**id))
            .fold((0usize, 0usize), |(entries, bytes), (_, image)| {
                (
                    entries.saturating_add(1),
                    bytes.saturating_add(image.as_raw().len()),
                )
            });
        let wallpaper_bytes = self
            .wallpaper
            .as_ref()
            .map_or(0, |image| image.as_raw().len());
        ShellImageCacheDiagnostics {
            launcher_icon_entries: launcher.entries,
            launcher_icon_bytes: launcher.retained_pixel_bytes,
            wallpaper_entries: usize::from(self.wallpaper.is_some()),
            wallpaper_bytes,
            tray_entries: self.tray.len().saturating_add(self.tray_icons.len()),
            tray_bytes: self
                .tray
                .iter()
                .map(|item| item.icon.as_raw().len())
                .chain(self.tray_icons.iter().map(|image| image.as_raw().len()))
                .sum(),
            preview_entries,
            preview_bytes,
        }
    }

    pub fn refresh_fast(&mut self) -> bool {
        !self.refresh_fast_changes().is_empty()
    }

    pub(crate) fn refresh_fast_changes(&mut self) -> Vec<SurfaceRole> {
        #[cfg(target_os = "linux")]
        self.sync_remote_lease_notifications();
        self.refresh_fast_changes_with_preview_source(|feed, window| feed.preview(window))
    }

    // The source transfers an owned frame; refresh owns comparison, admission,
    // retirement and timing. Keep those policies identical for every provider.
    fn refresh_fast_changes_with_preview_source(
        &mut self,
        mut preview_source: impl FnMut(
            &platform::WindowFeed,
            crate::model::WindowId,
        ) -> Option<crate::model::WindowPreview>,
    ) -> Vec<SurfaceRole> {
        let mut redraw = Vec::new();
        let mut changed = false;
        #[cfg(target_os = "linux")]
        let windows = self.internal_session_snapshot.take().map_or_else(
            || self.window_feed.snapshot(&self.launcher),
            |snapshot| {
                FeedState::Ready(
                    self.window_feed
                        .apply_internal_snapshot(snapshot, &self.launcher),
                )
            },
        );
        #[cfg(not(target_os = "linux"))]
        let windows = self.window_feed.snapshot(&self.launcher);
        if update_feed_status(&mut self.window_feed_status, windows.status(), "windows") {
            changed = true;
        }
        if let FeedState::Ready(windows) = windows {
            if windows != self.windows {
                self.windows = windows;
                changed = true;
            }
            let previous_icon_count = self.window_icons.len();
            self.window_icons
                .retain(|window, _| self.windows.iter().any(|item| item.id == *window));
            for window in &self.windows {
                if !self.window_icons.contains_key(&window.id)
                    && let Some(icon) = self.window_feed.icon(window.id)
                {
                    self.window_icons.insert(window.id, Arc::new(icon));
                }
            }
            changed |= self.window_icons.len() != previous_icon_count;
            let stale_window_menu = self.window_menu_snapshot.as_ref().map_or_else(
                || {
                    self.window_menu.is_some_and(|target| {
                        !self.windows.iter().any(|window| window.id == target)
                    })
                },
                |captured| {
                    self.windows
                        .iter()
                        .find(|window| window.id == captured.id)
                        .is_none_or(|current| {
                            !crate::window_preview::same_window_identity(captured, current)
                        })
                },
            );
            if stale_window_menu {
                self.close_window_preview();
                changed = true;
            }
            if self.application_menu_target.as_ref().is_some_and(|target| {
                let canonical_item_available = target
                    .application_id
                    .as_ref()
                    .is_some_and(|application| self.launcher.is_pinned(application.as_str()));
                !target.survives(&self.windows, canonical_item_available)
            }) {
                self.close_window_preview();
                changed = true;
            }
            if let Some(snapshot) = self.window_menu_snapshot.as_mut()
                && let Some(window) = self.windows.iter().find(|window| window.id == snapshot.id)
                && window != snapshot
            {
                snapshot.clone_from(window);
                changed = true;
            }
        }
        if changed {
            redraw.extend([
                SurfaceRole::Taskbar,
                SurfaceRole::Panel,
                SurfaceRole::Launcher,
                SurfaceRole::WindowPreview,
                SurfaceRole::WindowContextMenu,
            ]);
        }
        #[cfg(target_os = "linux")]
        let workspaces = self
            .internal_workspaces
            .take()
            .map_or_else(|| self.window_feed.workspaces(), FeedState::Ready);
        #[cfg(not(target_os = "linux"))]
        let workspaces = self.window_feed.workspaces();
        if update_feed_status(
            &mut self.workspace_feed_status,
            workspaces.status(),
            "workspaces",
        ) {
            redraw.push(SurfaceRole::Launcher);
        }
        if let FeedState::Ready(workspaces) = workspaces
            && workspaces != self.workspaces
        {
            self.workspaces = workspaces;
            self.desktop_host.application_mut().set_workspace(
                self.workspaces
                    .iter()
                    .find(|workspace| workspace.active)
                    .map(|workspace| workspace.id),
            );
            if self.window_menu.is_none() && self.application_menu_target.is_none() {
                self.close_window_preview();
            }
            redraw.extend([
                SurfaceRole::Desktop,
                SurfaceRole::Taskbar,
                SurfaceRole::WindowPreview,
                SurfaceRole::WindowContextMenu,
            ]);
        }
        changed = false;
        if self
            .preview_leave_deadline
            .is_some_and(|deadline| Instant::now() >= deadline)
        {
            self.close_window_preview();
            changed = true;
        }
        if let Some((index, deadline)) = self.preview_pending
            && Instant::now() >= deadline
        {
            self.preview_pending = None;
            self.open_window_preview(index);
            changed = true;
        }
        let preview_group = self.preview_group.and_then(|index| {
            self.panel_groups()
                .get(index)
                .map(|task| task.window_group())
        });
        let preview_refresh_now = Instant::now();
        if let Some(group) = preview_group
            && preview_refresh_due(self.preview_refresh_deadline, preview_refresh_now)
        {
            retain_preview_generation(&mut self.preview_images, &group.windows);
            for window in group.windows.iter().take(PREVIEW_CACHE_CAPACITY) {
                if let Some(preview) = preview_source(&self.window_feed, window.id) {
                    changed |=
                        update_preview_image(&mut self.preview_images, window.id, preview.image);
                }
            }
            self.preview_refresh_deadline = Some(preview_refresh_now + PREVIEW_REFRESH_INTERVAL);
        }
        if changed {
            redraw.extend([SurfaceRole::WindowPreview, SurfaceRole::WindowContextMenu]);
        }
        let icon_revision = self.launcher_icons.revision();
        if icon_revision != self.launcher_icon_revision {
            self.launcher_icon_revision = icon_revision;
            redraw.extend([SurfaceRole::Launcher, SurfaceRole::Taskbar]);
        }
        let tray = normalize_tray_items(self.tray_feed.snapshot());
        if tray != self.tray {
            self.tray = tray;
            self.tray_icons = panel_tray_icons(&self.tray);
            redraw.push(SurfaceRole::Taskbar);
        }
        let notification = self.notification_feed.snapshot();
        let notification = if notification.as_ref().is_some_and(|item| {
            self.dismissed_remote_lease_notifications.contains(&item.id)
                || self
                    .dismissed_codex_approval_notifications
                    .contains(&item.id)
        }) {
            self.notification_feed.history().into_iter().find(|item| {
                !self.dismissed_remote_lease_notifications.contains(&item.id)
                    && !self
                        .dismissed_codex_approval_notifications
                        .contains(&item.id)
            })
        } else {
            notification
        };
        if !self.notification_history_visible && notification != self.notification {
            self.notification = notification;
            self.notification_host
                .application_mut()
                .sync(self.notification.as_ref(), self.palette);
            self.notification_host.step(HostBatch {
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            redraw.push(SurfaceRole::Notification);
        }
        redraw
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn refresh_secure_storage(&mut self) -> bool {
        let secure_storage_state = match self.session_host.secure_storage_state() {
            Ok(state) => {
                if self.secure_storage_query_error.take().is_some() {
                    tracing::info!("secure-storage session query recovered");
                }
                state
            }
            Err(error) => {
                let now = Instant::now();
                let should_log =
                    self.secure_storage_query_error
                        .as_ref()
                        .is_none_or(|(previous, logged)| {
                            previous != &error
                                || now.duration_since(*logged) >= RECURRING_DIAGNOSTIC_INTERVAL
                        });
                if should_log {
                    tracing::warn!(%error, "secure-storage query failed during shell refresh");
                    self.secure_storage_query_error = Some((error, now));
                }
                platform::SecureStorageState::ControlUnavailable
            }
        };
        let mut changed = secure_storage_state != self.secure_storage_state;
        self.secure_storage_state = secure_storage_state;
        if self.launcher_status.is_some()
            && secure_storage_state == platform::SecureStorageState::Ready
        {
            self.launcher_status = None;
            self.secure_storage_override = None;
            changed = true;
        }
        changed
    }

    pub fn refresh_system(&mut self) -> bool {
        #[cfg(target_os = "linux")]
        let mut changed = self.refresh_secure_storage();
        #[cfg(not(target_os = "linux"))]
        let mut changed = false;
        changed |= self.appearance_capabilities.refresh_observed();
        if let Ok(catalog) = self.preferences_catalog() {
            let previous = self.preferences_capabilities.snapshot(&catalog);
            let next = self.preferences_capabilities.refresh(&catalog);
            changed |= previous != next;
        }
        let shell_settings = ShellSettings::load_default();
        let wallpaper_settings = WallpaperSettings::load_default();
        if self.refresh_configured_wallpaper(wallpaper_settings.image) {
            changed = true;
        }
        changed |= self.apply_shell_settings(shell_settings);
        let network = platform::network_status();
        if network != self.network {
            self.network = network;
            changed = true;
        }
        let bluetooth = platform::bluetooth_status();
        if bluetooth != self.bluetooth {
            self.bluetooth = bluetooth;
            changed = true;
        }
        let audio = platform::audio_status();
        if audio != self.audio {
            self.audio = audio;
            changed = true;
        }
        changed
    }

    pub(crate) fn apply_application_discovery(
        &mut self,
        discovery: crate::model::ApplicationDiscovery,
    ) -> (usize, bool) {
        #[cfg(target_os = "windows")]
        let previous_ids = self
            .launcher
            .discovered_applications()
            .map(|application| application.id().to_owned())
            .collect::<Vec<_>>();
        let applications = discovery.applications().len();
        let partial = matches!(
            discovery.status(),
            crate::model::ApplicationDiscoveryStatus::PartialFailure
        );
        self.launcher_status = application_discovery_status_label(discovery.status());
        self.launcher
            .replace_discovered_applications(discovery.into_applications());
        #[cfg(target_os = "windows")]
        if previous_ids
            != self
                .launcher
                .discovered_applications()
                .map(|application| application.id().to_owned())
                .collect::<Vec<_>>()
        {
            self.launcher_catalog_generation =
                self.launcher_catalog_generation.checked_add(1).unwrap_or(0);
        }
        self.launcher_icons.invalidate_application_inventory();
        (applications, partial)
    }

    pub(crate) fn apply_shell_settings(&mut self, shell_settings: ShellSettings) -> bool {
        let mut changed = false;
        self.launcher.set_places(crate::places::applications(
            shell_settings.preferred_file_manager.as_deref(),
        ));
        if self.all_windows_on_every_bar != shell_settings.all_windows_on_every_bar {
            self.all_windows_on_every_bar = shell_settings.all_windows_on_every_bar;
            self.close_window_preview();
            changed = true;
        }
        if self.configured_desktop_count != shell_settings.desktop_count {
            self.configured_desktop_count = shell_settings.desktop_count;
            changed = true;
        }
        #[cfg(target_os = "windows")]
        {
            let idle_policy = nickel_core::idle::IdlePolicy::from_seconds(
                shell_settings.idle_dim_seconds,
                shell_settings.idle_lock_seconds,
                shell_settings.idle_suspend_seconds,
            );
            if self.idle_policy != idle_policy {
                self.idle_policy = idle_policy;
                changed = true;
            }
        }
        let palette = ThemePalette::from_appearance(
            shell_settings.resolve_appearance(crate::appearance_capabilities::system_appearance()),
        );
        if palette != self.palette {
            self.palette = palette;
            self.lock_host.application_mut().palette = palette;
            self.launcher_icons.begin_visual_generation();
            if let Some(icon) = crate::icons::load_svg_bytes(
                include_bytes!("../../../assets/icons/nickel-start.svg"),
                96,
            ) {
                self.panel_icon = Arc::new(tint_panel_icon(icon, palette.text));
            }
            if let Some(icon) = crate::icons::load_svg_bytes(
                include_bytes!("../../../assets/icons/nickel-chat.svg"),
                96,
            ) {
                self.codex_icon = Arc::new(tint_panel_icon(icon, palette.text));
            }
            changed = true;
        }
        changed
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn windows_idle_preferences(
        &self,
    ) -> nickel_remote_control::idle_preferences::Preferences {
        use nickel_remote_control::idle_preferences::{Preferences, Timeout};
        let timeout = |value: Option<Duration>| {
            value.map_or(Timeout::Disabled, |duration| {
                Timeout::AfterSeconds(u32::try_from(duration.as_secs()).unwrap_or(u32::MAX))
            })
        };
        Preferences {
            dim: timeout(self.idle_policy.dim_after),
            suspend: timeout(self.idle_policy.suspend_after),
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn apply_file_icon_settings(&mut self, shell_settings: ShellSettings) {
        self.launcher_icons.begin_visual_generation();
        self.launcher_icons.invalidate_application_inventory();
        self.apply_shell_settings(shell_settings);
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn apply_wallpaper_settings(&mut self, settings: WallpaperSettings) {
        self.refresh_configured_wallpaper(settings.image);
        // Position is persisted by the production wallpaper owner. Request a
        // new desktop frame even when the image source itself did not change;
        // the response still does not claim that pixels were presented.
        self.desktop_application_dirty = true;
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn remote_shell_behavior_state(&self) -> (bool, u8, usize) {
        (
            self.all_windows_on_every_bar,
            self.configured_desktop_count,
            self.workspaces.len(),
        )
    }

    /// Apply a platform transition delivered by the compositor event loop.
    /// This avoids waiting for the retired client shell's subscription pump.
    pub(crate) fn apply_system_status_update(
        &mut self,
        update: platform::SystemStatusUpdate,
    ) -> bool {
        let activity = match &update {
            platform::SystemStatusUpdate::AudioWithActivity { activity, .. } => *activity,
            _ => Default::default(),
        };
        match update {
            platform::SystemStatusUpdate::Network(status) => {
                if self.network == status {
                    false
                } else {
                    self.network = status;
                    true
                }
            }
            platform::SystemStatusUpdate::Bluetooth(status) => {
                if self.bluetooth == status {
                    false
                } else {
                    self.bluetooth = status;
                    true
                }
            }
            platform::SystemStatusUpdate::Audio(status)
            | platform::SystemStatusUpdate::AudioWithActivity { status, .. } => {
                let value_changed = self.audio.volume_percent != status.volume_percent
                    || self.audio.muted != status.muted;
                let show = self.audio_status_observed
                    && self.audio.available
                    && status.available
                    && (activity.value_changed
                        || (!activity.availability_changed && value_changed));
                self.audio_status_observed = true;
                let changed = self.audio != status;
                self.audio = status;
                if show {
                    self.volume_osd_until = Some(Instant::now() + Duration::from_millis(1500));
                } else if !self.audio.available
                    || (activity.availability_changed && !activity.value_changed)
                {
                    return self.volume_osd_until.take().is_some() || changed;
                }
                changed || show
            }
            platform::SystemStatusUpdate::ShellSettingsChanged => self.refresh_system(),
            platform::SystemStatusUpdate::ApplicationInventory(discovery) => {
                platform::publish_application_discovery(&discovery);
                self.apply_application_discovery(discovery);
                true
            }
        }
    }

    fn refresh_configured_wallpaper(
        &mut self,
        configured_path: Option<std::path::PathBuf>,
    ) -> bool {
        let next_fingerprint = configured_path
            .as_deref()
            .and_then(wallpaper_source_fingerprint);
        if configured_path == self.wallpaper_path
            && next_fingerprint == self.wallpaper_source_fingerprint
        {
            return false;
        }

        self.wallpaper_path = configured_path;
        self.wallpaper_source_fingerprint = next_fingerprint;
        self.wallpaper_loaded_source_fingerprint = None;
        if self.wallpaper_path.is_none() {
            self.wallpaper_size = (0, 0);
            self.wallpaper = None;
        }
        self.desktop_application_dirty = true;
        true
    }

    pub fn semantic_theme(&self) -> nickel_ui::SemanticTheme {
        semantic_theme_from_palette(self.palette)
    }

    /// Only ordinary shell hosts have production protection evidence for remote capture.
    pub(crate) fn surface_remote_access_protected(&self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Desktop => {
                self.desktop_host.remote_access_protected()
                    || self
                        .desktop_viewports
                        .values()
                        .any(|viewport| viewport.host.remote_access_protected())
            }
            SurfaceRole::Taskbar => {
                self.plugin_taskbar_host
                    .as_ref()
                    .is_none_or(|host| host.remote_access_protected())
                    || self
                        .plugin_taskbar_hosts
                        .values()
                        .any(|host| host.remote_access_protected())
            }
            SurfaceRole::Launcher if self.run_visible => self
                .run_host_ref()
                .is_none_or(|host| host.remote_access_protected()),
            SurfaceRole::Launcher => self
                .launcher_host_ref()
                .is_none_or(|host| host.remote_access_protected()),
            SurfaceRole::ControlCenter => self
                .control_plugin_active()
                .then(|| {
                    self.control_plugin_host_ref()
                        .unwrap()
                        .remote_access_protected()
                })
                .unwrap_or_else(|| {
                    !self.control_host.application().view_state().projection_only
                        || self.control_host.remote_access_protected()
                }),
            SurfaceRole::Notification => {
                self.trusted_notification_visible()
                    || self.notification_plugin_host_ref().map_or_else(
                        || self.notification_host.remote_access_protected(),
                        |host| host.remote_access_protected(),
                    )
            }
            SurfaceRole::VolumeOsd => self
                .plugin_panel_host_ref(&crate::plugin_panel::volume_osd_surface_key())
                .is_none_or(|host| host.remote_access_protected()),
            SurfaceRole::WindowPreview => {
                self.preview_plugin_active()
                    || self
                        .preview_plugin_host_ref()
                        .is_none_or(|host| host.remote_access_protected())
            }
            SurfaceRole::WindowContextMenu => {
                if let Some(host) = self.window_menu_plugin_host.as_ref() {
                    host.remote_access_protected()
                } else if let Some(host) = self.application_menu_plugin_host.as_ref() {
                    host.remote_access_protected()
                } else {
                    true
                }
            }
            SurfaceRole::Screenshot => self.screenshot.remote_access_protected(),
            SurfaceRole::OnScreenKeyboard => self.keyboard_host.remote_access_protected(),
            _ => true,
        }
    }

    /// Computed production host layout for the opt-in nested test socket.
    pub(crate) fn layout_snapshot(
        &self,
        role: SurfaceRole,
        plugin: Option<&nickel_core::plugins::PluginSurfaceKey>,
        output: Option<&str>,
    ) -> Option<String> {
        if self.locked {
            return None;
        }
        let snapshot = match role {
            SurfaceRole::Desktop => output
                .filter(|output| *output != self.desktop_active_viewport)
                .and_then(|output| self.desktop_viewports.get(output))
                .map(|viewport| viewport.host.layout_snapshot())
                .or_else(|| Some(self.desktop_host.layout_snapshot())),
            SurfaceRole::Taskbar => {
                let host = if output == self.panel_output.as_deref() {
                    self.plugin_taskbar_host.as_ref()
                } else {
                    self.plugin_taskbar_hosts.get(&output.map(str::to_owned))
                };
                host.map(|host| host.layout_snapshot())
            }
            SurfaceRole::Panel => plugin
                .and_then(|key| self.plugin_panel_host_ref(key))
                .map(|host| host.layout_snapshot()),
            SurfaceRole::Launcher if self.run_visible => {
                self.run_host_ref().map(|host| host.layout_snapshot())
            }
            SurfaceRole::Launcher => self.launcher_host_ref().map(|host| host.layout_snapshot()),
            SurfaceRole::ControlCenter => self
                .control_plugin_host_ref()
                .map(|host| host.layout_snapshot())
                .or_else(|| Some(self.control_host.layout_snapshot())),
            SurfaceRole::Notification => self
                .notification_plugin_host_ref()
                .map(|host| host.layout_snapshot())
                .or_else(|| Some(self.notification_host.layout_snapshot())),
            SurfaceRole::VolumeOsd => self
                .plugin_panel_host_ref(&crate::plugin_panel::volume_osd_surface_key())
                .map(|host| host.layout_snapshot()),
            SurfaceRole::WindowPreview => self
                .preview_plugin_host_ref()
                .map(|host| host.layout_snapshot()),
            SurfaceRole::WindowContextMenu => self
                .window_menu_plugin_host
                .as_ref()
                .map(|host| host.layout_snapshot())
                .or_else(|| {
                    self.application_menu_plugin_host
                        .as_ref()
                        .map(|host| host.layout_snapshot())
                }),
            SurfaceRole::Screenshot => plugin
                .and_then(|key| self.plugin_surface_hosts.get(key))
                .map(|(_, host)| host.layout_snapshot())
                .or_else(|| Some(self.screenshot.layout_snapshot())),
            SurfaceRole::OnScreenKeyboard => Some(self.keyboard_host.layout_snapshot()),
            _ => None,
        };
        snapshot
    }

    pub fn scene(&mut self, role: SurfaceRole, width: u32, height: u32) -> Vec<PaintCommand> {
        let commands = match role {
            SurfaceRole::Desktop => self.desktop_scene(width, height),
            SurfaceRole::Taskbar => self.panel_scene(width, height),
            SurfaceRole::Panel => unreachable!("plugin panels render through their surface key"),
            SurfaceRole::Launcher if self.run_visible => self
                .plugin_panel_scene(&crate::plugin_panel::run_surface_key(), width, height)
                .unwrap_or_default(),
            SurfaceRole::Launcher => self
                .plugin_panel_scene(&crate::plugin_panel::launcher_surface_key(), width, height)
                .unwrap_or_default(),
            SurfaceRole::ControlCenter => {
                if self.control_plugin_active() {
                    self.plugin_panel_scene(
                        &crate::plugin_panel::control_center_surface_key(),
                        width,
                        height,
                    )
                    .unwrap_or_default()
                } else if self.control_host.application().view_state().projection_only {
                    self.sync_control_host(width, height);
                    self.control_host.commands().to_vec()
                } else {
                    Vec::new()
                }
            }
            SurfaceRole::Notification => {
                if self.trusted_notification_visible() {
                    self.sync_notification_host(width, height);
                    self.notification_host.commands().to_vec()
                } else if self.notification_plugin_active() {
                    self.plugin_panel_scene(
                        &crate::plugin_panel::notification_surface_key(),
                        width,
                        height,
                    )
                    .unwrap_or_default()
                } else {
                    Vec::new()
                }
            }
            SurfaceRole::VolumeOsd => self
                .plugin_panel_scene(
                    &crate::plugin_panel::volume_osd_surface_key(),
                    width,
                    height,
                )
                .unwrap_or_default(),
            SurfaceRole::WindowPreview => self.window_preview_scene(),
            SurfaceRole::WindowContextMenu => self.window_menu_scene(),
            SurfaceRole::Lock => self.lock_scene(width, height),
            SurfaceRole::Screenshot => self.screenshot.scene(width, height, self.palette),
            SurfaceRole::OnScreenKeyboard => {
                self.keyboard_host
                    .application_mut()
                    .set_palette(self.palette);
                self.keyboard_host.step(nickel_ui::HostBatch {
                    surface_size: Some((width, height)),
                    events: vec![nickel_ui::HostEvent::Poll],
                    ..Default::default()
                });
                self.keyboard_host.commands().to_vec()
            }
            SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat => Vec::new(),
            #[cfg(target_os = "windows")]
            SurfaceRole::TrustedControl => Vec::new(),
        };
        self.maybe_publish_plugin_status();
        commands
    }

    pub fn set_desktop_outputs(&mut self, outputs: Vec<DesktopOutput>) {
        let topology_changed = self.desktop_host.application().outputs != outputs;
        let live_outputs = outputs
            .iter()
            .map(|output| output.id.clone())
            .collect::<std::collections::HashSet<_>>();
        self.desktop_viewports
            .retain(|output, _| live_outputs.contains(output));
        self.desktop_host.application_mut().set_outputs(outputs);
        if topology_changed {
            self.desktop_application_dirty = true;
        }
    }

    pub fn desktop_output_projection(&self, output: &str) -> Option<(DesktopPoint, f32)> {
        self.desktop_host
            .application()
            .outputs
            .iter()
            .find(|candidate| candidate.id == output)
            .map(|candidate| {
                (
                    DesktopPoint {
                        x: candidate.work_area.x,
                        y: candidate.work_area.y,
                    },
                    candidate.scale,
                )
            })
    }

    pub fn set_desktop_output(&mut self, output: String, x: f32, y: f32, scale: f32) {
        let origin = DesktopPoint { x, y };
        if self.desktop_active_viewport != output {
            let next = self.desktop_viewports.remove(&output);
            let (
                mut next_application,
                next_host,
                next_token,
                next_deadline,
                next_overlay_pointer_capture,
            ) = match next {
                Some(viewport) => (
                    viewport.application,
                    Some(viewport.host),
                    viewport.change_token,
                    viewport.deadline,
                    viewport.overlay_pointer_capture,
                ),
                None => (
                    desktop::DesktopViewportState::new(output.clone(), origin, scale),
                    None,
                    HostChangeToken::default(),
                    None,
                    None,
                ),
            };
            next_application.set_projection(output.clone(), origin, scale);
            let previous_application = self
                .desktop_host
                .application_mut()
                .replace_viewport_state(next_application);
            let next_host = next_host.unwrap_or_else(|| self.desktop_host.new_viewport(1, 1));
            let previous_host = self.desktop_host.replace_viewport(next_host);
            let previous_output = std::mem::replace(&mut self.desktop_active_viewport, output);
            if self
                .desktop_host
                .application()
                .outputs
                .iter()
                .any(|candidate| candidate.id == previous_output)
            {
                self.desktop_viewports.insert(
                    previous_output,
                    DesktopSurfaceViewport {
                        host: previous_host,
                        application: previous_application,
                        change_token: self.desktop_change_token,
                        deadline: self.desktop_deadline,
                        overlay_pointer_capture: self.desktop_overlay_pointer_capture.take(),
                    },
                );
            }
            self.desktop_change_token = next_token;
            self.desktop_deadline = next_deadline;
            self.desktop_overlay_pointer_capture = next_overlay_pointer_capture;
            // The application model is shared and may have changed while this
            // surface was parked. Always rebuild its retained tree before use.
            self.desktop_application_dirty = true;
        } else {
            self.desktop_host
                .application_mut()
                .set_active_output(output, origin, scale);
        }
    }

    pub(crate) fn set_desktop_input_modifiers(&mut self, modifiers: &nickel_input::ModifierState) {
        self.desktop_host
            .application_mut()
            .set_input_modifiers(modifiers);
    }

    pub fn desktop_input(&mut self, event: nickel_input::InputEvent) -> bool {
        let (ingress, authority) =
            internal_normalized_ingress(event, None, "desktop", self.desktop_host.inspect(), None);
        self.desktop_host_event_authorized(ingress, Some(authority))
    }

    pub(crate) fn desktop_host_event_authorized(
        &mut self,
        ingress: HostEvent,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> bool {
        let event = normalized_input(&ingress)
            .expect("desktop host event must be normalized")
            .clone();
        if matches!(
            event,
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Leave { .. })
        ) {
            // Pointer position is not interaction focus: briefly moving away must
            // not dismiss the menu or clear selection. FocusLost below owns that
            // teardown; Leave only clears visual hover and host pointer state.
            let application = self.desktop_host.application_mut();
            let had_hover = application.pointer_seen;
            application.pointer_seen = false;
            let outcome = self.desktop_host.step(HostBatch {
                events: vec![ingress],
                normalized_authorities: authority.into_iter().collect(),
                application_changed: had_hover,
                ..Default::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
            return had_hover || outcome.changed;
        }
        if matches!(event, nickel_input::InputEvent::FocusLost { .. }) {
            // Focus can leave while another output owns the menu. Clear shared
            // selection and modifiers before either menu-ownership branch returns;
            // the corresponding key-up may be delivered to the newly focused client.
            let application = self.desktop_host.application_mut();
            application.modifiers = Default::default();
            application.layout.clear_selection();
        }
        let menu_belongs_to_active_output = self
            .desktop_host
            .application()
            .context_menu
            .as_ref()
            .is_none_or(|menu| menu.output == self.desktop_host.application().active_output);
        if !menu_belongs_to_active_output {
            let focus_departed = matches!(&event, nickel_input::InputEvent::FocusLost { .. });
            let outside_press = matches!(
                &event,
                nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                    edge: nickel_input::KeyEdge::Pressed,
                    ..
                }) | nickel_input::InputEvent::Touch(nickel_input::TouchEvent::Started { .. })
            );
            if outside_press || focus_departed {
                let reason = if focus_departed {
                    desktop::DesktopMenuDismissReason::FocusDeparted
                } else {
                    desktop::DesktopMenuDismissReason::OutsidePress
                };
                self.desktop_host
                    .application_mut()
                    .dismiss_context_menu(reason);
                self.desktop_host
                    .application_mut()
                    .cancel_pointer_transaction();
                self.desktop_overlay_pointer_capture = None;
                let outcome = self.desktop_host.step(HostBatch {
                    events: vec![HostEvent::Ui(UiEvent::Dismiss)],
                    application_changed: true,
                    ..HostBatch::default()
                });
                self.desktop_change_token = outcome.change_token;
                self.desktop_deadline = outcome.next_deadline;
                return true;
            }
            // Motion and passive events on another desktop surface do not
            // belong to the open menu's coordinate or focus scope.
            return false;
        }
        let pointer_cancelled = matches!(
            event,
            nickel_input::InputEvent::FocusLost { .. }
                | nickel_input::InputEvent::DeviceRemoved { .. }
        );
        if pointer_cancelled {
            let application = self.desktop_host.application_mut();
            if matches!(event, nickel_input::InputEvent::FocusLost { .. }) {
                application.dismiss_context_menu(desktop::DesktopMenuDismissReason::FocusDeparted);
            }
            application.cancel_pointer_transaction();
            self.desktop_overlay_pointer_capture = None;
        }
        if let nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
            button: nickel_input::PointerButton::Secondary,
            edge: nickel_input::KeyEdge::Pressed,
            position: Some(position),
            ..
        }) = &event
        {
            if self.desktop_host.inspect().open_overlay.is_some() {
                let _ = self.desktop_host.step(HostBatch {
                    events: vec![HostEvent::Ui(UiEvent::Dismiss)],
                    ..HostBatch::default()
                });
            }
            let application = self.desktop_host.application_mut();
            application.cancel_pointer_transaction();
            application.pointer_press(
                DesktopPoint {
                    x: position.x as f32,
                    y: position.y as f32,
                },
                true,
                application.modifiers,
            );
            let outcome = self.desktop_host.step(HostBatch {
                application_changed: true,
                ..HostBatch::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
            self.desktop_overlay_pointer_capture = Some(nickel_input::PointerButton::Secondary);
        } else if let nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
            button,
            edge: nickel_input::KeyEdge::Pressed,
            ..
        }) = &event
            && (self.desktop_host.inspect().open_overlay.is_some()
                || self.desktop_host.application().context_menu.is_some())
        {
            self.desktop_host
                .application_mut()
                .cancel_pointer_transaction();
            self.desktop_overlay_pointer_capture = Some(button.clone());
        }
        let captured_release = matches!(
            &event,
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                button,
                edge: nickel_input::KeyEdge::Released,
                ..
            }) if self.desktop_overlay_pointer_capture.as_ref() == Some(button)
        );
        let overlay_owns_event = self.desktop_host.application().context_menu.is_some()
            || self.desktop_host.inspect().open_overlay.is_some()
            || self.desktop_overlay_pointer_capture.is_some()
            || matches!(event, nickel_input::InputEvent::Touch(_))
            || pointer_cancelled
            || matches!(
                event,
                nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                    button: nickel_input::PointerButton::Secondary,
                    ..
                })
            );
        if overlay_owns_event {
            let outcome = self.desktop_host.step(HostBatch {
                events: vec![ingress],
                normalized_authorities: authority.into_iter().collect(),
                application_changed: pointer_cancelled,
                ..HostBatch::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
            if captured_release {
                self.desktop_overlay_pointer_capture = None;
            }
            return outcome.changed;
        }
        let coalesce_motion = matches!(
            &event,
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion { .. })
        );
        // Passive motion and scrolling must not snap the viewport back to the
        // selected item. Only keyboard/button selection changes request reveal.
        let reveal_selection = matches!(
            &event,
            nickel_input::InputEvent::Key(_)
                | nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button { .. })
        );
        let application = self.desktop_host.application_mut();
        let changed = match event {
            nickel_input::InputEvent::Key(key) => application.key(&key),
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Button {
                button,
                edge,
                position: Some(position),
                ..
            }) => {
                let point = DesktopPoint {
                    x: position.x as f32,
                    y: position.y as f32,
                };
                if edge == nickel_input::KeyEdge::Pressed
                    && button == nickel_input::PointerButton::Primary
                {
                    application.pointer_press(point, false, application.modifiers)
                } else if button == nickel_input::PointerButton::Primary {
                    application.pointer_release(point, Instant::now())
                } else {
                    false
                }
            }
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion {
                position,
                ..
            }) => application.pointer_motion(DesktopPoint {
                x: position.x as f32,
                y: position.y as f32,
            }),
            nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Axis {
                delta,
                discrete,
                ..
            }) => {
                let (cell_width, _) = application.layout.grid();
                // Fractional wheel lines live in delta, not the truncated integer
                // hint. Pixel deltas are already logical distances, not cell counts.
                let distance = if discrete.is_some() {
                    -delta.y as f32 * 3.0 * cell_width
                } else {
                    -delta.y as f32
                };
                application.scroll_overflow(distance)
            }
            _ => false,
        };
        let changed =
            changed | (reveal_selection && self.desktop_host.application_mut().reveal_active());
        if changed && coalesce_motion {
            self.desktop_application_dirty = true;
        } else if changed {
            let outcome = self.desktop_host.step(HostBatch {
                application_changed: true,
                ..HostBatch::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
        }
        changed
    }

    pub fn set_file_clipboard_available(&mut self, available: bool) {
        self.desktop_host.application_mut().file_clipboard_available = available;
    }

    pub fn desktop_controller(&mut self, action: ControllerAction) -> bool {
        let application = self.desktop_host.application_mut();
        let changed = match action {
            ControllerAction::Left => {
                application.layout.select_direction(-1, 0, false);
                true
            }
            ControllerAction::Right => {
                application.layout.select_direction(1, 0, false);
                true
            }
            ControllerAction::Up => {
                application.layout.select_direction(0, -1, false);
                true
            }
            ControllerAction::Down => {
                application.layout.select_direction(0, 1, false);
                true
            }
            ControllerAction::Confirm => {
                if let Some(id) = application.layout.active() {
                    application.request_activate(id);
                }
                true
            }
            ControllerAction::ContextMenu => {
                application.open_keyboard_context();
                true
            }
            ControllerAction::Cancel => {
                application.dismiss_context_menu(desktop::DesktopMenuDismissReason::Cancel);
                application.layout.clear_selection();
                true
            }
            ControllerAction::Launcher
            | ControllerAction::PreviousPane
            | ControllerAction::NextPane => false,
        };
        let changed = changed | application.reveal_active();
        if !changed {
            return false;
        }
        let outcome = self.desktop_host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
        self.desktop_change_token = outcome.change_token;
        self.desktop_deadline = outcome.next_deadline;
        true
    }

    pub fn desktop_file_drop(&mut self, source: &std::path::Path) -> bool {
        let application = self.desktop_host.application_mut();
        let target = application
            .hit(application.pointer_position)
            .and_then(|id| application.layout.items().iter().find(|item| item.id == id));
        if let Some(launcher) = target.filter(|item| {
            !item.entry.is_directory && nickel_file::is_application_launcher(&item.entry.path)
        }) {
            return match nickel_file::open_with_launcher(&launcher.entry.path, source) {
                Ok(()) => true,
                Err(error) => {
                    application.error = Some(format!(
                        "Could not open {} with {}: {error}",
                        source.display(),
                        launcher.entry.display_name()
                    ));
                    true
                }
            };
        }
        let destination = target
            .filter(|item| item.entry.is_directory)
            .map(|item| item.entry.path.clone())
            .unwrap_or_else(nickel_file::desktop_directory);
        match nickel_file::copy_into(source, &destination) {
            Ok(_) => application.refresh_directory(true),
            Err(error) => {
                application.error = Some(format!("Could not drop {}: {error}", source.display()));
                true
            }
        }
    }

    pub fn surface_visible(&self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Desktop => true,
            SurfaceRole::Taskbar => self.plugin_taskbar_host.is_some(),
            SurfaceRole::Panel => {
                self.plugin_taskbar_host.is_some() || !self.plugin_surface_hosts.is_empty()
            }
            SurfaceRole::Launcher => self.launcher_visible,
            SurfaceRole::ControlCenter => self.control_visible && self.control_surface_available(),
            SurfaceRole::Notification => {
                (self.notification.is_some() || self.notification_history_visible)
                    && (self.notification_plugin_host_ref().is_some()
                        || self.trusted_notification_visible())
            }
            SurfaceRole::VolumeOsd => {
                self.volume_osd_until.is_some()
                    && self
                        .plugin_panel_host_ref(&crate::plugin_panel::volume_osd_surface_key())
                        .is_some()
            }
            SurfaceRole::WindowPreview => {
                self.preview_plugin_host_ref().is_some()
                    && (self.preview_group.is_some() || self.task_switcher_group.is_some())
            }
            SurfaceRole::WindowContextMenu => {
                self.plugin_taskbar_host.is_some()
                    && (self.window_menu.is_some() || self.application_menu_target.is_some())
            }
            SurfaceRole::CodexProjectMenu => self.codex_project_menu_visible,
            SurfaceRole::Lock => self.locked,
            SurfaceRole::Screenshot => self.screenshot.visible(),
            SurfaceRole::OnScreenKeyboard => {
                self.keyboard_visible
                    && !self.plugin_surface_matches(
                        &crate::plugin_panel::on_screen_keyboard_surface_key(),
                    )
            }
            SurfaceRole::CodexChat => true,
            #[cfg(target_os = "windows")]
            SurfaceRole::TrustedControl => false,
        }
    }

    pub(crate) fn native_surface_visible(
        &self,
        role: SurfaceRole,
        key: Option<&nickel_core::plugins::PluginSurfaceKey>,
    ) -> bool {
        if key == Some(&crate::plugin_panel::codex_projects_surface_key()) {
            return role == SurfaceRole::Panel
                && self.codex_project_menu_visible
                && self.plugin_surface_matches(&crate::plugin_panel::codex_projects_surface_key());
        }
        if key == Some(&crate::plugin_panel::on_screen_keyboard_surface_key()) {
            return role == SurfaceRole::Panel
                && self.keyboard_visible
                && self.keyboard_enabled
                && self.plugin_surface_matches(&crate::plugin_panel::on_screen_keyboard_surface_key());
        }
        if role == SurfaceRole::CodexProjectMenu {
            return false;
        }
        if key == Some(&crate::plugin_panel::control_center_surface_key()) {
            return role == SurfaceRole::Panel
                && self.control_visible
                && self.control_plugin_active();
        }
        if role == SurfaceRole::ControlCenter {
            return self.control_visible
                && self.control_host.application().view_state().projection_only;
        }
        if key == Some(&crate::plugin_panel::notification_surface_key()) {
            return role == SurfaceRole::Panel
                && self.notification_plugin_active()
                && (self.notification.is_some() || self.notification_history_visible);
        }
        if role == SurfaceRole::Notification {
            return self.trusted_notification_visible();
        }
        self.surface_visible(role) && key.is_none_or(|key| self.plugin_surface_matches(key))
    }

    pub(crate) fn taskbar_reservation_height(&self) -> u32 {
        let key = self.taskbar_surface_key();
        self.shell_panel_surfaces()
            .into_iter()
            .find(|(candidate, surface)| {
                key.as_ref() == Some(candidate) && surface.reserve_work_area
            })
            .map_or(0, |(_, surface)| surface.height)
    }

    pub(crate) fn taskbar_surface_key(&self) -> Option<nickel_core::plugins::PluginSurfaceKey> {
        self.plugin_taskbar_host.as_ref()?;
        let manifest = &self
            .plugin_registry
            .get(&crate::plugin_panel::taskbar_manifest().id)?
            .manifest;
        let surface = manifest
            .surfaces
            .iter()
            .find(|surface| surface.reserve_work_area)?;
        Some(nickel_core::plugins::PluginSurfaceKey {
            plugin_id: manifest.id.clone(),
            surface_id: surface.id.clone(),
        })
    }

    pub(crate) fn active_launcher_surface_key(
        &self,
    ) -> Option<nickel_core::plugins::PluginSurfaceKey> {
        if self.run_visible {
            self.run_host_ref()?;
            Some(crate::plugin_panel::run_surface_key())
        } else {
            self.launcher_host_ref()?;
            Some(crate::plugin_panel::launcher_surface_key())
        }
    }

    pub fn plugin_registry(&self) -> &nickel_core::plugins::PluginRegistry {
        &self.plugin_registry
    }

    pub(crate) fn plugin_panel_surface(&self) -> &nickel_core::plugins::PluginSurface {
        self.plugin_surface_hosts
            .get(&self.primary_panel_key)
            .map(|(surface, _)| surface)
            .unwrap_or_else(|| crate::plugin_panel::surface())
    }

    pub(crate) fn plugin_surface_matches(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> bool {
        self.plugin_surface_hosts.contains_key(key)
            || self
                .shell_panel_surfaces()
                .iter()
                .any(|(candidate, _)| candidate == key)
    }

    pub(crate) fn plugin_panels(
        &self,
    ) -> Vec<(
        nickel_core::plugins::PluginSurfaceKey,
        nickel_core::plugins::PluginSurface,
    )> {
        let mut panels = Vec::new();
        if let Some((surface, _)) = self.plugin_surface_hosts.get(&self.primary_panel_key) {
            panels.push((self.primary_panel_key.clone(), surface.clone()));
        }
        panels.extend(
            self.plugin_surface_hosts
                .iter()
                .filter(|(key, _)| {
                    **key != self.primary_panel_key()
                        && self.external_plugin_packages.contains_key(&key.plugin_id)
                })
                .map(|(key, (surface, _))| (key.clone(), surface.clone())),
        );
        panels
    }

    /// Active panel declarations for compositor-owned output surfaces. The
    /// public package panel list excludes the bundled taskbar because its
    /// activation is managed with the other first-party shell plugins.
    pub(crate) fn shell_panel_surfaces(
        &self,
    ) -> Vec<(
        nickel_core::plugins::PluginSurfaceKey,
        nickel_core::plugins::PluginSurface,
    )> {
        let mut panels = Vec::new();
        if let Some(key) = self.taskbar_surface_key()
            && let Some(surface) = self.plugin_registry.get(&key.plugin_id).and_then(|entry| {
                entry
                    .manifest
                    .surfaces
                    .iter()
                    .find(|surface| surface.id == key.surface_id)
            })
        {
            panels.push((key, surface.clone()));
        }
        panels.extend(self.plugin_panels());
        let codex_key = crate::plugin_panel::codex_projects_surface_key();
        if let Some((surface, _)) = self.plugin_surface_hosts.get(&codex_key) {
            panels.push((codex_key, surface.clone()));
        }
        let keyboard_key = crate::plugin_panel::on_screen_keyboard_surface_key();
        if let Some((surface, _)) = self.plugin_surface_hosts.get(&keyboard_key) {
            panels.push((keyboard_key, surface.clone()));
        }
        if self.notification_plugin_host_ref().is_some() {
            panels.push((
                crate::plugin_panel::notification_surface_key(),
                crate::plugin_panel::notification_surface().clone(),
            ));
        }
        if self.control_plugin_host_ref().is_some() {
            panels.push((
                crate::plugin_panel::control_center_surface_key(),
                crate::plugin_panel::control_center_surface().clone(),
            ));
        }
        panels
    }

    pub(crate) fn shell_fixed_surface_keys(
        &self,
    ) -> HashSet<nickel_core::plugins::PluginSurfaceKey> {
        [
            crate::plugin_panel::launcher_surface_key(),
            crate::plugin_panel::run_surface_key(),
            crate::plugin_panel::volume_osd_surface_key(),
            crate::plugin_panel::window_preview_surface_key(),
        ]
        .into_iter()
        .filter(|key| self.plugin_surface_matches(key))
        .collect()
    }

    pub(crate) fn plugin_panel_placement(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<(
        nickel_core::plugins::PluginSurfaceKind,
        u32,
        nickel_core::plugins::PluginSurfaceAnchor,
        i32,
        i32,
    )> {
        self.shell_panel_surfaces()
            .into_iter()
            .find(|(candidate, _)| candidate == key)
            .map(|(_, surface)| {
                (
                    surface.kind,
                    surface.bottom_offset,
                    surface.anchor,
                    surface.offset_x,
                    surface.offset_y,
                )
            })
    }

    pub(crate) fn plugin_panel_change_token(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<HostChangeToken> {
        let inspection = if self.taskbar_surface_key().as_ref() == Some(key) {
            self.plugin_taskbar_host.as_ref()?.inspect()
        } else {
            self.plugin_panel_host_ref(key)?.inspect()
        };
        // A settings edit replaces the host, whose generation restarts at zero.
        // Include the activation revision so the presenter cannot reuse the old frame.
        Some(HostChangeToken {
            frame_generation: inspection
                .frame_generation
                .wrapping_add(self.plugin_activation_generation.rotate_left(32)),
            semantic_generation: inspection
                .semantic_generation
                .wrapping_add(self.plugin_activation_generation.rotate_left(32)),
        })
    }

    pub(crate) fn plugin_panel_title(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<&str> {
        self.plugin_panel_host_ref(key)
            .map(|host| host.application().title())
    }

    #[cfg(test)]
    pub(crate) fn plugin_surface_semantic_nodes(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<Vec<nickel_ui::SemanticNodeSnapshot>> {
        self.plugin_surface_hosts
            .get(key)
            .map(|(_, host)| host.semantic_nodes())
    }

    pub(crate) fn plugin_surface_change_token(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<HostChangeToken> {
        if *key == crate::plugin_panel::control_center_surface_key() {
            if !self.control_plugin_active() {
                return None;
            }
            let inspection = self.control_plugin_host_ref()?.inspect();
            return Some(HostChangeToken {
                frame_generation: inspection
                    .frame_generation
                    .wrapping_add(self.plugin_activation_generation.rotate_left(32)),
                semantic_generation: inspection
                    .semantic_generation
                    .wrapping_add(self.plugin_activation_generation.rotate_left(32)),
            });
        }
        if *key == crate::plugin_panel::notification_surface_key() {
            if !self.notification_plugin_active() {
                return None;
            }
            let inspection = self.notification_plugin_host_ref()?.inspect();
            return Some(HostChangeToken {
                frame_generation: inspection
                    .frame_generation
                    .wrapping_add(self.plugin_activation_generation.rotate_left(32)),
                semantic_generation: inspection
                    .semantic_generation
                    .wrapping_add(self.plugin_activation_generation.rotate_left(32)),
            });
        }
        let (inspection, surface_salt) = if *key == crate::plugin_panel::launcher_surface_key() {
            (self.launcher_host_ref()?.inspect(), 0)
        } else if *key == crate::plugin_panel::run_surface_key() {
            // Both plugins share one native popup. Distinguish their tokens
            // even when their host-local frame generations happen to match.
            (self.run_host_ref()?.inspect(), 1_u64 << 63)
        } else if *key == crate::plugin_panel::window_preview_surface_key() {
            if !self.preview_plugin_active() {
                return None;
            }
            (self.preview_plugin_host_ref()?.inspect(), 0)
        } else {
            return self.plugin_panel_change_token(key);
        };
        Some(HostChangeToken {
            frame_generation: inspection
                .frame_generation
                .wrapping_add(self.plugin_activation_generation.rotate_left(32))
                .wrapping_add(surface_salt),
            semantic_generation: inspection
                .semantic_generation
                .wrapping_add(self.plugin_activation_generation.rotate_left(32))
                .wrapping_add(surface_salt),
        })
    }

    fn external_plugin_windows(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let package = self.external_plugin_packages.get(plugin_id)?;
        if !package
            .manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::WindowsRead)
        {
            return None;
        }
        Some(serde_json::Value::Array(
            self.windows
                .iter()
                .take(128)
                .map(|window| {
                    serde_json::json!({
                        "id": window.id.0.to_string(),
                        "applicationId": window.application_id.as_ref().map(|id| id.as_str()),
                        "title": window.title.chars().take(120).collect::<String>(),
                        "active": window.active,
                        "minimized": window.state.minimized,
                        "workspace": window.state.workspace,
                        "output": window.state.output,
                        "canActivate": window.state.capabilities.activate,
                        "canClose": window.state.capabilities.close,
                    })
                })
                .collect(),
        ))
    }

    fn external_plugin_notifications(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let package = self.external_plugin_packages.get(plugin_id)?;
        if !package
            .manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::NotificationsRead)
        {
            return None;
        }
        let history = self
            .notification_feed
            .history()
            .into_iter()
            .filter(|notification| !self.trusted_notification_id(notification.id))
            .collect::<Vec<_>>();
        let mut projection = crate::plugin_panel::NotificationPluginProjection::from_feed(
            self.notification
                .as_ref()
                .filter(|notification| !self.trusted_notification_id(notification.id)),
            &history,
            true,
        );
        projection.history_visible = self.notification_history_visible;
        serde_json::from_str(&projection.to_json()).ok()
    }

    fn external_plugin_applications(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let package = self.external_plugin_packages.get(plugin_id)?;
        if !package
            .manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::ApplicationsRead)
        {
            return None;
        }
        Some(serde_json::Value::Array(
            self.launcher
                .applications()
                .take(256)
                .filter(|application| !application.id().is_empty() && application.id().len() <= 256)
                .map(|application| {
                    serde_json::json!({
                        "id": application.id(),
                        "name": application.name().chars().take(120).collect::<String>(),
                        "pinned": self.launcher.is_pinned(application.id()),
                        "pinOrder": self.launcher.preferences().favorites().iter().position(|id| application.matches_native_id(id)),
                    })
                })
                .collect(),
        ))
    }

    fn preferences_catalog(&self) -> Result<crate::preferences_capabilities::Catalog, String> {
        crate::preferences_capabilities::Catalog::new(
            self.launcher
                .discovered_applications()
                .filter_map(|application| {
                    Some((
                        application.id().to_owned(),
                        application.launch_command()?.first()?.clone(),
                    ))
                }),
            nickel_platform::installed_icon_themes(),
        )
    }

    pub(crate) fn take_preferences_commit(&mut self) -> Option<ShellSettings> {
        self.preferences_commit_pending.take()
    }

    fn plugin_preferences(&mut self, plugin_id: &str) -> Option<serde_json::Value> {
        use nickel_core::plugins::PluginCapability;
        let manifest = self
            .external_plugin_packages
            .get(plugin_id)
            .map(|package| &package.manifest)
            .or_else(|| {
                self.plugin_registry
                    .get(plugin_id)
                    .map(|entry| &entry.manifest)
            })?;
        if !manifest
            .capabilities
            .contains(&PluginCapability::PreferencesRead)
        {
            return None;
        }
        let writable = !self.locked
            && manifest
                .capabilities
                .contains(&PluginCapability::PreferencesControl);
        let mut snapshot = match self.preferences_catalog() {
            Ok(catalog) => self.preferences_capabilities.snapshot(&catalog),
            Err(reason) => serde_json::json!({"available":false,"reason":reason}),
        };
        snapshot["writable"] = writable.into();
        Some(snapshot)
    }

    fn plugin_appearance(&mut self, plugin_id: &str, wallpaper: bool) -> Option<serde_json::Value> {
        use nickel_core::plugins::PluginCapability;
        let manifest = self
            .external_plugin_packages
            .get(plugin_id)
            .map(|package| &package.manifest)
            .or_else(|| {
                self.plugin_registry
                    .get(plugin_id)
                    .map(|entry| &entry.manifest)
            })?;
        let read = if wallpaper {
            PluginCapability::WallpaperRead
        } else {
            PluginCapability::AppearanceRead
        };
        let control = if wallpaper {
            PluginCapability::WallpaperControl
        } else {
            PluginCapability::AppearanceControl
        };
        if !manifest.capabilities.contains(&read) {
            return None;
        }
        let can_write = manifest.capabilities.contains(&control) && !self.locked;
        let mut snapshot = self.appearance_capabilities.snapshot(if wallpaper {
            "wallpaper"
        } else {
            "appearance"
        });
        snapshot["writable"] = can_write.into();
        Some(snapshot)
    }

    fn plugin_connectivity(&self, plugin_id: &str, wifi: bool) -> Option<serde_json::Value> {
        let manifest = self
            .external_plugin_packages
            .get(plugin_id)
            .map(|package| &package.manifest)
            .or_else(|| {
                self.plugin_registry
                    .get(plugin_id)
                    .map(|entry| &entry.manifest)
            })?;
        let capability = if wifi {
            nickel_core::plugins::PluginCapability::NetworkRead
        } else {
            nickel_core::plugins::PluginCapability::BluetoothRead
        };
        manifest.capabilities.contains(&capability).then(|| {
            if wifi {
                crate::connectivity_capabilities::wifi_snapshot(&self.network)
            } else {
                crate::connectivity_capabilities::bluetooth_snapshot(&self.bluetooth)
            }
        })
    }

    fn plugin_associations(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let manifest = self
            .external_plugin_packages
            .get(plugin_id)
            .map(|package| &package.manifest)
            .or_else(|| {
                self.plugin_registry
                    .get(plugin_id)
                    .map(|entry| &entry.manifest)
            })?;
        manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::AssociationsRead)
            .then(|| {
                let mut snapshot = crate::associations_capabilities::snapshot(
                    self.associations_results.get(plugin_id),
                );
                let writable = !self.locked
                    && manifest
                        .capabilities
                        .contains(&nickel_core::plugins::PluginCapability::AssociationsControl);
                snapshot["writable"] = writable.into();
                if !writable {
                    snapshot["operations"]["setDefault"] = false.into();
                    snapshot["operations"]["openSystemSettings"] = false.into();
                }
                snapshot
            })
    }

    fn plugin_audio(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let manifest = self
            .external_plugin_packages
            .get(plugin_id)
            .map(|package| &package.manifest)
            .or_else(|| {
                self.plugin_registry
                    .get(plugin_id)
                    .map(|entry| &entry.manifest)
            })?;
        manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::AudioRead)
            .then(|| self.audio_plugin_data())
    }

    fn plugin_displays(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let manifest = self
            .external_plugin_packages
            .get(plugin_id)
            .map(|package| &package.manifest)
            .or_else(|| {
                self.plugin_registry
                    .get(plugin_id)
                    .map(|entry| &entry.manifest)
            })?;
        if !manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::DisplayControl)
        {
            return None;
        }
        #[cfg(target_os = "linux")]
        {
            return Some(match self.session_host.projection_outputs() {
                Ok(outputs) => serde_json::json!({"available": true, "outputs": outputs}),
                Err(error) => {
                    serde_json::json!({"available": false, "reason": error, "outputs": []})
                }
            });
        }
        #[cfg(target_os = "windows")]
        {
            let read = crate::windows_plugin_display::read();
            return Some(serde_json::json!({
                "available": read.available,
                "reason": read.reason,
                "outputs": read.outputs,
                "pending_confirmation": read.pending_confirmation,
            }));
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        Some(serde_json::json!({
            "available": false,
            "reason": "display layout control is unavailable on this platform",
            "outputs": [],
        }))
    }

    fn audio_plugin_data(&self) -> serde_json::Value {
        audio_plugin_data(&self.audio, self.locked)
    }

    fn plugin_slot_projection(&self, target_id: &str) -> Option<serde_json::Value> {
        self.plugin_slot_projection_with_context(target_id, None, None)
    }

    /// Filters item-scoped contributions before applying the projection limits.
    fn plugin_slot_projection_with_context(
        &self,
        target_id: &str,
        item_filter: Option<Option<&str>>,
        visible_badge_items: Option<&HashSet<String>>,
    ) -> Option<serde_json::Value> {
        use nickel_core::plugins::{PluginContributionMode, PluginSlotContract};

        let target = self.plugin_registry.get(target_id)?;
        let mut slots = serde_json::Map::new();
        for slot in &target.manifest.provides_slots {
            let limit = match slot.contract {
                PluginSlotContract::Badge if visible_badge_items.is_some() => 36,
                PluginSlotContract::Badge => 128,
                PluginSlotContract::Action => 32,
                PluginSlotContract::Widget => 8,
                PluginSlotContract::Section => 4,
            };
            let mut contributors = self
                .plugin_slot_hosts
                .iter()
                .filter(|(_, host)| {
                    host.target_plugin == target_id
                        && host.target_slot == slot.id
                        && host.contract == slot.contract
                })
                .collect::<Vec<_>>();
            contributors.sort_by_key(|(id, host)| (host.priority, id.as_str()));
            let replacement = contributors
                .iter()
                .rev()
                .find(|(_, host)| host.mode == PluginContributionMode::Replace)
                .copied();
            let mut ordered = replacement.into_iter().collect::<Vec<_>>();
            ordered.extend(
                contributors
                    .into_iter()
                    .filter(|(_, host)| host.mode == PluginContributionMode::Add),
            );
            let mut items: Vec<serde_json::Value> = Vec::new();
            for (id, host) in ordered {
                let application = &host.application;
                match slot.contract {
                    PluginSlotContract::Badge => {
                        if let Ok(badges) = application.badge_contributions() {
                            for (item, label, count, color) in badges {
                                if count == 0
                                    || visible_badge_items
                                        .is_some_and(|visible| !visible.contains(&item))
                                    || visible_badge_items.is_some()
                                        && items
                                            .iter()
                                            .filter(|badge| {
                                                badge["item"].as_str() == Some(item.as_str())
                                            })
                                            .count()
                                            >= 3
                                {
                                    continue;
                                }
                                items.push(serde_json::json!({
                                    "pluginId": id,
                                    "item": item,
                                    "label": label,
                                    "count": count,
                                    "color": color,
                                }));
                                if items.len() == limit {
                                    break;
                                }
                            }
                        }
                    }
                    PluginSlotContract::Widget => {
                        if let Ok(widgets) = application.widget_contributions() {
                            for widget in widgets.into_iter().take(limit - items.len()) {
                                items.push(serde_json::json!({
                                    "pluginId": id,
                                    "label": widget.label,
                                    "value": widget.value,
                                    "percent": widget.percent,
                                    "color": widget.color,
                                }));
                            }
                        }
                    }
                    PluginSlotContract::Action => {
                        if let Ok(actions) = application.action_contributions() {
                            for action in actions
                                .into_iter()
                                .filter(|action| {
                                    item_filter.is_none_or(|item| {
                                        action
                                            .item
                                            .as_deref()
                                            .is_none_or(|target| Some(target) == item)
                                    })
                                })
                                .take(limit - items.len())
                            {
                                items.push(serde_json::json!({
                                    "pluginId": id,
                                    "id": action.id,
                                    "label": action.label,
                                    "item": action.item,
                                }));
                            }
                        }
                    }
                    PluginSlotContract::Section => {
                        if let Ok(sections) = application.section_contributions() {
                            for section in sections.into_iter().take(limit - items.len()) {
                                items.push(serde_json::json!({
                                    "pluginId": id,
                                    "id": section.id,
                                    "label": section.label,
                                    "value": section.value,
                                }));
                            }
                        }
                    }
                }
                if items.len() == limit {
                    break;
                }
            }
            slots.insert(slot.id.clone(), serde_json::Value::Array(items));
        }
        (!slots.is_empty()).then_some(serde_json::Value::Object(slots))
    }

    fn slot_actions_for_item(
        &self,
        target_id: &str,
        slot_id: &str,
        item: Option<&str>,
        limit: usize,
    ) -> Vec<serde_json::Value> {
        self.plugin_slot_projection_with_context(target_id, Some(item), None)
            .and_then(|slots| {
                slots
                    .get(slot_id)
                    .and_then(serde_json::Value::as_array)
                    .cloned()
            })
            .into_iter()
            .flatten()
            .take(limit)
            .collect()
    }

    fn refresh_plugin_slot_hosts(&mut self, target_id: &str) {
        let Some(slots) = self.plugin_slot_projection(target_id) else {
            return;
        };
        let keys = self
            .plugin_panels()
            .into_iter()
            .filter(|(key, _)| key.plugin_id == target_id)
            .map(|(key, _)| key)
            .collect::<Vec<_>>();
        for key in keys {
            let Some(host) = self.plugin_panel_host_for(&key) else {
                continue;
            };
            let changed = match host.application_mut().sync_external_slots(&slots) {
                Ok(changed) => changed,
                Err(error) => {
                    self.fail_plugin_panel_runtime(&key.plugin_id, error);
                    break;
                }
            };
            if !changed {
                continue;
            }
            let outcome = host.step(HostBatch {
                application_changed: true,
                ..HostBatch::default()
            });
            if let Some(error) = host.application_mut().take_runtime_failure() {
                self.fail_plugin_panel_runtime(&key.plugin_id, error);
                break;
            }
            let image_bytes = host.application_mut().retained_image_bytes();
            self.record_plugin_panel_memory(
                &key,
                (outcome.telemetry.retained_frame_bytes as u64).saturating_add(image_bytes),
            );
            if let Err(error) = self.reconcile_plugin_surface_root(&key) {
                self.fail_plugin_panel_runtime(&key.plugin_id, error);
                break;
            }
        }
    }

    pub(crate) fn plugin_panel_scene(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        width: u32,
        height: u32,
    ) -> Option<Vec<PaintCommand>> {
        if self.taskbar_surface_key().as_ref() == Some(key) {
            return Some(self.panel_scene(width, height));
        }
        let launcher_changed = if *key == crate::plugin_panel::launcher_surface_key() {
            self.sync_plugin_launcher()?
        } else {
            false
        };
        let control_data = (*key == crate::plugin_panel::control_center_surface_key())
            .then(|| self.control_plugin_data(height));
        let notification_data = (*key == crate::plugin_panel::notification_surface_key())
            .then(|| self.notification_plugin_projection().to_json());
        let preview_projection = if *key == crate::plugin_panel::window_preview_surface_key() {
            let group = self.preview_plugin_group()?;
            Some(self.preview_plugin_projection(&group))
        } else {
            None
        };
        let slots = self.plugin_slot_projection(&key.plugin_id);
        let windows = self.external_plugin_windows(&key.plugin_id);
        let applications = self.external_plugin_applications(&key.plugin_id);
        let notifications = self.external_plugin_notifications(&key.plugin_id);
        let tray = self.plugin_registry.get(&key.plugin_id)
            .filter(|entry| entry.manifest.capabilities.contains(&nickel_core::plugins::PluginCapability::TrayRead))
            .map(|_| serde_json::Value::Array(self.tray.iter().take(128).map(|item| serde_json::json!({"id":item.id,"title":item.title,"icon":false})).collect()));
        let audio = self.plugin_audio(&key.plugin_id);
        let associations = self.plugin_associations(&key.plugin_id);
        let preferences = self.plugin_preferences(&key.plugin_id);
        let appearance = self.plugin_appearance(&key.plugin_id, false);
        let wallpaper = self.plugin_appearance(&key.plugin_id, true);
        let wifi = self.plugin_connectivity(&key.plugin_id, true);
        let bluetooth = self.plugin_connectivity(&key.plugin_id, false);
        let displays = self.plugin_displays(&key.plugin_id);
        let keyboard_data = (*key == crate::plugin_panel::on_screen_keyboard_surface_key())
            .then(|| self.keyboard_plugin_data());
        let result = (|| {
            let host = self.plugin_panel_host_for(key)?;
            let projected = (|| -> Result<bool, String> {
                let preview_changed = if let Some((data, images)) = preview_projection {
                    let data_changed = host.application_mut().sync_data(&data)?;
                    let images_changed = host.application_mut().sync_images(images);
                    data_changed || images_changed
                } else {
                    false
                };
                let control_changed = control_data
                    .as_ref()
                    .map(|data| host.application_mut().sync_data(data))
                    .transpose()?
                    .unwrap_or(false);
                let notification_changed = notification_data
                    .as_ref()
                    .map(|data| host.application_mut().sync_serialized_data(data.clone()))
                    .transpose()?
                    .unwrap_or(false);
                let keyboard_changed = keyboard_data
                    .as_ref()
                    .map(|data| host.application_mut().sync_data(data))
                    .transpose()?
                    .unwrap_or(false);
                let fields = [
                    ("slots", slots.as_ref()),
                    ("windows", windows.as_ref()),
                    ("applications", applications.as_ref()),
                    ("notifications", notifications.as_ref()),
                    ("audio", audio.as_ref()),
                    ("tray", tray.as_ref()),
                    ("associations", associations.as_ref()),
                    ("preferences", preferences.as_ref()),
                    ("appearance", appearance.as_ref()),
                    ("wallpaper", wallpaper.as_ref()),
                    ("wifi", wifi.as_ref()),
                    ("bluetooth", bluetooth.as_ref()),
                    ("displays", displays.as_ref()),
                ]
                .into_iter()
                .filter_map(|(field, value)| value.map(|value| (field, value)))
                .collect::<Vec<_>>();
                let resource_changed = host.application_mut().sync_host_data_fields(&fields)?;
                Ok(preview_changed
                    || control_changed
                    || notification_changed
                    || keyboard_changed
                    || resource_changed)
            })();
            let projected = match projected {
                Ok(changed) => changed,
                Err(error) => return Some(Err(error)),
            };
            Some(
                render_plugin_host(
                    host,
                    None,
                    HostBatch {
                        application_changed: projected || launcher_changed,
                        surface_size: Some((width, height)),
                        ..HostBatch::default()
                    },
                )
                .map(|(commands, bytes)| (commands, bytes, host.application_mut().take_effects())),
            )
        })()?;
        match result {
            Ok((commands, bytes, effects)) => {
                self.record_plugin_panel_memory(key, bytes);
                if let Err(error) = self.reconcile_plugin_surface_root(key) {
                    self.fail_plugin_panel_runtime(&key.plugin_id, error);
                    return None;
                }
                self.apply_plugin_effects(effects);
                Some(commands)
            }
            Err(error) => {
                self.fail_plugin_panel_runtime(&key.plugin_id, error);
                None
            }
        }
    }

    pub(crate) fn plugin_panel_scene_for_output(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        output: Option<&str>,
        width: u32,
        height: u32,
    ) -> Option<Vec<PaintCommand>> {
        if self.taskbar_surface_key().as_ref() == Some(key) {
            return Some(self.panel_scene_for_output(output, width, height));
        }
        self.plugin_panel_scene(key, width, height)
    }

    pub(crate) fn plugin_surface_scene_for_output(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        output: Option<&str>,
        width: u32,
        height: u32,
    ) -> Option<Vec<PaintCommand>> {
        if *key == crate::plugin_panel::control_center_surface_key() {
            if !self.control_plugin_active() {
                return None;
            }
        }
        if *key == crate::plugin_panel::notification_surface_key() {
            if !self.notification_plugin_active() {
                return None;
            }
        }
        self.plugin_panel_scene_for_output(key, output, width, height)
    }

    fn record_plugin_panel_memory(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        bytes: u64,
    ) {
        self.plugin_panel_memory.insert(key.clone(), bytes);
        let total = self
            .plugin_panel_memory
            .iter()
            .filter(|(surface, _)| surface.plugin_id == key.plugin_id)
            .map(|(_, bytes)| *bytes)
            .fold(0_u64, u64::saturating_add);
        let _ = self.plugin_registry.record_memory(
            &key.plugin_id,
            nickel_core::plugins::PluginMemory {
                native_ui_bytes: Some(total),
                ..Default::default()
            },
        );
        self.maybe_publish_plugin_status();
    }

    pub(crate) fn close_plugin_window(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Result<bool, String> {
        if !self
            .plugin_panel_placement(key)
            .is_some_and(|(kind, _, _, _, _)| {
                matches!(
                    kind,
                    nickel_core::plugins::PluginSurfaceKind::Window
                        | nickel_core::plugins::PluginSurfaceKind::Dialog
                        | nickel_core::plugins::PluginSurfaceKind::Overlay
                )
            })
        {
            return Ok(false);
        }
        let owned_dialogs = self
            .plugin_panels()
            .into_iter()
            .filter(|(surface, placement)| {
                surface.plugin_id == key.plugin_id
                    && placement.kind == nickel_core::plugins::PluginSurfaceKind::Dialog
                    && placement.owner.as_deref() == Some(key.surface_id.as_str())
            })
            .map(|(surface, _)| surface)
            .collect::<Vec<_>>();
        for dialog in owned_dialogs {
            self.close_plugin_window(&dialog)?;
        }
        // Transient surfaces depend on an ordinary surface to open them.
        // Retire the package when closing this one would leave only transients.
        if !self.plugin_panels().iter().any(|(surface, placement)| {
            surface.plugin_id == key.plugin_id
                && surface != key
                && !matches!(
                    placement.kind,
                    nickel_core::plugins::PluginSurfaceKind::Dialog
                        | nickel_core::plugins::PluginSurfaceKind::Overlay
                )
        }) {
            return self.set_plugin_enabled(&key.plugin_id, false);
        }
        if let Some(host) = self.plugin_panel_host_for(key) {
            host.application().retire_surface()?;
        }
        if self.primary_panel_key == *key {
            self.plugin_surface_hosts.remove(key);
            self.primary_panel_key = crate::plugin_panel::surface_key();
        } else {
            self.plugin_surface_hosts.remove(key);
        }
        self.plugin_panel_memory.remove(key);
        self.plugin_window_placement_overrides.remove(key);
        let remaining_bytes = self
            .plugin_panel_memory
            .iter()
            .filter(|(surface, _)| surface.plugin_id == key.plugin_id)
            .map(|(_, bytes)| *bytes)
            .fold(0_u64, u64::saturating_add);
        self.plugin_registry.record_memory(
            &key.plugin_id,
            nickel_core::plugins::PluginMemory {
                native_ui_bytes: Some(remaining_bytes),
                ..Default::default()
            },
        )?;
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        self.maybe_publish_plugin_status();
        Ok(true)
    }

    pub(crate) fn set_plugin_window_placement(
        &mut self,
        id: &str,
        surface_id: &str,
        anchor: nickel_core::plugins::PluginSurfaceAnchor,
        offset_x: i32,
        offset_y: i32,
    ) -> Result<bool, String> {
        let entry = self
            .plugin_registry
            .get(id)
            .ok_or_else(|| format!("unknown plugin {id:?}"))?;
        if !entry.desired_enabled || entry.health != nickel_core::plugins::PluginHealth::Running {
            return Err(format!("plugin {id:?} is not running"));
        }
        let declared = self
            .external_plugin_packages
            .get(id)
            .is_some_and(|package| {
                package.manifest.surfaces.iter().any(|surface| {
                    surface.id == surface_id
                        && surface.kind == nickel_core::plugins::PluginSurfaceKind::Window
                })
            });
        if !declared || !(-8192..=8192).contains(&offset_x) || !(-8192..=8192).contains(&offset_y) {
            return Err(format!("plugin {id:?} cannot place window {surface_id:?}"));
        }
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: id.to_owned(),
            surface_id: surface_id.to_owned(),
        };
        let surface = &mut self
            .plugin_surface_hosts
            .get_mut(&key)
            .ok_or_else(|| format!("plugin window {surface_id:?} is not open"))?
            .0;
        if surface.kind != nickel_core::plugins::PluginSurfaceKind::Window {
            return Err(format!("plugin surface {surface_id:?} is not a window"));
        }
        if (surface.anchor, surface.offset_x, surface.offset_y) == (anchor, offset_x, offset_y) {
            return Ok(false);
        }
        surface.anchor = anchor;
        surface.offset_x = offset_x;
        surface.offset_y = offset_y;
        self.plugin_window_placement_overrides
            .insert(key, (anchor, offset_x, offset_y));
        Ok(true)
    }

    fn focus_plugin_window(&mut self, id: &str, surface_id: &str) -> Result<bool, String> {
        let entry = self
            .plugin_registry
            .get(id)
            .ok_or_else(|| format!("unknown plugin {id:?}"))?;
        if !entry.desired_enabled || entry.health != nickel_core::plugins::PluginHealth::Running {
            return Err(format!("plugin {id:?} is not running"));
        }
        let declared = self
            .external_plugin_packages
            .get(id)
            .is_some_and(|package| {
                package.manifest.surfaces.iter().any(|surface| {
                    surface.id == surface_id
                        && !surface.passive
                        && matches!(
                            surface.kind,
                            nickel_core::plugins::PluginSurfaceKind::Window
                                | nickel_core::plugins::PluginSurfaceKind::Dialog
                                | nickel_core::plugins::PluginSurfaceKind::Overlay
                        )
                })
            });
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: id.to_owned(),
            surface_id: surface_id.to_owned(),
        };
        if !declared || self.locked || !self.plugin_surface_hosts.contains_key(&key) {
            return Err(format!(
                "plugin surface {id:?}/{surface_id:?} is unavailable for focus"
            ));
        }
        #[cfg(target_os = "linux")]
        return Ok(self.send_session_command(
            "plugin-surface-focus",
            ShellCommand::FocusPluginSurface { key },
        ));
        #[cfg(target_os = "windows")]
        {
            self.pending_plugin_surface_focus = Some(key);
            Ok(true)
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn take_pending_plugin_surface_focus(
        &mut self,
    ) -> Option<nickel_core::plugins::PluginSurfaceKey> {
        self.pending_plugin_surface_focus.take()
    }

    pub(crate) fn show_plugin_window(
        &mut self,
        id: &str,
        surface_id: &str,
    ) -> Result<bool, String> {
        let entry = self
            .plugin_registry
            .get(id)
            .ok_or_else(|| format!("unknown plugin {id:?}"))?;
        if !entry.desired_enabled || entry.health != nickel_core::plugins::PluginHealth::Running {
            return Err(format!("plugin {id:?} is not running"));
        }
        let descriptor = self
            .external_plugin_packages
            .get(id)
            .ok_or_else(|| format!("plugin {id:?} is not an installed package"))?;
        let surface = descriptor
            .manifest
            .surfaces
            .iter()
            .find(|surface| {
                surface.id == surface_id
                    && matches!(
                        surface.kind,
                        nickel_core::plugins::PluginSurfaceKind::Window
                            | nickel_core::plugins::PluginSurfaceKind::Dialog
                            | nickel_core::plugins::PluginSurfaceKind::Overlay
                    )
            })
            .cloned()
            .ok_or_else(|| {
                format!("plugin {id:?} has no declared window, dialog, or overlay {surface_id:?}")
            })?;
        let key = nickel_core::plugins::PluginSurfaceKey {
            plugin_id: id.to_owned(),
            surface_id: surface_id.to_owned(),
        };
        if self.plugin_surface_matches(&key) {
            return Ok(false);
        }
        if let Some(owner) = &surface.owner {
            let owner_key = nickel_core::plugins::PluginSurfaceKey {
                plugin_id: id.to_owned(),
                surface_id: owner.clone(),
            };
            if !self.plugin_surface_matches(&owner_key) {
                return Err(format!("dialog owner {owner:?} is closed"));
            }
        }
        let package = descriptor.load()?;
        let settings = self
            .plugin_settings
            .get(id)
            .cloned()
            .map(Ok)
            .unwrap_or_else(|| external_plugin_settings(&package.manifest))?;
        let runtime = self
            .plugin_surface_hosts
            .iter()
            .find(|(key, _)| key.plugin_id == id)
            .map(|(_, (_, host))| host.application().shared_runtime())
            .ok_or_else(|| format!("plugin {id:?} has no live sibling runtime"))?;
        let application =
            crate::plugin_panel::PluginPanelApplication::from_package_surface_with_runtime(
                &package,
                &settings,
                &surface,
                crate::plugin_panel::package_images(&package)?,
                Some(runtime),
            )?;
        let surface = application.resolved_surface(&surface)?;
        let host = nickel_ui::UiHost::new(application, surface.width, surface.height);
        if self.primary_panel_host_ref().is_none() {
            self.primary_panel_key = key.clone();
        }
        self.plugin_surface_hosts.insert(key, (surface, host));
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        self.maybe_publish_plugin_status();
        Ok(true)
    }

    #[cfg(test)]
    pub(crate) fn lock_password_len(&self) -> usize {
        self.lock_host.application().password.len()
    }

    pub fn plugin_status_snapshot(&self) -> nickel_session_protocol::PluginStatusSnapshot {
        use nickel_core::plugins::PluginContributionMode;
        use nickel_core::plugins::PluginHealth;
        use nickel_session_protocol::{
            PluginMemorySnapshot, PluginRuntimeHealth, PluginSettingKind, PluginSettingStatus,
            PluginStatus, PluginStatusSnapshot,
        };

        PluginStatusSnapshot {
            activation_generation: self.plugin_activation_generation,
            plugins: self
                .plugin_registry
                .entries()
                .map(|entry| PluginStatus {
                    id: entry.manifest.id.clone(),
                    name: entry.manifest.name.clone(),
                    author: entry.manifest.author.clone(),
                    version: entry.manifest.version.clone(),
                    desired_enabled: entry.desired_enabled,
                    health: match &entry.health {
                        PluginHealth::Disabled => PluginRuntimeHealth::Disabled,
                        PluginHealth::Starting
                            if entry.manifest.id == crate::settings_plugin_report::ID =>
                        {
                            PluginRuntimeHealth::Idle
                        }
                        PluginHealth::Starting => PluginRuntimeHealth::Starting,
                        PluginHealth::Running => PluginRuntimeHealth::Running,
                        PluginHealth::Failed(error) => {
                            PluginRuntimeHealth::Failed(error.chars().take(256).collect())
                        }
                    },
                    capabilities: entry
                        .manifest
                        .capabilities
                        .iter()
                        .map(|capability| capability.as_str().to_owned())
                        .collect(),
                    surfaces: entry
                        .manifest
                        .surfaces
                        .iter()
                        .map(|surface| match &surface.owner {
                            Some(owner) => format!(
                                "{}: {} (owned by {})",
                                surface.id,
                                surface.kind.as_str(),
                                owner
                            ),
                            None => format!("{}: {}", surface.id, surface.kind.as_str()),
                        })
                        .collect(),
                    composition: entry
                        .manifest
                        .provides_slots
                        .iter()
                        .map(|slot| {
                            format!(
                                "Provides {} ({}){}",
                                slot.id,
                                slot.contract.as_str(),
                                if slot.replaceable {
                                    ", replaceable"
                                } else {
                                    ""
                                }
                            )
                        })
                        .chain(entry.manifest.contributes.iter().map(|contribution| {
                            let target_running = self
                                .plugin_registry
                                .get(&contribution.target_plugin)
                                .is_some_and(|target| target.health == PluginHealth::Running);
                            format!(
                                "{} {}/{} ({}){}",
                                contribution.mode.as_str(),
                                contribution.target_plugin,
                                contribution.target_slot,
                                contribution.contract.as_str(),
                                if target_running {
                                    if entry.desired_enabled
                                        && contribution.mode == PluginContributionMode::Replace
                                        && self
                                            .plugin_slot_hosts
                                            .iter()
                                            .filter(|(_, host)| {
                                                host.target_plugin == contribution.target_plugin
                                                    && host.target_slot == contribution.target_slot
                                                    && host.contract == contribution.contract
                                                    && host.mode == PluginContributionMode::Replace
                                            })
                                            .max_by_key(|(id, host)| (host.priority, id.as_str()))
                                            .is_some_and(|(winner, _)| winner != &entry.manifest.id)
                                    {
                                        " (superseded by another replacement)"
                                    } else {
                                        ""
                                    }
                                } else {
                                    " (target inactive)"
                                }
                            )
                        }))
                        .collect(),
                    settings: entry
                        .manifest
                        .settings
                        .iter()
                        .map(|setting| PluginSettingStatus {
                            id: setting.id.clone(),
                            label: setting.label.clone(),
                            description: setting.description.clone(),
                            kind: match &setting.kind {
                                nickel_core::plugins::PluginSettingKind::Boolean { .. } => {
                                    PluginSettingKind::Boolean
                                }
                                nickel_core::plugins::PluginSettingKind::Integer {
                                    min,
                                    max,
                                    ..
                                } => PluginSettingKind::Integer {
                                    min: *min,
                                    max: *max,
                                },
                                nickel_core::plugins::PluginSettingKind::Text {
                                    max_length,
                                    ..
                                } => PluginSettingKind::Text {
                                    max_length: *max_length,
                                },
                                nickel_core::plugins::PluginSettingKind::Choice {
                                    options, ..
                                } => PluginSettingKind::Choice {
                                    options: options.clone(),
                                },
                            },
                            value: self
                                .plugin_settings
                                .get(&entry.manifest.id)
                                .and_then(|values| values.get(&setting.id))
                                .cloned()
                                .unwrap_or_else(|| setting.kind.default_value()),
                        })
                        .collect(),
                    memory: PluginMemorySnapshot {
                        js_heap_bytes: entry.memory.js_heap_bytes,
                        native_ui_bytes: entry.memory.native_ui_bytes,
                        texture_bytes: entry.memory.texture_bytes,
                        tracked_peak_bytes: entry.tracked_peak_bytes,
                        timers: entry.memory.timers,
                        subscriptions: entry.memory.subscriptions,
                    },
                })
                .collect(),
        }
    }

    fn refresh_package_settings(&mut self) {
        let mut runtimes = std::collections::BTreeMap::new();
        for (key, (_, host)) in &self.plugin_surface_hosts {
            runtimes
                .entry(key.plugin_id.clone())
                .or_insert_with(|| host.application().shared_runtime());
        }
        for (id, host) in &self.plugin_slot_hosts {
            runtimes
                .entry(id.clone())
                .or_insert_with(|| host.application.shared_runtime());
        }
        let retired = self
            .package_settings_runtimes
            .keys()
            .filter(|id| !runtimes.contains_key(*id))
            .cloned()
            .collect::<Vec<_>>();
        for id in retired {
            self.package_settings_registry.retire_provider(&id);
            self.package_settings_runtimes.remove(&id);
            self.package_settings_values.remove(&id);
            self.package_settings_value_revisions.remove(&id);
            self.associations_results.remove(&id);
        }
        let mut changed_runtime = false;
        for (id, runtime) in &runtimes {
            if self
                .package_settings_runtimes
                .get(id)
                .is_some_and(|previous| std::rc::Rc::ptr_eq(previous, runtime))
            {
                continue;
            }
            self.associations_results.remove(id);
            match runtime
                .borrow_mut()
                .publish_settings(&mut self.package_settings_registry, id)
            {
                Ok(()) => {
                    self.package_settings_runtimes
                        .insert(id.clone(), runtime.clone());
                    changed_runtime = true;
                    self.package_settings_value_revisions.remove(id);
                    self.package_settings_values.remove(id);
                }
                Err(error) => {
                    self.package_settings_registry.retire_provider(id);
                    self.package_settings_values.remove(id);
                    self.package_settings_value_revisions.remove(id);
                    tracing::warn!(plugin_id = %id, %error, "package Settings registration rejected");
                }
            }
        }
        let mut values_changed = false;
        for (id, runtime) in &self.package_settings_runtimes {
            let revision = runtime.borrow().settings_revision();
            if self.package_settings_value_revisions.get(id) == Some(&revision) {
                continue;
            }
            self.package_settings_value_revisions
                .insert(id.clone(), revision);
            match runtime
                .borrow_mut()
                .read_settings_values(&self.package_settings_registry)
            {
                Ok(values) => {
                    if self.package_settings_values.get(id) != Some(&values) {
                        self.package_settings_values.insert(id.clone(), values);
                        values_changed = true;
                    }
                }
                Err(error) => {
                    tracing::warn!(plugin_id = %id, %error, "package Settings value snapshot rejected")
                }
            }
        }
        let generation = self
            .package_settings_registry
            .settings_snapshot()
            .generation;
        if changed_runtime || values_changed || generation != self.package_settings_generation {
            for (id, runtime) in runtimes {
                if let Err(error) = runtime
                    .borrow_mut()
                    .set_settings_registry(&self.package_settings_registry)
                {
                    tracing::warn!(plugin_id = %id, %error, "package Settings snapshot failed");
                }
            }
            for runtime in self.package_settings_runtimes.values() {
                if let Err(error) = runtime
                    .borrow_mut()
                    .set_settings_values(&self.package_settings_values)
                {
                    tracing::warn!(%error, "package Settings values failed");
                }
            }
            for (_, host) in self.plugin_surface_hosts.values_mut() {
                match host.application_mut().refresh_settings_render() {
                    Ok(changed) => {
                        host.step(HostBatch {
                            application_changed: changed,
                            ..HostBatch::default()
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%error, "package Settings surface refresh rejected")
                    }
                }
            }
            for host in self.plugin_slot_hosts.values_mut() {
                if let Err(error) = host.application.refresh_settings_render() {
                    tracing::warn!(%error, "package Settings slot refresh rejected");
                }
            }
            self.package_settings_generation = generation;
        }
    }

    fn maybe_publish_plugin_status(&mut self) {
        self.refresh_package_settings();
        #[cfg(target_os = "linux")]
        {
            let snapshot = self.plugin_status_snapshot();
            if self.last_published_plugin_status.as_ref() == Some(&snapshot) {
                return;
            }
            if self.send_session_command(
                "publish-plugin-status",
                ShellCommand::PublishPluginStatus {
                    snapshot: snapshot.clone(),
                },
            ) {
                self.last_published_plugin_status = Some(snapshot);
            }
        }
    }

    /// Applies a declared preference and refreshes an active installed plugin.
    pub fn set_plugin_setting(
        &mut self,
        id: &str,
        key: &str,
        value: serde_json::Value,
    ) -> Result<bool, String> {
        let entry = self
            .plugin_registry
            .get(id)
            .ok_or_else(|| format!("unknown plugin {id:?}"))?;
        let manifest = entry.manifest.clone();
        let setting = manifest
            .settings
            .iter()
            .find(|setting| setting.id == key)
            .ok_or_else(|| format!("unknown setting {key:?}"))?;
        if !setting.kind.accepts(&value) {
            return Err(format!("invalid value for setting {key:?}"));
        }
        let mut values = self.plugin_settings.get(id).cloned().unwrap_or_else(|| {
            manifest
                .settings
                .iter()
                .map(|setting| (setting.id.clone(), setting.kind.default_value()))
                .collect()
        });
        if values.get(key) == Some(&value) {
            return Ok(false);
        }
        values.insert(key.to_owned(), value.clone());
        let replacement = if entry.desired_enabled {
            self.external_plugin_packages
                .get(id)
                .map(|descriptor| {
                    let package = descriptor.load()?;
                    if !manifest.contributes.is_empty() {
                        let application = crate::plugin_panel::PluginPanelApplication::from_package_with_settings(
                            &package, &values,
                        )?;
                        application.validate_contribution()?;
                        Ok::<_, String>(vec![(None, application)])
                    } else {
                        let active_ids = self
                            .plugin_panels()
                            .into_iter()
                            .filter(|(surface, _)| surface.plugin_id == id)
                            .map(|(surface, _)| surface.surface_id)
                            .collect::<std::collections::HashSet<_>>();
                        let first_surface = package
                            .manifest
                            .surfaces
                            .iter()
                            .find(|surface| {
                                active_ids.contains(&surface.id)
                                    && !matches!(
                                        surface.kind,
                                        nickel_core::plugins::PluginSurfaceKind::Dialog
                                            | nickel_core::plugins::PluginSurfaceKind::Overlay
                                    )
                            })
                            .ok_or("installed plugin has no open ordinary surface")?;
                        let runtime = crate::plugin_panel::PluginPanelApplication::shared_package_runtime(
                            &package,
                            &values,
                            first_surface,
                        )?;
                        let images = crate::plugin_panel::package_images(&package)?;
                        package
                            .manifest
                            .surfaces
                            .iter()
                            .filter(|surface| active_ids.contains(&surface.id))
                            .map(|surface| {
                                crate::plugin_panel::PluginPanelApplication::from_package_surface_with_runtime(
                                    &package, &values, surface, images.clone(), Some(runtime.clone()),
                                )
                                .map(|application| (Some(surface.id.clone()), application))
                            })
                            .collect::<Result<Vec<_>, String>>()
                    }
                })
                .transpose()?
        } else {
            None
        };
        #[cfg(not(test))]
        nickel_core::plugins::PluginPreferences::update_default(&manifest, key, value)
            .map_err(|error| format!("could not save plugin setting: {error}"))?;
        self.plugin_settings.insert(id.to_owned(), values);
        let mut replaced_panels = false;
        if let Some(replacements) = replacement {
            let mut extension_bytes = None;
            replaced_panels = replacements
                .iter()
                .any(|(surface_id, _)| surface_id.is_some());
            for (surface_id, application) in replacements {
                if let Some(surface_id) = surface_id {
                    let grant = manifest
                        .surfaces
                        .iter()
                        .find(|surface| surface.id == surface_id)
                        .expect("replacement surface belongs to the validated manifest");
                    let mut resolved = application.resolved_surface(grant)?;
                    let key = nickel_core::plugins::PluginSurfaceKey {
                        plugin_id: id.to_owned(),
                        surface_id: surface_id.clone(),
                    };
                    if resolved.kind == nickel_core::plugins::PluginSurfaceKind::Window
                        && let Some((anchor, offset_x, offset_y)) =
                            self.plugin_window_placement_overrides.get(&key).copied()
                    {
                        resolved.anchor = anchor;
                        resolved.offset_x = offset_x;
                        resolved.offset_y = offset_y;
                    }
                    if let Some((surface, host)) = self.plugin_surface_hosts.get_mut(&key) {
                        *surface = resolved.clone();
                        *host =
                            nickel_ui::UiHost::new(application, resolved.width, resolved.height);
                    }
                } else if let Some(host) = self.plugin_slot_hosts.get_mut(id) {
                    extension_bytes = Some(application.retained_contribution_bytes());
                    host.application = application;
                    if host.contract == nickel_core::plugins::PluginSlotContract::Action {
                        self.clear_application_menu_plugin_host();
                    }
                }
            }
            if let Some(bytes) = extension_bytes {
                let _ = self.plugin_registry.record_memory(
                    id,
                    nickel_core::plugins::PluginMemory {
                        native_ui_bytes: Some(bytes),
                        ..Default::default()
                    },
                );
            }
            self.plugin_panel_memory
                .retain(|key, _| key.plugin_id != id);
            if replaced_panels {
                let _ = self.plugin_registry.record_memory(
                    id,
                    nickel_core::plugins::PluginMemory {
                        native_ui_bytes: Some(0),
                        ..Default::default()
                    },
                );
            }
        }
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        if let Some(host) = self.plugin_slot_hosts.get(id) {
            let target = host.target_plugin.clone();
            self.refresh_plugin_slot_hosts(&target);
        }
        if replaced_panels {
            for (surface, placement) in self
                .plugin_panels()
                .into_iter()
                .filter(|(surface, _)| surface.plugin_id == id)
            {
                let _ = self.plugin_panel_scene(&surface, placement.width, placement.height);
            }
        }
        self.maybe_publish_plugin_status();
        Ok(true)
    }

    /// Retire a failed installed runtime while preserving its desired activation.
    fn fail_installed_plugin_runtime(&mut self, id: &str, error: String) -> bool {
        if !self.external_plugin_packages.contains_key(id)
            || !self
                .plugin_registry
                .get(id)
                .is_some_and(|entry| entry.health == nickel_core::plugins::PluginHealth::Running)
        {
            return false;
        }
        let extension_target = self
            .plugin_registry
            .get(id)
            .and_then(|entry| entry.manifest.contributes.first())
            .map(|contribution| contribution.target_plugin.clone());
        tracing::warn!(plugin = id, %error, "installed plugin runtime failed");
        let _ = self.plugin_registry.mark_failed(id, error);
        if self
            .plugin_slot_hosts
            .remove(id)
            .is_some_and(|host| host.contract == nickel_core::plugins::PluginSlotContract::Action)
        {
            self.clear_application_menu_plugin_host();
        }
        self.plugin_surface_hosts
            .retain(|key, _| key.plugin_id != id);
        self.plugin_panel_memory
            .retain(|key, _| key.plugin_id != id);
        self.plugin_window_placement_overrides
            .retain(|key, _| key.plugin_id != id);
        if self.primary_panel_key.plugin_id == id {
            self.plugin_surface_hosts.remove(&self.primary_panel_key());
            self.primary_panel_key = crate::plugin_panel::surface_key();
        }
        if let Some(target) = extension_target {
            self.refresh_plugin_slot_hosts(&target);
        }
        self.refresh_plugin_slot_hosts(id);
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        self.maybe_publish_plugin_status();
        true
    }

    fn retire_taskbar_plugin_state(&mut self) {
        if self.window_menu.is_some() || self.application_menu_target.is_some() {
            self.dismiss_window_menu();
        }
        self.plugin_taskbar_host = None;
        self.plugin_taskbar_hosts.clear();
        self.plugin_taskbar_memory.clear();
        self.plugin_taskbar_menu_memory = 0;
        self.panel_pet_deadline = None;
        self.panel_deadline = None;
        self.clear_application_menu_plugin_host();
        self.clear_window_menu_plugin_host();
    }

    fn clear_application_menu_plugin_host(&mut self) {
        if let Some(host) = self.application_menu_plugin_host.take() {
            let _ = host.application().retire_surface();
        }
    }

    fn clear_window_menu_plugin_host(&mut self) {
        if let Some(host) = self.window_menu_plugin_host.take() {
            let _ = host.application().retire_surface();
        }
    }

    fn fail_taskbar_plugin_runtime(&mut self, error: String) {
        self.fail_bundled_plugin_runtime(
            &crate::plugin_panel::taskbar_manifest().id,
            error,
            Self::retire_taskbar_plugin_state,
        );
    }

    /// Fail one first-party runtime, retire its owned state, then publish one
    /// activation change. Desired enablement stays intact for Settings retry.
    fn fail_bundled_plugin_runtime(&mut self, id: &str, error: String, retire: fn(&mut Self)) {
        if !self
            .plugin_registry
            .get(id)
            .is_some_and(|entry| entry.health == nickel_core::plugins::PluginHealth::Running)
        {
            return;
        }
        tracing::warn!(plugin = id, %error, "bundled plugin runtime failed");
        let _ = self.plugin_registry.mark_failed(id, error);
        retire(self);
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        self.maybe_publish_plugin_status();
    }

    fn retire_extra_panel_plugin_state(&mut self, id: &str) {
        self.plugin_surface_hosts
            .retain(|key, _| key.plugin_id != id);
        self.plugin_panel_memory
            .retain(|key, _| key.plugin_id != id);
        self.plugin_window_placement_overrides
            .retain(|key, _| key.plugin_id != id);
        if id == crate::plugin_panel::codex_projects_manifest().id {
            self.codex_project_menu_visible = false;
            self.codex_menu_requests.clear();
        } else if id == crate::plugin_panel::on_screen_keyboard_manifest().id {
            self.keyboard_gesture_leases.clear();
        }
    }

    fn retire_codex_projects_plugin_state(&mut self) {
        self.retire_extra_panel_plugin_state(&crate::plugin_panel::codex_projects_manifest().id);
    }

    fn retire_keyboard_plugin_state(&mut self) {
        self.retire_extra_panel_plugin_state(
            &crate::plugin_panel::on_screen_keyboard_manifest().id,
        );
    }

    fn retire_development_panel_plugin_state(&mut self) {
        let id = &crate::plugin_panel::manifest().id;
        self.plugin_surface_hosts
            .retain(|key, _| key.plugin_id != *id);
        self.plugin_panel_memory
            .retain(|key, _| key.plugin_id != *id);
    }

    fn fail_plugin_panel_runtime(&mut self, id: &str, error: String) -> bool {
        let retire = if id == crate::plugin_panel::manifest().id {
            Self::retire_development_panel_plugin_state as fn(&mut Self)
        } else if id == crate::plugin_panel::codex_projects_manifest().id {
            Self::retire_codex_projects_plugin_state as fn(&mut Self)
        } else if id == crate::plugin_panel::on_screen_keyboard_manifest().id {
            Self::retire_keyboard_plugin_state
        } else if id == crate::plugin_panel::volume_osd_manifest().id {
            Self::retire_volume_osd_plugin_state
        } else if id == crate::plugin_panel::run_manifest().id {
            Self::retire_run_plugin_state
        } else if id == crate::plugin_panel::launcher_manifest().id {
            Self::retire_launcher_plugin_state
        } else if id == crate::plugin_panel::control_center_manifest().id {
            Self::retire_control_plugin_state
        } else if id == crate::plugin_panel::notification_manifest().id {
            Self::retire_notification_plugin_state
        } else if id == crate::plugin_panel::window_preview_manifest().id {
            Self::retire_preview_plugin_state
        } else {
            return self.fail_installed_plugin_runtime(id, error);
        };
        self.fail_bundled_plugin_runtime(id, error, retire);
        true
    }

    fn retire_launcher_plugin_state(&mut self) {
        self.retire_extra_panel_plugin_state(&crate::plugin_panel::launcher_manifest().id);
        self.launcher_plugin_result_page = 0;
        self.launcher_plugin_dashboard_page = 0;
        if self.launcher_visible && !self.run_visible {
            self.set_launcher_visible(false);
        }
    }

    fn fail_launcher_plugin_runtime(&mut self, error: String) {
        self.fail_bundled_plugin_runtime(
            &crate::plugin_panel::launcher_manifest().id,
            error,
            Self::retire_launcher_plugin_state,
        );
    }

    fn retire_run_plugin_state(&mut self) {
        self.retire_extra_panel_plugin_state(&crate::plugin_panel::run_manifest().id);
        if self.run_visible {
            self.run_visible = false;
            self.set_launcher_visible(false);
        }
    }

    fn fail_run_plugin_runtime(&mut self, error: String) {
        self.fail_bundled_plugin_runtime(
            &crate::plugin_panel::run_manifest().id,
            error,
            Self::retire_run_plugin_state,
        );
    }

    fn retire_control_plugin_state(&mut self) {
        self.retire_extra_panel_plugin_state(&crate::plugin_panel::control_center_manifest().id);
        if self.control_visible && !self.control_surface_available() {
            self.set_control_visible(false);
        }
    }

    fn fail_control_plugin_runtime(&mut self, error: String) {
        self.fail_bundled_plugin_runtime(
            &crate::plugin_panel::control_center_manifest().id,
            error,
            Self::retire_control_plugin_state,
        );
    }

    fn retire_notification_plugin_state(&mut self) {
        self.retire_extra_panel_plugin_state(&crate::plugin_panel::notification_manifest().id);
        if !self.trusted_notification_visible() {
            self.notification = None;
            self.notification_history_visible = false;
        }
    }

    fn fail_notification_plugin_runtime(&mut self, error: String) {
        self.fail_bundled_plugin_runtime(
            &crate::plugin_panel::notification_manifest().id,
            error,
            Self::retire_notification_plugin_state,
        );
    }

    fn retire_volume_osd_plugin_state(&mut self) {
        let key = crate::plugin_panel::volume_osd_surface_key();
        self.plugin_surface_hosts.remove(&key);
        self.plugin_panel_memory.remove(&key);
        self.volume_osd_until = None;
    }

    fn retire_preview_plugin_state(&mut self) {
        self.retire_extra_panel_plugin_state(&crate::plugin_panel::window_preview_manifest().id);
        let preview_was_open = self.preview_group.is_some() || self.task_switcher_group.is_some();
        if self.task_switcher_group.is_some() {
            self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::CancelSwitch);
        }
        if preview_was_open {
            self.close_window_preview();
        } else {
            self.preview_pending = None;
        }
    }

    fn fail_preview_plugin_runtime(&mut self, error: String) {
        self.fail_bundled_plugin_runtime(
            &crate::plugin_panel::window_preview_manifest().id,
            error,
            Self::retire_preview_plugin_state,
        );
    }

    /// Starts or retires a plugin instance after Settings has shown its grants.
    pub fn set_plugin_enabled(&mut self, id: &str, enabled: bool) -> Result<bool, String> {
        let Some(entry) = self.plugin_registry.get(id) else {
            return Err(format!("unknown plugin {id:?}"));
        };
        if entry.desired_enabled == enabled {
            return Ok(false);
        }
        let extension_target = entry.manifest.contributes.first().map(|contribution| {
            (
                contribution.target_plugin.clone(),
                contribution.target_slot.clone(),
            )
        });
        let external_extension = if enabled && !entry.manifest.contributes.is_empty() {
            let (kind, priority, mode) =
                validated_extension_contract(&entry.manifest, &self.plugin_registry)?;
            let descriptor = self
                .external_plugin_packages
                .get(id)
                .ok_or("extension is not an installed package")?;
            let package = descriptor.load()?;
            let settings = self
                .plugin_settings
                .get(id)
                .cloned()
                .map(Ok)
                .unwrap_or_else(|| external_plugin_settings(&package.manifest))?;
            let application =
                crate::plugin_panel::PluginPanelApplication::from_package_with_settings(
                    &package, &settings,
                )?;
            application.validate_contribution()?;
            Some((kind, priority, mode, application))
        } else {
            None
        };
        let external_panel = if enabled && external_extension.is_none() {
            if let Some(descriptor) = self.external_plugin_packages.get(id) {
                let surfaces = &descriptor.manifest.surfaces;
                Some(
                    if !surfaces.is_empty()
                        && surfaces.iter().all(|surface| {
                            matches!(
                                surface.kind,
                                nickel_core::plugins::PluginSurfaceKind::Panel
                                    | nickel_core::plugins::PluginSurfaceKind::Dock
                                    | nickel_core::plugins::PluginSurfaceKind::Window
                                    | nickel_core::plugins::PluginSurfaceKind::Dialog
                                    | nickel_core::plugins::PluginSurfaceKind::Overlay
                            )
                        })
                        && surfaces.iter().any(|surface| {
                            !matches!(
                                surface.kind,
                                nickel_core::plugins::PluginSurfaceKind::Dialog
                                    | nickel_core::plugins::PluginSurfaceKind::Overlay
                            )
                        })
                    {
                        descriptor.load().and_then(|package| {
                            crate::plugin_panel::PluginPanelApplication::validate_package(
                                &package,
                            )?;
                            let settings = self
                                .plugin_settings
                                .get(id)
                                .cloned()
                                .map(Ok)
                                .unwrap_or_else(|| external_plugin_settings(&package.manifest))?;
                            let images = crate::plugin_panel::package_images(&package)?;
                            let first_surface = surfaces.iter().find(|surface| {
                                !matches!(
                                    surface.kind,
                                    nickel_core::plugins::PluginSurfaceKind::Dialog
                                        | nickel_core::plugins::PluginSurfaceKind::Overlay
                                )
                            }).expect("validated package has an ordinary surface");
                            let runtime = crate::plugin_panel::PluginPanelApplication::shared_package_runtime(
                                &package,
                                &settings,
                                first_surface,
                            )?;
                            surfaces
                                .iter()
                                .filter(|surface| {
                                    !matches!(
                                        surface.kind,
                                        nickel_core::plugins::PluginSurfaceKind::Dialog
                                            | nickel_core::plugins::PluginSurfaceKind::Overlay
                                    )
                                })
                                .map(|surface| {
                                    crate::plugin_panel::PluginPanelApplication::from_package_surface_with_runtime(
                                        &package, &settings, surface, images.clone(), Some(runtime.clone()),
                                    )
                                    .and_then(|application| {
                                        let resolved = application.resolved_surface(surface)?;
                                        Ok((application, resolved))
                                    })
                                })
                                .collect::<Result<Vec<_>, _>>()
                        })
                    } else {
                        Err("installed plugin needs a panel, dock, or window to open its declared transient surfaces".into())
                    },
                )
            } else {
                None
            }
        } else {
            None
        };
        #[cfg(not(test))]
        if self.external_plugin_packages.contains_key(id) {
            nickel_core::plugins::PluginActivationSettings::update_manifest_default(
                &entry.manifest,
                &self
                    .external_plugin_packages
                    .get(id)
                    .expect("installed plugin descriptor exists")
                    .source_digest,
                enabled,
            )
            .map_err(|error| format!("could not save plugin activation: {error}"))?;
        } else {
            nickel_core::plugins::PluginActivationSettings::update_default(id, enabled)
                .map_err(|error| format!("could not save plugin activation: {error}"))?;
        }
        self.plugin_registry.set_enabled(id, enabled)?;
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        if id == crate::settings_plugin_report::ID {
            self.maybe_publish_plugin_status();
            return Ok(true);
        }
        if !enabled {
            if self.plugin_slot_hosts.remove(id).is_some_and(|host| {
                host.contract == nickel_core::plugins::PluginSlotContract::Action
            }) {
                self.clear_application_menu_plugin_host();
            }
            self.plugin_surface_hosts
                .retain(|key, _| key.plugin_id != id);
            self.plugin_panel_memory
                .retain(|key, _| key.plugin_id != id);
            self.plugin_window_placement_overrides
                .retain(|key, _| key.plugin_id != id);
            if id == self.primary_panel_key.plugin_id {
                self.primary_panel_key = crate::plugin_panel::surface_key();
            } else if id == crate::plugin_panel::launcher_manifest().id {
                self.retire_launcher_plugin_state();
            } else if id == crate::plugin_panel::run_manifest().id {
                self.retire_run_plugin_state();
            } else if id == crate::plugin_panel::taskbar_manifest().id {
                self.retire_taskbar_plugin_state();
            } else if id == crate::plugin_panel::notification_manifest().id {
                self.retire_notification_plugin_state();
            } else if id == crate::plugin_panel::volume_osd_manifest().id {
                self.retire_volume_osd_plugin_state();
            } else if id == crate::plugin_panel::control_center_manifest().id {
                self.retire_control_plugin_state();
            } else if id == crate::plugin_panel::codex_projects_manifest().id {
                self.retire_codex_projects_plugin_state();
            } else if id == crate::plugin_panel::on_screen_keyboard_manifest().id {
                self.retire_keyboard_plugin_state();
            } else if id == crate::plugin_panel::window_preview_manifest().id {
                self.retire_preview_plugin_state();
            }
            if let Some((target, _)) = &extension_target {
                self.refresh_plugin_slot_hosts(target);
            }
            self.maybe_publish_plugin_status();
            return Ok(true);
        }
        let extension_bytes = external_extension
            .as_ref()
            .map(|(_, _, _, application)| application.retained_contribution_bytes());
        let started = if let Some((contract, priority, mode, application)) = external_extension {
            let (target_plugin, target_slot) = extension_target
                .clone()
                .expect("validated extension has a target");
            self.plugin_slot_hosts.insert(
                id.to_owned(),
                PluginSlotHost {
                    target_plugin,
                    target_slot,
                    contract,
                    priority,
                    mode,
                    application,
                },
            );
            if contract == nickel_core::plugins::PluginSlotContract::Action {
                self.clear_application_menu_plugin_host();
            }
            Ok(())
        } else if let Some(external_panel) = external_panel {
            external_panel.map(|panels| {
                for (application, surface) in panels {
                    let host = nickel_ui::UiHost::new(application, surface.width, surface.height);
                    let key = nickel_core::plugins::PluginSurfaceKey {
                        plugin_id: id.to_owned(),
                        surface_id: surface.id.clone(),
                    };
                    if self.primary_panel_host_ref().is_none() {
                        self.primary_panel_key = key.clone();
                    }
                    self.plugin_surface_hosts.insert(key, (surface, host));
                }
            })
        } else if id == crate::plugin_panel::manifest().id {
            crate::plugin_panel::PluginPanelApplication::bundled().map(|application| {
                let surface = crate::plugin_panel::surface().clone();
                let host = nickel_ui::UiHost::new(application, surface.width, surface.height);
                if self.primary_panel_host_ref().is_none() {
                    self.primary_panel_key = crate::plugin_panel::surface_key();
                }
                self.plugin_surface_hosts.insert(
                    nickel_core::plugins::PluginSurfaceKey {
                        plugin_id: id.to_owned(),
                        surface_id: surface.id.clone(),
                    },
                    (surface, host),
                );
            })
        } else if id == crate::plugin_panel::launcher_manifest().id {
            self.launcher_plugin_result_page = 0;
            self.launcher_plugin_dashboard_page = 0;
            let projection = self.current_plugin_launcher_projection();
            let images =
                launcher_plugin_images(&self.launcher, &mut self.launcher_icons, &projection);
            self.install_bundled_surface(
                crate::plugin_panel::launcher_manifest(),
                projection.to_json(),
                images,
            )
        } else if id == crate::plugin_panel::run_manifest().id {
            self.install_bundled_surface(
                crate::plugin_panel::run_manifest(),
                serde_json::json!({ "status": null }).to_string(),
                crate::plugin_panel::PluginImages::new(),
            )
        } else if id == crate::plugin_panel::taskbar_manifest().id {
            self.plugin_taskbar_hosts.clear();
            self.plugin_taskbar_memory.clear();
            self.panel_pet_frame = 0;
            self.panel_pet_deadline = None;
            let (clock, _) = panel_clock_text();
            let (data, images) = self.taskbar_plugin_render_data(&clock);
            bundled_surface_host(crate::plugin_panel::taskbar_manifest(), data, images).map(
                |(_, _, host)| {
                    self.plugin_taskbar_host = Some(host);
                },
            )
        } else if id == crate::plugin_panel::notification_manifest().id {
            let projection = self.notification_plugin_projection();
            self.install_bundled_surface(
                crate::plugin_panel::notification_manifest(),
                projection.to_json(),
                crate::plugin_panel::PluginImages::new(),
            )
        } else if id == crate::plugin_panel::volume_osd_manifest().id {
            let data = serde_json::json!({"audio": self.audio_plugin_data()});
            self.install_bundled_surface(
                crate::plugin_panel::volume_osd_manifest(),
                data.to_string(),
                crate::plugin_panel::PluginImages::new(),
            )
        } else if id == crate::plugin_panel::control_center_manifest().id {
            let data = self.control_plugin_data(720);
            self.install_bundled_surface(
                crate::plugin_panel::control_center_manifest(),
                data.to_string(),
                crate::plugin_panel::PluginImages::new(),
            )
        } else if id == crate::plugin_panel::codex_projects_manifest().id {
            let projection = nickel_codex_ui::ProjectMenuProjection::from_state(
                &nickel_codex_ui::ChatState::default(),
            );
            serde_json::to_string(&projection)
                .map_err(|error| error.to_string())
                .and_then(|data| {
                    self.install_bundled_surface(
                        crate::plugin_panel::codex_projects_manifest(),
                        data,
                        crate::plugin_panel::PluginImages::new(),
                    )
                })
        } else if id == crate::plugin_panel::on_screen_keyboard_manifest().id {
            let data = self.keyboard_plugin_data();
            self.install_bundled_surface(
                crate::plugin_panel::on_screen_keyboard_manifest(),
                data.to_string(),
                crate::plugin_panel::PluginImages::new(),
            )
        } else if id == crate::plugin_panel::window_preview_manifest().id {
            let data = serde_json::json!({"windows": []});
            self.install_bundled_surface(
                crate::plugin_panel::window_preview_manifest(),
                data.to_string(),
                crate::plugin_panel::PluginImages::new(),
            )
        } else {
            Err(format!("plugin {id:?} has no runtime host"))
        };
        let result = match started {
            Ok(()) => self.plugin_registry.mark_running(id).map(|()| {
                if let Some(bytes) = extension_bytes {
                    let _ = self.plugin_registry.record_memory(
                        id,
                        nickel_core::plugins::PluginMemory {
                            native_ui_bytes: Some(bytes),
                            ..Default::default()
                        },
                    );
                }
                true
            }),
            Err(error) => {
                self.plugin_registry.mark_failed(id, error.clone())?;
                Err(error)
            }
        };
        if result.is_ok() {
            if let Some((target, _)) = &extension_target {
                self.refresh_plugin_slot_hosts(target);
            }
            self.refresh_plugin_slot_hosts(id);
        }
        self.maybe_publish_plugin_status();
        result
    }

    fn install_bundled_surface(
        &mut self,
        manifest: &nickel_core::plugins::PluginManifest,
        data: String,
        images: crate::plugin_panel::PluginImages,
    ) -> Result<(), String> {
        let (key, surface, host) = bundled_surface_host(manifest, data, images)?;
        self.plugin_surface_hosts.insert(key, (surface, host));
        Ok(())
    }

    fn start_initial_bundled_surface(
        &mut self,
        manifest: &nickel_core::plugins::PluginManifest,
        data: String,
    ) -> Result<(), String> {
        let id = &manifest.id;
        self.plugin_registry.set_enabled(id, true)?;
        match self.install_bundled_surface(manifest, data, crate::plugin_panel::PluginImages::new())
        {
            Ok(()) => self.plugin_registry.mark_running(id),
            Err(error) => {
                tracing::error!(plugin = id, %error, "plugin failed to start");
                self.plugin_registry.mark_failed(id, error)
            }
        }
    }

    pub fn launcher_surface_size(&self) -> Option<(u32, u32)> {
        self.run_visible.then(|| {
            let surface = crate::plugin_panel::run_surface();
            (surface.width, surface.height)
        })
    }

    pub(crate) fn launcher_preferred_surface_size(&self, maximum: (u32, u32)) -> (u32, u32) {
        let size = self.launcher_surface_size().unwrap_or_else(|| {
            let surface = crate::plugin_panel::launcher_surface();
            (surface.width, surface.height)
        });
        (size.0.min(maximum.0), size.1.min(maximum.1))
    }

    pub fn next_host_deadline(&self) -> Option<Instant> {
        self.host_deadline_sources()
            .into_iter()
            .map(|(_, deadline)| deadline)
            .min()
    }

    pub fn host_deadline_sources(&self) -> Vec<(&'static str, Instant)> {
        let mut sources = Vec::new();
        let mut push = |name, deadline| {
            if let Some(deadline) = deadline {
                sources.push((name, deadline));
            }
        };
        push(
            "desktop",
            self.desktop_viewports
                .values()
                .filter_map(|viewport| viewport.deadline)
                .chain(self.desktop_deadline)
                .min(),
        );
        push("on-screen-keyboard", Some(self.keyboard_deadline));
        push("launcher-preferences", self.launcher_preference_deadline);
        push(
            "panel",
            self.plugin_taskbar_hosts
                .values()
                .filter_map(|host| host.next_deadline())
                .chain(
                    self.plugin_taskbar_host
                        .as_ref()
                        .and_then(|host| host.next_deadline()),
                )
                .chain(self.panel_deadline)
                .chain(self.panel_pet_deadline)
                .min(),
        );
        push("lock", self.lock_deadline);
        push(
            "control",
            self.control_deadline.or_else(|| {
                self.control_plugin_host_ref()
                    .and_then(|host| host.next_deadline())
            }),
        );
        push("screenshot", self.screenshot.next_deadline());
        push(
            "window-preview-host",
            self.preview_plugin_host_ref()
                .filter(|_| self.preview_plugin_active())
                .and_then(|host| host.next_deadline()),
        );
        push(
            "window-preview-open",
            self.preview_pending.map(|(_, deadline)| deadline),
        );
        push("window-preview-close", self.preview_leave_deadline);
        push("task-switcher-peek", self.task_switcher.peek_deadline());
        push("volume-osd", self.volume_osd_until);
        push(
            "display-preview",
            self.display_preview
                .as_ref()
                .map(|preview| preview.deadline),
        );
        sources
    }

    pub fn scene_change_token(&self, role: SurfaceRole) -> Option<HostChangeToken> {
        let host_token = |inspection: nickel_ui::HostInspection| HostChangeToken {
            frame_generation: inspection.frame_generation,
            semantic_generation: inspection.semantic_generation,
        };
        match role {
            SurfaceRole::Desktop => Some(self.desktop_change_token),
            SurfaceRole::Taskbar => Some(self.panel_change_token),
            SurfaceRole::Panel => self
                .primary_panel_host_ref()
                .map(|host| host_token(host.inspect())),
            SurfaceRole::Lock => Some(self.lock_change_token),
            SurfaceRole::Launcher if self.run_visible => {
                self.run_host_ref().map(|host| host_token(host.inspect()))
            }
            SurfaceRole::Launcher => self
                .launcher_host_ref()
                .map(|host| host_token(host.inspect())),
            SurfaceRole::ControlCenter => self
                .control_plugin_active()
                .then(|| host_token(self.control_plugin_host_ref().unwrap().inspect()))
                .or(Some(self.control_change_token)),
            SurfaceRole::Notification => {
                let trusted = self.trusted_notification_visible();
                let inspection = if trusted {
                    self.notification_host.inspect()
                } else {
                    self.notification_plugin_host_ref()
                        .map_or_else(|| self.notification_host.inspect(), |host| host.inspect())
                };
                let mut token = host_token(inspection);
                if trusted {
                    token.frame_generation = token.frame_generation.wrapping_add(1_u64 << 63);
                    token.semantic_generation = token.semantic_generation.wrapping_add(1_u64 << 63);
                }
                Some(token)
            }
            SurfaceRole::VolumeOsd => self
                .plugin_panel_host_ref(&crate::plugin_panel::volume_osd_surface_key())
                .map(|host| host_token(host.inspect())),
            SurfaceRole::WindowPreview => self
                .preview_plugin_active()
                .then(|| host_token(self.preview_plugin_host_ref().unwrap().inspect())),
            SurfaceRole::WindowContextMenu => self
                .window_menu_plugin_host
                .as_ref()
                .map(|host| host_token(host.inspect()))
                .or_else(|| {
                    self.application_menu_plugin_host
                        .as_ref()
                        .map(|host| host_token(host.inspect()))
                }),
            SurfaceRole::Screenshot => Some(self.screenshot.change_token()),
            SurfaceRole::OnScreenKeyboard => Some(host_token(self.keyboard_host.inspect())),
            SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat => None,
            #[cfg(target_os = "windows")]
            SurfaceRole::TrustedControl => None,
        }
    }

    pub fn launcher_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        clipboard_text: Option<String>,
        width: u32,
        height: u32,
    ) -> nickel_ui::HostEventOutcome {
        if !self.run_visible && self.launcher_host_ref().is_none() {
            return nickel_ui::HostEventOutcome::default();
        }
        let recipient = if self.run_visible {
            let Some(host) = self.run_host_ref() else {
                return nickel_ui::HostEventOutcome::default();
            };
            host.inspect()
        } else if let Some(host) = self.launcher_host_ref() {
            host.inspect()
        } else {
            return nickel_ui::HostEventOutcome::default();
        };
        let (event, authority) =
            internal_normalized_ingress(input, clipboard_text, "launcher", recipient, None);
        self.launcher_host_event_with_authority(event, width, height, None, Some(authority))
    }

    pub(crate) fn launcher_host_event_with_clipboard_limit(
        &mut self,
        event: HostEvent,
        width: u32,
        height: u32,
        limit: Option<usize>,
    ) -> nickel_ui::HostEventOutcome {
        self.launcher_host_event_with_authority(event, width, height, limit, None)
    }

    pub(crate) fn launcher_host_event_with_authority(
        &mut self,
        event: HostEvent,
        width: u32,
        height: u32,
        limit: Option<usize>,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> nickel_ui::HostEventOutcome {
        if !self.run_visible && self.launcher_host_ref().is_none() {
            return nickel_ui::HostEventOutcome::default();
        }
        if self.run_visible {
            if let Some(host) = self.run_host_mut() {
                let outcome = host.step(HostBatch {
                    clipboard_text_limit: limit,
                    surface_size: Some((width, height)),
                    events: vec![event],
                    normalized_authorities: authority.into_iter().collect(),
                    ..HostBatch::default()
                });
                let effects = host.application_mut().take_effects();
                if let Some(error) = host.application_mut().take_runtime_failure() {
                    self.fail_run_plugin_runtime(error);
                    return nickel_ui::HostEventOutcome::default();
                }
                self.apply_plugin_effects(effects);
                self.host_runtime_samples.record(outcome.telemetry);
                return outcome;
            }
            return nickel_ui::HostEventOutcome::default();
        }
        if self.launcher_host_ref().is_some() {
            let Some(application_changed) = self.sync_plugin_launcher() else {
                return nickel_ui::HostEventOutcome::default();
            };
            let host = self
                .launcher_host_mut()
                .expect("launcher plugin host exists");
            let overlay_open = host.inspect().open_overlay.is_some();
            host.application_mut().set_overlay_open(overlay_open);
            let event = match event {
                HostEvent::Shortcut(Shortcut::Escape) if overlay_open => {
                    HostEvent::Ui(UiEvent::Dismiss)
                }
                HostEvent::Shortcut(Shortcut::Submit) if overlay_open => {
                    HostEvent::Ui(UiEvent::KeyboardNavigateActivate)
                }
                HostEvent::Shortcut(Shortcut::Submit) => {
                    let inspection = host.inspect();
                    let focused_button = inspection
                        .keyboard_focus
                        .into_iter()
                        .chain(inspection.controller_target)
                        .find(|id| {
                            host.query_unique(&nickel_ui::SemanticSelector::Id(id.clone()))
                                .is_ok_and(|target| target.role == Some(SemanticRole::Button))
                        });
                    focused_button.map_or(HostEvent::Shortcut(Shortcut::Submit), |target| {
                        HostEvent::Ui(UiEvent::AccessibilityActivate(target))
                    })
                }
                event => event,
            };
            let outcome = host.step(HostBatch {
                clipboard_text_limit: limit,
                surface_size: Some((width, height)),
                application_changed,
                events: vec![event],
                normalized_authorities: authority.into_iter().collect(),
                ..HostBatch::default()
            });
            let effects = host.application_mut().take_effects();
            if let Some(error) = host.application_mut().take_runtime_failure() {
                self.fail_launcher_plugin_runtime(error);
                return nickel_ui::HostEventOutcome::default();
            }
            self.apply_plugin_effects(effects);
            self.host_runtime_samples.record(outcome.telemetry);
            return outcome;
        }
        nickel_ui::HostEventOutcome::default()
    }

    #[cfg(any(test, target_os = "linux"))]
    pub fn launcher_host_controller(
        &mut self,
        action: ControllerAction,
        _family: nickel_ui::ControllerFamily,
    ) -> bool {
        if !self.run_visible && self.launcher_host_ref().is_none() {
            return false;
        }
        if self.run_visible {
            if let Some(host) = self.run_host_mut() {
                let event =
                    launcher_controller_host_event(action, host.inspect().open_overlay.is_some());
                let outcome = host.step(HostBatch {
                    events: vec![event],
                    ..HostBatch::default()
                });
                let show_keyboard = action == ControllerAction::Confirm
                    && outcome.text_input_active
                    && host.controller_targets_text_input();
                let effects = host.application_mut().take_effects();
                if let Some(error) = host.application_mut().take_runtime_failure() {
                    self.fail_run_plugin_runtime(error);
                    return false;
                }
                if show_keyboard {
                    self.set_keyboard_visible(true);
                }
                self.apply_plugin_effects(effects);
                self.host_runtime_samples.record(outcome.telemetry);
                return outcome.changed;
            }
            return false;
        }
        if self.launcher_host_ref().is_some() {
            let Some(application_changed) = self.sync_plugin_launcher() else {
                return false;
            };
            let host = self
                .launcher_host_mut()
                .expect("launcher plugin host exists");
            let overlay_open = host.inspect().open_overlay.is_some();
            host.application_mut().set_overlay_open(overlay_open);
            let event =
                launcher_controller_host_event(action, host.inspect().open_overlay.is_some());
            let outcome = host.step(HostBatch {
                application_changed,
                events: vec![event],
                ..HostBatch::default()
            });
            let show_keyboard = action == ControllerAction::Confirm
                && outcome.text_input_active
                && host.controller_targets_text_input();
            let effects = host.application_mut().take_effects();
            if let Some(error) = host.application_mut().take_runtime_failure() {
                self.fail_launcher_plugin_runtime(error);
                return false;
            }
            if show_keyboard {
                self.set_keyboard_visible(true);
            }
            self.apply_plugin_effects(effects);
            self.host_runtime_samples.record(outcome.telemetry);
            return outcome.changed;
        }
        false
    }

    pub fn poll_host_deadlines(&mut self, now: Instant) -> Vec<SurfaceRole> {
        let mut changed = Vec::new();
        if self.poll_launcher_preferences() {
            changed.extend([SurfaceRole::Launcher, SurfaceRole::Taskbar]);
        }

        let mut due_desktop_outputs = self
            .desktop_viewports
            .iter()
            .filter(|(_, viewport)| viewport.deadline.is_some_and(|deadline| now >= deadline))
            .map(|(output, _)| output.clone())
            .collect::<Vec<_>>();
        if self
            .desktop_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            due_desktop_outputs.push(self.desktop_active_viewport.clone());
        }
        let mut desktop_changed = false;
        for output in due_desktop_outputs {
            if output != self.desktop_active_viewport
                && let Some((origin, scale)) = self.desktop_output_projection(&output)
            {
                self.set_desktop_output(output, origin.x, origin.y, scale);
            }
            let outcome = self.desktop_host.step(HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            self.desktop_change_token = outcome.change_token;
            self.desktop_deadline = outcome.next_deadline;
            desktop_changed |= outcome.changed;
        }
        if desktop_changed {
            changed.push(SurfaceRole::Desktop);
        }
        let plugin_clock_due = self.plugin_taskbar_host.is_some()
            && self.panel_deadline.is_some_and(|deadline| now >= deadline);
        if plugin_clock_due {
            self.panel_deadline = Some(now + taskbar::duration_until_next_minute());
            changed.push(SurfaceRole::Taskbar);
        }
        if self.plugin_taskbar_host.is_some()
            && self
                .panel_pet_deadline
                .is_some_and(|deadline| now >= deadline)
        {
            self.panel_pet_frame = (self.panel_pet_frame + 1) % 4;
            self.panel_pet_deadline = Some(now + Duration::from_millis(360));
            changed.push(SurfaceRole::Taskbar);
        }
        if self.lock_deadline.is_some_and(|deadline| now >= deadline) {
            let outcome = self.lock_host.step(HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            self.lock_change_token = outcome.change_token;
            self.lock_deadline = outcome.next_deadline;
            if outcome.changed {
                changed.push(SurfaceRole::Lock);
            }
        }
        if self
            .control_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            let control_changed = self.step_control_host(HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            self.apply_control_effects();
            if control_changed {
                changed.push(SurfaceRole::ControlCenter);
            }
        }
        changed
    }

    pub fn poll_deadlines(&mut self, now: Instant) -> ShellDeadlineOutcome {
        let mut outcome = ShellDeadlineOutcome {
            redraw: self.poll_host_deadlines(now),
            ..ShellDeadlineOutcome::default()
        };
        if now >= self.keyboard_deadline {
            let visible = self.keyboard_visible;
            if self.refresh_keyboard() {
                outcome.redraw.push(SurfaceRole::OnScreenKeyboard);
                outcome.redraw.push(SurfaceRole::Taskbar);
            }
            outcome.visibility_changed |= visible != self.keyboard_visible;
            self.keyboard_deadline =
                now + Duration::from_millis(if self.keyboard_enabled { 100 } else { 1000 });
        }
        if let Some((index, deadline)) = self.preview_pending
            && now >= deadline
        {
            self.preview_pending = None;
            let previous = self.preview_group;
            self.open_window_preview(index);
            outcome.visibility_changed |= self.preview_group != previous;
            if self.preview_group.is_some() {
                outcome.redraw.push(SurfaceRole::WindowPreview);
            }
        }
        if let Some(_window) = self.task_switcher.poll_peek(now) {
            #[cfg(target_os = "windows")]
            let _ = self.send_session_command(
                "task-switcher-peek",
                ShellCommand::ShowTaskSwitcherPeek {
                    window: Some(_window),
                },
            );
            outcome.visibility_changed = true;
        }
        if self
            .preview_leave_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.preview_leave_deadline = None;
            let was_open = self.preview_group.is_some();
            self.close_window_preview();
            outcome.visibility_changed |= was_open;
        }
        outcome.capture_screenshot =
            self.screenshot.capture_ready_at(now) || self.screenshot_capture_pending;
        if self.screenshot.poll_pointer_deadline(now) {
            outcome.redraw.push(SurfaceRole::Screenshot);
        }
        if self
            .volume_osd_until
            .is_some_and(|deadline| now >= deadline)
        {
            self.volume_osd_until = None;
            outcome.visibility_changed = true;
        }
        if self
            .projection_rollback_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.rollback_projection();
            outcome.visibility_changed = true;
        }
        if self
            .display_preview
            .as_ref()
            .is_some_and(|preview| now >= preview.deadline)
        {
            let reverted = self.revert_plugin_display_layout(None);
            outcome.visibility_changed |= reverted;
            if !reverted {
                if let Some(preview) = self.display_preview.as_mut() {
                    preview.deadline = now + Duration::from_secs(1);
                }
            }
        }
        outcome
    }

    pub fn notification_click(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        if !self.surface_visible(SurfaceRole::Notification) {
            return false;
        }
        if self.notification_plugin_active() {
            let point = Point { x, y };
            return self
                .step_notification_plugin(HostBatch {
                    surface_size: Some((width, height)),
                    events: vec![
                        HostEvent::Ui(UiEvent::PointerPressed(point)),
                        HostEvent::Ui(UiEvent::PointerReleased(point)),
                    ],
                    ..HostBatch::default()
                })
                .is_some_and(|outcome| outcome.changed);
        }
        self.sync_notification_host(width, height);
        let point = Point { x, y };
        let outcome = self.notification_host.step(HostBatch {
            events: vec![
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                HostEvent::Ui(UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        if outcome.effects.is_empty() {
            self.notification_host.application_mut().request_dismiss();
        }
        self.apply_notification_effects()
    }

    pub(crate) fn notification_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        if !self.surface_visible(SurfaceRole::Notification) {
            return false;
        }
        if self.notification_plugin_active() {
            let host = self.notification_plugin_host_ref().unwrap();
            let (ingress, authority) =
                internal_normalized_ingress(input, None, "notification", host.inspect(), None);
            return self.notification_host_event_authorized(
                ingress,
                width,
                height,
                Some(authority),
            );
        }
        self.sync_notification_host(width, height);
        let (ingress, authority) = internal_normalized_ingress(
            input,
            None,
            "notification",
            self.notification_host.inspect(),
            None,
        );
        self.notification_host_event_authorized(ingress, width, height, Some(authority))
    }

    pub(crate) fn notification_host_event_authorized(
        &mut self,
        ingress: HostEvent,
        width: u32,
        height: u32,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> bool {
        if !self.surface_visible(SurfaceRole::Notification) {
            return false;
        }
        if self.notification_plugin_active() {
            return self
                .step_notification_plugin(HostBatch {
                    surface_size: Some((width, height)),
                    events: vec![ingress],
                    normalized_authorities: authority.into_iter().collect(),
                    ..HostBatch::default()
                })
                .is_some_and(|outcome| outcome.changed);
        }
        self.sync_notification_host(width, height);
        let outcome = self.notification_host.step(HostBatch {
            events: vec![ingress],
            normalized_authorities: authority.into_iter().collect(),
            ..HostBatch::default()
        });
        outcome.changed | self.apply_notification_effects()
    }

    pub fn notification_key(&mut self, key: Option<KeyCode>) -> bool {
        if !self.surface_visible(SurfaceRole::Notification) {
            return false;
        }
        let event = match key {
            Some(KeyCode::Escape) => HostEvent::Shortcut(Shortcut::Escape),
            Some(KeyCode::ArrowLeft | KeyCode::ArrowUp) => {
                HostEvent::Controller(ControllerAction::Left)
            }
            Some(KeyCode::ArrowRight | KeyCode::ArrowDown | KeyCode::Tab) => {
                HostEvent::Controller(ControllerAction::Right)
            }
            Some(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                HostEvent::Controller(ControllerAction::Confirm)
            }
            _ => return false,
        };
        if self.notification_plugin_active() {
            self.step_notification_plugin(HostBatch {
                events: vec![event],
                ..HostBatch::default()
            });
            return true;
        }
        self.sync_notification_host(420, 180);
        self.notification_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        self.apply_notification_effects();
        true
    }

    pub fn notification_controller(&mut self, action: ControllerAction) -> bool {
        if !self.surface_visible(SurfaceRole::Notification) {
            return false;
        }
        let event = if action == ControllerAction::Cancel {
            HostEvent::Shortcut(Shortcut::Escape)
        } else {
            HostEvent::Controller(action)
        };
        if self.notification_plugin_active() {
            return self
                .step_notification_plugin(HostBatch {
                    events: vec![event],
                    ..HostBatch::default()
                })
                .is_some_and(|outcome| outcome.changed);
        }
        self.sync_notification_host(420, 180);
        let outcome = self.notification_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        self.apply_notification_effects();
        outcome.changed
    }

    pub(crate) fn panel_host_ui(&mut self, event: UiEvent, width: u32) -> bool {
        self.step_taskbar_plugin(vec![HostEvent::Ui(event)], width)
            .is_some_and(|outcome| outcome.changed)
    }

    fn primary_panel_key(&self) -> nickel_core::plugins::PluginSurfaceKey {
        self.primary_panel_key.clone()
    }

    fn primary_panel_host_ref(
        &self,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_ref(&self.primary_panel_key())
    }

    fn plugin_panel_host_for(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<&mut nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_surface_hosts.get_mut(key).map(|(_, host)| host)
    }

    fn plugin_panel_host_ref(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_surface_hosts.get(key).map(|(_, host)| host)
    }

    fn run_host_ref(
        &self,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_ref(&crate::plugin_panel::run_surface_key())
    }

    fn run_host_mut(
        &mut self,
    ) -> Option<&mut nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_for(&crate::plugin_panel::run_surface_key())
    }

    fn launcher_host_ref(
        &self,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_ref(&crate::plugin_panel::launcher_surface_key())
    }

    fn launcher_host_mut(
        &mut self,
    ) -> Option<&mut nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_for(&crate::plugin_panel::launcher_surface_key())
    }

    fn control_plugin_host_ref(
        &self,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_ref(&crate::plugin_panel::control_center_surface_key())
    }

    fn control_plugin_host_mut(
        &mut self,
    ) -> Option<&mut nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_for(&crate::plugin_panel::control_center_surface_key())
    }

    fn notification_plugin_host_ref(
        &self,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_ref(&crate::plugin_panel::notification_surface_key())
    }

    fn notification_plugin_host_mut(
        &mut self,
    ) -> Option<&mut nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_for(&crate::plugin_panel::notification_surface_key())
    }

    fn preview_plugin_host_ref(
        &self,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_ref(&crate::plugin_panel::window_preview_surface_key())
    }

    fn preview_plugin_host_mut(
        &mut self,
    ) -> Option<&mut nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_for(&crate::plugin_panel::window_preview_surface_key())
    }

    fn reconcile_plugin_surface_root(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Result<bool, String> {
        let Some(grant) = self
            .external_plugin_packages
            .get(&key.plugin_id)
            .map(|package| &package.manifest)
            .or_else(|| {
                self.plugin_registry
                    .get(&key.plugin_id)
                    .map(|entry| &entry.manifest)
            })
            .and_then(|manifest| {
                manifest
                    .surfaces
                    .iter()
                    .find(|surface| surface.id == key.surface_id)
            })
            .cloned()
        else {
            return Ok(false);
        };
        let Some(host) = self.plugin_panel_host_for(key) else {
            return Ok(false);
        };
        let mut resolved = host.application().resolved_surface(&grant)?;
        if resolved.kind == nickel_core::plugins::PluginSurfaceKind::Window
            && let Some((anchor, offset_x, offset_y)) =
                self.plugin_window_placement_overrides.get(key).copied()
        {
            resolved.anchor = anchor;
            resolved.offset_x = offset_x;
            resolved.offset_y = offset_y;
        }
        let current = &mut self
            .plugin_surface_hosts
            .get_mut(key)
            .ok_or("plugin surface host disappeared")?
            .0;
        if *current == resolved {
            return Ok(false);
        }
        *current = resolved;
        Ok(true)
    }

    fn step_generic_plugin_surface(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        batch: HostBatch,
        keyboard_epoch: Option<u64>,
    ) -> bool {
        let result = {
            let Some(host) = self.plugin_panel_host_for(key) else {
                return false;
            };
            step_plugin_host(host, None, batch)
                .map(|(outcome, _)| (outcome.changed, host.application_mut().take_effects()))
        };
        let (changed, effects) = match result {
            Ok(result) => result,
            Err(error) => return self.fail_plugin_panel_runtime(&key.plugin_id, error),
        };
        let root_changed = match self.reconcile_plugin_surface_root(key) {
            Ok(changed) => changed,
            Err(error) => return self.fail_plugin_panel_runtime(&key.plugin_id, error),
        };
        changed
            | root_changed
            | self.apply_plugin_effects_with_keyboard_epoch(effects, keyboard_epoch)
    }

    pub(crate) fn plugin_panel_host_input_for(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        if *key == crate::plugin_panel::on_screen_keyboard_surface_key()
            && !self.native_surface_visible(SurfaceRole::Panel, Some(key))
        {
            return false;
        }
        let keyboard_epoch = (*key == crate::plugin_panel::on_screen_keyboard_surface_key())
            .then(|| self.keyboard_gesture_epoch(&input))
            .flatten();
        if *key == crate::plugin_panel::control_center_surface_key() {
            if !self.native_surface_visible(SurfaceRole::Panel, Some(key)) {
                return false;
            }
            let host = self.control_plugin_host_ref().unwrap();
            let (event, authority) =
                internal_normalized_ingress(input, None, "control-center", host.inspect(), None);
            return self
                .control_host_event_authorized(event, (width, height), None, Some(authority))
                .changed;
        }
        if *key == crate::plugin_panel::notification_surface_key() {
            return self.native_surface_visible(SurfaceRole::Panel, Some(key))
                && self.notification_host_input(input, width, height);
        }
        if self.taskbar_surface_key().as_ref() == Some(key) {
            let Some(host) = self.plugin_taskbar_host.as_ref() else {
                return false;
            };
            let (event, authority) =
                internal_normalized_ingress(input, None, "taskbar-panel", host.inspect(), None);
            return self
                .step_taskbar_plugin_batch(
                    HostBatch {
                        events: vec![event],
                        normalized_authorities: vec![authority],
                        ..HostBatch::default()
                    },
                    width,
                    height,
                )
                .is_some_and(|outcome| outcome.changed);
        }
        let (event, authority) = {
            let Some(host) = self.plugin_panel_host_for(key) else {
                return false;
            };
            internal_normalized_ingress(input, None, "plugin-panel", host.inspect(), None)
        };
        self.step_generic_plugin_surface(
            key,
            HostBatch {
                surface_size: Some((width, height)),
                events: vec![event],
                normalized_authorities: vec![authority],
                ..HostBatch::default()
            },
            keyboard_epoch,
        )
    }

    pub(crate) fn plugin_panel_host_controller_for(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        action: ControllerAction,
        width: u32,
        height: u32,
    ) -> bool {
        if *key == crate::plugin_panel::on_screen_keyboard_surface_key()
            && action == ControllerAction::Cancel
        {
            return self.set_keyboard_visible(false);
        }
        let keyboard_epoch = (*key == crate::plugin_panel::on_screen_keyboard_surface_key())
            .then(|| {
                self.keyboard_recipient
                    .as_ref()
                    .map(|snapshot| snapshot.epoch)
            })
            .flatten();
        if *key == crate::plugin_panel::control_center_surface_key() {
            return self.native_surface_visible(SurfaceRole::Panel, Some(key))
                && self.control_controller(action, width, height);
        }
        if *key == crate::plugin_panel::notification_surface_key() {
            return self.native_surface_visible(SurfaceRole::Panel, Some(key))
                && self.notification_controller(action);
        }
        if self.taskbar_surface_key().as_ref() == Some(key) {
            return self
                .step_taskbar_plugin_batch(
                    HostBatch {
                        events: vec![HostEvent::Controller(action)],
                        ..HostBatch::default()
                    },
                    width,
                    height,
                )
                .is_some_and(|outcome| outcome.changed);
        }
        self.step_generic_plugin_surface(
            key,
            HostBatch {
                surface_size: Some((width, height)),
                events: vec![HostEvent::Controller(action)],
                ..HostBatch::default()
            },
            keyboard_epoch,
        )
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn plugin_panel_host_ui_for(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        event: UiEvent,
        width: u32,
        height: u32,
    ) -> bool {
        let keyboard_epoch = (*key == crate::plugin_panel::on_screen_keyboard_surface_key()
            && matches!(&event, UiEvent::AccessibilityActivate(_)))
        .then(|| {
            self.keyboard_recipient
                .as_ref()
                .map(|snapshot| snapshot.epoch)
        })
        .flatten();
        if *key == crate::plugin_panel::control_center_surface_key() {
            return self.native_surface_visible(SurfaceRole::Panel, Some(key))
                && self.shell_role_host_ui(SurfaceRole::ControlCenter, event, width, height);
        }
        if *key == crate::plugin_panel::notification_surface_key() {
            return self.native_surface_visible(SurfaceRole::Panel, Some(key))
                && self.shell_role_host_ui(SurfaceRole::Notification, event, width, height);
        }
        if self.taskbar_surface_key().as_ref() == Some(key) {
            return self
                .step_taskbar_plugin_batch(
                    HostBatch {
                        events: vec![HostEvent::Ui(event)],
                        ..HostBatch::default()
                    },
                    width,
                    height,
                )
                .is_some_and(|outcome| outcome.changed);
        }
        self.step_generic_plugin_surface(
            key,
            HostBatch {
                surface_size: Some((width, height)),
                events: vec![HostEvent::Ui(event)],
                ..HostBatch::default()
            },
            keyboard_epoch,
        )
    }

    fn apply_plugin_effects(&mut self, effects: Vec<crate::plugin_panel::PluginEffect>) -> bool {
        self.apply_plugin_effects_with_keyboard_epoch(effects, None)
    }

    fn apply_plugin_effects_with_keyboard_epoch(
        &mut self,
        effects: Vec<crate::plugin_panel::PluginEffect>,
        keyboard_epoch: Option<u64>,
    ) -> bool {
        let mut changed = false;
        for effect in effects {
            match effect {
                crate::plugin_panel::PluginEffect::SetDisplayLayout { plugin_id, layout } => {
                    changed |= self.preview_plugin_display_layout(plugin_id, layout);
                }
                crate::plugin_panel::PluginEffect::ConfirmDisplayLayout { plugin_id } => {
                    changed |= self.confirm_plugin_display_layout(&plugin_id);
                }
                crate::plugin_panel::PluginEffect::RevertDisplayLayout { plugin_id } => {
                    changed |= self.revert_plugin_display_layout(Some(&plugin_id));
                }
                effect @ (crate::plugin_panel::PluginEffect::KeyboardKey { .. }
                | crate::plugin_panel::PluginEffect::KeyboardHide { .. }
                | crate::plugin_panel::PluginEffect::KeyboardDock { .. }
                | crate::plugin_panel::PluginEffect::KeyboardHold { .. }
                | crate::plugin_panel::PluginEffect::KeyboardResize { .. }) => {
                    changed |= self.apply_keyboard_plugin_effect(effect, keyboard_epoch);
                }
                crate::plugin_panel::PluginEffect::ShowLauncher => {
                    changed |= self.global_shortcut(platform::GlobalShortcut::ShowLauncher);
                }
                crate::plugin_panel::PluginEffect::ShowSettings(screen) => {
                    changed |= self.launch_settings(screen.as_deref());
                }
                crate::plugin_panel::PluginEffect::ShowPluginSurface {
                    plugin_id,
                    surface_id,
                } => match self.show_plugin_window(&plugin_id, &surface_id) {
                    Ok(shown) => changed |= shown,
                    Err(error) => {
                        tracing::warn!(plugin = plugin_id, surface = surface_id, %error, "plugin window request failed");
                    }
                },
                crate::plugin_panel::PluginEffect::HidePluginSurface {
                    plugin_id,
                    surface_id,
                } => {
                    let key = nickel_core::plugins::PluginSurfaceKey {
                        plugin_id,
                        surface_id,
                    };
                    match self.close_plugin_window(&key) {
                        Ok(closed) => changed |= closed,
                        Err(error) => {
                            tracing::warn!(plugin = key.plugin_id, surface = key.surface_id, %error, "plugin transient surface close failed");
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::FocusPluginSurface {
                    plugin_id,
                    surface_id,
                } => match self.focus_plugin_window(&plugin_id, &surface_id) {
                    Ok(focused) => changed |= focused,
                    Err(error) => {
                        tracing::warn!(plugin = plugin_id, surface = surface_id, %error, "plugin surface focus request failed");
                    }
                },
                crate::plugin_panel::PluginEffect::SetPluginSurfacePlacement {
                    plugin_id,
                    surface_id,
                    anchor,
                    offset_x,
                    offset_y,
                } => match self.set_plugin_window_placement(
                    &plugin_id,
                    &surface_id,
                    anchor,
                    offset_x,
                    offset_y,
                ) {
                    Ok(moved) => changed |= moved,
                    Err(error) => {
                        tracing::warn!(plugin = plugin_id, surface = surface_id, %error, "plugin window placement request failed");
                    }
                },
                crate::plugin_panel::PluginEffect::InvokeRegisteredSetting {
                    caller,
                    provider,
                    id,
                    value,
                } => {
                    let granted = self.plugin_registry.get(&caller).is_some_and(|entry| {
                        entry.desired_enabled
                            && entry
                                .manifest
                                .capabilities
                                .contains(&nickel_core::plugins::PluginCapability::SettingsWrite)
                    });
                    let valid = self
                        .package_settings_registry
                        .settings_snapshot()
                        .settings
                        .iter()
                        .any(|setting| {
                            setting.provider_package == provider
                                && setting.registration.id == id
                                && setting.registration.accepts_value(&value)
                        });
                    if !granted || !valid || self.package_settings_invoking {
                        continue;
                    }
                    let result = self
                        .plugin_surface_hosts
                        .iter_mut()
                        .find(|(key, _)| key.plugin_id == provider)
                        .map(|(_, (_, host))| {
                            host.application_mut()
                                .invoke_registered_setting(&id, &value)
                        })
                        .or_else(|| {
                            self.plugin_slot_hosts
                                .get_mut(&provider)
                                .map(|host| host.application.invoke_registered_setting(&id, &value))
                        });
                    if let Some(Ok(effects)) = result {
                        self.package_settings_invoking = true;
                        changed |= self.apply_plugin_effects(effects);
                        self.package_settings_invoking = false;
                    }
                }
                crate::plugin_panel::PluginEffect::SetPluginSetting {
                    plugin_id,
                    key,
                    value,
                } => {
                    let granted = self.plugin_registry.get(&plugin_id).is_some_and(|entry| {
                        entry.desired_enabled
                            && entry
                                .manifest
                                .capabilities
                                .contains(&nickel_core::plugins::PluginCapability::SettingsWrite)
                    });
                    if !granted {
                        tracing::warn!(
                            plugin = plugin_id,
                            setting = key,
                            "plugin setting grant is unavailable"
                        );
                        continue;
                    }
                    match self.set_plugin_setting(&plugin_id, &key, value) {
                        Ok(updated) => changed |= updated,
                        Err(error) => {
                            tracing::warn!(plugin = plugin_id, setting = key, %error, "plugin setting failed");
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::RunSubmit(command) => {
                    match platform::execute_run_command(&command) {
                        Ok(()) => {
                            self.set_launcher_visible(false);
                            changed = true;
                        }
                        Err(error) => {
                            let status =
                                format!("Could not run command: {}", launch_error_summary(&error));
                            if let Some(host) = self.run_host_mut() {
                                match host
                                    .application_mut()
                                    .sync_data(&serde_json::json!({ "status": status }))
                                {
                                    Ok(projected) => changed |= projected,
                                    Err(error) => self.fail_run_plugin_runtime(error),
                                }
                            }
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::RunDismiss => {
                    self.set_launcher_visible(false);
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::ToggleLauncher => {
                    self.apply_panel_action(TaskbarAction::Launcher);
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::ToggleControlCenter => {
                    self.apply_panel_action(TaskbarAction::Control);
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::ShowControlCenter => {
                    self.control_host.application_mut().show_control_center();
                    self.set_control_visible(true);
                    if self.control_visible {
                        self.set_launcher_visible(false);
                    }
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::ActivateWindow(window) => {
                    if self
                        .windows
                        .iter()
                        .any(|current| current.id == window && current.state.capabilities.activate)
                    {
                        changed |= self.try_send_window_action(window, WindowAction::Activate);
                    }
                }
                crate::plugin_panel::PluginEffect::CloseWindow(window) => {
                    if self
                        .windows
                        .iter()
                        .any(|current| current.id == window && current.state.capabilities.close)
                    {
                        changed |= self.try_send_window_action(window, WindowAction::Close);
                    }
                }
                crate::plugin_panel::PluginEffect::ToggleOnScreenKeyboard => {
                    if self.keyboard_enabled {
                        self.apply_panel_action(TaskbarAction::OnScreenKeyboard);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ToggleCodexProjects => {
                    if self.launcher.codex_available() {
                        self.apply_panel_action(TaskbarAction::Codex);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::CodexProjectRefresh => {
                    if self.codex_project_menu_visible && self.codex_menu_requests.len() < 8 {
                        self.codex_menu_requests.push(CodexMenuRequest::Refresh);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::CodexProjectClose => {
                    if self.codex_project_menu_visible {
                        self.codex_project_menu_visible = false;
                        self.codex_menu_requests.clear();
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::CodexProjectOpen { token, revision } => {
                    if self.codex_project_menu_visible && self.codex_menu_requests.len() < 8 {
                        self.codex_menu_requests
                            .push(CodexMenuRequest::Open { token, revision });
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ActivateTaskbarItem { index, id } => {
                    if crate::plugin_panel::taskbar_item_matches(&self.panel_groups(), index, &id) {
                        self.apply_panel_action(TaskbarAction::Task(index));
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ContextTaskbarItem { index, id } => {
                    if crate::plugin_panel::taskbar_item_matches(&self.panel_groups(), index, &id) {
                        self.apply_panel_action(TaskbarAction::TaskContext(index));
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::MoveApplicationPin { id, direction } => {
                    if matches!(direction, -1 | 1)
                        && self.launcher.is_pinned(&id)
                        && self.launcher.move_pin(&id, isize::from(direction))
                    {
                        self.persist_launcher_preferences();
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::MoveTaskbarPin {
                    index,
                    id,
                    direction,
                } => {
                    let groups = self.panel_groups();
                    if matches!(direction, -1 | 1)
                        && groups.get(index).is_some_and(|group| {
                            group.pinned
                                && group
                                    .application_id
                                    .as_ref()
                                    .is_some_and(|application| application.as_str() == id)
                        })
                    {
                        self.apply_panel_action(if direction < 0 {
                            TaskbarAction::MoveTaskPinLeft(id)
                        } else {
                            TaskbarAction::MoveTaskPinRight(id)
                        });
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::CloseTaskbarMenuWindows => {
                    self.apply_application_menu_action(ApplicationMenuAction::CloseAll);
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::InvokePluginSlotAction {
                    target_plugin,
                    slot_id,
                    plugin_id,
                    id,
                    item,
                } => {
                    if !self
                        .shell_panel_surfaces()
                        .iter()
                        .any(|(key, _)| key.plugin_id == target_plugin)
                    {
                        continue;
                    }
                    let visible = self
                        .plugin_slot_projection(&target_plugin)
                        .and_then(|slots| {
                            slots
                                .get(&slot_id)
                                .and_then(serde_json::Value::as_array)
                                .cloned()
                        })
                        .is_some_and(|actions| {
                            actions.iter().any(|action| {
                                action.get("pluginId").and_then(serde_json::Value::as_str)
                                    == Some(plugin_id.as_str())
                                    && action.get("id").and_then(serde_json::Value::as_str)
                                        == Some(id.as_str())
                                    && action["item"]
                                        .as_str()
                                        .is_none_or(|target| Some(target) == item.as_deref())
                            })
                        });
                    if !visible {
                        continue;
                    }
                    let Some(host) = self.plugin_slot_hosts.get_mut(&plugin_id) else {
                        continue;
                    };
                    if host.target_plugin != target_plugin
                        || host.target_slot != slot_id
                        || host.contract != nickel_core::plugins::PluginSlotContract::Action
                    {
                        continue;
                    }
                    let handled = host
                        .application
                        .activate_action(&id, item.as_deref().unwrap_or(""));
                    let extension_effects = host.application.take_effects();
                    let retained_bytes = host.application.retained_contribution_bytes();
                    if handled {
                        let _ = self.plugin_registry.record_memory(
                            &plugin_id,
                            nickel_core::plugins::PluginMemory {
                                native_ui_bytes: Some(retained_bytes),
                                ..Default::default()
                            },
                        );
                        self.refresh_plugin_slot_hosts(&target_plugin);
                        changed = true;
                        changed |= self.apply_plugin_effects(extension_effects);
                    }
                }
                crate::plugin_panel::PluginEffect::InvokePluginSlotSection {
                    target_plugin,
                    slot_id,
                    plugin_id,
                    id,
                } => {
                    if !self
                        .shell_panel_surfaces()
                        .iter()
                        .any(|(key, _)| key.plugin_id == target_plugin)
                    {
                        continue;
                    }
                    let visible = self
                        .plugin_slot_projection(&target_plugin)
                        .and_then(|slots| {
                            slots
                                .get(&slot_id)
                                .and_then(serde_json::Value::as_array)
                                .cloned()
                        })
                        .is_some_and(|sections| {
                            sections.iter().any(|section| {
                                section.get("pluginId").and_then(serde_json::Value::as_str)
                                    == Some(plugin_id.as_str())
                                    && section.get("id").and_then(serde_json::Value::as_str)
                                        == Some(id.as_str())
                            })
                        });
                    if !visible {
                        continue;
                    }
                    let Some(host) = self.plugin_slot_hosts.get_mut(&plugin_id) else {
                        continue;
                    };
                    if host.target_plugin != target_plugin
                        || host.target_slot != slot_id
                        || host.contract != nickel_core::plugins::PluginSlotContract::Section
                    {
                        continue;
                    }
                    let handled = host.application.activate_section(&id);
                    let extension_effects = host.application.take_effects();
                    let retained_bytes = host.application.retained_contribution_bytes();
                    if handled {
                        let _ = self.plugin_registry.record_memory(
                            &plugin_id,
                            nickel_core::plugins::PluginMemory {
                                native_ui_bytes: Some(retained_bytes),
                                ..Default::default()
                            },
                        );
                        self.refresh_plugin_slot_hosts(&target_plugin);
                        changed = true;
                        changed |= self.apply_plugin_effects(extension_effects);
                    }
                }
                crate::plugin_panel::PluginEffect::InvokeTaskbarWindowMenu { page, index } => {
                    if self.plugin_taskbar_host.is_none() {
                        continue;
                    }
                    let Some(snapshot) = self.window_menu_snapshot.as_ref() else {
                        continue;
                    };
                    let outputs = self.window_feed.outputs();
                    let entries = match page.as_str() {
                        "root" => window_menu_entries(snapshot, &self.workspaces, &outputs),
                        "workspaces" => workspace_menu_entries(snapshot, &self.workspaces),
                        "displays" => display_menu_entries(snapshot, &outputs),
                        _ => continue,
                    };
                    if let Some((_, action)) = entries.get(index)
                        && !matches!(
                            action,
                            MenuAction::ShowWorkspaces
                                | MenuAction::ShowDisplays
                                | MenuAction::Back
                        )
                    {
                        self.apply_window_menu_action(action.clone());
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ActivateTrayItem { id } => {
                    if self.tray.iter().take(128).any(|item| item.id == id) {
                        self.apply_panel_action(TaskbarAction::Tray(id));
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ContextTrayItem { id } => {
                    if self.tray.iter().take(128).any(|item| item.id == id) {
                        self.apply_panel_action(TaskbarAction::TrayContext(id));
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::SetLauncherQuery(query) => {
                    self.apply_launcher_action(LauncherAction::SetQuery(query));
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::SetLauncherPage { dashboard, page } => {
                    let matching_view = (self.launcher.mode()
                        == crate::launcher::LauncherMode::Dashboard)
                        == dashboard;
                    if matching_view {
                        let projection = self.current_plugin_launcher_projection();
                        let count = if dashboard {
                            projection.dashboard_page_count
                        } else {
                            projection.result_page_count
                        };
                        if page < count {
                            let current = if dashboard {
                                &mut self.launcher_plugin_dashboard_page
                            } else {
                                &mut self.launcher_plugin_result_page
                            };
                            changed |= *current != page;
                            *current = page;
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::DismissLauncher => {
                    if self.launcher_visible {
                        self.apply_launcher_action(LauncherAction::Dismiss);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ActivateLauncherResult { index, id } => {
                    if self
                        .current_plugin_launcher_projection()
                        .results
                        .iter()
                        .any(|result| result.index == index && result.id == id)
                    {
                        self.apply_launcher_action(LauncherAction::ActivateResult(index));
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::LaunchApplication { id } => {
                    if self
                        .launcher
                        .applications()
                        .any(|application| application.id() == id)
                    {
                        self.launch_application_by_id(&id);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::SetLauncherView(view) => {
                    if self.launcher.mode() == crate::launcher::LauncherMode::Dashboard {
                        self.apply_launcher_action(LauncherAction::SetView(view));
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ToggleApplicationPin { id } => {
                    if self.launcher.is_pinned(&id)
                        || self
                            .launcher
                            .applications()
                            .any(|application| application.id() == id)
                        || self.windows.iter().any(|window| {
                            window
                                .application_id
                                .as_ref()
                                .is_some_and(|application| application.as_str() == id)
                        })
                    {
                        let dismiss_menu =
                            self.application_menu_target.as_ref().is_some_and(|target| {
                                target
                                    .application_id
                                    .as_ref()
                                    .is_some_and(|application| application.as_str() == id)
                            });
                        self.apply_launcher_action(LauncherAction::TogglePin(id));
                        if dismiss_menu {
                            self.dismiss_window_menu();
                        }
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::RetryApplicationPinSave => {
                    if self.launcher_status.as_deref().is_some_and(|status| {
                        status.starts_with("Launcher preferences could not be saved:")
                    }) {
                        self.apply_launcher_action(LauncherAction::RetryPreferencePersistence);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::LauncherOpenProject { id } => {
                    let projection = self.current_plugin_launcher_projection();
                    if projection.dashboard_visible
                        && projection.projects.iter().any(|project| project.id == id)
                    {
                        self.apply_launcher_action(LauncherAction::OpenProject(id));
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::LauncherSeeAllProjects => {
                    if self.launcher.mode() == crate::launcher::LauncherMode::Dashboard
                        && self.launcher.codex_available()
                    {
                        self.apply_launcher_action(LauncherAction::SeeAllProjects);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::LauncherRequestLogout => {
                    if self.launcher.mode() == crate::launcher::LauncherMode::Dashboard
                        && self.launcher.logout_available()
                    {
                        self.apply_launcher_action(LauncherAction::RequestLogout);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::InvokeNotification { id, key } => {
                    if !self.notification_history_visible
                        && !self.trusted_notification_id(id)
                        && self.notification.as_ref().is_some_and(|item| item.id == id)
                    {
                        self.notification_host.application_mut().request_effect(
                            NotificationEffect::Invoke {
                                notification_id: id,
                                key,
                            },
                        );
                        changed |= self.apply_notification_effects();
                    }
                }
                crate::plugin_panel::PluginEffect::DismissNotification { id } => {
                    if !self.notification_history_visible
                        && !self.trusted_notification_id(id)
                        && self.notification.as_ref().is_some_and(|item| item.id == id)
                    {
                        self.notification_host.application_mut().request_effect(
                            NotificationEffect::Dismiss {
                                notification_id: id,
                            },
                        );
                        changed |= self.apply_notification_effects();
                    }
                }
                crate::plugin_panel::PluginEffect::CloseNotificationHistory => {
                    if self.notification_history_visible && !self.trusted_notification_visible() {
                        self.notification_host
                            .application_mut()
                            .request_effect(NotificationEffect::CloseHistory);
                        changed |= self.apply_notification_effects();
                    }
                }
                crate::plugin_panel::PluginEffect::Associations { plugin_id, effect } => {
                    let granted = self.external_plugin_packages.get(&plugin_id).map(|package| &package.manifest)
                        .or_else(|| self.plugin_registry.get(&plugin_id).map(|entry| &entry.manifest))
                        .is_some_and(|manifest| manifest.capabilities.contains(&effect.capability())
                            && (!matches!(effect, crate::associations_capabilities::AssociationsEffect::SetDefault { .. })
                                || manifest.capabilities.contains(&nickel_core::plugins::PluginCapability::AssociationsRead)));
                    if granted && !self.locked {
                        let result = effect.execute_native().unwrap_or_else(|error| serde_json::json!({"status":"rejected","detail":error.chars().take(512).collect::<String>()}));
                        self.associations_results.insert(plugin_id, result);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::Preferences { plugin_id, effect } => {
                    use nickel_core::plugins::PluginCapability;
                    let granted = !self.locked
                        && self
                            .external_plugin_packages
                            .get(&plugin_id)
                            .map(|package| &package.manifest)
                            .or_else(|| {
                                self.plugin_registry
                                    .get(&plugin_id)
                                    .map(|entry| &entry.manifest)
                            })
                            .is_some_and(|manifest| {
                                manifest
                                    .capabilities
                                    .contains(&PluginCapability::PreferencesRead)
                                    && manifest.capabilities.contains(&effect.capability())
                            });
                    if granted {
                        let result = self.preferences_catalog().and_then(|catalog| {
                            self.preferences_capabilities
                                .execute(&effect, &catalog, || {
                                    if granted {
                                        Ok(())
                                    } else {
                                        Err("preferences grant was retired".into())
                                    }
                                })
                        });
                        match result {
                            Ok(settings) => {
                                self.preferences_commit_pending = Some(settings.clone());
                                self.launcher_icons.begin_visual_generation();
                                self.launcher_icons.invalidate_application_inventory();
                                self.apply_shell_settings(settings);
                                changed = true;
                            }
                            Err(error) => tracing::warn!(%error, "preferences capability rejected"),
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::Appearance { plugin_id, effect } => {
                    let granted = self
                        .external_plugin_packages
                        .get(&plugin_id)
                        .map(|package| &package.manifest)
                        .or_else(|| {
                            self.plugin_registry
                                .get(&plugin_id)
                                .map(|entry| &entry.manifest)
                        })
                        .is_some_and(|manifest| {
                            manifest.capabilities.contains(&effect.capability())
                                && manifest.capabilities.contains(&effect.read_capability())
                        });
                    if granted && !self.locked {
                        match self.appearance_capabilities.execute(&effect, || {
                            if granted {
                                Ok(())
                            } else {
                                Err("appearance grant was retired".into())
                            }
                        }) {
                            Ok(
                                crate::appearance_capabilities::CommittedAppearance::Appearance(
                                    settings,
                                ),
                            ) => {
                                changed |= self.apply_shell_settings(settings);
                            }
                            Ok(crate::appearance_capabilities::CommittedAppearance::Wallpaper(
                                settings,
                            )) => {
                                self.refresh_configured_wallpaper(settings.image);
                                self.desktop_application_dirty = true;
                                self.appearance_capabilities.wallpaper_reconciled();
                                changed = true;
                            }
                            Err(error) => tracing::warn!(%error, "appearance capability rejected"),
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::Connectivity { plugin_id, effect } => {
                    let granted = self
                        .external_plugin_packages
                        .get(&plugin_id)
                        .map(|package| &package.manifest)
                        .or_else(|| {
                            self.plugin_registry
                                .get(&plugin_id)
                                .map(|entry| &entry.manifest)
                        })
                        .is_some_and(|manifest| {
                            manifest.capabilities.contains(&effect.capability())
                        });
                    if granted && !self.locked {
                        match effect.execute() {
                            Ok(success) => {
                                log_control_result("connectivity-capability", success);
                                changed |= success;
                            }
                            Err(error) => {
                                tracing::warn!(%error, "connectivity capability rejected")
                            }
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::Control(action) => {
                    if self.control_plugin_action_allowed(&action) {
                        self.control_host.application_mut().update(action);
                        self.apply_control_effects();
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::Preview(action) => {
                    if self.preview_plugin_action_allowed(action) {
                        self.apply_preview_action(action);
                        changed = true;
                    }
                }
            }
        }
        changed
    }

    fn sync_plugin_launcher(&mut self) -> Option<bool> {
        if self.launcher_host_ref().is_none() {
            return None;
        }
        let projection = self.current_plugin_launcher_projection();
        self.launcher_plugin_result_page = projection.result_page;
        self.launcher_plugin_dashboard_page = projection.dashboard_page;
        let images = launcher_plugin_images(&self.launcher, &mut self.launcher_icons, &projection);
        let palette = self.palette;
        let host = self
            .launcher_host_mut()
            .expect("launcher plugin host exists");
        let palette_changed = match host.application_mut().sync_theme_palette(palette) {
            Ok(changed) => changed,
            Err(error) => {
                self.fail_launcher_plugin_runtime(error);
                return None;
            }
        };
        let image_changed = host.application_mut().sync_images(images);
        let projection_changed = match host
            .application_mut()
            .sync_serialized_data(projection.to_json())
        {
            Ok(changed) => changed,
            Err(error) => {
                self.fail_launcher_plugin_runtime(error);
                return None;
            }
        };
        Some(palette_changed || image_changed || projection_changed)
    }

    fn current_plugin_launcher_projection(&self) -> crate::plugin_panel::LauncherPluginProjection {
        crate::plugin_panel::LauncherPluginProjection::from_launcher_pages(
            &self.launcher,
            self.launcher_plugin_result_page,
            self.launcher_plugin_dashboard_page,
        )
        .with_status(self.launcher_status_text())
    }

    pub(crate) fn launcher_host_ui(&mut self, event: UiEvent, width: u32, height: u32) -> bool {
        self.launcher_host_event_with_clipboard_limit(HostEvent::Ui(event), width, height, None)
            .changed
    }

    pub(crate) fn control_host_event(
        &mut self,
        event: HostEvent,
        size: (u32, u32),
        limit: Option<usize>,
    ) -> nickel_ui::HostEventOutcome {
        self.control_host_event_authorized(event, size, limit, None)
    }

    pub(crate) fn control_host_event_authorized(
        &mut self,
        event: HostEvent,
        size: (u32, u32),
        limit: Option<usize>,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> nickel_ui::HostEventOutcome {
        if !self.control_visible {
            return Default::default();
        }
        if self.control_plugin_active() {
            return self.control_plugin_event(event, size, limit, authority);
        }
        if !self.control_host.application().view_state().projection_only {
            return Default::default();
        }
        self.sync_control_host(size.0, size.1);
        let mut outcome = self.control_host.step(HostBatch {
            surface_size: Some(size),
            clipboard_text_limit: limit,
            events: vec![event],
            normalized_authorities: authority.into_iter().collect(),
            ..Default::default()
        });
        self.host_runtime_samples.record(outcome.telemetry);
        outcome.changed |= outcome.change_token != self.control_change_token;
        self.control_change_token = outcome.change_token;
        self.control_deadline = outcome.next_deadline;
        self.apply_control_effects();
        outcome
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn shell_field_lease(&self, role: SurfaceRole) -> Option<(nickel_ui::UiId, u64)> {
        let inspection = match role {
            SurfaceRole::Launcher if self.run_visible => self.run_host_ref()?.inspect(),
            SurfaceRole::Launcher => self.launcher_host_ref()?.inspect(),
            SurfaceRole::ControlCenter => self
                .control_plugin_active()
                .then(|| self.control_plugin_host_ref().unwrap().inspect())
                .or_else(|| {
                    self.control_host
                        .application()
                        .view_state()
                        .projection_only
                        .then(|| self.control_host.inspect())
                })?,
            _ => return None,
        };
        Some((
            inspection.keyboard_focus?,
            inspection.keyboard_focus_generation,
        ))
    }

    /// Dispatches compositor-owned semantic UI events through the same hosts and
    /// effect reducers used by the windowed shell.
    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn shell_role_host_ui(
        &mut self,
        role: SurfaceRole,
        event: UiEvent,
        width: u32,
        height: u32,
    ) -> bool {
        match role {
            SurfaceRole::Taskbar => self.panel_host_ui(event, width),
            // Plugin panels always carry a surface key and use
            // `plugin_panel_host_ui_for` at the compositor boundary.
            SurfaceRole::Panel => false,
            SurfaceRole::Launcher => self.launcher_host_ui(event, width, height),
            SurfaceRole::ControlCenter => {
                if !self.control_visible || !self.control_surface_available() {
                    return false;
                }
                if self.control_plugin_active() {
                    return self
                        .control_plugin_event(HostEvent::Ui(event), (width, height), None, None)
                        .changed;
                }
                self.sync_control_host(width, height);
                let changed = self.step_control_host(HostBatch {
                    surface_size: Some((width, height)),
                    events: vec![HostEvent::Ui(event)],
                    ..HostBatch::default()
                });
                self.apply_control_effects();
                changed
            }
            SurfaceRole::Notification => {
                if !self.surface_visible(SurfaceRole::Notification) {
                    return false;
                }
                if self.notification_plugin_active() {
                    return self
                        .step_notification_plugin(HostBatch {
                            surface_size: Some((width, height)),
                            events: vec![HostEvent::Ui(event)],
                            ..HostBatch::default()
                        })
                        .is_some_and(|outcome| outcome.changed);
                }
                self.sync_notification_host(width, height);
                let outcome = self.notification_host.step(HostBatch {
                    surface_size: Some((width, height)),
                    events: vec![HostEvent::Ui(event)],
                    ..HostBatch::default()
                });
                outcome.changed | self.apply_notification_effects()
            }
            SurfaceRole::WindowPreview => {
                self.preview_plugin_active()
                    && self
                        .preview_plugin_event(HostEvent::Ui(event), (width, height), None)
                        .changed
            }
            SurfaceRole::WindowContextMenu => {
                if !self.surface_visible(SurfaceRole::WindowContextMenu) {
                    return false;
                }
                if matches!(
                    event,
                    UiEvent::KeyboardNavigateBack | UiEvent::ControllerBack
                ) {
                    return self.window_menu_host_key(Some(KeyCode::Escape));
                }
                if self.application_menu_target.is_some() {
                    self.application_menu_plugin_event(HostEvent::Ui(event), width, height, None)
                } else {
                    self.window_menu_plugin_event(HostEvent::Ui(event), width, height, None)
                }
            }
            SurfaceRole::Lock => {
                if !self.locked {
                    return false;
                }
                let (window_focused, events) = match event {
                    UiEvent::FocusGained => (Some(true), Vec::new()),
                    UiEvent::FocusLost => (Some(false), Vec::new()),
                    event => (None, vec![HostEvent::Ui(event)]),
                };
                let outcome = self.lock_host.step(HostBatch {
                    surface_size: Some((width, height)),
                    window_focused,
                    events,
                    ..HostBatch::default()
                });
                outcome.changed | self.apply_lock_effects()
            }
            SurfaceRole::OnScreenKeyboard => self.keyboard_step(
                HostBatch {
                    surface_size: Some((width, height)),
                    events: vec![HostEvent::Ui(event)],
                    ..HostBatch::default()
                },
                self.keyboard_recipient
                    .as_ref()
                    .map(|recipient| recipient.epoch),
            ),
            SurfaceRole::Screenshot => match event {
                UiEvent::PointerMoved(point) => {
                    self.screenshot_pointer_moved(point.x, point.y, width, height)
                }
                UiEvent::PointerPressed(point) => {
                    self.screenshot_pointer_pressed(point.x, point.y, width, height)
                }
                UiEvent::PointerReleased(_) => self.screenshot_pointer_released(),
                event => self.screenshot_host_event(HostEvent::Ui(event), width, height),
            },
            // These are intentionally passive or hosted outside LiveShell.
            SurfaceRole::Desktop
            | SurfaceRole::VolumeOsd
            | SurfaceRole::CodexProjectMenu
            | SurfaceRole::CodexChat => false,
            #[cfg(target_os = "windows")]
            SurfaceRole::TrustedControl => false,
        }
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn shell_role_host_shortcut(
        &mut self,
        role: SurfaceRole,
        shortcut: Shortcut,
        width: u32,
        height: u32,
    ) -> bool {
        match role {
            SurfaceRole::Screenshot => {
                self.screenshot_host_event(HostEvent::Shortcut(shortcut), width, height)
            }
            SurfaceRole::Launcher => {
                let outcome = self.launcher_host_event_with_clipboard_limit(
                    HostEvent::Shortcut(shortcut),
                    width,
                    height,
                    None,
                );
                // Native scene input is queued before this host can report whether
                // Submit was handled. Perform the activation fallback here, at
                // the launcher owner, so dashboard rows receive Enter too.
                if shortcut == Shortcut::Submit && !outcome.changed {
                    self.launcher_host_event_with_clipboard_limit(
                        HostEvent::Ui(UiEvent::KeyboardNavigateActivate),
                        width,
                        height,
                        None,
                    )
                    .changed
                } else {
                    outcome.changed
                }
            }
            SurfaceRole::WindowContextMenu => {
                if shortcut == Shortcut::Escape {
                    self.window_menu_host_key(Some(KeyCode::Escape))
                } else {
                    self.window_menu_host_event(HostEvent::Shortcut(shortcut), width, height)
                }
            }
            SurfaceRole::Lock if self.locked => {
                let outcome = self.lock_host.step(HostBatch {
                    surface_size: Some((width, height)),
                    events: vec![HostEvent::Shortcut(shortcut)],
                    ..HostBatch::default()
                });
                self.lock_change_token = outcome.change_token;
                self.lock_deadline = outcome.next_deadline;
                outcome.changed | self.apply_lock_effects()
            }
            _ => false,
        }
    }

    fn apply_panel_action(&mut self, action: TaskbarAction) {
        let anchored_role = match &action {
            TaskbarAction::Codex => Some((ShellRole::ProjectMenu, "panel-codex")),
            TaskbarAction::Control => Some((ShellRole::ControlCenter, "panel-control")),
            _ => None,
        };
        if anchored_role.is_some() {
            self.pending_popover_anchor = None;
        }
        let plugin_control = match action {
            TaskbarAction::Control => Some("taskbar-control"),
            TaskbarAction::Codex => Some("taskbar-codex"),
            _ => None,
        };
        let anchor_bounds = self
            .plugin_taskbar_host
            .as_ref()
            .and_then(|host| plugin_control.and_then(|id| taskbar_plugin_control_bounds(host, id)));
        if let Some((role, control)) = anchored_role
            && let (Some(output), Some(bounds)) = (self.panel_output.clone(), anchor_bounds)
        {
            self.pending_popover_anchor = Some(PendingPopoverAnchor {
                role,
                control: plugin_control.unwrap_or(control).to_owned(),
                output,
                bounds,
            });
        }
        match action {
            TaskbarAction::Launcher => self.set_launcher_visible(!self.launcher_visible),
            TaskbarAction::OnScreenKeyboard => {
                self.set_keyboard_visible(!self.keyboard_visible);
            }
            TaskbarAction::Task(index) => {
                let groups = self.panel_groups();
                if let Some(window) = groups.get(index).and_then(|group| group.windows.last()) {
                    let _ = self.send_session_command(
                        "activate-window",
                        ShellCommand::WindowAction {
                            window: window.id,
                            action: WindowAction::Activate,
                        },
                    );
                    self.close_window_preview();
                } else if let Some(application_id) = groups
                    .get(index)
                    .filter(|group| group.available)
                    .and_then(|group| group.application_id.as_ref())
                {
                    self.launch_application_by_id(application_id.as_str());
                }
            }
            TaskbarAction::TaskContext(index) => {
                let groups = self.panel_groups();
                let Some(group) = groups.get(index) else {
                    return;
                };
                let target = ApplicationMenuTarget::capture(&group.window_group());
                let pinned = target
                    .application_id
                    .as_ref()
                    .is_some_and(|id| self.launcher.is_pinned(id.as_str()));
                if application_menu_entries(&target, pinned).is_empty() {
                    return;
                }
                self.close_window_preview();
                self.window_menu_generation = self.window_menu_generation.saturating_add(1);
                self.application_menu_target = Some(target);
                self.clear_application_menu_plugin_host();
                self.plugin_taskbar_menu_memory = 0;
                let x = self
                    .plugin_taskbar_host
                    .as_ref()
                    .and_then(|host| {
                        taskbar_plugin_control_bounds(host, &format!("taskbar-item-{index}"))
                            .map(|bounds| bounds.origin.x.round() as i32)
                    })
                    .unwrap_or((PANEL_ITEM_WIDTH * (index + 1) as f32).round() as i32);
                self.window_menu_anchor_x = Some(self.panel_origin_x + x);
                self.window_menu_anchor_y = Some(self.panel_origin_y);
                let _ = self.send_session_command(
                    "show-context-menu",
                    ShellCommand::ShowContextMenu {
                        x: self.panel_origin_x + x,
                        y: self.panel_origin_y,
                        width: MENU_WIDTH as i32,
                        height: self.window_context_menu_height(),
                    },
                );
                #[cfg(target_os = "linux")]
                let _ =
                    self.send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
            }
            TaskbarAction::MoveTaskPinLeft(id) => {
                if self.launcher.move_pin(&id, -1) {
                    self.persist_launcher_preferences();
                }
            }
            TaskbarAction::MoveTaskPinRight(id) => {
                if self.launcher.move_pin(&id, 1) {
                    self.persist_launcher_preferences();
                }
            }
            // Drag gestures are reduced by `TaskbarApplication` into a typed move action.
            TaskbarAction::Codex => {
                if !self.launcher.codex_available()
                    || !self
                        .plugin_surface_matches(&crate::plugin_panel::codex_projects_surface_key())
                {
                    self.codex_project_menu_visible = false;
                    return;
                }
                if self.launcher_visible {
                    self.set_launcher_visible(false);
                }
                self.codex_project_menu_visible = !self.codex_project_menu_visible;
            }
            TaskbarAction::Tray(id) => self.tray_feed.activate(&id),
            TaskbarAction::TrayContext(id) => self.tray_feed.context_menu(&id),
            TaskbarAction::Control => {
                if self.launcher_visible {
                    self.set_launcher_visible(false);
                }
                if !self.control_visible {
                    if let Some(taskbar) = self.plugin_taskbar_host.as_ref() {
                        let _ = self
                            .control_host
                            .adopt_input_modality(taskbar.inspect().modality);
                    }
                }
                self.set_control_visible(!self.control_visible);
            }
        }
    }

    #[cfg(any(target_os = "linux", test))]
    fn semantic_panel_output(&self, requested: Option<&String>) -> Option<String> {
        requested
            .cloned()
            .or_else(|| self.panel_output.clone())
            .or_else(|| {
                self.plugin_taskbar_hosts
                    .keys()
                    .filter_map(Clone::clone)
                    .min()
            })
    }

    #[cfg(any(target_os = "linux", test))]
    fn semantic_panel_host(
        &self,
        output: &Option<String>,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        if output == &self.panel_output {
            self.plugin_taskbar_host.as_ref()
        } else {
            self.plugin_taskbar_hosts.get(output)
        }
    }

    /// Resolves a test/accessibility semantic target from the same live group
    /// and renderer frame records used by pointer hit testing. The caller is
    /// responsible for dispatching the returned point as ordinary input.
    #[cfg(any(target_os = "linux", test))]
    pub fn resolve_semantic_target(
        &self,
        target: &ShellSemanticTarget,
    ) -> Option<ResolvedShellTarget> {
        match target {
            ShellSemanticTarget::OnScreenKeyboard { key } => {
                let plugin_key = crate::plugin_panel::on_screen_keyboard_surface_key();
                if self.keyboard_visible
                    && self.plugin_surface_matches(&plugin_key)
                    && let Some((_, host)) = self.plugin_surface_hosts.get(&plugin_key)
                {
                    let button = match key.as_str() {
                        "osk-hide" => "osk-plugin-hide",
                        "osk-larger" => "osk-plugin-larger",
                        "osk-smaller" => "osk-plugin-smaller",
                        "osk-dock" => "osk-plugin-dock",
                        "osk-persistent-modifiers" => "osk-plugin-hold",
                        key => key,
                    };
                    let bounds = taskbar_plugin_control_bounds(host, button)?;
                    return Some(ResolvedShellTarget {
                        role: ShellRole::OnScreenKeyboard,
                        output: None,
                        x: (bounds.origin.x + bounds.size.width / 2.0).round() as i32,
                        y: (bounds.origin.y + bounds.size.height / 2.0).round() as i32,
                        interaction: PointerInteraction::LeftClick,
                    });
                }
                use nickel_core::on_screen_keyboard::{
                    KeyboardPanel, compact_us_keyboard_rows, us_keyboard_rows,
                };
                use nickel_ui::on_screen_keyboard::KeyboardMessage;
                let message = if key == "osk-hide" {
                    KeyboardMessage::Hide
                } else if key == "osk-larger" {
                    KeyboardMessage::ResizeBy(32)
                } else if key == "osk-smaller" {
                    KeyboardMessage::ResizeBy(-32)
                } else if key == "osk-dock" {
                    KeyboardMessage::ToggleDock
                } else if key == "osk-persistent-modifiers" {
                    KeyboardMessage::PersistentModifiers
                } else {
                    let definition = [
                        KeyboardPanel::Letters,
                        KeyboardPanel::Symbols,
                        KeyboardPanel::Navigation,
                    ]
                    .into_iter()
                    .flat_map(|panel| {
                        us_keyboard_rows(panel)
                            .into_iter()
                            .chain(compact_us_keyboard_rows(panel))
                    })
                    .flatten()
                    .find(|definition| definition.id == *key)?;
                    KeyboardMessage::Key(definition.key)
                };
                let node = self
                    .keyboard_host
                    .semantic_targets_for_message(&message)
                    .into_iter()
                    .next()?;
                Some(ResolvedShellTarget {
                    role: ShellRole::OnScreenKeyboard,
                    output: None,
                    x: (node.bounds.origin.x + node.bounds.size.width / 2.0).round() as i32,
                    y: (node.bounds.origin.y + node.bounds.size.height / 2.0).round() as i32,
                    interaction: PointerInteraction::LeftClick,
                })
            }
            ShellSemanticTarget::OnScreenKeyboardToggle => {
                let output = self.semantic_panel_output(None);
                let host = self.semantic_panel_host(&output)?;
                let bounds = taskbar_plugin_control_bounds(host, "taskbar-keyboard")?;
                Some(ResolvedShellTarget {
                    role: ShellRole::Panel,
                    output,
                    x: (bounds.origin.x + bounds.size.width / 2.0).round() as i32,
                    y: (bounds.origin.y + bounds.size.height / 2.0).round() as i32,
                    interaction: PointerInteraction::LeftClick,
                })
            }
            ShellSemanticTarget::PanelApplication {
                application_id,
                output,
                interaction,
            } => {
                let output = self.semantic_panel_output(output.as_ref());
                let plugin_host = self.semantic_panel_host(&output)?;
                let windows = self
                    .windows
                    .iter()
                    .filter(|window| {
                        window_belongs_to_panel(
                            self.all_windows_on_every_bar,
                            output.as_deref(),
                            window.state.output.as_deref(),
                        )
                    })
                    .cloned()
                    .collect::<Vec<_>>();
                let groups = self.launcher.taskbar_applications(&windows);
                let index = groups.iter().take(12).position(|group| {
                    group
                        .application_id
                        .as_ref()
                        .is_some_and(|id| id.as_str() == application_id)
                })?;
                if !plugin_host
                    .application()
                    .rendered_taskbar_item_matches(index, application_id)
                {
                    return None;
                }
                let bounds =
                    taskbar_plugin_control_bounds(plugin_host, &format!("taskbar-item-{index}"))?;
                Some(ResolvedShellTarget {
                    role: ShellRole::Panel,
                    output,
                    x: (bounds.origin.x + bounds.size.width / 2.0).round() as i32,
                    y: (bounds.origin.y + bounds.size.height / 2.0).round() as i32,
                    interaction: *interaction,
                })
            }
            ShellSemanticTarget::PanelControlCenter { output } => {
                let output = self.semantic_panel_output(output.as_ref());
                let plugin_host = self.semantic_panel_host(&output)?;
                let bounds = taskbar_plugin_control_bounds(plugin_host, "taskbar-control")?;
                Some(ResolvedShellTarget {
                    role: ShellRole::Panel,
                    output,
                    x: (bounds.origin.x + bounds.size.width / 2.0).round() as i32,
                    y: (bounds.origin.y + bounds.size.height / 2.0).round() as i32,
                    interaction: PointerInteraction::LeftClick,
                })
            }
            ShellSemanticTarget::ControlCenterLock => {
                if !self.control_visible {
                    return None;
                }
                let bounds = if self.control_plugin_active() {
                    taskbar_plugin_control_bounds(self.control_plugin_host_ref()?, "session-lock")?
                } else {
                    self.control_host
                        .semantic_targets_for_message(&ControlAction::SessionAction(
                            crate::platform::SessionAction::Lock,
                        ))
                        .into_iter()
                        .next()?
                        .bounds
                };
                Some(ResolvedShellTarget {
                    role: ShellRole::ControlCenter,
                    output: None,
                    x: (bounds.origin.x + bounds.size.width / 2.0).round() as i32,
                    y: (bounds.origin.y + bounds.size.height / 2.0).round() as i32,
                    interaction: PointerInteraction::LeftClick,
                })
            }
            ShellSemanticTarget::PreviewWindow { window, action } => {
                let window = crate::model::WindowId(window.0);
                let preview_action = match action {
                    PreviewTargetAction::Hover | PreviewTargetAction::Activate => {
                        PreviewAction::Activate(window)
                    }
                    PreviewTargetAction::Close => PreviewAction::Close(window),
                    PreviewTargetAction::OpenMenu => PreviewAction::OpenMenu(window),
                };
                let bounds = self.preview_plugin_bounds(preview_action)?;
                let point = Point {
                    x: bounds.origin.x + bounds.size.width / 2.0,
                    y: bounds.origin.y + bounds.size.height / 2.0,
                };
                Some(ResolvedShellTarget {
                    role: ShellRole::Preview,
                    output: None,
                    x: point.x.round() as i32,
                    y: point.y.round() as i32,
                    interaction: match action {
                        PreviewTargetAction::Hover => PointerInteraction::Hover,
                        PreviewTargetAction::OpenMenu => PointerInteraction::RightClick,
                        PreviewTargetAction::Activate | PreviewTargetAction::Close => {
                            PointerInteraction::LeftClick
                        }
                    },
                })
            }
            ShellSemanticTarget::WindowMenu { window, action } => {
                let window = crate::model::WindowId(window.0);
                let menu_action = match action {
                    WindowMenuTargetAction::Close => MenuAction::Close(window),
                    WindowMenuTargetAction::MaximizeRestore => MenuAction::MaximizeRestore(window),
                    WindowMenuTargetAction::Minimize => MenuAction::Minimize(window),
                };
                let host = self.window_menu_plugin_host.as_ref()?;
                let snapshot = self.window_menu_snapshot.as_ref()?;
                if snapshot.id != window {
                    return None;
                }
                let outputs = self.window_feed.outputs();
                let entries = window_menu_entries(snapshot, &self.workspaces, &outputs);
                let index = entries
                    .iter()
                    .position(|(_, entry)| entry == &menu_action)?;
                let id = format!("window-menu-action-{index}");
                let bounds = host
                    .semantic_nodes()
                    .into_iter()
                    .find(|node| {
                        node.id.as_str() == id || node.id.as_str().ends_with(&format!("/{id}"))
                    })?
                    .bounds;
                let point = Point {
                    x: bounds.origin.x + bounds.size.width / 2.0,
                    y: bounds.origin.y + bounds.size.height / 2.0,
                };
                Some(ResolvedShellTarget {
                    role: ShellRole::ContextMenu,
                    output: None,
                    x: point.x.round() as i32,
                    y: point.y.round() as i32,
                    interaction: PointerInteraction::LeftClick,
                })
            }
            ShellSemanticTarget::Screenshot { .. } => None,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn perform_screenshot_semantic_action(
        &mut self,
        action: nickel_session_protocol::ScreenshotTargetAction,
    ) -> bool {
        self.screenshot.perform_semantic_action(action)
    }

    fn sync_panel_hover_from_host(&mut self) -> bool {
        let hovered = self
            .plugin_taskbar_host
            .as_ref()
            .and_then(|host| host.inspect().pointer_hover)
            .and_then(|id| {
                let id = id.as_str().rsplit('/').next()?;
                if id == "taskbar-launcher" {
                    Some(TaskbarHover::Launcher)
                } else if id == "taskbar-control" {
                    Some(TaskbarHover::Control)
                } else if let Some(tray_id) = id.strip_prefix("taskbar-tray-") {
                    self.tray
                        .iter()
                        .rev()
                        .take(4)
                        .rev()
                        .position(|item| item.id == tray_id)
                        .map(TaskbarHover::Tray)
                } else {
                    id.strip_prefix("taskbar-item-")
                        .and_then(|index| index.parse().ok())
                        .map(TaskbarHover::Task)
                }
            });
        let changed = hovered != self.panel_hover;
        self.panel_hover = hovered;
        self.panel_hover_output.clone_from(&self.panel_output);
        if let Some(TaskbarHover::Task(index)) = hovered {
            if self.preview_group != Some(index)
                && self.preview_pending.map(|(pending, _)| pending) != Some(index)
            {
                self.preview_pending = Some((index, Instant::now() + PREVIEW_HOVER_DELAY));
            }
            self.preview_leave_deadline = None;
        } else {
            self.preview_pending = None;
            if self.preview_group.is_some() && !self.preview_pointer_inside {
                self.preview_leave_deadline = Some(Instant::now() + PREVIEW_LEAVE_DELAY);
            }
        }
        changed
    }

    pub fn set_panel_origin_x(&mut self, origin_x: i32) {
        self.panel_origin_x = origin_x;
    }

    pub fn set_panel_origin_y(&mut self, origin_y: i32) {
        self.panel_origin_y = origin_y;
    }

    pub fn set_panel_output(&mut self, output: impl Into<String>) {
        self.switch_panel_output(Some(output.into()));
    }

    fn switch_panel_output(&mut self, output: Option<String>) {
        if self.panel_output == output {
            return;
        }
        let previous_output = std::mem::replace(&mut self.panel_output, output);
        if let Some(current) = self.plugin_taskbar_host.take() {
            let runtime = current.application().shared_runtime();
            let next = self
                .plugin_taskbar_hosts
                .remove(&self.panel_output)
                .or_else(|| {
                    let (clock, _) = panel_clock_text();
                    let (data, images) = self.taskbar_plugin_render_data(&clock);
                    let runtime_surface_id = format!(
                        "taskbar-output:{}",
                        self.panel_output.as_deref().unwrap_or("default")
                    );
                    match crate::plugin_panel::PluginPanelApplication::bundled_with_shared_runtime(
                        crate::plugin_panel::taskbar_manifest(),
                        "main.js",
                        data,
                        &runtime_surface_id,
                        runtime,
                    ) {
                        Ok(mut application) => {
                            application.sync_images(images);
                            Some(nickel_ui::UiHost::new(application, 1920, 56))
                        }
                        Err(error) => {
                            tracing::error!(%error, "taskbar plugin failed on an output");
                            let _ = self
                                .plugin_registry
                                .mark_failed(&crate::plugin_panel::taskbar_manifest().id, error);
                            None
                        }
                    }
                });
            if next.is_some() {
                if self.plugin_taskbar_hosts.len() >= 32 {
                    for host in self.plugin_taskbar_hosts.values() {
                        let _ = host.application().retire_surface();
                    }
                    self.plugin_taskbar_hosts.clear();
                }
                self.plugin_taskbar_hosts.insert(previous_output, current);
                self.plugin_taskbar_host = next;
            } else {
                self.plugin_taskbar_hosts.clear();
                self.plugin_taskbar_memory.clear();
                self.panel_pet_deadline = None;
            }
        }
        self.panel_deadline = if self.plugin_taskbar_host.is_some() {
            Some(Instant::now() + taskbar::duration_until_next_minute())
        } else {
            None
        };
    }

    /// Render a concrete output without transferring popover/input ownership.
    pub fn panel_scene_for_output(
        &mut self,
        output: Option<&str>,
        width: u32,
        height: u32,
    ) -> Vec<PaintCommand> {
        let input_output = self.panel_output.clone();
        let input_change_token = self.panel_change_token;
        self.switch_panel_output(output.map(str::to_owned));
        let scene = self.panel_scene(width, height);
        self.switch_panel_output(input_output);
        self.panel_change_token = input_change_token;
        scene
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn popover_anchor(&self, preferred: AnchorSide) -> Option<(ShellRole, ShellPopoverAnchor)> {
        let anchor = self.pending_popover_anchor.as_ref()?;
        let visible = match anchor.role {
            ShellRole::ControlCenter => self.control_visible,
            ShellRole::ProjectMenu => self.codex_project_menu_visible,
            _ => false,
        };
        if !visible {
            return None;
        }
        let bounds = Geometry {
            x: anchor.bounds.origin.x.floor() as i32,
            y: anchor.bounds.origin.y.floor() as i32,
            width: anchor.bounds.size.width.ceil().max(1.0) as i32,
            height: anchor.bounds.size.height.ceil().max(1.0) as i32,
        };
        Some((
            anchor.role,
            ShellPopoverAnchor {
                control: anchor.control.clone(),
                output: anchor.output.clone(),
                bounds,
                preferred,
            },
        ))
    }

    fn panel_windows(&self) -> Vec<OpenWindow> {
        self.panel_window_iter().cloned().collect()
    }

    fn panel_window_iter(&self) -> impl Iterator<Item = &OpenWindow> {
        self.windows.iter().filter(|window| {
            window_belongs_to_panel(
                self.all_windows_on_every_bar,
                self.panel_output.as_deref(),
                window.state.output.as_deref(),
            )
        })
    }

    #[cfg(test)]
    pub(crate) fn taskbar_has_application(&self, application_id: &str) -> bool {
        self.launcher
            .taskbar_applications(&self.windows)
            .iter()
            .any(|application| {
                application
                    .application_id
                    .as_ref()
                    .is_some_and(|id| id.as_str() == application_id)
                    && !application.windows.is_empty()
            })
    }

    pub fn primary_output_name(&self) -> Option<String> {
        self.window_feed.primary_output()
    }

    pub fn panel_pointer_left(&mut self) -> bool {
        if self.panel_hover.is_none() || self.panel_hover_output != self.panel_output {
            return false;
        }
        self.panel_hover = None;
        self.panel_hover_output = None;
        self.preview_pending = None;
        if self.preview_group.is_some() && !self.preview_pointer_inside {
            self.preview_leave_deadline = Some(Instant::now() + PREVIEW_LEAVE_DELAY);
        }
        true
    }

    pub fn preview_controller(&mut self, action: ControllerAction) -> bool {
        if self.window_menu.is_some() || self.application_menu_target.is_some() {
            return self.window_menu_host_controller(action);
        }
        if self.task_switcher_group.is_some() && self.preview_plugin_active() {
            use nickel_core::hotkeys::HotkeyAction;
            return match action {
                ControllerAction::Left | ControllerAction::Up => {
                    self.apply_task_switch_action(HotkeyAction::SwitchPrevious)
                }
                ControllerAction::Right | ControllerAction::Down => {
                    self.apply_task_switch_action(HotkeyAction::SwitchNext)
                }
                ControllerAction::Confirm => {
                    self.apply_task_switch_action(HotkeyAction::CommitSwitch)
                }
                ControllerAction::Cancel => {
                    self.apply_task_switch_action(HotkeyAction::CancelSwitch)
                }
                _ => false,
            };
        }
        if self.preview_plugin_active() {
            if action == ControllerAction::Cancel {
                self.close_window_preview();
                return true;
            }
            let Some(size) = self.preview_plugin_size() else {
                return false;
            };
            if self
                .preview_plugin_host_ref()
                .is_some_and(|host| host.inspect().controller_target.is_none())
            {
                self.preview_plugin_event(
                    HostEvent::Controller(ControllerAction::Right),
                    size,
                    None,
                );
            }
            return self
                .preview_plugin_event(HostEvent::Controller(action), size, None)
                .changed;
        }
        false
    }

    pub fn panel_pointer_entered(&mut self) -> bool {
        self.preview_leave_deadline.take().is_some()
    }

    pub fn preview_pointer_entered(&mut self, entered: bool) -> bool {
        if self.preview_pointer_inside == entered {
            return false;
        }
        self.preview_pointer_inside = entered;
        if entered {
            self.preview_leave_deadline = None;
        } else {
            self.preview_leave_deadline = Some(Instant::now() + PREVIEW_LEAVE_DELAY);
            if self.preview_hovered.take().is_some() {
                let _ = self.send_session_command(
                    "clear-window-highlight",
                    ShellCommand::ClearWindowHighlight,
                );
            }
        }
        true
    }

    pub fn preview_pointer_moved(&mut self, x: f32, y: f32) -> bool {
        if self.preview_plugin_active() {
            let Some(size) = self.preview_plugin_size() else {
                return false;
            };
            let event_changed = self
                .preview_plugin_event(
                    HostEvent::Ui(UiEvent::PointerMoved(Point { x, y })),
                    size,
                    None,
                )
                .changed;
            let group = self.preview_plugin_group();
            let hovered = group.and_then(|group| {
                let limit = if self.task_switcher_group.is_some() {
                    5
                } else {
                    12
                };
                group.windows.iter().take(limit).find_map(|window| {
                    let bounds = self.preview_plugin_bounds(PreviewAction::Activate(window.id))?;
                    (x >= bounds.origin.x
                        && y >= bounds.origin.y
                        && x < bounds.origin.x + bounds.size.width
                        && y < bounds.origin.y + bounds.size.height)
                        .then_some(window.id)
                })
            });
            if hovered == self.preview_hovered {
                return event_changed;
            }
            self.preview_hovered = hovered;
            let command = hovered.map_or(
                ShellCommand::ClearWindowHighlight,
                ShellCommand::HighlightWindow,
            );
            let _ = self.send_session_command("highlight-preview-window", command);
            return true;
        }
        false
    }

    pub fn preview_click(&mut self, x: f32, y: f32, right_click: bool) -> bool {
        if self.preview_plugin_active() {
            let Some(size) = self.preview_plugin_size() else {
                return false;
            };
            let point = Point { x, y };
            if right_click {
                return self
                    .preview_plugin_event(HostEvent::Ui(UiEvent::PointerContext(point)), size, None)
                    .changed;
            }
            let pressed = self
                .preview_plugin_event(HostEvent::Ui(UiEvent::PointerPressed(point)), size, None)
                .changed;
            let released = self
                .preview_plugin_event(HostEvent::Ui(UiEvent::PointerReleased(point)), size, None)
                .changed;
            return pressed || released;
        }
        false
    }

    pub fn preview_host_input(
        &mut self,
        input: nickel_input::InputEvent,
    ) -> nickel_ui::HostEventOutcome {
        if self.preview_plugin_active() {
            let Some(host) = self.preview_plugin_host_ref() else {
                return Default::default();
            };
            let (ingress, authority) =
                internal_normalized_ingress(input, None, "window-preview", host.inspect(), None);
            return self.preview_host_event_authorized(ingress, Some(authority));
        }
        Default::default()
    }

    pub(crate) fn preview_host_event_authorized(
        &mut self,
        ingress: HostEvent,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> nickel_ui::HostEventOutcome {
        if self.preview_plugin_active() {
            let Some(size) = self.preview_plugin_size() else {
                return Default::default();
            };
            return self.preview_plugin_event(ingress, size, authority);
        }
        Default::default()
    }

    fn apply_preview_action(&mut self, action: PreviewAction) {
        match action {
            PreviewAction::Activate(window) => {
                self.send_window_action(window, WindowAction::Activate);
                if self.task_switcher_group.is_some() {
                    self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::CancelSwitch);
                } else {
                    self.close_window_preview();
                }
            }
            PreviewAction::Close(window) => {
                self.send_window_action(window, WindowAction::Close);
            }
            PreviewAction::OpenMenu(window) => {
                if self.plugin_taskbar_host.is_none() {
                    return;
                }
                let x = self
                    .preview_group
                    .and_then(|index| {
                        let groups = self.panel_groups();
                        let group = groups.get(index)?;
                        let (width, _) = preview_dimensions(group.windows.len().min(12));
                        let preview_origin = self.preview_origin_x(index, width);
                        let card = self.preview_plugin_bounds(PreviewAction::Activate(window))?;
                        Some(preview_origin + card.origin.x.round() as i32)
                    })
                    .unwrap_or(self.panel_origin_x);
                self.application_menu_target = None;
                self.clear_application_menu_plugin_host();
                self.plugin_taskbar_menu_memory = 0;
                self.window_menu_generation = self.window_menu_generation.saturating_add(1);
                self.window_menu = Some(window);
                self.window_menu_snapshot = self
                    .windows
                    .iter()
                    .find(|candidate| candidate.id == window)
                    .cloned();
                self.clear_window_menu_plugin_host();
                self.window_menu_anchor_x = Some(x);
                self.window_menu_anchor_y = Some(self.panel_origin_y);
                let _ = self.send_session_command(
                    "show-context-menu",
                    ShellCommand::ShowContextMenu {
                        x,
                        y: self.panel_origin_y,
                        width: MENU_WIDTH as i32,
                        height: self.window_context_menu_height(),
                    },
                );
                #[cfg(target_os = "linux")]
                let _ =
                    self.send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
            }
        }
    }

    pub fn preview_key(&mut self, key: Option<KeyCode>) -> bool {
        if self.window_menu.is_some() || self.application_menu_target.is_some() {
            return self.window_menu_host_key(key);
        }
        if self.task_switcher_group.is_some() && self.preview_plugin_active() {
            use nickel_core::hotkeys::HotkeyAction;
            return match key {
                Some(KeyCode::Escape) => self.apply_task_switch_action(HotkeyAction::CancelSwitch),
                Some(KeyCode::ArrowLeft | KeyCode::ArrowUp) => {
                    self.apply_task_switch_action(HotkeyAction::SwitchPrevious)
                }
                Some(KeyCode::ArrowRight | KeyCode::ArrowDown | KeyCode::Tab) => {
                    self.apply_task_switch_action(HotkeyAction::SwitchNext)
                }
                Some(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                    self.apply_task_switch_action(HotkeyAction::CommitSwitch)
                }
                Some(KeyCode::Delete) => {
                    let Some(window) = self.task_switcher.selected().copied() else {
                        return false;
                    };
                    if self.preview_plugin_action_allowed(PreviewAction::Close(window)) {
                        self.apply_preview_action(PreviewAction::Close(window));
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            };
        }
        if self.preview_plugin_active() {
            let Some(size) = self.preview_plugin_size() else {
                return false;
            };
            match key {
                Some(KeyCode::Escape) => {
                    self.close_window_preview();
                    #[cfg(target_os = "linux")]
                    let _ = self.send_session_command(
                        "restore-application-focus",
                        ShellCommand::RestoreApplicationFocus,
                    );
                    return true;
                }
                Some(KeyCode::ArrowLeft | KeyCode::ArrowUp) => {
                    return self
                        .preview_plugin_event(
                            HostEvent::Controller(ControllerAction::Left),
                            size,
                            None,
                        )
                        .changed;
                }
                Some(KeyCode::ArrowRight | KeyCode::ArrowDown | KeyCode::Tab) => {
                    return self
                        .preview_plugin_event(
                            HostEvent::Controller(ControllerAction::Right),
                            size,
                            None,
                        )
                        .changed;
                }
                Some(KeyCode::Delete) => {
                    let Some(window) = self.preview_plugin_selected_window() else {
                        return false;
                    };
                    if self.preview_plugin_action_allowed(PreviewAction::Close(window)) {
                        self.apply_preview_action(PreviewAction::Close(window));
                        return true;
                    }
                    return false;
                }
                Some(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                    return self
                        .preview_plugin_event(
                            HostEvent::Controller(ControllerAction::Confirm),
                            size,
                            None,
                        )
                        .changed;
                }
                _ => return false,
            }
        }
        false
    }

    fn apply_window_menu_action(&mut self, action: MenuAction) {
        if !matches!(
            action,
            MenuAction::ShowWorkspaces | MenuAction::ShowDisplays | MenuAction::Back
        ) {
            let Some(captured) = self.window_menu_snapshot.as_ref() else {
                return;
            };
            let Some(current) = self.windows.iter().find(|window| window.id == captured.id) else {
                return;
            };
            if !window_menu_action_is_current(captured, current, &action) {
                return;
            }
        }
        let dispatched = match action {
            MenuAction::ShowWorkspaces | MenuAction::ShowDisplays | MenuAction::Back => return,
            MenuAction::Activate(window) => {
                self.try_send_window_action(window, WindowAction::Activate)
            }
            MenuAction::Close(window) => self.try_send_window_action(window, WindowAction::Close),
            MenuAction::MaximizeRestore(window) => {
                self.try_send_window_action(window, WindowAction::Maximize)
            }
            MenuAction::Minimize(window) => {
                self.try_send_window_action(window, WindowAction::Minimize)
            }
            MenuAction::FullscreenRestore(window) => {
                self.try_send_window_action(window, WindowAction::Fullscreen)
            }
            MenuAction::SnapLeading(window) => {
                self.try_send_window_action(window, WindowAction::SnapLeading)
            }
            MenuAction::SnapTrailing(window) => {
                self.try_send_window_action(window, WindowAction::SnapTrailing)
            }
            MenuAction::MoveToWorkspace(window, workspace) => self.send_session_command(
                "move-window-to-workspace",
                ShellCommand::MoveWindowToWorkspace { window, workspace },
            ),
            MenuAction::MoveToDisplay(window, output) => self.send_session_command(
                "move-window-to-display",
                ShellCommand::MoveWindowToDisplay { window, output },
            ),
        };
        if dispatched {
            self.dismiss_window_menu();
        }
    }

    fn apply_application_menu_action(&mut self, action: ApplicationMenuAction) {
        match action {
            ApplicationMenuAction::TogglePin(application) => {
                let Some(target) = self.application_menu_target.as_ref() else {
                    return;
                };
                let canonical_item_available = target
                    .application_id
                    .as_ref()
                    .is_some_and(|id| self.launcher.is_pinned(id.as_str()));
                if target.application_id.as_ref() != Some(&application)
                    || !target.survives(&self.windows, canonical_item_available)
                {
                    return;
                }
                self.apply_launcher_action(LauncherAction::TogglePin(
                    application.as_str().to_owned(),
                ));
                self.dismiss_window_menu();
            }
            ApplicationMenuAction::CloseAll => {
                let Some(target) = self.application_menu_target.as_ref() else {
                    return;
                };
                let targets = validated_application_close_targets(target, &self.windows);
                let mut dispatched = false;
                for window in targets {
                    dispatched |= self.try_send_window_action(window, WindowAction::Close);
                }
                if dispatched {
                    self.dismiss_window_menu();
                }
            }
        }
    }

    fn application_menu_plugin_event(
        &mut self,
        event: HostEvent,
        width: u32,
        height: u32,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> bool {
        if self.application_menu_plugin_host.is_none() {
            let _ = self.application_menu_scene();
        }
        let Some(host) = self.application_menu_plugin_host.as_mut() else {
            return false;
        };
        let outcome = match step_plugin_host(
            host,
            None,
            HostBatch {
                surface_size: Some((width, height)),
                events: vec![event],
                normalized_authorities: authority.into_iter().collect(),
                ..HostBatch::default()
            },
        ) {
            Ok((outcome, _)) => outcome,
            Err(error) => {
                self.fail_taskbar_plugin_runtime(error);
                return true;
            }
        };
        let effects = host.application_mut().take_effects();
        let requested = !effects.is_empty();
        let changed = outcome.changed | self.apply_plugin_effects(effects);
        if requested {
            self.close_window_preview();
        }
        changed
    }

    fn window_menu_plugin_event(
        &mut self,
        event: HostEvent,
        width: u32,
        height: u32,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> bool {
        if self.window_menu_plugin_host.is_none() {
            let _ = self.window_menu_scene();
        }
        let Some(host) = self.window_menu_plugin_host.as_mut() else {
            return false;
        };
        let outcome = match step_plugin_host(
            host,
            None,
            HostBatch {
                surface_size: Some((width, height)),
                events: vec![event],
                normalized_authorities: authority.into_iter().collect(),
                ..HostBatch::default()
            },
        ) {
            Ok((outcome, _)) => outcome,
            Err(error) => {
                self.fail_taskbar_plugin_runtime(error);
                return true;
            }
        };
        let effects = host.application_mut().take_effects();
        outcome.changed | self.apply_plugin_effects(effects)
    }

    pub fn window_menu_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        if !self.surface_visible(SurfaceRole::WindowContextMenu) {
            return false;
        }
        let recipient = if self.application_menu_target.is_some() {
            if self.application_menu_plugin_host.is_none() {
                let _ = self.application_menu_scene();
            }
            self.application_menu_plugin_host
                .as_ref()
                .map(|host| host.inspect())
        } else {
            if self.window_menu_plugin_host.is_none() {
                let _ = self.window_menu_scene();
            }
            self.window_menu_plugin_host
                .as_ref()
                .map(|host| host.inspect())
        };
        let Some(recipient) = recipient else {
            return false;
        };
        let (event, authority) =
            internal_normalized_ingress(input, None, "window-menu", recipient, None);
        self.window_menu_host_event_authorized(event, width, height, Some(authority))
    }

    pub(crate) fn window_menu_host_event(
        &mut self,
        event: HostEvent,
        width: u32,
        height: u32,
    ) -> bool {
        self.window_menu_host_event_authorized(event, width, height, None)
    }

    pub(crate) fn window_menu_host_event_authorized(
        &mut self,
        event: HostEvent,
        width: u32,
        height: u32,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> bool {
        if !self.surface_visible(SurfaceRole::WindowContextMenu) {
            return false;
        }
        if self.application_menu_target.is_some() {
            self.application_menu_plugin_event(event, width, height, authority)
        } else {
            self.window_menu_plugin_event(event, width, height, authority)
        }
    }

    pub fn window_menu_host_key(&mut self, key: Option<KeyCode>) -> bool {
        if !self.surface_visible(SurfaceRole::WindowContextMenu) {
            return false;
        }
        let event = match key {
            Some(KeyCode::Escape) => HostEvent::Shortcut(Shortcut::Escape),
            Some(KeyCode::ArrowUp | KeyCode::ArrowLeft) => {
                HostEvent::Controller(ControllerAction::Up)
            }
            Some(KeyCode::ArrowDown | KeyCode::ArrowRight | KeyCode::Tab) => {
                HostEvent::Controller(ControllerAction::Down)
            }
            Some(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                HostEvent::Controller(ControllerAction::Confirm)
            }
            _ => return false,
        };
        let width = MENU_WIDTH.ceil() as u32;
        let height = self.window_context_menu_height().max(1) as u32;
        let changed = if self.application_menu_target.is_some() {
            self.application_menu_plugin_event(event, width, height, None)
        } else {
            self.window_menu_plugin_event(event, width, height, None)
        };
        if key == Some(KeyCode::Escape) {
            self.dismiss_window_menu();
        }
        changed
    }

    pub fn window_menu_host_controller(&mut self, action: ControllerAction) -> bool {
        if !self.surface_visible(SurfaceRole::WindowContextMenu) {
            return false;
        }
        let event = HostEvent::Controller(action);
        let width = MENU_WIDTH.ceil() as u32;
        let height = self.window_context_menu_height().max(1) as u32;
        let changed = if self.application_menu_target.is_some() {
            self.application_menu_plugin_event(event, width, height, None)
        } else {
            self.window_menu_plugin_event(event, width, height, None)
        };
        if action == ControllerAction::Cancel {
            self.dismiss_window_menu();
        }
        changed
    }

    pub fn sync_transient_overlays(&mut self) {
        if let Some(group) = &self.task_switcher_group {
            let windows = group
                .windows
                .iter()
                .take(5)
                .map(|window| window.id)
                .collect::<Vec<_>>();
            let (width, height) = task_switcher_dimensions(windows.len());
            #[cfg(any(target_os = "windows", test))]
            let thumbnail_bounds = self.preview_thumbnail_bounds(&windows).unwrap_or_default();
            let _ = self.send_session_command(
                "show-task-switcher",
                ShellCommand::ShowTaskSwitcher {
                    width: width as i32,
                    height: height as i32,
                    windows,
                    #[cfg(any(target_os = "windows", test))]
                    thumbnail_bounds,
                },
            );
        } else if let Some(index) = self.preview_group {
            let groups = self.panel_groups();
            if let Some(group) = groups.get(index) {
                let windows = group
                    .windows
                    .iter()
                    .take(12)
                    .map(|window| window.id)
                    .collect::<Vec<_>>();
                let (width, height) = preview_dimensions(windows.len());
                let x = self.preview_origin_x(index, width);
                #[cfg(any(target_os = "windows", test))]
                let thumbnail_bounds = self.preview_thumbnail_bounds(&windows).unwrap_or_default();
                let _ = self.send_session_command(
                    "show-preview",
                    ShellCommand::ShowPreview {
                        x,
                        y: self.panel_origin_y,
                        width: width as i32,
                        height: height as i32,
                        windows,
                        #[cfg(any(target_os = "windows", test))]
                        thumbnail_bounds,
                    },
                );
                if self.preview_focus_requested {
                    #[cfg(target_os = "linux")]
                    let _ = self.send_session_command("focus-preview", ShellCommand::FocusPreview);
                    self.preview_focus_requested = false;
                }
            }
        }
        if self.window_menu.is_some() || self.application_menu_target.is_some() {
            let x = self.window_menu_anchor_x.unwrap_or(self.panel_origin_x);
            let y = self.window_menu_anchor_y.unwrap_or(self.panel_origin_y);
            let _ = self.send_session_command(
                "show-context-menu",
                ShellCommand::ShowContextMenu {
                    x,
                    y,
                    width: MENU_WIDTH as i32,
                    height: self.window_context_menu_height(),
                },
            );
        }
    }

    fn preview_origin_x(&self, index: usize, width: u32) -> i32 {
        let control_bounds = self
            .plugin_taskbar_host
            .as_ref()
            .and_then(|host| taskbar_plugin_control_bounds(host, &format!("taskbar-item-{index}")))
            .unwrap_or_else(|| {
                Rect::new(
                    PANEL_ITEM_WIDTH + index as f32 * PANEL_ITEM_WIDTH,
                    0.0,
                    PANEL_ITEM_WIDTH,
                    PANEL_ITEM_WIDTH,
                )
            });
        TaskbarPreviewAnchor::new(self.panel_origin_x, control_bounds).preview_origin_x(width)
    }

    fn window_context_menu_height(&self) -> i32 {
        if let Some(target) = &self.application_menu_target {
            let pinned = target
                .application_id
                .as_ref()
                .is_some_and(|id| self.launcher.is_pinned(id.as_str()));
            let rows = application_menu_entries(target, pinned).len();
            return if self.plugin_taskbar_host.is_some() {
                let actions = self.slot_actions_for_item(
                    &crate::plugin_panel::taskbar_manifest().id,
                    "task-action",
                    target.application_id.as_ref().map(|id| id.as_str()),
                    4,
                );
                16 + 48 * (rows + actions.len()) as i32
            } else {
                menu_height_for_rows(rows) as i32
            };
        }
        let Some(window) = self.window_menu_snapshot.as_ref().or_else(|| {
            self.window_menu
                .and_then(|id| self.windows.iter().find(|candidate| candidate.id == id))
        }) else {
            return menu_height(&self.workspaces) as i32;
        };
        let outputs = self.window_feed.outputs();
        let rows = window_menu_max_rows(window, &self.workspaces, &outputs);
        if self.plugin_taskbar_host.is_some() {
            16 + 48 * rows.min(32) as i32
        } else {
            menu_height_for_rows(rows) as i32
        }
    }

    fn send_window_action(&self, window: crate::model::WindowId, action: WindowAction) {
        let _ = self.try_send_window_action(window, action);
    }

    fn try_send_window_action(&self, window: crate::model::WindowId, action: WindowAction) -> bool {
        self.send_session_command(
            "window-action",
            ShellCommand::WindowAction { window, action },
        )
    }

    fn open_window_preview(&mut self, index: usize) {
        if self.preview_plugin_host_ref().is_none() {
            return;
        }
        if self.preview_group == Some(index) {
            self.preview_pending = None;
            return;
        }
        let groups = self.panel_groups();
        if groups
            .get(index)
            .is_none_or(|group| group.windows.is_empty())
        {
            return;
        }
        self.preview_pending = None;
        self.preview_group = Some(index);
        self.preview_images.clear();
        self.preview_refresh_deadline = None;
        self.preview_hovered = None;
        self.window_menu = None;
        self.window_menu_snapshot = None;
        self.window_menu_anchor_x = None;
        self.window_menu_anchor_y = None;
        self.clear_window_menu_plugin_host();
        self.application_menu_target = None;
        self.clear_application_menu_plugin_host();
        self.plugin_taskbar_menu_memory = 0;
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn native_preview_cache_empty(&self) -> bool {
        self.preview_images.is_empty()
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn native_preview_windows(&mut self) -> Vec<crate::model::WindowId> {
        let Some(index) = self.preview_group else {
            return Vec::new();
        };
        self.panel_groups()
            .get(index)
            .map(|group| {
                group
                    .windows
                    .iter()
                    .take(PREVIEW_CACHE_CAPACITY)
                    .map(|window| window.id)
                    .collect()
            })
            .unwrap_or_default()
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn sync_native_preview_pixels<'a>(
        &mut self,
        mut frame_for: impl FnMut(crate::model::WindowId) -> Option<(u16, u16, &'a [u8])>,
    ) -> bool {
        let windows = self.native_preview_windows();
        let previous = self.preview_images.len();
        self.preview_images
            .retain(|id, _| windows.contains(id) && frame_for(*id).is_some());
        let mut changed = previous != self.preview_images.len();
        for id in windows {
            let Some((width, height, rgba)) = frame_for(id) else {
                continue;
            };
            if self.preview_images.get(&id).is_some_and(|image| {
                image.dimensions() == (u32::from(width), u32::from(height))
                    && image.as_raw().as_slice() == rgba
            }) {
                continue;
            }
            // The session retains its completed frame for other consumers. Copy
            // once into the existing UI owner, never through JSON or self-RPC.
            if let Some(image) =
                image::RgbaImage::from_raw(u32::from(width), u32::from(height), rgba.to_vec())
            {
                changed |= update_preview_image(&mut self.preview_images, id, image);
            }
        }
        changed
    }

    fn close_window_preview(&mut self) {
        self.preview_group = None;
        self.preview_pending = None;
        self.preview_focus_requested = false;
        self.preview_pointer_inside = false;
        self.preview_leave_deadline = None;
        self.preview_hovered = None;
        self.preview_images.clear();
        self.preview_refresh_deadline = None;
        self.clear_preview_plugin_payload();
        self.window_menu = None;
        self.window_menu_snapshot = None;
        self.window_menu_anchor_x = None;
        self.window_menu_anchor_y = None;
        self.clear_window_menu_plugin_host();
        self.application_menu_target = None;
        self.clear_application_menu_plugin_host();
        self.plugin_taskbar_menu_memory = 0;
        if self.plugin_taskbar_host.is_some() {
            self.record_taskbar_memory();
        }
        let _ =
            self.send_session_command("clear-window-highlight", ShellCommand::ClearWindowHighlight);
        let _ = self.send_session_command("hide-context-menu", ShellCommand::HideContextMenu);
    }

    fn clear_preview_plugin_payload(&mut self) {
        if let Some(host) = self.preview_plugin_host_mut() {
            let data_changed = match host
                .application_mut()
                .sync_data(&serde_json::json!({"windows": []}))
            {
                Ok(changed) => changed,
                Err(error) => {
                    self.fail_preview_plugin_runtime(error);
                    return;
                }
            };
            let images_changed = host.application_mut().sync_images(Default::default());
            let outcome = host.step(HostBatch {
                application_changed: data_changed || images_changed,
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            if let Some(error) = host.application_mut().take_runtime_failure() {
                self.fail_preview_plugin_runtime(error);
                return;
            }
            let _ = self.plugin_registry.record_memory(
                &crate::plugin_panel::window_preview_manifest().id,
                nickel_core::plugins::PluginMemory {
                    native_ui_bytes: Some(outcome.telemetry.retained_frame_bytes as u64),
                    ..Default::default()
                },
            );
        }
        self.maybe_publish_plugin_status();
    }

    fn dismiss_window_menu(&mut self) {
        let focused_menu = self.window_menu.is_some() || self.application_menu_target.is_some();
        self.close_window_preview();
        if focused_menu {
            #[cfg(target_os = "linux")]
            let _ = self.send_session_command(
                "restore-window-menu-focus",
                ShellCommand::RestoreApplicationFocus,
            );
        }
    }

    pub fn global_shortcut(&mut self, shortcut: platform::GlobalShortcut) -> bool {
        match shortcut {
            platform::GlobalShortcut::ReloadShellSettings => self.refresh_system(),
            platform::GlobalShortcut::SetPluginEnabled {
                id,
                enabled,
                observed_generation,
            } => {
                if observed_generation != self.plugin_activation_generation {
                    return false;
                }
                match self.set_plugin_enabled(&id, enabled) {
                    Ok(changed) => changed,
                    Err(error) => {
                        tracing::warn!(plugin = id, %error, "plugin activation failed");
                        true
                    }
                }
            }
            platform::GlobalShortcut::SetPluginSetting {
                id,
                key,
                value,
                observed_generation,
            } => {
                if observed_generation != self.plugin_activation_generation {
                    return false;
                }
                match self.set_plugin_setting(&id, &key, value) {
                    Ok(changed) => changed,
                    Err(error) => {
                        tracing::warn!(plugin = id, setting = key, %error, "plugin setting failed");
                        true
                    }
                }
            }
            platform::GlobalShortcut::ToggleLauncher => {
                self.apply_launcher_signal(!self.launcher_visible);
                true
            }
            platform::GlobalShortcut::ShowLauncher => {
                self.apply_launcher_signal(true);
                true
            }
            platform::GlobalShortcut::HideLauncher => {
                self.apply_launcher_signal(false);
                true
            }
            platform::GlobalShortcut::SwitchNext => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchNext)
            }
            platform::GlobalShortcut::SwitchPrevious => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchPrevious)
            }
            platform::GlobalShortcut::SwitchGroupNext => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchGroupNext)
            }
            platform::GlobalShortcut::SwitchGroupPrevious => self
                .apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::SwitchGroupPrevious),
            platform::GlobalShortcut::CommitSwitch => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::CommitSwitch)
            }
            platform::GlobalShortcut::CancelSwitch => {
                self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::CancelSwitch)
            }
            platform::GlobalShortcut::LockState { locked } => {
                #[cfg(target_os = "windows")]
                {
                    if locked && !platform::lock_workstation() {
                        tracing::warn!("native Windows workstation lock request failed");
                    }
                    true
                }
                #[cfg(not(target_os = "windows"))]
                {
                    self.locked = locked;
                    let application = self.lock_host.application_mut();
                    application.password.zeroize();
                    application.status = None;
                    if locked {
                        self.desktop_host
                            .application_mut()
                            .dismiss_context_menu(desktop::DesktopMenuDismissReason::FocusDeparted);
                        self.launcher_visible = false;
                        self.control_visible = false;
                        self.codex_project_menu_visible = false;
                        self.close_window_preview();
                    }
                    true
                }
            }
            platform::GlobalShortcut::ShowRun => self.set_run_visible(true),
            platform::GlobalShortcut::OpenFiles => self.launch_named_application("Nickel File"),
            platform::GlobalShortcut::OpenSettings => self.launch_settings(None),
            platform::GlobalShortcut::ShowControlCenter => {
                self.control_host.application_mut().show_control_center();
                self.set_control_visible(true);
                true
            }
            platform::GlobalShortcut::ShowNotifications => {
                if self.notification_plugin_host_ref().is_none()
                    && !self.trusted_notification_visible()
                {
                    return false;
                }
                let history = self.notification_feed.history();
                self.notification_host
                    .application_mut()
                    .sync_history(&history, self.palette);
                self.notification = history.first().cloned();
                self.notification_history_visible = true;
                true
            }
            platform::GlobalShortcut::ShowDesktop => {
                self.send_session_command("toggle-show-desktop", ShellCommand::ToggleShowDesktop)
            }
            platform::GlobalShortcut::ProjectDisplays => {
                self.control_host
                    .application_mut()
                    .show_projection_chooser();
                self.set_control_visible(true);
                self.sync_control_host(420, 320);
                self.step_control_host(HostBatch {
                    events: vec![HostEvent::Controller(ControllerAction::Down)],
                    ..HostBatch::default()
                });
                true
            }
            platform::GlobalShortcut::ShowWindowMenu => self.open_active_window_menu(),
            platform::GlobalShortcut::ConsumerControl(control) => {
                if !self.session_host.consumer_control(control) {
                    tracing::warn!(?control, "consumer action could not be queued");
                    return false;
                }
                // A clamped adjustment may produce no backend state transition.
                // Display only the already-observed value, never an optimistic increment.
                let at_limit = matches!(control,
                    nickel_session_protocol::ConsumerControl::VolumeUp if self.audio.volume_percent == 100)
                    || matches!(control,
                        nickel_session_protocol::ConsumerControl::VolumeDown if self.audio.volume_percent == 0);
                if at_limit && self.audio_status_observed && self.audio.available {
                    self.volume_osd_until = Some(Instant::now() + Duration::from_millis(1500));
                    true
                } else {
                    false
                }
            }
            platform::GlobalShortcut::AudioChanged {
                available,
                volume_percent,
                muted,
                output_name,
            } => {
                self.audio.available = available;
                self.audio.volume_percent = volume_percent;
                self.audio.muted = muted;
                if let Some(name) = output_name {
                    for device in &mut self.audio.devices {
                        device.is_default = device.name == name;
                    }
                    if !self.audio.devices.iter().any(|device| device.name == name) {
                        self.audio.devices.push(platform::AudioDeviceStatus {
                            id: name.clone(),
                            name,
                            is_default: true,
                        });
                    }
                }
                self.volume_osd_until =
                    available.then(|| Instant::now() + Duration::from_millis(1500));
                true
            }
            platform::GlobalShortcut::Screenshot(platform::ScreenshotAction::ActiveWindow) => {
                #[cfg(target_os = "linux")]
                let result = self.request_active_window_capture(false);
                #[cfg(not(target_os = "linux"))]
                let result = platform::capture_active_window();
                if let Err(error) = result {
                    tracing::warn!(%error, "failed to copy active window screenshot");
                    self.screenshot.show_error(error);
                    self.set_screenshot_focus(true);
                    return true;
                }
                cfg!(target_os = "linux")
            }
            platform::GlobalShortcut::Screenshot(
                platform::ScreenshotAction::ActiveWindowToFile,
            ) => {
                #[cfg(target_os = "linux")]
                let result = self.request_active_window_capture(true);
                #[cfg(not(target_os = "linux"))]
                let result = platform::capture_active_window_to_file();
                if let Err(error) = result {
                    tracing::warn!(%error, "failed to capture active window to a temporary file");
                    self.screenshot.show_error(error);
                    self.set_screenshot_focus(true);
                    return true;
                }
                cfg!(target_os = "linux")
            }
            platform::GlobalShortcut::Screenshot(platform::ScreenshotAction::InteractiveRegion) => {
                #[cfg(target_os = "linux")]
                {
                    self.active_window_capture = None;
                }
                let was_visible = self.screenshot.visible();
                self.screenshot.request_capture();
                if was_visible {
                    self.set_screenshot_focus(false);
                }
                true
            }
            platform::GlobalShortcut::Screenshot(
                platform::ScreenshotAction::InteractiveRegionToFile,
            ) => {
                #[cfg(target_os = "linux")]
                {
                    self.active_window_capture = None;
                }
                let was_visible = self.screenshot.visible();
                self.screenshot.request_capture_to_file();
                if was_visible {
                    self.set_screenshot_focus(false);
                }
                true
            }
        }
    }

    fn apply_task_switch_action(&mut self, action: nickel_core::hotkeys::HotkeyAction) -> bool {
        if self.task_switcher.session().is_none()
            && matches!(
                action,
                nickel_core::hotkeys::HotkeyAction::SwitchNext
                    | nickel_core::hotkeys::HotkeyAction::SwitchPrevious
                    | nickel_core::hotkeys::HotkeyAction::SwitchGroupNext
                    | nickel_core::hotkeys::HotkeyAction::SwitchGroupPrevious
            )
            && let FeedState::Ready(windows) = self.window_feed.snapshot(&self.launcher)
        {
            // Shortcut dispatch can run between periodic feed refreshes. Starting a switch from a
            // fresh snapshot keeps the active application and MRU order aligned with Win32 now.
            self.windows = windows;
        }
        let windows = self
            .windows
            .iter()
            .map(|window| SwitchWindow {
                id: window.id,
                application_id: window
                    .application_id
                    .as_ref()
                    .map(|id| id.as_str().to_owned())
                    .unwrap_or_else(|| window.title.clone()),
                active: window.active,
            })
            .collect::<Vec<_>>();
        let effects = self.task_switcher.apply(action, &windows);
        let changed = !effects.is_empty();
        for effect in effects {
            match effect {
                TaskSwitchEffect::ActivateWindow(window) => {
                    let _ = self.send_session_command(
                        "task-switcher-activate",
                        ShellCommand::WindowAction {
                            window,
                            action: WindowAction::Activate,
                        },
                    );
                }
                TaskSwitchEffect::HideFlip { .. } => {
                    #[cfg(target_os = "windows")]
                    let _ = self.send_session_command(
                        "task-switcher-peek-clear",
                        ShellCommand::ShowTaskSwitcherPeek { window: None },
                    );
                    self.task_switcher_group = None;
                    self.preview_images.clear();
                    self.clear_preview_plugin_payload();
                }
                TaskSwitchEffect::SelectPreview(_) => {
                    #[cfg(target_os = "windows")]
                    let _ = self.send_session_command(
                        "task-switcher-peek-clear",
                        ShellCommand::ShowTaskSwitcherPeek { window: None },
                    );
                }
                TaskSwitchEffect::ShowFlip { .. } | TaskSwitchEffect::RequestPreviews(_) => {}
            }
        }
        if self.task_switcher.session().is_some() {
            self.rebuild_task_switcher_preview();
        }
        changed
    }

    fn rebuild_task_switcher_preview(&mut self) {
        let visible = self.task_switcher.visible_range(5);
        let ids = self.task_switcher.candidates()[visible].to_vec();
        let windows = ids
            .iter()
            .filter_map(|id| self.windows.iter().find(|window| window.id == *id).cloned())
            .collect::<Vec<_>>();
        self.preview_images.clear();
        for window in &windows {
            if let Some(preview) = self.window_feed.preview(window.id) {
                self.preview_images
                    .insert(window.id, Arc::new(preview.image));
            }
        }
        self.preview_hovered = self.task_switcher.selected().copied();
        self.task_switcher_group = Some(WindowGroup {
            application_id: None,
            application_name: "Open windows".into(),
            windows,
        });
        // Keep the host alive across switch steps. Recreating it resets its
        // HostChangeToken, which can equal the token of the frame already
        // presented by Winit. In that case present_host_frame correctly
        // deduplicates the token but leaves the old selection on screen.
        // window_preview_scene synchronizes the retained host with this group
        // and advances its token for the new selection.
    }

    /// Pointer ownership retained by any coordinator-owned host or parked viewport.
    pub(crate) fn pointer_interaction_active(&self) -> bool {
        self.desktop_host.pointer_interaction_active()
            || self.desktop_viewports.values().any(|viewport| {
                viewport.host.pointer_interaction_active()
                    || viewport.overlay_pointer_capture.is_some()
            })
            || self.desktop_overlay_pointer_capture.is_some()
            || self
                .plugin_surface_hosts
                .values()
                .any(|(_, host)| host.pointer_interaction_active())
            || self.lock_host.pointer_interaction_active()
            || self
                .plugin_taskbar_host
                .as_ref()
                .is_some_and(|host| host.pointer_interaction_active())
            || self
                .plugin_taskbar_hosts
                .values()
                .any(|host| host.pointer_interaction_active())
            || self
                .window_menu_plugin_host
                .as_ref()
                .is_some_and(|host| host.pointer_interaction_active())
            || self
                .application_menu_plugin_host
                .as_ref()
                .is_some_and(|host| host.pointer_interaction_active())
            || self.notification_host.pointer_interaction_active()
            || self
                .notification_plugin_host_ref()
                .is_some_and(|host| host.pointer_interaction_active())
            || self.control_host.pointer_interaction_active()
            || self
                .control_plugin_host_ref()
                .is_some_and(|host| host.pointer_interaction_active())
            || self.keyboard_host.pointer_interaction_active()
            || self.keyboard_resize.is_some()
            || !self.keyboard_gesture_leases.is_empty()
            || self.screenshot.pointer_interaction_active()
    }

    /// Requests a launcher toggle initiated by shell-owned input such as a controller.
    ///
    /// Linux compositor shortcut notifications use [`Self::global_shortcut`] after the
    /// compositor has already changed visibility. Shell-originated input must instead send the
    /// visibility request to the compositor before mirroring the resulting state.
    pub fn request_launcher_toggle(&mut self) -> bool {
        let visible = !self.launcher_visible;
        if visible && self.launcher_host_ref().is_none() {
            return false;
        }
        let command = if visible {
            ShellCommand::ShowFromController
        } else {
            ShellCommand::Hide
        };
        if !self.send_session_command("controller-launcher-visibility", command) {
            self.launcher_status = Some("Nickel could not update the launcher.".to_owned());
            return false;
        }
        self.clear_launcher_visibility_error();
        self.apply_session_launcher_visibility(visible);
        platform::launcher_visibility_applied(visible);
        self.launcher_visible == visible
    }

    pub fn capture_screenshot(&mut self) -> bool {
        match self
            .session_host
            .capture_desktop(self.screenshot_output.as_deref())
        {
            #[cfg(target_os = "linux")]
            crate::session_host::DesktopCapturePoll::Pending => {
                self.screenshot_capture_pending = true;
                false
            }
            crate::session_host::DesktopCapturePoll::Ready(result) => match result {
                Ok(capture) => {
                    self.screenshot_capture_pending = false;
                    #[cfg(target_os = "linux")]
                    if let Some(active) = self.active_window_capture.take() {
                        self.screenshot_output = None;
                        let result = platform::crop_output_geometry(
                            capture.image,
                            active.output,
                            active.window,
                        )
                        .and_then(|image| {
                            if active.copy_path {
                                self.session_host.copy_image_path(&image).map(|_| ())
                            } else {
                                self.session_host.copy_image(image)
                            }
                        });
                        return match result {
                            Ok(()) => false,
                            Err(error) => {
                                tracing::warn!(%error, "failed to finish active window capture");
                                self.screenshot.show_error(error);
                                self.set_screenshot_focus(true);
                                true
                            }
                        };
                    }
                    self.screenshot.show(capture.image);
                    self.set_screenshot_focus(true);
                    true
                }
                Err(error) => {
                    self.screenshot_capture_pending = false;
                    self.screenshot_output = None;
                    #[cfg(target_os = "linux")]
                    {
                        self.active_window_capture = None;
                    }
                    tracing::warn!(%error, "failed to capture desktop");
                    self.screenshot.show_error(error);
                    self.set_screenshot_focus(true);
                    true
                }
            },
        }
    }

    #[cfg(test)]
    pub(crate) fn request_native_screenshot_fixture(&mut self) {
        self.screenshot.request_capture();
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn screenshot_host_event(
        &mut self,
        event: HostEvent,
        width: u32,
        height: u32,
    ) -> bool {
        self.screenshot_host_event_authorized(event, width, height, None)
    }

    #[cfg(any(test, target_os = "linux"))]
    pub(crate) fn screenshot_host_event_authorized(
        &mut self,
        event: HostEvent,
        width: u32,
        height: u32,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> bool {
        if !self.screenshot.visible() {
            return false;
        }
        let changed = self
            .screenshot
            .host_event_authorized(event, width, height, authority);
        if !self.screenshot.visible() {
            self.set_screenshot_focus(false);
        }
        changed
    }

    pub fn screenshot_pointer_moved(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        self.screenshot.queue_pointer_moved(x, y, width, height);
        false
    }

    pub fn screenshot_pointer_pressed(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        let was_visible = self.screenshot.visible();
        let handled = self.screenshot.pointer_pressed(x, y, width, height);
        if was_visible && !self.screenshot.visible() {
            self.set_screenshot_focus(false);
        }
        handled
    }

    pub fn screenshot_pointer_released(&mut self) -> bool {
        let was_visible = self.screenshot.visible();
        let handled = self.screenshot.pointer_released();
        // Toolbar buttons complete on release, including Save and Copy. Return
        // focus before the screenshot surface is unmapped so the OSK recipient
        // remains the application the capture was opened from.
        if was_visible && !self.screenshot.visible() {
            self.set_screenshot_focus(false);
        }
        handled
    }

    pub fn screenshot_key(&mut self, key: Option<KeyCode>) -> bool {
        if key == Some(KeyCode::Escape) && self.screenshot.visible() {
            let _ = self.screenshot.escape();
            self.set_screenshot_focus(false);
            true
        } else {
            false
        }
    }

    pub fn screenshot_controller(&mut self, action: ControllerAction) -> bool {
        if !self.screenshot.visible() {
            return false;
        }
        let changed = self.screenshot.controller_action(action);
        if !self.screenshot.visible() {
            self.set_screenshot_focus(false);
        }
        changed
    }

    fn set_screenshot_focus(&self, visible: bool) {
        #[cfg(target_os = "linux")]
        let _ = self.send_session_command(
            "screenshot-focus",
            if visible {
                ShellCommand::FocusScreenshot
            } else {
                ShellCommand::RestoreApplicationFocus
            },
        );
        #[cfg(not(target_os = "linux"))]
        let _ = visible;
    }

    fn set_launcher_visible(&mut self, visible: bool) {
        if visible && self.launcher_host_ref().is_none() {
            return;
        }
        if self.request_launcher_visibility(visible) {
            self.run_visible = false;
        }
    }

    pub(crate) fn can_show_launcher(&self) -> bool {
        self.launcher_host_ref().is_some() || (self.run_visible && self.run_host_ref().is_some())
    }

    fn request_launcher_visibility(&mut self, visible: bool) -> bool {
        if !self.send_session_command(
            "launcher-visibility",
            if visible {
                ShellCommand::Show
            } else {
                ShellCommand::Hide
            },
        ) {
            self.launcher_status = Some("Nickel could not update the launcher.".to_owned());
            return false;
        }
        self.clear_launcher_visibility_error();
        if self.session_host.stages_effects() {
            return true;
        }
        self.run_visible = false;
        self.apply_session_launcher_visibility(visible);
        platform::launcher_visibility_applied(visible);
        true
    }

    fn clear_launcher_visibility_error(&mut self) {
        if self.launcher_status.as_deref() == Some("Nickel could not update the launcher.") {
            self.launcher_status = None;
        }
    }

    fn set_run_visible(&mut self, visible: bool) -> bool {
        if visible && self.run_host_ref().is_none() {
            return false;
        }
        if !self.request_launcher_visibility(visible)
            || (!self.session_host.stages_effects() && self.launcher_visible != visible)
        {
            return false;
        }
        self.run_visible = visible;
        if visible {
            if let Some(host) = self.run_host_mut() {
                if let Err(error) = host
                    .application_mut()
                    .sync_data(&serde_json::json!({ "status": null }))
                {
                    self.fail_run_plugin_runtime(error);
                    return false;
                }
                host.step(HostBatch {
                    application_changed: true,
                    ..HostBatch::default()
                });
                if let Some(error) = host.application_mut().take_runtime_failure() {
                    self.fail_run_plugin_runtime(error);
                    return false;
                }
                if let Ok(field) =
                    host.query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
                {
                    let _ = host.request_focus(field.id);
                }
                return true;
            }
        }
        true
    }

    fn set_control_visible(&mut self, visible: bool) {
        if visible && !self.control_surface_available() {
            return;
        }
        #[cfg(target_os = "linux")]
        if !self.send_session_command(
            "control-center-focus",
            if visible {
                if self.control_plugin_active() {
                    ShellCommand::FocusPluginSurface {
                        key: crate::plugin_panel::control_center_surface_key(),
                    }
                } else {
                    ShellCommand::FocusControlCenter
                }
            } else {
                ShellCommand::RestoreApplicationFocus
            },
        ) {
            self.launcher_status = Some("Nickel could not update Quick Settings.".to_owned());
            return;
        }
        if self.session_host.stages_effects() {
            return;
        }
        self.apply_control_visibility(visible);
    }

    pub(crate) fn apply_control_visibility(&mut self, visible: bool) {
        self.control_visible = visible;
        if !visible {
            self.control_host.application_mut().show_control_center();
        }
    }

    fn apply_launcher_signal(&mut self, visible: bool) {
        #[cfg(target_os = "linux")]
        if visible && self.launcher_host_ref().is_none() && !self.run_visible {
            // The compositor handles Super before notifying the shell. Retire
            // the just-opened surface when its UI plugin is disabled.
            self.set_launcher_visible(false);
        } else {
            self.apply_session_launcher_visibility(visible);
        }
        #[cfg(not(target_os = "linux"))]
        self.set_launcher_visible(visible);
    }

    pub(crate) fn apply_session_launcher_visibility(&mut self, visible: bool) {
        self.launcher_visible = visible;
        if visible {
            self.control_visible = false;
            self.focus_launcher();
        } else {
            self.run_visible = false;
            self.launcher.clear();
            self.launcher.set_view(LauncherView::Favorites);
        }
    }

    pub fn focus_launcher(&mut self) -> bool {
        if !self.run_visible && self.launcher_host_ref().is_none() {
            return false;
        }
        if self.run_visible {
            if let Some(host) = self.run_host_mut() {
                let outcome = host.step(HostBatch {
                    window_focused: Some(true),
                    ..HostBatch::default()
                });
                if let Some(error) = host.application_mut().take_runtime_failure() {
                    self.fail_run_plugin_runtime(error);
                    return false;
                }
                return outcome.changed;
            }
            return false;
        }
        if self.launcher_host_ref().is_some() {
            let Some(application_changed) = self.sync_plugin_launcher() else {
                return false;
            };
            let host = self
                .launcher_host_mut()
                .expect("launcher plugin host exists");
            let mut changed = host
                .step(HostBatch {
                    application_changed,
                    window_focused: Some(true),
                    ..HostBatch::default()
                })
                .changed;
            if let Some(error) = host.application_mut().take_runtime_failure() {
                self.fail_launcher_plugin_runtime(error);
                return false;
            }
            if let Ok(search) =
                host.query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
            {
                changed |= host.request_focus(search.id).changed;
            }
            return changed;
        }
        false
    }

    pub fn control_click(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        if !self.control_surface_available() {
            return false;
        }
        if self.control_plugin_active() {
            let point = Point { x, y };
            let pressed = self.control_plugin_event(
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                (width, height),
                None,
                None,
            );
            let released = self.control_plugin_event(
                HostEvent::Ui(UiEvent::PointerReleased(point)),
                (width, height),
                None,
                None,
            );
            return pressed.changed || released.changed;
        }
        self.sync_control_host(width, height);
        let point = Point { x, y };
        self.step_control_host(HostBatch {
            events: vec![
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                HostEvent::Ui(UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        self.apply_control_effects();
        true
    }

    pub fn control_key(&mut self, key: Option<KeyCode>, width: u32, height: u32) -> bool {
        if !self.control_visible || !self.control_surface_available() {
            return false;
        }
        self.sync_control_host(width, height);
        let action = match key {
            Some(KeyCode::Escape) => {
                self.set_control_visible(false);
                return true;
            }
            Some(KeyCode::ArrowDown | KeyCode::Tab) => ControllerAction::Down,
            Some(KeyCode::ArrowRight) => ControllerAction::Right,
            Some(KeyCode::ArrowUp) => ControllerAction::Up,
            Some(KeyCode::ArrowLeft) => ControllerAction::Left,
            Some(KeyCode::Enter | KeyCode::NumpadEnter) => ControllerAction::Confirm,
            _ => return false,
        };
        if self.control_plugin_active() {
            return self
                .control_plugin_event(HostEvent::Controller(action), (width, height), None, None)
                .changed;
        }
        self.step_control_host(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        self.apply_control_effects();
        true
    }

    pub fn control_controller(
        &mut self,
        action: ControllerAction,
        width: u32,
        height: u32,
    ) -> bool {
        if !self.control_visible || !self.control_surface_available() {
            return false;
        }
        if self.control_plugin_active() {
            let changed = self
                .control_plugin_event(HostEvent::Controller(action), (width, height), None, None)
                .changed;
            let dismissed = action == ControllerAction::Cancel && self.control_visible;
            if dismissed {
                self.set_control_visible(false);
            }
            return changed || dismissed;
        }
        self.sync_control_host(width, height);
        let changed = self.step_control_host(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        self.apply_control_effects();
        let dismissed = action == ControllerAction::Cancel && self.control_visible;
        if dismissed {
            self.set_control_visible(false);
        }
        changed || dismissed
    }

    /// Focus already belongs to the destination; dismissal must not restore
    /// whichever application was active before this surface opened.
    pub(crate) fn dismiss_ephemeral_on_focus_loss(&mut self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Launcher => {
                if !self.launcher_visible {
                    return false;
                }
                // Focus already moved to the destination. Do not use the
                // explicit Hide command, which restores the pre-launcher
                // window as though the user had cancelled the launcher.
                self.apply_session_launcher_visibility(false);
                true
            }
            SurfaceRole::ControlCenter => {
                self.control_host.application_mut().show_control_center();
                std::mem::replace(&mut self.control_visible, false)
            }
            SurfaceRole::CodexProjectMenu => {
                std::mem::replace(&mut self.codex_project_menu_visible, false)
            }
            SurfaceRole::WindowContextMenu => {
                let visible = self.window_menu.is_some() || self.application_menu_target.is_some();
                if visible {
                    self.close_window_preview();
                }
                visible
            }
            _ => false,
        }
    }

    #[allow(dead_code)]
    pub fn hide_overlay(&mut self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Launcher if self.launcher_visible => {
                self.apply_session_launcher_visibility(false);
                let _ = self.send_session_command("hide-launcher", ShellCommand::Hide);
                true
            }
            SurfaceRole::ControlCenter if self.control_visible => {
                self.set_control_visible(false);
                true
            }
            SurfaceRole::CodexProjectMenu if self.codex_project_menu_visible => {
                self.codex_project_menu_visible = false;
                true
            }
            SurfaceRole::Screenshot if self.screenshot.visible() => {
                self.screenshot.hide();
                self.set_screenshot_focus(false);
                true
            }
            _ => false,
        }
    }

    pub fn scroll(&mut self, delta: f32) -> bool {
        if self.control_visible {
            self.sync_control_host(800, 600);
            self.step_control_host(HostBatch {
                events: vec![HostEvent::Ui(UiEvent::Scroll {
                    point: Point { x: 400.0, y: 300.0 },
                    delta_y: delta * 36.0,
                })],
                ..HostBatch::default()
            });
            self.apply_control_effects();
            return true;
        }
        false
    }

    fn launch_result(&mut self, index: usize) {
        let Some(application) = self.launcher.result_at(index).cloned() else {
            return;
        };
        self.launch_application(application);
    }

    fn launch_application_by_id(&mut self, id: &str) {
        let Some(application) = self
            .launcher
            .applications()
            .find(|application| application.id() == id)
            .cloned()
        else {
            return;
        };
        self.launch_application(application);
    }

    fn launch_named_application(&mut self, name: &str) -> bool {
        let Some(application) = self
            .launcher
            .applications()
            .find(|application| application.name() == name)
            .cloned()
        else {
            tracing::warn!(application = name, "shortcut application is unavailable");
            self.shortcut_action_status = Some(format!("{name} is unavailable."));
            self.set_launcher_visible(true);
            return false;
        };
        self.shortcut_action_status = None;
        self.launch_application(application);
        true
    }

    fn launch_settings(&mut self, screen: Option<&str>) -> bool {
        let sibling = std::env::current_exe().ok().map(|path| {
            path.with_file_name(if cfg!(target_os = "windows") {
                "nickel-settings.exe"
            } else {
                "nickel-settings"
            })
        });
        #[cfg(target_os = "windows")]
        if let Some(path) = sibling.as_ref().filter(|path| path.is_file()) {
            let mut command = std::process::Command::new(path);
            if let Some(screen) = screen {
                command.args(["--screen", screen]);
            }
            match command.spawn() {
                Ok(_) => {
                    self.launcher_status = None;
                    self.set_launcher_visible(false);
                }
                Err(error) => {
                    tracing::warn!(%error, "failed to launch Nickel Settings");
                    self.launcher_status =
                        Some(format!("Could not launch Nickel Settings: {error}"));
                }
            }
            return true;
        }
        let application = sibling
            .filter(|path| path.is_file())
            .and_then(|path| path.to_str().map(str::to_owned))
            .map(|program| {
                let mut command = vec![program];
                if let Some(screen) = screen {
                    command.extend(["--screen".into(), screen.into()]);
                }
                Application::new(
                    crate::settings_plugin_report::ID.into(),
                    "Nickel Settings".into(),
                    None,
                    None,
                    Some(command),
                )
            })
            .or_else(|| {
                self.launcher
                    .applications()
                    .find(|application| application.name() == "Nickel Settings")
                    .cloned()
                    .map(|application| {
                        if let (Some(screen), Some(command)) =
                            (screen, application.launch_command())
                        {
                            let mut command = command.to_vec();
                            command.extend(["--screen".into(), screen.into()]);
                            Application::new(
                                application.id().to_owned(),
                                application.name().to_owned(),
                                application.icon().map(str::to_owned),
                                application.icon_path().map(std::path::Path::to_owned),
                                Some(command),
                            )
                        } else {
                            application
                        }
                    })
            });
        let Some(application) = application else {
            return self.launch_named_application("Nickel Settings");
        };
        self.launch_application(application);
        true
    }

    fn open_active_window_menu(&mut self) -> bool {
        self.open_active_window_menu_at(self.panel_origin_x, self.panel_origin_y)
    }

    pub(crate) fn window_menu_generation(&self) -> Option<u64> {
        (self.window_menu.is_some() && self.window_menu_generation < u64::MAX)
            .then_some(self.window_menu_generation)
    }

    pub(crate) fn retire_window_menu(&mut self, generation: u64) -> bool {
        if self.window_menu_generation() != Some(generation) {
            return false;
        }
        self.close_window_preview();
        true
    }

    pub(crate) fn window_menu_geometry(&self) -> Option<(i32, i32, u32, u32)> {
        (self.window_menu.is_some() || self.application_menu_target.is_some()).then(|| {
            (
                self.window_menu_anchor_x.unwrap_or(self.panel_origin_x),
                self.window_menu_anchor_y.unwrap_or(self.panel_origin_y),
                MENU_WIDTH.ceil() as u32,
                self.window_context_menu_height().max(1) as u32,
            )
        })
    }

    pub(crate) fn preview_geometry(&mut self) -> Option<(i32, i32, u32, u32)> {
        let index = self.preview_group?;
        let groups = self.panel_groups();
        let group = groups.get(index)?;
        let (width, height) = preview_dimensions(group.windows.len().min(12));
        Some((
            self.preview_origin_x(index, width),
            self.panel_origin_y,
            width,
            height,
        ))
    }

    pub(crate) fn open_active_window_menu_at(&mut self, x: i32, y: i32) -> bool {
        let Some(id) = self
            .windows
            .iter()
            .find(|window| window.active)
            .map(|window| window.id.0)
        else {
            return false;
        };
        self.open_window_menu_at(id, x, y)
    }

    pub(crate) fn open_window_menu_at(&mut self, id: u64, x: i32, y: i32) -> bool {
        if self.plugin_taskbar_host.is_none() {
            return false;
        }
        let Some(snapshot) = self
            .windows
            .iter()
            .find(|window| window.id.0 == id)
            .cloned()
        else {
            return false;
        };
        self.window_menu_generation = self.window_menu_generation.saturating_add(1);
        self.window_menu = Some(snapshot.id);
        self.window_menu_snapshot = Some(snapshot);
        self.clear_window_menu_plugin_host();
        self.window_menu_anchor_x = Some(x);
        self.window_menu_anchor_y = Some(y);
        let sent = self.send_session_command(
            "show-context-menu",
            ShellCommand::ShowContextMenu {
                x,
                y,
                width: MENU_WIDTH as i32,
                height: self.window_context_menu_height(),
            },
        );
        #[cfg(target_os = "linux")]
        let _ = self.send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
        sent
    }

    fn launch_application(&mut self, application: Application) {
        if application.id().starts_with("place:")
            && let Some(path) = application
                .launch_command()
                .and_then(|command| command.get(1))
                .map(std::path::PathBuf::from)
        {
            let result = self
                .desktop_host
                .application()
                .file_window_host
                .dispatch(crate::file_window_host::browse_request(path));
            match result {
                Ok(()) => {
                    self.launcher.record_launch(application.id());
                    self.persist_launcher_preferences_with_recent(Some(
                        application.id().to_owned(),
                    ));
                    self.set_launcher_visible(false);
                }
                Err(error) => {
                    self.launcher_status =
                        Some(format!("Could not launch {}: {error}", application.name()))
                }
            }
            return;
        }
        #[cfg(target_os = "linux")]
        if platform::application_requires_secure_storage(&application)
            && self
                .session_host
                .secure_storage_state()
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "secure-storage query failed before application launch");
                    platform::SecureStorageState::ControlUnavailable
                })
                != platform::SecureStorageState::Ready
            && self.secure_storage_override.as_deref() != Some(application.id())
        {
            if let Err(error) = self.session_host.request_secure_storage_retry() {
                tracing::warn!(%error, "secure-storage retry command failed");
            }
            self.secure_storage_override = Some(application.id().to_owned());
            self.launcher_status = Some(format!(
                "Secure storage is not ready. Activate {} again to launch without credentials.",
                application.name()
            ));
            return;
        }
        self.secure_storage_override = None;
        self.launcher_status = None;
        #[cfg(target_os = "linux")]
        let result = if application.name() == "Nickel Settings" {
            platform::launch_session_application(&application)
        } else {
            platform::launch_application(&application)
        };
        #[cfg(not(target_os = "linux"))]
        let result = platform::launch_application(&application);
        match result {
            Ok(_) => {
                self.launcher.record_launch(application.id());
                self.persist_launcher_preferences_with_recent(Some(application.id().to_owned()));
                self.set_launcher_visible(false);
            }
            Err(error) => {
                tracing::warn!(
                    application = application.name(),
                    ?error,
                    "failed to launch application from launcher"
                );
                self.launcher_status = Some(format!(
                    "Could not launch {}: {}",
                    application.name(),
                    launch_error_summary(&error)
                ));
            }
        }
    }

    fn desktop_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        self.load_wallpaper_for(width, height);
        let application = self.desktop_host.application_mut();
        let wallpaper_changed = match (&application.wallpaper, &self.wallpaper) {
            (Some(current), Some(next)) => !Arc::ptr_eq(current, next),
            (None, None) => false,
            _ => true,
        };
        let palette_changed = application.palette != self.palette;
        if palette_changed {
            application.icon_cache.clear();
        }
        application.wallpaper.clone_from(&self.wallpaper);
        if wallpaper_changed {
            application.wallpaper_generation = application.wallpaper_generation.wrapping_add(1);
        }
        application.palette = self.palette;
        let application_changed =
            self.desktop_application_dirty || wallpaper_changed || palette_changed;
        let outcome = self.desktop_host.step(HostBatch {
            application_changed,
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.desktop_application_dirty = false;
        self.desktop_change_token = outcome.change_token;
        self.desktop_deadline = outcome.next_deadline;
        let commands = self.desktop_host.commands().to_vec();
        #[cfg(target_os = "windows")]
        let commands = {
            let mut commands = commands;
            if let Some((output, _, index)) = &self.output_identification
                && output == &self.desktop_active_viewport
            {
                let size = 144.0_f32.min(width as f32).min(height as f32);
                let rect = Rect::new(
                    (width as f32 - size) / 2.0,
                    (height as f32 - size) / 2.0,
                    size,
                    size,
                );
                commands.push(PaintCommand::RoundedFill {
                    rect,
                    color: 0xee202124,
                    radius: 24.0,
                });
                commands.push(PaintCommand::Text {
                    bounds: rect,
                    text: (index + 1).to_string(),
                    scale: 64.0,
                    color: 0xffffffff,
                    align: TextAlign::Center,
                    bold: true,
                    wrap: false,
                });
            }
            commands
        };
        commands
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn identify_output(&mut self, output: String, generation: u64, index: usize) {
        self.output_identification = Some((output, generation, index));
        self.desktop_application_dirty = true;
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn clear_output_identification(&mut self, generation: u64) -> bool {
        if output_identification_is_current(
            self.output_identification
                .as_ref()
                .map(|(_, current, _)| *current),
            generation,
        ) {
            self.output_identification = None;
            self.desktop_application_dirty = true;
            true
        } else {
            false
        }
    }

    fn lock_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let outcome = self.lock_host.step(HostBatch {
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.lock_change_token = outcome.change_token;
        self.lock_deadline = outcome.next_deadline;
        self.lock_host.commands().to_vec()
    }

    pub fn lock_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        if !self.locked {
            return false;
        }
        self.lock_host.step(HostBatch {
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        if !self.lock_host.input_context().text_focused
            && let Ok(target) = self
                .lock_host
                .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
        {
            let point = Point {
                x: target.bounds.origin.x + target.bounds.size.width / 2.0,
                y: target.bounds.origin.y + target.bounds.size.height / 2.0,
            };
            self.lock_host.step(HostBatch {
                events: vec![
                    HostEvent::Ui(UiEvent::PointerPressed(point)),
                    HostEvent::Ui(UiEvent::PointerReleased(point)),
                ],
                ..HostBatch::default()
            });
        }
        let (ingress, authority) =
            internal_normalized_ingress(input, None, "lock", self.lock_host.inspect(), None);
        let outcome = self.lock_host.step(HostBatch {
            events: vec![ingress],
            normalized_authorities: vec![authority],
            ..HostBatch::default()
        });
        let changed = outcome.changed;
        changed | self.apply_lock_effects()
    }

    pub fn lock_host_controller(&mut self, action: ControllerAction) -> bool {
        if !self.locked {
            return false;
        }
        let event = if action == ControllerAction::Confirm {
            HostEvent::Shortcut(Shortcut::Submit)
        } else {
            HostEvent::Controller(action)
        };
        let outcome = self.lock_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        outcome.changed | self.apply_lock_effects()
    }

    fn apply_lock_effects(&mut self) -> bool {
        let effects = std::mem::take(&mut self.lock_host.application_mut().effects);
        let changed = !effects.is_empty();
        for effect in effects {
            match effect {
                LockEffect::Authenticate(password) => {
                    let username = std::env::var("USER").unwrap_or_default();
                    let authenticated: Result<bool, String> = {
                        #[cfg(target_os = "linux")]
                        {
                            crate::lock_auth::authenticate(&username, &password)
                        }
                        #[cfg(not(target_os = "linux"))]
                        {
                            let _ = (&username, &password);
                            Err("system authentication is unavailable on this platform".into())
                        }
                    };
                    let application = self.lock_host.application_mut();
                    match authenticated {
                        Ok(true) => {
                            #[cfg(target_os = "linux")]
                            if let Err(error) = self.session_host.dispatch(ShellCommand::Unlock) {
                                tracing::warn!(%error, "session unlock command failed");
                                application.status = Some("Could not contact the session".into());
                            }
                        }
                        Ok(false) => application.status = Some("Authentication failed".into()),
                        Err(error) => {
                            tracing::error!(%error, "lock authentication failed");
                            application.status = Some("Authentication service unavailable".into());
                        }
                    }
                }
            }
        }
        changed
    }

    fn load_wallpaper_for(&mut self, width: u32, height: u32) {
        let requested = (width.max(1), height.max(1));
        let source_changed =
            self.wallpaper_loaded_source_fingerprint != self.wallpaper_source_fingerprint;
        let target = if source_changed {
            requested
        } else {
            let Some(target) = wallpaper_cache_target(self.wallpaper_size, requested) else {
                return;
            };
            target
        };
        let Some(path) = self.wallpaper_path.as_deref() else {
            return;
        };
        let Ok(image) = image::open(path) else {
            // A failed replacement must not blank the last successfully decoded
            // desktop. Retry only when the source fingerprint changes again.
            self.wallpaper_loaded_source_fingerprint = self.wallpaper_source_fingerprint.clone();
            return;
        };
        self.wallpaper = Some(Arc::new(image.thumbnail(target.0, target.1).into_rgba8()));
        self.wallpaper_size = target;
        self.wallpaper_loaded_source_fingerprint = self.wallpaper_source_fingerprint.clone();
    }

    fn sync_notification_host(&mut self, width: u32, height: u32) {
        if self.notification_history_visible {
            let history = self.notification_feed.history();
            self.notification_host
                .application_mut()
                .sync_history(&history, self.palette);
        } else {
            self.notification_host
                .application_mut()
                .sync(self.notification.as_ref(), self.palette);
        }
        self.notification_host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }

    fn trusted_notification_id(&self, id: u32) -> bool {
        self.remote_lease_notifications.contains_key(&id)
            || self.codex_approval_notifications.contains_key(&id)
    }

    fn trusted_notification_visible(&self) -> bool {
        self.notification
            .as_ref()
            .is_some_and(|notification| self.trusted_notification_id(notification.id))
            || (self.notification_history_visible
                && (!self.remote_lease_notifications.is_empty()
                    || !self.codex_approval_notifications.is_empty()))
    }

    fn notification_plugin_active(&self) -> bool {
        self.notification_plugin_host_ref().is_some() && !self.trusted_notification_visible()
    }

    fn notification_plugin_projection(&self) -> crate::plugin_panel::NotificationPluginProjection {
        let history = if self.notification_history_visible {
            self.notification_feed
                .history()
                .into_iter()
                .filter(|notification| !self.trusted_notification_id(notification.id))
                .collect::<Vec<_>>()
        } else {
            Vec::new()
        };
        crate::plugin_panel::NotificationPluginProjection::from_feed(
            self.notification
                .as_ref()
                .filter(|notification| !self.trusted_notification_id(notification.id)),
            &history,
            self.notification_history_visible,
        )
    }

    fn step_notification_plugin(
        &mut self,
        batch: HostBatch,
    ) -> Option<nickel_ui::HostEventOutcome> {
        let projection = self.notification_plugin_projection();
        let host = self.notification_plugin_host_mut()?;
        let (mut outcome, retained_bytes) =
            match step_plugin_host(host, Some(projection.to_json()), batch) {
                Ok(result) => result,
                Err(error) => {
                    self.fail_notification_plugin_runtime(error);
                    return None;
                }
            };
        let effects = host.application_mut().take_effects();
        self.record_plugin_panel_memory(
            &crate::plugin_panel::notification_surface_key(),
            retained_bytes,
        );
        outcome.changed |= self.apply_plugin_effects(effects);
        Some(outcome)
    }

    fn apply_notification_effects(&mut self) -> bool {
        for failure in self.notification_host.application_mut().take_failures() {
            tracing::warn!(?failure, "notification application rejected an action");
        }
        let effects = self.notification_host.application_mut().take_effects();
        let handled = !effects.is_empty();
        for effect in effects {
            match effect {
                NotificationEffect::Invoke {
                    notification_id,
                    key,
                } => {
                    let offered = self.notification_feed.history().iter().any(|notification| {
                        notification.id == notification_id
                            && notification.actions.iter().any(|action| action.key == key)
                    });
                    if !offered {
                        tracing::warn!(notification_id, %key, "notification action was not offered");
                        continue;
                    }
                    #[cfg(any(target_os = "linux", target_os = "windows"))]
                    if let Some(pending) = self
                        .remote_lease_notifications
                        .get(&notification_id)
                        .cloned()
                    {
                        if (key == "approve" || key == "deny")
                            && !self.remote_lease_submitting.contains(&notification_id)
                            && self
                                .notification_feed
                                .mark_internal_submitting(notification_id)
                        {
                            self.remote_lease_submitting.insert(notification_id);
                            let allow = key == "approve";
                            #[cfg(target_os = "linux")]
                            if let Err(error) =
                                self.session_host.decide_remote_lease(&pending, allow)
                            {
                                tracing::warn!(%error, "remote lease notification decision is unconfirmed");
                            }
                            #[cfg(target_os = "windows")]
                            self.remote_lease_decisions.push((pending, allow));
                            if self
                                .notification
                                .as_ref()
                                .is_some_and(|item| item.id == notification_id)
                            {
                                self.notification = self
                                    .notification_feed
                                    .history()
                                    .into_iter()
                                    .find(|item| item.id == notification_id);
                            }
                        }
                        continue;
                    }
                    if let Some((owner, pending)) = self
                        .codex_approval_notifications
                        .get(&notification_id)
                        .cloned()
                    {
                        if key == "review" && pending.needs_review() && pending.actionable {
                            self.codex_approval_reviews.push(owner);
                            self.dismiss_notification_transport(notification_id);
                            continue;
                        }
                        let choice = pending.notification_choice(&key);
                        if let Some(choice) = choice
                            && self
                                .notification_feed
                                .mark_internal_submitting(notification_id)
                        {
                            // The owning app revalidates the complete snapshot before dispatch.
                            self.codex_approval_decisions.push((owner, pending, choice));
                        }
                        continue;
                    }
                    self.notification_feed.invoke(notification_id, &key);
                    self.dismiss_notification_transport(notification_id);
                }
                NotificationEffect::Dismiss { notification_id } => {
                    self.dismiss_notification_transport(notification_id);
                }
                NotificationEffect::CloseHistory => {
                    self.notification_history_visible = false;
                    self.notification = None;
                    #[cfg(target_os = "linux")]
                    let _ = self.send_session_command(
                        "hide-notification-history",
                        ShellCommand::SetShellRoleVisible {
                            role: nickel_session_protocol::ShellRole::Notification,
                            visible: false,
                        },
                    );
                    #[cfg(target_os = "linux")]
                    let _ = self.send_session_command(
                        "restore-notification-focus",
                        ShellCommand::RestoreApplicationFocus,
                    );
                }
            }
        }
        handled
    }

    fn sync_remote_lease_notifications(&mut self) {
        let pending = self.session_host.remote_pending_leases();
        self.sync_remote_lease_notifications_from(pending);
    }

    pub(crate) fn sync_codex_approval_notifications(
        &mut self,
        pending: Vec<(
            CodexApprovalOwner,
            nickel_codex_ui::CodexApprovalNotification,
        )>,
    ) {
        use nickel_codex_ui::PendingInteraction;

        let identity =
            |owner: CodexApprovalOwner, snapshot: &nickel_codex_ui::CodexApprovalNotification| {
                let PendingInteraction::Approval { request_id, .. } = &snapshot.interaction else {
                    unreachable!("approval projection cannot contain a question")
                };
                (owner, snapshot.connection_generation, request_id.clone())
            };
        let overflow_identity =
            |owner: CodexApprovalOwner, snapshot: &nickel_codex_ui::CodexApprovalNotification| {
                let (owner, generation, request_id) = identity(owner, snapshot);
                (owner, generation, request_id, snapshot.request_revision)
            };
        self.codex_approval_overflow_outcomes.retain(|key| {
            pending
                .iter()
                .any(|(owner, snapshot)| overflow_identity(*owner, snapshot) == *key)
        });
        let stale = self
            .codex_approval_notifications
            .iter()
            .filter(|(_, (owner, shown))| {
                !pending.iter().any(|(current_owner, current)| {
                    identity(*current_owner, current) == identity(*owner, shown)
                })
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in stale {
            self.codex_approval_notifications.remove(&id);
            self.dismissed_codex_approval_notifications.remove(&id);
            self.notification_feed.close_internal(id);
        }
        for (owner, snapshot) in pending {
            let key = identity(owner, &snapshot);
            let existing =
                self.codex_approval_notifications
                    .iter()
                    .find_map(|(id, (shown_owner, shown))| {
                        (identity(*shown_owner, shown) == key).then_some((*id, shown.clone()))
                    });
            if existing
                .as_ref()
                .is_some_and(|(_, shown)| shown == &snapshot)
            {
                continue;
            }
            let displayable = snapshot.presentation.is_within_budget();
            let request = NotificationRequest {
                app_name: "Nickel Codex".into(),
                summary: if displayable {
                    snapshot.presentation.title()
                } else {
                    "Codex request cannot be displayed safely".into()
                },
                body: if displayable {
                    // Notification history and copied summaries can outlive the
                    // request. A raw command may contain credentials, so retain
                    // exact detail only in the owning Codex interaction surface.
                    let mut body = snapshot.presentation.notification_body();
                    if snapshot.presentation.detail.is_some() {
                        body.push_str(" Review the full operation in Codex before approving.");
                    }
                    body
                } else {
                    "Request details exceed the local display limit; approval is unavailable."
                        .into()
                },
                actions: if snapshot.actionable && snapshot.needs_review() {
                    vec![NotificationAction {
                        key: "review".into(),
                        label: "Review in Codex".into(),
                    }]
                } else if snapshot.actionable {
                    snapshot
                        .notification_actions()
                        .into_iter()
                        .filter(|(key, _)| {
                            displayable || matches!(key.as_str(), "cancel" | "decline")
                        })
                        .map(|(key, label)| NotificationAction { key, label })
                        .collect()
                } else {
                    Vec::new()
                },
                expire_timeout_ms: 0,
            };
            let id = match existing {
                Some((id, _)) => self.notification_feed.replace_internal(id, request),
                None => self.notification_feed.notify_internal(request),
            };
            if id != 0 {
                if self
                    .codex_approval_overflow_outcomes
                    .remove(&overflow_identity(owner, &snapshot))
                {
                    self.codex_approval_delivery_updates
                        .push((owner, snapshot.clone(), true));
                }
                self.codex_approval_notifications
                    .insert(id, (owner, snapshot));
            } else if self
                .codex_approval_overflow_outcomes
                .insert(overflow_identity(owner, &snapshot))
            {
                // Preserve source authority: the backend may not have offered
                // Decline, and a fabricated response would fail validation.
                if let Some(choice) = snapshot.overflow_refusal_choice() {
                    self.codex_approval_decisions
                        .push((owner, snapshot, choice));
                } else {
                    self.codex_approval_delivery_updates
                        .push((owner, snapshot, false));
                    tracing::warn!(
                        "Codex approval could not enter the notification feed and has no offered refusal; it remains pending in Codex"
                    );
                }
            }
        }
    }

    pub(crate) fn take_codex_approval_decisions(
        &mut self,
    ) -> Vec<(
        CodexApprovalOwner,
        nickel_codex_ui::CodexApprovalNotification,
        nickel_codex_ui::CodexApprovalChoice,
    )> {
        std::mem::take(&mut self.codex_approval_decisions)
    }

    pub(crate) fn take_codex_approval_reviews(&mut self) -> Vec<CodexApprovalOwner> {
        std::mem::take(&mut self.codex_approval_reviews)
    }

    pub(crate) fn take_codex_approval_delivery_updates(
        &mut self,
    ) -> Vec<(
        CodexApprovalOwner,
        nickel_codex_ui::CodexApprovalNotification,
        bool,
    )> {
        std::mem::take(&mut self.codex_approval_delivery_updates)
    }

    pub(crate) fn sync_remote_lease_notifications_from(
        &mut self,
        pending: Vec<nickel_session_protocol::RemotePendingLease>,
    ) {
        self.remote_lease_overflow_rejections
            .retain(|(client_id, generation)| {
                pending.iter().any(|request| {
                    request.client_id == *client_id && request.pending_generation == *generation
                })
            });
        let stale = self
            .remote_lease_notifications
            .iter()
            .filter(|(_, shown)| {
                !pending.iter().any(|current| {
                    current.client_id == shown.client_id
                        && current.pending_generation == shown.pending_generation
                })
            })
            .map(|(id, _)| *id)
            .collect::<Vec<_>>();
        for id in stale {
            self.remote_lease_notifications.remove(&id);
            self.remote_lease_submitting.remove(&id);
            self.dismissed_remote_lease_notifications.remove(&id);
            self.notification_feed.close_internal(id);
        }
        for request in pending {
            if self.remote_lease_notifications.values().any(|shown| {
                shown.client_id == request.client_id
                    && shown.pending_generation == request.pending_generation
            }) {
                continue;
            }
            let scope =
                request
                    .resource_label
                    .clone()
                    .unwrap_or_else(|| match &request.request.scope {
                        nickel_session_protocol::RemoteResourceScope::FullSession => {
                            "the full desktop".to_owned()
                        }
                        nickel_session_protocol::RemoteResourceScope::Application(_) => {
                            "an application".to_owned()
                        }
                        nickel_session_protocol::RemoteResourceScope::Window(_) => {
                            "a window".to_owned()
                        }
                        nickel_session_protocol::RemoteResourceScope::Surface(_) => {
                            "a surface".to_owned()
                        }
                        nickel_session_protocol::RemoteResourceScope::Output(_) => {
                            "a display".to_owned()
                        }
                    });
            let duration = Some(request.request.duration_seconds.map_or_else(
                || "until logout".to_owned(),
                |seconds| {
                    if seconds % 3_600 == 0 {
                        format!("{} hours", seconds / 3_600)
                    } else if seconds % 60 == 0 {
                        format!("{} minutes", seconds / 60)
                    } else {
                        format!("{seconds} seconds")
                    }
                },
            ));
            let mut warnings = Vec::new();
            if request.request.full_debug {
                warnings.push("Full Nickel debugging access is requested.");
            }
            if request.request.allow_resumption {
                warnings.push("This lease may resume after the client reconnects.");
            }
            if request.changes.access_changed {
                warnings.push("This request broadens access from the previous request.");
            }
            if request.changes.duration_increased {
                warnings.push("This request increases the duration.");
            }
            if request.request.renewal.is_some() {
                warnings.push("This renews an existing lease; it does not extend until approved.");
            }
            let presentation = nickel_ui::approval::ApprovalPresentation {
                requester: request.client_label.clone(),
                identity: nickel_ui::approval::RequesterIdentity::SelfReportedRemote,
                action: "Control desktop resources".into(),
                scope: Some(scope),
                duration,
                warning: (!warnings.is_empty()).then(|| warnings.join(" ")),
                // These operations are implied by the resource lease, not separate grants.
                detail: Some("Control may include observing and capturing the covered resource, pointer and keyboard input, and window management. Covered application launches may be possible when the scope permits. Protected surfaces and clipboard or filesystem transfer are excluded.".into()),
            };
            let displayable = presentation.is_within_budget();
            let id = self.notification_feed.notify_internal(NotificationRequest {
                app_name: "Nickel".into(),
                summary: if displayable {
                    presentation.title()
                } else {
                    "Remote control request cannot be displayed safely".into()
                },
                body: if displayable {
                    presentation.notification_body_with_detail()
                } else {
                    "Request details exceed the local display limit. Approval is unavailable; you can deny the request."
                        .into()
                },
                actions: [
                    NotificationAction {
                        key: "deny".into(),
                        label: "Deny".into(),
                    },
                ]
                .into_iter()
                .chain(displayable.then_some(NotificationAction {
                    key: "approve".into(),
                    label: "Approve".into(),
                }))
                .collect(),
                expire_timeout_ms: 0,
            });
            if id != 0 {
                self.remote_lease_notifications.insert(id, request);
            } else if self
                .remote_lease_overflow_rejections
                .insert((request.client_id.clone(), request.pending_generation))
            {
                tracing::warn!(
                    "remote approval could not enter the bounded notification feed; rejecting the request"
                );
                #[cfg(target_os = "linux")]
                if let Err(error) = self.session_host.decide_remote_lease(&request, false) {
                    tracing::warn!(%error, "remote approval overflow rejection was unconfirmed");
                }
                #[cfg(target_os = "windows")]
                self.remote_lease_decisions.push((request, false));
            }
        }
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn take_remote_lease_decisions(
        &mut self,
    ) -> Vec<(nickel_session_protocol::RemotePendingLease, bool)> {
        std::mem::take(&mut self.remote_lease_decisions)
    }

    fn dismiss_notification_transport(&mut self, notification_id: u32) {
        if self.notification.as_ref().map(|item| item.id) != Some(notification_id) {
            return;
        }
        self.notification = None;
        if self
            .remote_lease_notifications
            .contains_key(&notification_id)
        {
            self.dismissed_remote_lease_notifications
                .insert(notification_id);
        } else if self
            .codex_approval_notifications
            .contains_key(&notification_id)
        {
            self.dismissed_codex_approval_notifications
                .insert(notification_id);
        } else {
            self.notification_feed.dismiss(notification_id);
        }
        self.notification_host
            .application_mut()
            .sync(None, self.palette);
        self.notification_host.step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }

    fn window_preview_scene(&mut self) -> Vec<PaintCommand> {
        if !self.preview_plugin_active() {
            return Vec::new();
        }
        let Some(group) = self.preview_plugin_group() else {
            return Vec::new();
        };
        let (width, height) = if self.task_switcher_group.is_some() {
            task_switcher_dimensions(group.windows.len().min(5))
        } else {
            preview_dimensions(group.windows.len().min(12))
        };
        self.plugin_panel_scene(
            &crate::plugin_panel::window_preview_surface_key(),
            width,
            height,
        )
        .unwrap_or_default()
    }

    fn preview_plugin_active(&self) -> bool {
        self.preview_plugin_host_ref().is_some()
            && (self.preview_group.is_some() || self.task_switcher_group.is_some())
    }

    fn preview_plugin_group(&mut self) -> Option<crate::model::WindowGroup> {
        self.task_switcher_group.clone().or_else(|| {
            self.preview_group.and_then(|index| {
                self.panel_groups()
                    .get(index)
                    .map(|task| task.window_group())
            })
        })
    }

    fn preview_plugin_action_allowed(&mut self, action: PreviewAction) -> bool {
        if !self.preview_plugin_active() {
            return false;
        }
        let (PreviewAction::Activate(id) | PreviewAction::Close(id) | PreviewAction::OpenMenu(id)) =
            action;
        let Some(group) = self.preview_plugin_group() else {
            return false;
        };
        let limit = if self.task_switcher_group.is_some() {
            5
        } else {
            12
        };
        let Some(projected) = group
            .windows
            .iter()
            .take(limit)
            .find(|window| window.id == id)
        else {
            return false;
        };
        let Some(current) = self.windows.iter().find(|window| window.id == id) else {
            return false;
        };
        if current.application_id != projected.application_id {
            return false;
        }
        match action {
            PreviewAction::Activate(_) => current.state.capabilities.activate,
            PreviewAction::Close(_) => current.state.capabilities.close,
            PreviewAction::OpenMenu(_) => self.plugin_taskbar_host.is_some(),
        }
    }

    fn preview_plugin_bounds(&self, action: PreviewAction) -> Option<Rect> {
        let id = match action {
            PreviewAction::Activate(window) | PreviewAction::OpenMenu(window) => {
                format!("preview-window-{}", window.0)
            }
            PreviewAction::Close(window) => format!("preview-close-{}", window.0),
        };
        let host = self.preview_plugin_host_ref()?;
        let message = host.application().button_message(&id)?;
        host.semantic_targets_for_message(&message)
            .into_iter()
            .next()
            .map(|target| target.bounds)
    }

    #[cfg(any(target_os = "windows", test))]
    fn preview_thumbnail_bounds(
        &mut self,
        windows: &[crate::model::WindowId],
    ) -> Option<Vec<crate::platform::PreviewThumbnailBounds>> {
        // The preview can open before its first paint. Resolve the JSX tree now
        // so the native thumbnail uses the same image-button geometry.
        let _ = self.window_preview_scene();
        let (width, height) = self.preview_plugin_size()?;
        windows
            .iter()
            .map(|window| {
                let bounds = self.preview_plugin_bounds(PreviewAction::Activate(*window))?;
                let x = bounds.origin.x;
                let y = bounds.origin.y;
                let right = x + bounds.size.width;
                let bottom = y + bounds.size.height;
                if ![x, y, right, bottom].into_iter().all(f32::is_finite) {
                    return None;
                }
                let left = (x.floor() as i32).clamp(0, width as i32);
                let top = (y.floor() as i32).clamp(0, height as i32);
                let right = (right.ceil() as i32).clamp(0, width as i32);
                let bottom = (bottom.ceil() as i32).clamp(0, height as i32);
                (right > left && bottom > top).then_some(crate::platform::PreviewThumbnailBounds {
                    left,
                    top,
                    right,
                    bottom,
                })
            })
            .collect()
    }

    fn preview_plugin_size(&mut self) -> Option<(u32, u32)> {
        let group = self.preview_plugin_group()?;
        Some(if self.task_switcher_group.is_some() {
            task_switcher_dimensions(group.windows.len().min(5))
        } else {
            preview_dimensions(group.windows.len().min(12))
        })
    }

    fn preview_plugin_selected_window(&mut self) -> Option<crate::model::WindowId> {
        let selected = self
            .preview_plugin_host_ref()?
            .inspect()
            .controller_target?;
        let group = self.preview_plugin_group()?;
        let limit = if self.task_switcher_group.is_some() {
            5
        } else {
            12
        };
        group.windows.iter().take(limit).find_map(|window| {
            let message = self
                .preview_plugin_host_ref()?
                .application()
                .button_message(&format!("preview-window-{}", window.id.0))?;
            self.preview_plugin_host_ref()?
                .semantic_targets_for_message(&message)
                .iter()
                .any(|target| target.id == selected)
                .then_some(window.id)
        })
    }

    fn preview_plugin_projection(
        &self,
        group: &crate::model::WindowGroup,
    ) -> (serde_json::Value, crate::plugin_panel::PluginImages) {
        let switcher = self.task_switcher_group.is_some();
        let limit = if switcher { 5 } else { 12 };
        let selected = self.task_switcher.selected().copied();
        let windows = group
            .windows
            .iter()
            .take(limit)
            .enumerate()
            .map(|(index, window)| {
                let title = if window.title.is_empty() {
                    &group.application_name
                } else {
                    &window.title
                };
                let title = if title.is_empty() {
                    "Untitled window"
                } else {
                    title
                };
                let title = title.chars().take(120).collect::<String>();
                let accessible_name = if switcher {
                    format!(
                        "{}, {} of {}{}",
                        title,
                        index + 1,
                        group.windows.len(),
                        if selected == Some(window.id) {
                            ", selected"
                        } else {
                            ""
                        }
                    )
                } else {
                    title.clone()
                };
                serde_json::json!({
                    "id": window.id.0.to_string(),
                    "title": title,
                    "accessibleName": accessible_name,
                    "closable": window.state.capabilities.close,
                    "selected": switcher && selected == Some(window.id),
                    "imageWidth": if switcher { 188 } else { 244 },
                    "index": index,
                })
            })
            .collect::<Vec<_>>();
        let images = group
            .windows
            .iter()
            .take(limit)
            .enumerate()
            .filter_map(|(index, window)| {
                self.preview_images.get(&window.id).map(|image| {
                    (
                        format!("window:{}", window.id.0),
                        (0x7000_u16 + index as u16, Arc::clone(image)),
                    )
                })
            })
            .collect();
        (
            serde_json::json!({"windows": windows, "taskSwitcher": switcher}),
            images,
        )
    }

    fn preview_plugin_event(
        &mut self,
        event: HostEvent,
        size: (u32, u32),
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> nickel_ui::HostEventOutcome {
        let Some(group) = self.preview_plugin_group() else {
            return Default::default();
        };
        let (data, images) = self.preview_plugin_projection(&group);
        let Some(host) = self.preview_plugin_host_mut() else {
            return Default::default();
        };
        let data_changed = match host.application_mut().sync_data(&data) {
            Ok(changed) => changed,
            Err(error) => {
                self.fail_preview_plugin_runtime(error);
                return Default::default();
            }
        };
        let images_changed = host.application_mut().sync_images(images);
        let mut outcome = host.step(HostBatch {
            application_changed: data_changed || images_changed,
            surface_size: Some(size),
            events: vec![event],
            normalized_authorities: authority.into_iter().collect(),
            ..Default::default()
        });
        let effects = host.application_mut().take_effects();
        if let Some(error) = host.application_mut().take_runtime_failure() {
            self.fail_preview_plugin_runtime(error);
            return Default::default();
        }
        outcome.changed |= self.apply_plugin_effects(effects);
        outcome
    }

    fn taskbar_window_menu_projection(
        snapshot: &OpenWindow,
        workspaces: &[platform::WorkspaceSummary],
        outputs: &[String],
    ) -> crate::plugin_panel::TaskbarWindowMenuPluginProjection {
        let entries = |items: Vec<(String, MenuAction)>| {
            items
                .into_iter()
                .take(32)
                .map(|(label, action)| {
                    let navigate = match action {
                        MenuAction::ShowWorkspaces => Some("workspaces"),
                        MenuAction::ShowDisplays => Some("displays"),
                        MenuAction::Back => Some("root"),
                        _ => None,
                    };
                    (label, navigate)
                })
                .collect()
        };
        crate::plugin_panel::TaskbarWindowMenuPluginProjection {
            root: entries(window_menu_entries(snapshot, workspaces, outputs)),
            workspaces: entries(workspace_menu_entries(snapshot, workspaces)),
            displays: entries(display_menu_entries(snapshot, outputs)),
        }
    }

    fn record_taskbar_memory(&mut self) {
        let retained_frames = self
            .plugin_taskbar_memory
            .values()
            .copied()
            .fold(self.plugin_taskbar_menu_memory, u64::saturating_add);
        let mut seen_images = std::collections::HashSet::new();
        let retained_images = self
            .plugin_taskbar_host
            .iter()
            .chain(self.plugin_taskbar_hosts.values())
            .flat_map(|host| host.application().retained_image_allocations())
            .filter(|(address, _)| seen_images.insert(*address))
            .map(|(_, bytes)| bytes)
            .fold(0_u64, u64::saturating_add);
        let _ = self.plugin_registry.record_memory(
            &crate::plugin_panel::taskbar_manifest().id,
            nickel_core::plugins::PluginMemory {
                native_ui_bytes: Some(retained_frames.saturating_add(retained_images)),
                ..nickel_core::plugins::PluginMemory::default()
            },
        );
        self.maybe_publish_plugin_status();
    }

    fn window_menu_scene(&mut self) -> Vec<PaintCommand> {
        if self.plugin_taskbar_host.is_none() {
            return Vec::new();
        }
        if self.application_menu_target.is_some() {
            return self.application_menu_scene();
        }
        if self.window_menu.is_none() && self.window_menu_snapshot.is_none() {
            self.clear_window_menu_plugin_host();
            return Vec::new();
        }
        let Some(snapshot) = self.window_menu_snapshot.clone().or_else(|| {
            self.window_menu.and_then(|window| {
                self.windows
                    .iter()
                    .find(|candidate| candidate.id == window)
                    .cloned()
            })
        }) else {
            self.close_window_preview();
            return Vec::new();
        };
        self.window_menu_snapshot
            .get_or_insert_with(|| snapshot.clone());
        let outputs = self.window_feed.outputs();
        if self.plugin_taskbar_host.is_some() {
            let projection =
                Self::taskbar_window_menu_projection(&snapshot, &self.workspaces, &outputs);
            let height = (16
                + 48 * window_menu_max_rows(&snapshot, &self.workspaces, &outputs).min(32))
                as u32;
            if self.window_menu_plugin_host.is_none() {
                let runtime = self
                    .plugin_taskbar_host
                    .as_ref()
                    .unwrap()
                    .application()
                    .shared_runtime();
                match crate::plugin_panel::PluginPanelApplication::bundled_with_shared_entry_runtime(
                    crate::plugin_panel::taskbar_manifest(),
                    "window-menu.js",
                    projection.to_json(),
                    "taskbar-window-menu",
                    runtime,
                ) {
                    Ok(application) => {
                        self.window_menu_plugin_host = Some(nickel_ui::UiHost::new(
                            application,
                            MENU_WIDTH.ceil() as u32,
                            height,
                        ));
                    }
                    Err(error) => {
                        tracing::error!(%error, "taskbar JSX window menu failed to start")
                    }
                }
            }
            if let Some(host) = self.window_menu_plugin_host.as_mut() {
                let (commands, bytes) = match render_plugin_host(
                    host,
                    Some(projection.to_json()),
                    HostBatch {
                        surface_size: Some((MENU_WIDTH.ceil() as u32, height)),
                        events: vec![HostEvent::Poll],
                        ..HostBatch::default()
                    },
                ) {
                    Ok(frame) => frame,
                    Err(error) => {
                        self.fail_taskbar_plugin_runtime(error);
                        return Vec::new();
                    }
                };
                self.plugin_taskbar_menu_memory = bytes;
                self.record_taskbar_memory();
                return commands;
            }
        }
        Vec::new()
    }

    fn application_menu_scene(&mut self) -> Vec<PaintCommand> {
        if self.plugin_taskbar_host.is_none() {
            return Vec::new();
        }
        let Some(target) = self.application_menu_target.clone() else {
            self.clear_application_menu_plugin_host();
            self.plugin_taskbar_menu_memory = 0;
            return Vec::new();
        };
        let pinned = target
            .application_id
            .as_ref()
            .is_some_and(|id| self.launcher.is_pinned(id.as_str()));
        if self.plugin_taskbar_host.is_some() {
            let actions = self.slot_actions_for_item(
                &crate::plugin_panel::taskbar_manifest().id,
                "task-action",
                target.application_id.as_ref().map(|id| id.as_str()),
                4,
            );
            let height = (16
                + 48 * (application_menu_entries(&target, pinned).len() + actions.len()))
                as u32;
            let data = serde_json::json!({
                "applicationId": target.application_id.as_ref().map(|id| id.as_str()),
                "slotContext": {"item": target.application_id.as_ref().map(|id| id.as_str())},
                "pinned": pinned,
                "closeAll": target.all_closeable,
                "slots": {"task-action": actions},
            })
            .to_string();
            if self.application_menu_plugin_host.is_none() {
                let runtime = self
                    .plugin_taskbar_host
                    .as_ref()
                    .unwrap()
                    .application()
                    .shared_runtime();
                match crate::plugin_panel::PluginPanelApplication::bundled_with_shared_entry_runtime(
                    crate::plugin_panel::taskbar_manifest(),
                    "menu.js",
                    data.clone(),
                    "taskbar-application-menu",
                    runtime,
                ) {
                    Ok(application) => {
                        self.application_menu_plugin_host = Some(nickel_ui::UiHost::new(
                            application,
                            MENU_WIDTH.ceil() as u32,
                            height,
                        ));
                    }
                    Err(error) => {
                        tracing::error!(%error, "taskbar JSX menu failed to start");
                    }
                }
            }
            if let Some(host) = self.application_menu_plugin_host.as_mut() {
                let (commands, bytes) = match render_plugin_host(
                    host,
                    Some(data),
                    HostBatch {
                        surface_size: Some((MENU_WIDTH.ceil() as u32, height)),
                        events: vec![HostEvent::Poll],
                        ..HostBatch::default()
                    },
                ) {
                    Ok(frame) => frame,
                    Err(error) => {
                        self.fail_taskbar_plugin_runtime(error);
                        return Vec::new();
                    }
                };
                self.plugin_taskbar_menu_memory = bytes;
                self.record_taskbar_memory();
                return commands;
            }
        }
        Vec::new()
    }

    fn launcher_status_text(&self) -> Option<String> {
        self.launcher_status
            .as_deref()
            .or(self.shortcut_action_status.as_deref())
            .or(self.shortcut_capability_status.as_deref())
            .or_else(|| secure_storage_status_label(self.secure_storage_state))
            .or_else(|| {
                session_feed_status_label(self.window_feed_status, self.workspace_feed_status)
            })
            .map(str::to_owned)
    }

    pub fn set_global_shortcut_capability(
        &mut self,
        capability: &nickel_input::global::ShortcutCapability,
    ) {
        self.shortcut_capability_status = shortcut_capability_status(capability);
    }

    fn panel_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        self.step_taskbar_plugin_batch(
            HostBatch {
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            },
            width,
            height,
        );
        self.plugin_taskbar_host
            .as_ref()
            .map(|host| host.commands().to_vec())
            .unwrap_or_default()
    }

    fn step_taskbar_plugin(
        &mut self,
        events: Vec<HostEvent>,
        width: u32,
    ) -> Option<nickel_ui::HostEventOutcome> {
        self.step_taskbar_plugin_batch(
            HostBatch {
                events,
                ..HostBatch::default()
            },
            width,
            crate::plugin_panel::taskbar_surface().height,
        )
    }

    fn step_taskbar_plugin_batch(
        &mut self,
        mut batch: HostBatch,
        width: u32,
        height: u32,
    ) -> Option<nickel_ui::HostEventOutcome> {
        let pointer_moved = batch.events.iter().any(|event| match event {
            HostEvent::Ui(UiEvent::PointerMoved(_)) => true,
            HostEvent::Normalized { input, .. } => matches!(
                input,
                nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion { .. })
            ),
            HostEvent::NormalizedIngress(envelope) => matches!(
                &envelope.input,
                nickel_input::InputEvent::Pointer(nickel_input::PointerEvent::Motion { .. })
            ),
            _ => false,
        });
        let (clock, _) = panel_clock_text();
        let (data, images) = self.taskbar_plugin_render_data(&clock);
        let host = self.plugin_taskbar_host.as_mut()?;
        let image_changed = host.application_mut().sync_images(images);
        batch.application_changed |= image_changed;
        batch.surface_size = Some((width, height));
        let (mut outcome, _) = match step_plugin_host(host, Some(data), batch) {
            Ok(result) => result,
            Err(error) => {
                self.fail_taskbar_plugin_runtime(error);
                return None;
            }
        };
        let effects = host.application_mut().take_effects();
        if self.plugin_taskbar_memory.len() >= 32
            && !self.plugin_taskbar_memory.contains_key(&self.panel_output)
        {
            self.plugin_taskbar_memory.clear();
        }
        self.plugin_taskbar_memory.insert(
            self.panel_output.clone(),
            outcome.telemetry.retained_frame_bytes as u64,
        );
        self.record_taskbar_memory();
        self.panel_change_token = outcome.change_token;
        self.panel_deadline = outcome
            .next_deadline
            .or_else(|| Some(Instant::now() + taskbar::duration_until_next_minute()));
        let pets_visible = self.windows.iter().any(|window| {
            window
                .application_id
                .as_ref()
                .is_some_and(|id| id.as_str().starts_with("io.nickel.codex.project."))
        }) || self
            .launcher
            .preferences()
            .favorites()
            .iter()
            .any(|id| id.starts_with("io.nickel.codex.project."));
        if pets_visible {
            self.panel_pet_deadline
                .get_or_insert_with(|| Instant::now() + Duration::from_millis(360));
        } else {
            self.panel_pet_deadline = None;
        }
        if pointer_moved {
            outcome.changed |= self.sync_panel_hover_from_host();
        }
        outcome.changed |= self.apply_plugin_effects(effects);
        Some(outcome)
    }

    fn panel_groups(&mut self) -> Arc<Vec<crate::launcher::TaskbarApplication>> {
        self.panel_projections.retain(|_, projection| {
            Arc::ptr_eq(self.launcher.taskbar_revision(), &projection.revision)
        });
        let current = self
            .panel_projections
            .get(&self.panel_output)
            .is_some_and(|projection| {
                Arc::ptr_eq(self.launcher.taskbar_revision(), &projection.revision)
                    && self.panel_window_iter().eq(projection.windows.iter())
            });
        if !current {
            // A corrupt or unbounded stream of native output names cannot
            // retain an unbounded history of task projections.
            if self.panel_projections.len() >= 32
                && !self.panel_projections.contains_key(&self.panel_output)
            {
                self.panel_projections.clear();
            }
            let windows = self.panel_windows();
            let groups = Arc::new(self.launcher.taskbar_applications(&windows));
            self.panel_projections.insert(
                self.panel_output.clone(),
                PanelTaskProjection {
                    windows,
                    groups,
                    revision: Arc::clone(self.launcher.taskbar_revision()),
                },
            );
        }
        Arc::clone(&self.panel_projections[&self.panel_output].groups)
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn retain_panel_outputs(
        &mut self,
        outputs: &[crate::internal_shell::InternalOutput],
    ) {
        let previous_host_count = self.plugin_taskbar_hosts.len();
        let previous_memory_count = self.plugin_taskbar_memory.len();
        if self
            .panel_output
            .as_ref()
            .is_some_and(|name| !outputs.iter().any(|output| &output.name == name))
        {
            self.switch_panel_output(None);
        }
        self.panel_projections.retain(|output, _| {
            output
                .as_ref()
                .is_none_or(|name| outputs.iter().any(|output| &output.name == name))
        });
        self.plugin_taskbar_hosts.retain(|output, host| {
            let keep = output
                .as_ref()
                .is_none_or(|name| outputs.iter().any(|output| &output.name == name));
            if !keep {
                let _ = host.application().retire_surface();
            }
            keep
        });
        self.plugin_taskbar_memory.retain(|output, _| {
            output
                .as_ref()
                .is_none_or(|name| outputs.iter().any(|output| &output.name == name))
        });
        if self.plugin_taskbar_host.is_some()
            && (self.plugin_taskbar_hosts.len() != previous_host_count
                || self.plugin_taskbar_memory.len() != previous_memory_count)
        {
            self.record_taskbar_memory();
        }
    }

    fn resolve_task_icons(
        &mut self,
        groups: &[crate::launcher::TaskbarApplication],
        pet_frame: u8,
    ) -> Vec<Option<(u16, Arc<image::RgbaImage>)>> {
        groups
            .iter()
            .take(12)
            .map(|group| {
                // Project identity takes precedence over the generic Codex icon and
                // is shared by every window in the same project group.
                if let Some(pet) = group
                    .application_id
                    .as_ref()
                    .and_then(|id| crate::icons::codex_pet(id.as_str(), pet_frame))
                {
                    return Some(pet);
                }
                group
                    .application_id
                    .as_ref()
                    .and_then(|id| self.launcher.application(id))
                    .and_then(|application| self.launcher_icons.resolve(application))
                    .or_else(|| {
                        group
                            .application_id
                            .as_ref()
                            .is_some_and(|id| id.as_str().starts_with("io.nickel.codex.project."))
                            .then(|| (0x3002, Arc::clone(&self.codex_icon)))
                    })
                    .or_else(|| {
                        group
                            .application_id
                            .as_ref()
                            .and_then(|id| crate::icons::nickel_application(id.as_str()))
                    })
                    .or_else(|| crate::icons::nickel_application(&group.application_name))
                    .or_else(|| {
                        group.windows.first().and_then(|window| {
                            self.window_icons.get(&window.id).cloned().map(|icon| {
                                self.launcher_icons.resolve_window_icon(window.id, icon)
                            })
                        })
                    })
            })
            .collect()
    }

    fn taskbar_plugin_projection(
        &mut self,
        clock: &str,
    ) -> (
        crate::plugin_panel::TaskbarPluginProjection,
        crate::plugin_panel::PluginImages,
    ) {
        let groups = self.panel_groups();
        let pet_frame = self.panel_pet_frame;
        let task_icons = self.resolve_task_icons(groups.as_ref(), pet_frame);
        taskbar_plugin_data(
            TaskbarProjectionInput {
                groups: groups.as_ref(),
                keyboard_enabled: self.keyboard_enabled,
                codex_available: self.launcher.codex_available(),
                panel_icon: &self.panel_icon,
                codex_icon: &self.codex_icon,
                task_icons: &task_icons,
                tray: &self.tray,
                tray_icons: &self.tray_icons,
            },
            clock,
        )
    }

    fn taskbar_plugin_render_data(
        &mut self,
        clock: &str,
    ) -> (String, crate::plugin_panel::PluginImages) {
        let (projection, images) = self.taskbar_plugin_projection(clock);
        let mut data: serde_json::Value =
            serde_json::from_str(&projection.to_json()).expect("taskbar projection produces JSON");
        let visible_items = projection
            .items
            .iter()
            .map(|item| item.id.clone())
            .collect::<HashSet<_>>();
        data["slots"] = self
            .plugin_slot_projection_with_context(
                &crate::plugin_panel::taskbar_manifest().id,
                None,
                Some(&visible_items),
            )
            .unwrap_or_else(|| serde_json::json!({}));
        if let Some(key) = self.taskbar_surface_key() {
            if let Some(snapshot) = self.plugin_preferences(&key.plugin_id) {
                data["preferences"] = snapshot;
            }
            if let Some(snapshot) = self.plugin_appearance(&key.plugin_id, false) {
                data["appearance"] = snapshot;
            }
            if let Some(snapshot) = self.plugin_appearance(&key.plugin_id, true) {
                data["wallpaper"] = snapshot;
            }
        }
        (data.to_string(), images)
    }
}

#[cfg(test)]
mod output_identification_generation_tests {
    use super::output_identification_is_current;

    #[test]
    fn stale_expiry_cannot_clear_a_replacement_badge() {
        assert!(output_identification_is_current(Some(8), 8));
        assert!(!output_identification_is_current(Some(9), 8));
        assert!(!output_identification_is_current(None, 8));
    }
}

fn supported_projection_modes(
    _session_host: &dyn SessionHost,
) -> Vec<nickel_core::display_projection::ProjectionMode> {
    #[cfg(target_os = "linux")]
    {
        use nickel_core::display_projection::{ProjectionChooser, ProjectionOutput};
        let Ok(outputs) = _session_host.projection_outputs() else {
            return Vec::new();
        };
        let outputs = outputs
            .into_iter()
            .map(|output| ProjectionOutput {
                internal: output.name.starts_with("eDP") || output.name.starts_with("LVDS"),
                name: output.name,
                width: output.geometry.width,
                height: output.geometry.height,
                scale: nickel_core::dpi::Scale120::new(output.scale_120).unwrap_or_default(),
            })
            .collect::<Vec<_>>();
        ProjectionChooser::supported(&outputs)
    }
    #[cfg(not(target_os = "linux"))]
    {
        Vec::new()
    }
}

fn window_belongs_to_panel(
    all_windows: bool,
    panel_output: Option<&str>,
    window_output: Option<&str>,
) -> bool {
    all_windows
        || panel_output.is_none()
        || window_output.is_none()
        || panel_output == window_output
}

impl LiveShell {
    fn sync_control_host(&mut self, width: u32, height: u32) {
        self.control_host
            .application_mut()
            .set_palette(self.palette);
        let supported_projection_modes = supported_projection_modes(self.session_host.as_ref());
        self.control_host.application_mut().sync(
            &self.network,
            &self.bluetooth,
            &self.audio,
            &self.workspaces,
            &supported_projection_modes,
        );
        self.step_control_host(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }

    fn control_plugin_action_allowed(&self, action: &ControlAction) -> bool {
        if !self.control_visible || self.control_plugin_host_ref().is_none() {
            return false;
        }
        match action {
            ControlAction::SetWifiEnabled(enabled) => {
                self.network.available && *enabled != self.network.enabled
            }
            ControlAction::ActivateWifi { id } => self
                .network
                .networks
                .iter()
                .take(8)
                .any(|item| item.id == *id && item.saved && !item.connected),
            ControlAction::SetBluetoothPowered(powered) => {
                self.bluetooth.available && *powered != self.bluetooth.powered
            }
            ControlAction::SetBluetoothDiscovery(discovering) => {
                self.bluetooth.available
                    && self.bluetooth.powered
                    && *discovering != self.bluetooth.discovering
            }
            ControlAction::ToggleBluetoothDevice { id } => {
                self.bluetooth.devices.iter().take(8).any(|item| {
                    item.id == *id
                        && if cfg!(target_os = "windows") {
                            !item.paired
                        } else {
                            item.paired
                        }
                })
            }
            ControlAction::SetAudioMuted(muted) => {
                self.audio.available && *muted != self.audio.muted
            }
            ControlAction::SetAudioVolume(percent) => self.audio.available && *percent <= 100,
            ControlAction::SelectAudioDevice { id } => {
                self.audio.devices.iter().take(8).any(|item| item.id == *id)
            }
            ControlAction::SwitchWorkspace(id) => {
                self.workspaces.iter().take(10).any(|item| item.id == *id)
            }
            ControlAction::CreateWorkspace => true,
            ControlAction::RemoveWorkspace(id) => {
                self.workspaces.len() > 1
                    && self
                        .workspaces
                        .iter()
                        .any(|item| item.id == *id && item.active)
            }
            ControlAction::ToggleShowDesktop | ControlAction::ShowNotifications => true,
            ControlAction::PreviewProjection(mode) => {
                self.control_host
                    .application()
                    .view_state()
                    .pending_projection
                    .is_none()
                    && supported_projection_modes(self.session_host.as_ref()).contains(mode)
            }
            ControlAction::ConfirmProjection | ControlAction::CancelProjection => self
                .control_host
                .application()
                .view_state()
                .pending_projection
                .is_some(),
            ControlAction::SessionAction(platform::SessionAction::Lock) => true,
            ControlAction::RequestSessionAction(_) => self
                .control_host
                .application()
                .view_state()
                .pending_session_action
                .is_none(),
            ControlAction::ConfirmSessionAction | ControlAction::CancelSessionAction => self
                .control_host
                .application()
                .view_state()
                .pending_session_action
                .is_some(),
            _ => false,
        }
    }

    fn control_plugin_data(&self, height: u32) -> serde_json::Value {
        use nickel_core::display_projection::ProjectionMode;
        let bounded = |value: &str| value.chars().take(120).collect::<String>();
        let slots = self
            .plugin_slot_projection(&crate::plugin_panel::control_center_manifest().id)
            .unwrap_or_else(|| serde_json::json!({}));
        let modes = supported_projection_modes(self.session_host.as_ref())
            .into_iter()
            .map(|mode| match mode {
                ProjectionMode::InternalOnly => {
                    serde_json::json!({"id":"internal","label":"Internal"})
                }
                ProjectionMode::Duplicate => {
                    serde_json::json!({"id":"duplicate","label":"Duplicate"})
                }
                ProjectionMode::Extend => serde_json::json!({"id":"extend","label":"Extend"}),
                ProjectionMode::ExternalOnly => {
                    serde_json::json!({"id":"external","label":"External"})
                }
            })
            .collect::<Vec<_>>();
        serde_json::json!({
            "height": height.clamp(1, 8192),
            "scrollHeight": height.saturating_sub(64).clamp(1, 8192),
            "network": {
                "available": self.network.available,
                "enabled": self.network.enabled,
                "networks": self.network.networks.iter().take(8).filter(|item| !item.id.is_empty() && item.id.len() <= 256).map(|item| serde_json::json!({
                    "id": item.id, "name": bounded(&item.name),
                    "saved": item.saved, "connected": item.connected,
                })).collect::<Vec<_>>(),
            },
            "bluetooth": {
                "available": self.bluetooth.available,
                "powered": self.bluetooth.powered,
                "discovering": self.bluetooth.discovering,
                "devices": self.bluetooth.devices.iter().take(8).filter(|item| !item.id.is_empty() && item.id.len() <= 256).map(|item| serde_json::json!({
                    "id": item.id, "name": bounded(&item.name),
                    "paired": item.paired, "connected": item.connected,
                })).collect::<Vec<_>>(),
            },
            "audio": {
                "available": self.audio.available,
                "percent": self.audio.volume_percent.min(100),
                "muted": self.audio.muted,
                "devices": self.audio.devices.iter().take(8).filter(|item| !item.id.is_empty() && item.id.len() <= 256).map(|item| serde_json::json!({
                    "id": item.id, "name": bounded(&item.name),
                    "isDefault": item.is_default,
                })).collect::<Vec<_>>(),
            },
            "workspaces": self.workspaces.iter().take(10).map(|item| serde_json::json!({
                "id": item.id, "active": item.active,
            })).collect::<Vec<_>>(),
            "activeWorkspace": self.workspaces.iter().find(|item| item.active).map(|item| item.id),
            "projectionModes": modes,
            "pendingProjection": self.control_host.application().view_state().pending_projection.is_some(),
            "slots": slots,
        })
    }

    fn control_plugin_active(&self) -> bool {
        self.control_plugin_host_ref().is_some()
            && !self.control_host.application().view_state().projection_only
    }

    pub(crate) fn control_surface_available(&self) -> bool {
        self.control_plugin_host_ref().is_some()
            || self.control_host.application().view_state().projection_only
    }

    fn control_plugin_event(
        &mut self,
        event: HostEvent,
        size: (u32, u32),
        limit: Option<usize>,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> nickel_ui::HostEventOutcome {
        let data = self.control_plugin_data(size.1);
        let Some(host) = self.control_plugin_host_mut() else {
            return nickel_ui::HostEventOutcome::default();
        };
        let changed = match host.application_mut().sync_data(&data) {
            Ok(changed) => changed,
            Err(error) => {
                self.fail_control_plugin_runtime(error);
                return nickel_ui::HostEventOutcome::default();
            }
        };
        let mut outcome = host.step(HostBatch {
            application_changed: changed,
            surface_size: Some(size),
            clipboard_text_limit: limit,
            events: vec![event],
            normalized_authorities: authority.into_iter().collect(),
            ..HostBatch::default()
        });
        let effects = host.application_mut().take_effects();
        if let Some(error) = host.application_mut().take_runtime_failure() {
            self.fail_control_plugin_runtime(error);
            return nickel_ui::HostEventOutcome::default();
        }
        outcome.changed |= self.apply_plugin_effects(effects);
        outcome
    }

    fn step_control_host(&mut self, batch: HostBatch) -> bool {
        let outcome = self.control_host.step(batch);
        self.host_runtime_samples.record(outcome.telemetry);
        let changed = outcome.change_token != self.control_change_token;
        self.control_change_token = outcome.change_token;
        self.control_deadline = outcome.next_deadline;
        changed
    }

    fn apply_control_effects(&mut self) {
        let effects = self.control_host.application_mut().take_effects();
        for action in effects {
            self.apply_control_action(action);
        }
    }

    fn apply_launcher_action(&mut self, action: LauncherAction) {
        match &action {
            LauncherAction::SetQuery(_) => self.launcher_plugin_result_page = 0,
            LauncherAction::SetView(_) => self.launcher_plugin_dashboard_page = 0,
            _ => {}
        }
        let Some(effect) = reduce_launcher_action(&mut self.launcher, action) else {
            return;
        };
        self.apply_launcher_effect(effect);
    }

    fn apply_launcher_effect(&mut self, effect: LauncherShellEffect) {
        match effect {
            LauncherShellEffect::ActivateResult(index) => self.launch_result(index),
            LauncherShellEffect::TogglePin(id) => {
                self.launcher.toggle_pin(&id);
                self.persist_launcher_preferences();
            }
            LauncherShellEffect::RetryPreferencePersistence => {
                self.persist_launcher_preferences();
            }
            LauncherShellEffect::OpenProject(id) => {
                self.set_launcher_visible(false);
                self.requested_codex_project = Some(id);
            }
            LauncherShellEffect::SeeAllProjects => {
                self.set_launcher_visible(false);
                self.codex_project_menu_visible =
                    self.plugin_surface_matches(&crate::plugin_panel::codex_projects_surface_key());
            }
            LauncherShellEffect::RequestLogout => {
                self.set_control_visible(true);
                if self.control_visible {
                    self.set_launcher_visible(false);
                    self.control_host
                        .application_mut()
                        .request_session_action(platform::SessionAction::LogOut);
                    self.step_control_host(HostBatch {
                        events: vec![HostEvent::Poll],
                        ..HostBatch::default()
                    });
                }
            }
            LauncherShellEffect::Dismiss => self.set_launcher_visible(false),
        }
    }

    fn persist_launcher_preferences(&mut self) {
        self.persist_launcher_preferences_with_recent(None);
    }

    fn persist_launcher_preferences_with_recent(&mut self, recent: Option<String>) {
        #[cfg(test)]
        {
            self.launcher_persistence_attempts += 1;
        }
        #[cfg(test)]
        let path = self
            .launcher_preferences_path
            .clone()
            .map(Ok)
            .unwrap_or_else(nickel_core::launcher_preferences::preferences_path);
        #[cfg(not(test))]
        let path = nickel_core::launcher_preferences::preferences_path();
        let result = path
            .map_err(|_| "preference path unavailable")
            .and_then(|path| {
                self.launcher_preference_persistence.enqueue(
                    path,
                    self.launcher.preferences().clone(),
                    recent,
                )
            });
        if let Err(error) = result {
            self.launcher_status =
                Some(format!("Launcher preferences could not be saved: {error}"));
        }
        self.launcher_preference_deadline = self
            .launcher_preference_persistence
            .busy()
            .then(|| Instant::now() + Duration::from_millis(16));
    }

    fn poll_launcher_preferences(&mut self) -> bool {
        let result = self.launcher_preference_persistence.poll();
        self.launcher_preference_deadline = self
            .launcher_preference_persistence
            .busy()
            .then(|| Instant::now() + Duration::from_millis(16));
        let Some((preferences, failed)) = result else {
            return false;
        };
        if !failed {
            self.launcher.set_preferences(preferences);
        }
        if failed {
            self.launcher_status = Some("Launcher preferences could not be saved: configuration changed or storage unavailable".into());
        } else if self
            .launcher_status
            .as_deref()
            .is_some_and(|status| status.starts_with("Launcher preferences could not be saved:"))
        {
            self.launcher_status = None;
        }
        true
    }

    pub(crate) fn launcher_favorites_match(&self, preferences: &LauncherPreferences) -> bool {
        self.launcher.preferences().favorites() == preferences.favorites()
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn launcher_favorite_catalog(
        &self,
    ) -> Result<crate::windows_remote_launcher_favorites::Catalog, String> {
        crate::windows_remote_launcher_favorites::Catalog::current(
            self.launcher_catalog_generation,
            self.launcher
                .discovered_applications()
                .map(|application| application.id().to_owned()),
        )
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn preferred_application_catalog(
        &self,
    ) -> Result<crate::remote_preferred_applications::Catalog, String> {
        crate::remote_preferred_applications::Catalog::new(
            self.launcher_catalog_generation,
            self.launcher
                .discovered_applications()
                .filter_map(|application| {
                    Some((
                        application.id().to_owned(),
                        application.launch_command()?.first()?.clone(),
                    ))
                }),
        )
    }

    pub(crate) fn launcher_preferences_busy(&self) -> bool {
        self.launcher_preference_persistence.busy()
    }

    pub(crate) fn apply_committed_launcher_preferences(
        &mut self,
        preferences: LauncherPreferences,
    ) -> Result<(), &'static str> {
        self.launcher_preference_persistence
            .replace_committed(preferences.clone())?;
        self.launcher.set_preferences(preferences);
        let _ = self.refresh_fast_changes();
        Ok(())
    }

    fn apply_control_action(&mut self, action: ControlAction) {
        match action {
            ControlAction::ToggleWifiSection | ControlAction::WifiScroll => {}
            ControlAction::SetWifiEnabled(enabled) => {
                log_control_result("set-wifi-enabled", platform::set_wifi_enabled(enabled));
            }
            ControlAction::ActivateWifi { id } => {
                log_control_result(
                    "activate-wifi-network",
                    platform::activate_wifi_network(&id),
                );
            }
            ControlAction::ToggleBluetoothSection | ControlAction::BluetoothScroll => {}
            ControlAction::SetBluetoothPowered(powered) => {
                log_control_result(
                    "set-bluetooth-powered",
                    platform::set_bluetooth_powered(powered),
                );
            }
            ControlAction::SetBluetoothDiscovery(discovering) => {
                log_control_result(
                    "set-bluetooth-discovery",
                    platform::set_bluetooth_discovery(discovering),
                );
            }
            ControlAction::ToggleBluetoothDevice { id } => {
                log_control_result(
                    "toggle-bluetooth-device",
                    platform::toggle_bluetooth_device(&id),
                );
            }
            ControlAction::ToggleAudioSection | ControlAction::AudioScroll => {}
            ControlAction::SetAudioMuted(muted) => {
                if platform::audio_status().muted != muted {
                    platform::handle_consumer_control(
                        nickel_session_protocol::ConsumerControl::VolumeMute,
                    );
                }
            }
            ControlAction::SetAudioVolume(volume) => {
                log_control_result("set-audio-volume", platform::set_audio_volume(volume));
            }
            ControlAction::SelectAudioDevice { id } => {
                log_control_result("select-audio-device", platform::select_audio_device(&id));
            }
            ControlAction::SwitchWorkspace(workspace) => {
                let _ = self.send_session_command(
                    "switch-workspace",
                    ShellCommand::SwitchWorkspace(workspace),
                );
            }
            ControlAction::CreateWorkspace => {
                let _ =
                    self.send_session_command("create-workspace", ShellCommand::CreateWorkspace);
            }
            ControlAction::ToggleShowDesktop => {
                let _ = self
                    .send_session_command("toggle-show-desktop", ShellCommand::ToggleShowDesktop);
            }
            ControlAction::ShowNotifications => {
                self.global_shortcut(platform::GlobalShortcut::ShowNotifications);
            }
            ControlAction::RemoveWorkspace(workspace) => {
                let _ = self.send_session_command(
                    "remove-workspace",
                    ShellCommand::RemoveWorkspace(workspace),
                );
            }
            ControlAction::PreviewProjection(mode) => {
                if !self.preview_projection(mode) {
                    self.control_host
                        .application_mut()
                        .projection_preview_failed();
                }
            }
            ControlAction::ConfirmProjection => {
                self.projection_chooser.confirm();
                self.projection_rollback_deadline = None;
            }
            ControlAction::CancelProjection => self.rollback_projection(),
            ControlAction::RequestSessionAction(_)
            | ControlAction::CancelSessionAction
            | ControlAction::ConfirmSessionAction => {}
            ControlAction::SessionAction(action) => {
                if self.task_switcher.session().is_some() {
                    self.apply_task_switch_action(nickel_core::hotkeys::HotkeyAction::CancelSwitch);
                }
                let _ = self
                    .send_session_command("session-action", ShellCommand::SessionAction(action));
            }
        }
        let _ = self.refresh();
    }

    fn preview_projection(
        &mut self,
        mode: nickel_core::display_projection::ProjectionMode,
    ) -> bool {
        #[cfg(target_os = "linux")]
        {
            use nickel_core::display_projection::{
                ProjectionChooser, ProjectionOutput, ProjectionPlacement,
            };
            let Ok(outputs) = self.session_host.projection_outputs() else {
                return false;
            };
            let topology = outputs
                .iter()
                .map(|output| ProjectionOutput {
                    name: output.name.clone(),
                    internal: output.name.starts_with("eDP") || output.name.starts_with("LVDS"),
                    width: output.geometry.width,
                    height: output.geometry.height,
                    scale: nickel_core::dpi::Scale120::new(output.scale_120).unwrap_or_default(),
                })
                .collect::<Vec<_>>();
            let Some(plan) = ProjectionChooser::plan(mode, &topology) else {
                return false;
            };
            let previous = outputs
                .iter()
                .map(|output| ProjectionPlacement {
                    name: output.name.clone(),
                    x: output.geometry.x,
                    y: output.geometry.y,
                    enabled: output.enabled,
                    scale: nickel_core::dpi::Scale120::new(output.scale_120).unwrap_or_default(),
                })
                .collect();
            let primary = outputs
                .iter()
                .find(|output| {
                    output.primary
                        && plan
                            .placements
                            .iter()
                            .any(|entry| entry.name == output.name && entry.enabled)
                })
                .or_else(|| {
                    outputs.iter().find(|output| {
                        plan.placements
                            .iter()
                            .any(|entry| entry.name == output.name && entry.enabled)
                    })
                })
                .map(|output| output.name.clone())
                .unwrap_or_default();
            let layout = nickel_session_protocol::OutputLayout {
                primary,
                placements: plan
                    .placements
                    .iter()
                    .map(|entry| nickel_session_protocol::OutputPlacement {
                        name: entry.name.clone(),
                        x: entry.x,
                        y: entry.y,
                        enabled: entry.enabled,
                        scale_120: entry.scale.units(),
                        mode: None,
                    })
                    .collect(),
            };
            if self.send_session_command("preview-projection", ShellCommand::ApplyOutputs(layout)) {
                self.projection_chooser.preview(previous, plan);
                self.projection_rollback_deadline = Some(Instant::now() + Duration::from_secs(15));
                return true;
            }
            false
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = mode;
            false
        }
    }

    fn rollback_projection(&mut self) {
        self.projection_rollback_deadline = None;
        #[cfg(target_os = "linux")]
        if let Some(mut previous) = self.projection_chooser.rollback() {
            if let Ok(outputs) = self.session_host.projection_outputs() {
                previous.retain(|entry| outputs.iter().any(|output| output.name == entry.name));
                if !previous.iter().any(|entry| entry.enabled)
                    && let Some(first) = previous.first_mut()
                {
                    first.enabled = true;
                }
            }
            if previous.is_empty() {
                return;
            }
            let primary = previous
                .iter()
                .find(|entry| entry.enabled)
                .map(|entry| entry.name.clone())
                .unwrap_or_default();
            let layout = nickel_session_protocol::OutputLayout {
                primary,
                placements: previous
                    .into_iter()
                    .map(|entry| nickel_session_protocol::OutputPlacement {
                        name: entry.name,
                        x: entry.x,
                        y: entry.y,
                        enabled: entry.enabled,
                        scale_120: entry.scale.units(),
                        mode: None,
                    })
                    .collect(),
            };
            let _ = self
                .send_session_command("rollback-projection", ShellCommand::ApplyOutputs(layout));
        }
    }

    fn preview_plugin_display_layout(
        &mut self,
        plugin_id: String,
        layout: nickel_session_protocol::OutputLayout,
    ) -> bool {
        #[cfg(target_os = "linux")]
        {
            if self.display_preview.is_some() || self.projection_rollback_deadline.is_some() {
                return false;
            }
            let Ok(outputs) = self.session_host.projection_outputs() else {
                return false;
            };
            if validate_plugin_display_layout(&outputs, &layout).is_err() {
                return false;
            }
            let previous = output_layout_from_snapshot(&outputs);
            if validate_plugin_display_layout(&outputs, &previous).is_err() {
                return false;
            }
            if self.send_session_command(
                "preview-plugin-display-layout",
                ShellCommand::ApplyOutputs(layout.clone()),
            ) {
                self.display_preview = Some(DisplayPreview {
                    owner: plugin_id,
                    previous,
                    applied: normalized_output_layout_with_modes(layout, &outputs),
                    deadline: Instant::now() + Duration::from_secs(15),
                });
                return true;
            }
            false
        }
        #[cfg(target_os = "windows")]
        {
            match crate::windows_plugin_display::set_layout(&plugin_id, &layout) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "plugin display layout preview failed");
                    false
                }
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (plugin_id, layout);
            false
        }
    }

    fn confirm_plugin_display_layout(&mut self, plugin_id: &str) -> bool {
        #[cfg(target_os = "windows")]
        {
            return match crate::windows_plugin_display::confirm(plugin_id) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "plugin display layout confirmation failed");
                    false
                }
            };
        }
        if self
            .display_preview
            .as_ref()
            .is_some_and(|preview| preview.owner == plugin_id && Instant::now() < preview.deadline)
        {
            self.display_preview = None;
            return true;
        }
        false
    }

    fn revert_plugin_display_layout(&mut self, plugin_id: Option<&str>) -> bool {
        #[cfg(target_os = "linux")]
        {
            let Some(preview) = self.display_preview.as_ref() else {
                return false;
            };
            if plugin_id.is_some_and(|owner| owner != preview.owner) {
                return false;
            }
            let Ok(outputs) = self.session_host.projection_outputs() else {
                return false;
            };
            // A separate display owner may have changed the topology during the
            // preview. Never overwrite a layout that is no longer our preview.
            let current = output_layout_from_snapshot(&outputs);
            if current != preview.applied && current != preview.previous {
                self.display_preview = None;
                return false;
            }
            if validate_plugin_display_layout(&outputs, &preview.previous).is_err() {
                self.display_preview = None;
                return false;
            }
            let previous = preview.previous.clone();
            if self.send_session_command(
                "revert-plugin-display-layout",
                ShellCommand::ApplyOutputs(previous),
            ) {
                self.display_preview = None;
                return true;
            }
            false
        }
        #[cfg(target_os = "windows")]
        {
            let Some(plugin_id) = plugin_id else {
                return false;
            };
            match crate::windows_plugin_display::revert(plugin_id) {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "plugin display layout recovery failed");
                    false
                }
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = plugin_id;
            false
        }
    }

    pub(crate) fn dispatch_session_command(
        &self,
        operation: &'static str,
        command: ShellCommand,
    ) -> bool {
        match self.session_host.dispatch(command) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(operation, %error, "session command failed");
                false
            }
        }
    }

    fn send_session_command(&self, operation: &'static str, command: ShellCommand) -> bool {
        self.dispatch_session_command(operation, command)
    }
}

fn log_control_result(operation: &'static str, succeeded: bool) {
    if !succeeded {
        tracing::warn!(operation, "control action failed");
    }
}

fn secure_storage_status_label(state: platform::SecureStorageState) -> Option<&'static str> {
    match state {
        platform::SecureStorageState::Starting => Some("Secure storage is starting…"),
        platform::SecureStorageState::Locked => Some("Secure storage is locked."),
        platform::SecureStorageState::PromptRequired => {
            Some("Secure storage is waiting for its unlock prompt.")
        }
        platform::SecureStorageState::Unavailable => Some("Secure storage is unavailable."),
        platform::SecureStorageState::UnavailableReason(reason) => Some(match reason {
            nickel_session_protocol::SecureStorageUnavailableReason::Connection => {
                "Secure storage cannot connect to the session bus."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::MissingDefaultCollection => {
                "Secure storage has no default collection."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::PromptTimedOut => {
                "The secure-storage unlock prompt timed out."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::ProviderDisappeared => {
                "The secure-storage provider disappeared."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::ProviderConfiguration
            | nickel_session_protocol::SecureStorageUnavailableReason::UnexpectedProvider => {
                "The secure-storage provider configuration is invalid."
            }
            nickel_session_protocol::SecureStorageUnavailableReason::Protocol
            | nickel_session_protocol::SecureStorageUnavailableReason::ReadinessCheck => {
                "Secure storage failed its readiness check."
            }
        }),
        platform::SecureStorageState::ControlUnavailable => {
            Some("Nickel cannot reach the session service.")
        }
        platform::SecureStorageState::Ready => None,
    }
}

fn update_feed_status(current: &mut FeedStatus, next: FeedStatus, feed: &'static str) -> bool {
    if *current == next {
        return false;
    }
    tracing::info!(feed, status = ?next, "shell feed state changed");
    *current = next;
    true
}

fn session_feed_status_label(
    window_status: FeedStatus,
    workspace_status: FeedStatus,
) -> Option<&'static str> {
    match (window_status, workspace_status) {
        (FeedStatus::Loading, FeedStatus::Loading) => Some("Loading session data…"),
        (FeedStatus::Loading, _) => Some("Loading session windows…"),
        (_, FeedStatus::Loading) => Some("Loading session workspaces…"),
        (FeedStatus::Disconnected, _) => Some("Session window data is disconnected."),
        (_, FeedStatus::Disconnected) => Some("Session workspace data is disconnected."),
        (FeedStatus::Failed, _) => Some("Session window data failed to load."),
        (_, FeedStatus::Failed) => Some("Session workspace data failed to load."),
        (FeedStatus::Ready, FeedStatus::Ready) => None,
    }
}

fn application_discovery_status_label(
    status: crate::model::ApplicationDiscoveryStatus,
) -> Option<String> {
    let localizer = nickel_i18n::Localizer::system();
    match status {
        crate::model::ApplicationDiscoveryStatus::ReadyEmpty => {
            Some(localizer.text("launcher-discovery-empty"))
        }
        crate::model::ApplicationDiscoveryStatus::Ready => None,
        crate::model::ApplicationDiscoveryStatus::PartialFailure => {
            Some(localizer.text("launcher-discovery-partial"))
        }
    }
}

/// Pick desktop-label ink from the pixels which are actually behind that label.
///
/// The desktop image is stretched to the output host, so output origins and
/// physical scale have already been projected away when this receives the
/// host-local rectangle. Transparent image pixels are composited over the
/// themed desktop base, followed by any selected/hover/focus tile surface.
fn desktop_label_foreground(
    wallpaper: Option<&image::RgbaImage>,
    viewport: Size,
    label: Rect,
    desktop_base: nickel_ui::Color,
    interaction_surface: Option<nickel_ui::Color>,
) -> nickel_ui::Color {
    const DARK_INK: nickel_ui::Color = 0x111111;
    const LIGHT_INK: nickel_ui::Color = 0xffffff;
    const SAMPLE_COLUMNS: usize = 9;
    const SAMPLE_ROWS: usize = 5;

    let base = color_channels(desktop_base);
    let mut dark = [0.0; SAMPLE_COLUMNS * SAMPLE_ROWS];
    let mut light = [0.0; SAMPLE_COLUMNS * SAMPLE_ROWS];
    let mut sample = 0;
    for row in 0..SAMPLE_ROWS {
        for column in 0..SAMPLE_COLUMNS {
            let x =
                label.origin.x + label.size.width * (column as f32 + 0.5) / SAMPLE_COLUMNS as f32;
            let y = label.origin.y + label.size.height * (row as f32 + 0.5) / SAMPLE_ROWS as f32;
            let mut pixel = wallpaper
                .filter(|image| {
                    image.width() > 0
                        && image.height() > 0
                        && viewport.width > 0.0
                        && viewport.height > 0.0
                })
                .map(|image| {
                    let source_x = ((x / viewport.width).clamp(0.0, 1.0)
                        * image.width().saturating_sub(1) as f32)
                        .round() as u32;
                    let source_y = ((y / viewport.height).clamp(0.0, 1.0)
                        * image.height().saturating_sub(1) as f32)
                        .round() as u32;
                    let source = image.get_pixel(source_x, source_y).0;
                    composite_rgba([source[0], source[1], source[2], source[3]], base)
                })
                .unwrap_or(base);
            if let Some(surface) = interaction_surface {
                pixel = composite_rgba(color_channels(surface), pixel);
            }
            let luminance = rgba_luminance(pixel);
            dark[sample] = (luminance + 0.05) / 0.05;
            light[sample] = 1.05 / (luminance + 0.05);
            sample += 1;
        }
    }

    // A low percentile keeps a small bright/dark detail from controlling the
    // whole label while still preferring ink that survives a mixed image.
    dark.sort_by(f32::total_cmp);
    light.sort_by(f32::total_cmp);
    let percentile = dark.len() / 5;
    let dark_score = (dark[percentile], dark.iter().sum::<f32>());
    let light_score = (light[percentile], light.iter().sum::<f32>());
    if dark_score >= light_score {
        DARK_INK
    } else {
        LIGHT_INK
    }
}

fn color_channels(color: nickel_ui::Color) -> [u8; 4] {
    let encoded_alpha = ((color >> 24) & 0xff) as u8;
    [
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
        if color <= 0x00ff_ffff {
            255
        } else {
            encoded_alpha
        },
    ]
}

fn composite_rgba(foreground: [u8; 4], background: [u8; 4]) -> [u8; 4] {
    let alpha = foreground[3] as f32 / 255.0;
    let blend = |index| {
        (foreground[index] as f32 * alpha + background[index] as f32 * (1.0 - alpha)).round() as u8
    };
    [blend(0), blend(1), blend(2), 255]
}

fn rgba_luminance(pixel: [u8; 4]) -> f32 {
    let channel = |value: u8| {
        let value = value as f32 / 255.0;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * channel(pixel[0]) + 0.7152 * channel(pixel[1]) + 0.0722 * channel(pixel[2])
}

fn launch_error_summary(error: &platform::LaunchError) -> String {
    match error {
        platform::LaunchError::EmptyCommand => "no launch command".into(),
        platform::LaunchError::InvalidQuotes => "invalid launch command quoting".into(),
        platform::LaunchError::MissingTarget(target) => format!("missing target {target}"),
        platform::LaunchError::NotFound(target) => format!("target not found: {target}"),
        platform::LaunchError::PathNotFound(path) => format!("path not found: {path}"),
        platform::LaunchError::AccessDenied(target) => format!("access denied: {target}"),
        platform::LaunchError::NoAssociation(target) => {
            format!("no application association for {target}")
        }
        platform::LaunchError::Platform(message) => message.clone(),
    }
}

fn wallpaper_cache_target(current: (u32, u32), requested: (u32, u32)) -> Option<(u32, u32)> {
    let bounded = (
        requested.0.clamp(1, WALLPAPER_MAX_WIDTH),
        requested.1.clamp(1, WALLPAPER_MAX_HEIGHT),
    );
    if current == (0, 0) || bounded.0 > current.0 || bounded.1 > current.1 {
        return Some(bounded);
    }
    let current_pixels = u64::from(current.0) * u64::from(current.1);
    let requested_pixels = u64::from(bounded.0) * u64::from(bounded.1);
    (requested_pixels.saturating_mul(4) <= current_pixels).then_some(bounded)
}

fn initial_wallpaper(
    configured_path: Option<std::path::PathBuf>,
    system_image: impl FnOnce() -> Option<image::RgbaImage>,
) -> (
    Option<std::path::PathBuf>,
    Option<Arc<image::RgbaImage>>,
    (u32, u32),
) {
    if configured_path.is_some() {
        return (configured_path, None, (0, 0));
    }
    let wallpaper = system_image().map(Arc::new);
    let size = wallpaper
        .as_deref()
        .map_or((0, 0), |image| image.dimensions());
    (None, wallpaper, size)
}

fn update_preview_image(
    images: &mut HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    window: crate::model::WindowId,
    image: image::RgbaImage,
) -> bool {
    if images
        .get(&window)
        .is_some_and(|current| **current == image)
    {
        return false;
    }
    images.insert(window, Arc::new(image));
    true
}

// Historical copying baseline for release comparisons, never a production path.
#[cfg(test)]
fn legacy_preview_copy(image: &image::RgbaImage) -> image::RgbaImage {
    image.clone()
}

fn retain_preview_generation(
    images: &mut HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    windows: &[OpenWindow],
) {
    images.retain(|window, _| {
        windows
            .iter()
            .take(PREVIEW_CACHE_CAPACITY)
            .any(|candidate| candidate.id == *window)
    });
}

#[cfg(test)]
#[path = "live_shell/tests.rs"]
mod tests;
