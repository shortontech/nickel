mod preference_persistence;
mod shell_selection;

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
    ShellPopoverAnchor, ShellSemanticTarget,
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
    control_view::ControlAction,
    file_window_host::{FileWindowHost, default_file_window_host},
    launcher::{DashboardAccount, DashboardProject, DashboardSection, Launcher},
    launcher_icon_cache::LauncherIconCache,
    model::{Application, OpenWindow, TrayItem, WindowGroup},
    notification::DesktopNotification,
    notification_view::{NotificationApp, NotificationEffect, NotificationHost},
    platform::{
        self, AudioStatus, BluetoothStatus, FeedState, FeedStatus, NetworkStatus, NotificationFeed,
        NotificationSource, ShellCommand, TrayFeed, TraySource, WindowAction, WindowFeed,
    },
    projection_recovery::{ProjectionRecoveryApp, ProjectionRecoveryHost},
    screenshot::ScreenshotTool,
    session_host::{SessionHost, default_session_host},
    window_preview::{
        PreviewAction, preview_dimensions, semantic_theme_from_palette, task_switcher_dimensions,
    },
    winit_shell::SurfaceRole,
};

use nickel_input::KeyCode;
#[cfg(not(target_os = "windows"))]
use zeroize::Zeroize;
use zeroize::Zeroizing;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum CodexApprovalOwner {
    // Internal surfaces are owned by the Linux compositor. Windows retains
    // this variant in fixtures that verify cross-platform approval identity.
    #[cfg_attr(all(target_os = "windows", not(test)), allow(dead_code))]
    Internal(InternalSurfaceId),
    Winit(crate::winit_shell::SurfaceId),
}

const PANEL_TRAY_ICON_SIZE: u32 = 18;
const PREVIEW_LEAVE_DELAY: Duration = Duration::from_millis(500);
const PREVIEW_HOVER_DELAY: Duration = Duration::from_millis(300);
const PREVIEW_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "linux")]
const RECURRING_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(30);
const WALLPAPER_MAX_WIDTH: u32 = 7680;
const WALLPAPER_MAX_HEIGHT: u32 = 4320;
const PREVIEW_CACHE_CAPACITY: usize = 32;

#[path = "live_shell/icon_resources.rs"]
mod icon_resources;
pub(crate) mod remote_semantics;
use icon_resources::{normalize_tray_items, panel_tray_icons, tint_panel_icon};

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
use desktop::SettingsDestination;
#[cfg(test)]
use desktop::retain_unchanged_desktop_icons;
#[allow(unused_imports)]
pub use desktop::{DesktopApplication, DesktopCommand, DesktopMessage};

struct WallpaperChooserRequest {
    plugin_id: String,
    identity: nickel_core::plugins::PluginManifest,
    activation: u64,
    effect: crate::appearance_capabilities::AppearanceEffect,
    receiver: std::sync::mpsc::Receiver<nickel_platform::FileDialogOutcome>,
}

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
                        .color(password_color),
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
                transform: Some(output.transform),
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
        if placement.transform.is_none() {
            placement.transform = outputs
                .iter()
                .find(|output| output.name == placement.name)
                .map(|output| output.transform);
        }
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

#[derive(Clone)]
enum RetainedPackageRuntime {
    Ordinary(std::rc::Rc<std::cell::RefCell<nickel_plugin_runtime::JsxRuntime>>),
    Composed(
        std::rc::Rc<
            std::cell::RefCell<nickel_plugin_runtime::composition_runtime::ShellCompositionRuntime>,
        >,
    ),
}

impl RetainedPackageRuntime {
    fn contexts(
        &self,
        active: &str,
    ) -> std::collections::BTreeMap<
        String,
        std::rc::Rc<std::cell::RefCell<nickel_plugin_runtime::JsxRuntime>>,
    > {
        match self {
            Self::Ordinary(runtime) => {
                std::collections::BTreeMap::from([(active.to_owned(), runtime.clone())])
            }
            Self::Composed(host) => {
                let host = host.borrow();
                host.participating_owners()
                    .filter_map(|owner| {
                        host.shared_owner_runtime(owner)
                            .ok()
                            .map(|runtime| (owner.id.clone(), runtime))
                    })
                    .collect()
            }
        }
    }
}

pub struct LiveShell {
    application_scale_service: crate::application_scale_capability::ApplicationScaleService,
    appearance_capabilities: crate::appearance_capabilities::AppearanceCapabilities,
    wallpaper_chooser: Option<WallpaperChooserRequest>,
    wallpaper_chooser_results: std::collections::BTreeMap<String, serde_json::Value>,
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
    plugins_results: HashMap<String, serde_json::Value>,
    shell_selection_preview: Option<shell_selection::ShellPreview>,
    shell_preview_sequence: u64,
    confirmed_shell_package_id: String,
    settings_navigation: Option<serde_json::Value>,
    settings_navigation_revision: u64,
    volume_osd_until: Option<Instant>,
    launcher_visible: bool,
    run_status: HashMap<String, String>,
    locked: bool,
    lock_host: nickel_ui::UiHost<LockApplication>,
    lock_change_token: HostChangeToken,
    lock_deadline: Option<Instant>,
    control_visible: bool,
    codex_project_menu_visible: bool,
    plugin_registry: nickel_core::plugins::PluginRegistry,
    package_settings_registry: nickel_core::settings_registry::SettingsRegistry,
    package_settings_runtimes: std::collections::BTreeMap<
        String,
        std::rc::Rc<std::cell::RefCell<nickel_plugin_runtime::JsxRuntime>>,
    >,
    active_shell_package_id: String,
    package_runtimes: std::collections::BTreeMap<String, RetainedPackageRuntime>,
    package_settings_generation: u64,
    package_settings_values: nickel_plugin_runtime::settings::SettingsValueSnapshot,
    package_settings_value_revisions: std::collections::BTreeMap<String, u64>,
    package_settings_invoking: bool,
    plugin_settings:
        std::collections::BTreeMap<String, std::collections::BTreeMap<String, serde_json::Value>>,
    external_plugin_packages:
        std::collections::BTreeMap<String, nickel_core::plugins::PluginPackageSource>,
    application_search: crate::application_capabilities::ApplicationSearch,
    application_catalog_cache: [Option<ApplicationCatalogCache>; 2],
    #[cfg(test)]
    application_catalog_builds: u64,
    feature_client: crate::feature_capabilities::FeatureClient,
    shortcut_capability_observed: bool,
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
    plugin_pointer_paint: Option<nickel_core::plugins::PluginSurfaceKey>,
    plugin_panel_memory: std::collections::BTreeMap<nickel_core::plugins::PluginSurfaceKey, u64>,
    plugin_window_placement_overrides: std::collections::BTreeMap<
        nickel_core::plugins::PluginSurfaceKey,
        (nickel_core::plugins::PluginSurfaceAnchor, i32, i32),
    >,
    #[cfg(target_os = "windows")]
    pending_plugin_surface_focus: Option<nickel_core::plugins::PluginSurfaceKey>,
    panel_projections: HashMap<Option<String>, PanelTaskProjection>,
    clock_deadline: Instant,
    panel_output: Option<String>,
    pending_popover_anchor: Option<PendingPopoverAnchor>,
    all_windows_on_every_bar: bool,
    #[cfg(target_os = "windows")]
    idle_policy: nickel_core::idle::IdlePolicy,
    preview_group: Option<usize>,
    preview_generation: u64,
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
    notification_host: NotificationHost,
    panel_origin_x: i32,
    panel_origin_y: i32,
    control_host: ProjectionRecoveryHost,
    control_change_token: HostChangeToken,
    control_deadline: Option<Instant>,
    projection_chooser: nickel_core::display_projection::ProjectionChooser,
    projection_rollback_deadline: Option<Instant>,
    display_preview: Option<DisplayPreview>,
    launcher_icons: LauncherIconCache,
    launcher_icon_revision: u64,

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
    secure_storage_state: platform::SecureStorageState,
    #[cfg(target_os = "linux")]
    secure_storage_query_error: Option<(platform::SessionRequestError, Instant)>,
    requested_codex_project: Option<String>,
    screenshot: ScreenshotTool,
    keyboard_host: nickel_ui::UiHost<nickel_ui::on_screen_keyboard::KeyboardApp>,
    keyboard_service_generation: u64,
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

struct ApplicationCatalogCache {
    launcher_revision: Arc<()>,
    windows: Vec<OpenWindow>,
    value: serde_json::Value,
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

fn should_auto_start_installed_plugin(id: &str, desired_enabled: bool, safe_mode: bool) -> bool {
    // The selected fallback shell is started through the dedicated package
    // lifecycle below. Running it through the installed-plugin approval pass
    // again can overwrite its Running state with a stale approval failure.
    id != "nickel-default" && desired_enabled && !safe_mode
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

pub(crate) fn passive_pointer_batch(batch: &HostBatch) -> bool {
    if batch.window_focused.is_some() || batch.application_changed || batch.events.len() != 1 {
        return false;
    }
    match &batch.events[0] {
        HostEvent::Ui(UiEvent::PointerMoved(_) | UiEvent::PointerCancelled) => true,
        event => matches!(
            normalized_input(event),
            Some(nickel_input::InputEvent::Pointer(
                nickel_input::PointerEvent::Motion { .. }
                    | nickel_input::PointerEvent::Leave { .. }
            ))
        ),
    }
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
        let control_host = ProjectionRecoveryHost::new(ProjectionRecoveryApp::new(), 380, 650);
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
        let launcher_icons = LauncherIconCache::new();
        let mut plugin_registry = nickel_core::plugins::PluginRegistry::default();
        plugin_registry.register(crate::plugin_panel::manifest().clone())?;
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
            if plugin_registry.entries().count() >= 63 {
                tracing::warn!(plugin = %id, "plugin status capacity reached");
                continue;
            }
            match plugin_registry.register(descriptor.manifest.clone()) {
                Ok(()) => {
                    external_plugin_packages.insert(id, descriptor.into());
                }
                Err(error) => {
                    tracing::warn!(plugin = %id, %error, "installed plugin was not registered")
                }
            }
        }
        // Bundled source is an ordinary package, initially disabled. Persisted
        // activation still goes through the same reviewed lifecycle as disk sources.
        let package = crate::bundled_plugin_assets::load_package("nickel-default")?;
        let id = package.manifest.id.clone();
        if !external_plugin_packages.contains_key(&id) {
            plugin_registry.register(package.manifest.clone())?;
            external_plugin_packages.insert(
                id,
                nickel_core::plugins::PluginPackageSource::embedded(package),
            );
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
        let launcher_icon_revision = launcher_icons.revision();
        let mut shell = Self {
            application_scale_service: Default::default(),
            appearance_capabilities: Default::default(),
            wallpaper_chooser: None,
            wallpaper_chooser_results: Default::default(),
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
            plugins_results: HashMap::new(),
            shell_selection_preview: None,
            shell_preview_sequence: 0,
            confirmed_shell_package_id: "nickel-default".into(),
            settings_navigation: None,
            settings_navigation_revision: 0,
            launcher_visible: false,
            run_status: HashMap::new(),
            locked: false,
            lock_host,
            lock_change_token: HostChangeToken::default(),
            lock_deadline: None,
            control_visible: false,
            codex_project_menu_visible: false,
            plugin_registry,
            package_settings_registry: Default::default(),
            package_settings_runtimes: Default::default(),
            active_shell_package_id: "nickel-default".into(),
            package_runtimes: Default::default(),
            package_settings_generation: 0,
            package_settings_values: Default::default(),
            package_settings_value_revisions: Default::default(),
            package_settings_invoking: false,
            plugin_settings,
            external_plugin_packages,
            application_search: Default::default(),
            application_catalog_cache: [None, None],
            #[cfg(test)]
            application_catalog_builds: 0,
            feature_client: Default::default(),
            shortcut_capability_observed: false,
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
            plugin_pointer_paint: None,
            plugin_panel_memory: std::collections::BTreeMap::new(),
            plugin_window_placement_overrides: std::collections::BTreeMap::new(),
            #[cfg(target_os = "windows")]
            pending_plugin_surface_focus: None,
            panel_projections: HashMap::new(),
            clock_deadline: Instant::now() + crate::clock_capabilities::until_next_minute(),
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
            preview_generation: 1,
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
            secure_storage_state,
            #[cfg(target_os = "linux")]
            secure_storage_query_error,
            requested_codex_project: None,
            screenshot: ScreenshotTool::default().with_session_host(session_host),
            keyboard_host: nickel_ui::UiHost::new(
                nickel_ui::on_screen_keyboard::KeyboardApp::new(palette),
                1280,
                nickel_core::on_screen_keyboard::KEYBOARD_HEIGHT,
            ),
            keyboard_service_generation: 1,
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
        if plugin_activation.desired_enabled("nickel-default", true) || safe_mode {
            shell.set_plugin_enabled("nickel-default", true)?;
        }
        #[cfg(not(test))]
        for id in shell
            .external_plugin_packages
            .keys()
            .cloned()
            .collect::<Vec<_>>()
        {
            if should_auto_start_installed_plugin(
                &id,
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
        #[cfg(not(test))]
        if !safe_mode
            && let Some(selected) = plugin_activation.selected_shell()
            && selected != "nickel-default"
            && shell.package_runtimes.contains_key(selected)
        {
            if let Err(error) = shell.select_shell_package(selected) {
                tracing::warn!(%error, "selected shell could not start");
            } else {
                shell.confirmed_shell_package_id = selected.into();
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
            if self.window_menu.is_none() {
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
            let outcome = self.notification_host.step(HostBatch {
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            if outcome.changed || self.apply_notification_effects() {
                redraw.push(SurfaceRole::Notification);
            }
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
            changed = true;
        }
        changed
    }

    pub fn refresh_system(&mut self) -> bool {
        #[cfg(target_os = "linux")]
        let mut changed = self.refresh_secure_storage();
        #[cfg(not(target_os = "linux"))]
        let mut changed = false;
        changed |= self.poll_wallpaper_chooser();
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
                    self.show_volume_osd();
                } else if !self.audio.available
                    || (activity.availability_changed && !activity.value_changed)
                {
                    return self.hide_volume_osd() || changed;
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
            SurfaceRole::Taskbar => true,

            SurfaceRole::Launcher => true,
            SurfaceRole::ControlCenter => self
                .quick_settings_surface_active()
                .then(|| {
                    self.plugin_panel_host_ref(&self.active_shell_surface_key("quick-settings"))
                        .unwrap()
                        .remote_access_protected()
                })
                .unwrap_or_else(|| {
                    !self
                        .control_host
                        .application()
                        .view_state()
                        .trusted_visible()
                        || self.control_host.remote_access_protected()
                }),
            SurfaceRole::Notification => {
                self.trusted_notification_visible()
                    || self.notification_host.remote_access_protected()
            }
            SurfaceRole::VolumeOsd => true,
            SurfaceRole::WindowPreview => {
                self.preview_plugin_active()
                    || self
                        .preview_plugin_host_ref()
                        .is_none_or(|host| host.remote_access_protected())
            }
            SurfaceRole::WindowContextMenu => true,
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
            SurfaceRole::Taskbar => None,

            SurfaceRole::Panel => plugin
                .and_then(|key| self.plugin_panel_host_ref(key))
                .map(|host| host.layout_snapshot()),
            SurfaceRole::Launcher => None,
            SurfaceRole::ControlCenter => self
                .plugin_panel_host_ref(&self.active_shell_surface_key("quick-settings"))
                .map(|host| host.layout_snapshot())
                .or_else(|| Some(self.control_host.layout_snapshot())),
            SurfaceRole::Notification => Some(self.notification_host.layout_snapshot()),
            SurfaceRole::VolumeOsd => None,
            SurfaceRole::WindowPreview => self
                .preview_plugin_host_ref()
                .map(|host| host.layout_snapshot()),
            SurfaceRole::WindowContextMenu => None,
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
            SurfaceRole::Taskbar => Vec::new(),
            SurfaceRole::Panel => unreachable!("plugin panels render through their surface key"),
            SurfaceRole::Launcher => Vec::new(),
            SurfaceRole::ControlCenter => {
                if self.quick_settings_surface_active() {
                    self.plugin_panel_scene(
                        &self.active_shell_surface_key("quick-settings"),
                        width,
                        height,
                    )
                    .unwrap_or_default()
                } else if self
                    .control_host
                    .application()
                    .view_state()
                    .trusted_visible()
                {
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
                } else {
                    Vec::new()
                }
            }
            SurfaceRole::VolumeOsd => Vec::new(),
            SurfaceRole::WindowPreview => self.window_preview_scene(),
            SurfaceRole::WindowContextMenu => Vec::new(),
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
            SurfaceRole::Taskbar => false,
            SurfaceRole::Panel => !self.plugin_surface_hosts.is_empty(),
            SurfaceRole::Launcher => false,
            SurfaceRole::ControlCenter => {
                self.control_visible
                    && self
                        .control_host
                        .application()
                        .view_state()
                        .trusted_visible()
            }
            SurfaceRole::Notification => self.trusted_notification_visible(),
            SurfaceRole::VolumeOsd => false,
            SurfaceRole::WindowPreview => {
                self.preview_plugin_host_ref().is_some()
                    && (self.preview_group.is_some() || self.task_switcher_group.is_some())
            }
            SurfaceRole::WindowContextMenu => false,
            SurfaceRole::CodexProjectMenu => self.codex_project_menu_visible,
            SurfaceRole::Lock => self.locked,
            SurfaceRole::Screenshot => self.screenshot.visible(),
            SurfaceRole::OnScreenKeyboard => false,
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
        if key == Some(&self.active_shell_surface_key("keyboard")) {
            return role == SurfaceRole::Panel
                && self.keyboard_visible
                && self.keyboard_enabled
                && self.plugin_surface_matches(&self.active_shell_surface_key("keyboard"));
        }
        if role == SurfaceRole::CodexProjectMenu {
            return self.codex_project_menu_visible
                && self.launcher.codex_available()
                && !self.locked;
        }
        if role == SurfaceRole::ControlCenter {
            return self.control_visible
                && self
                    .control_host
                    .application()
                    .view_state()
                    .trusted_visible();
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
        let key = self.active_shell_surface_key("taskbar");
        self.plugin_surface_hosts.contains_key(&key).then_some(key)
    }

    fn default_shell_surface_key(surface: &str) -> nickel_core::plugins::PluginSurfaceKey {
        nickel_core::plugins::PluginSurfaceKey {
            plugin_id: "nickel-default".into(),
            surface_id: surface.into(),
        }
    }

    pub(crate) fn active_shell_surface_key(
        &self,
        surface: &str,
    ) -> nickel_core::plugins::PluginSurfaceKey {
        nickel_core::plugins::PluginSurfaceKey {
            plugin_id: self.active_shell_package_id.clone(),
            surface_id: surface.into(),
        }
    }

    pub(crate) fn is_shell_package(&self, id: &str) -> bool {
        let mut current = Some(id.to_owned());
        for _ in 0..nickel_core::package_composition::MAX_COMPOSITION_DEPTH {
            let Some(id) = current else {
                return false;
            };
            let Some(composition) = self
                .plugin_registry
                .get(&id)
                .and_then(|entry| entry.manifest.composition.as_ref())
            else {
                return false;
            };
            if composition.exports.contains_key("shell") {
                return true;
            }
            current = composition.extends.clone();
        }
        false
    }

    pub(crate) fn shell_package_selected(&self, id: &str) -> bool {
        self.active_shell_package_id == id
    }

    pub fn select_shell_package(&mut self, id: &str) -> Result<bool, String> {
        if !self.is_shell_package(id) {
            return Err("package does not export a shell".into());
        }
        if self.active_shell_package_id == id && self.package_runtimes.contains_key(id) {
            return Ok(false);
        }
        // Construct and validate the requested runtime before retiring the visible shell.
        self.set_plugin_enabled(id, true)?;
        if !self.package_runtimes.contains_key(id) {
            return Err("shell runtime is unavailable".into());
        }
        let old = std::mem::replace(&mut self.active_shell_package_id, id.into());
        let initial = self
            .external_plugin_packages
            .get(id)
            .ok_or("shell package is unavailable")?
            .manifest
            .surfaces
            .iter()
            .filter(|surface| {
                surface.initially_open
                    && matches!(
                        surface.kind,
                        nickel_core::plugins::PluginSurfaceKind::Panel
                            | nickel_core::plugins::PluginSurfaceKind::Dock
                            | nickel_core::plugins::PluginSurfaceKind::Window
                    )
            })
            .map(|surface| surface.id.clone())
            .collect::<Vec<_>>();
        let transition = (|| -> Result<(), String> {
            for surface in initial {
                self.show_plugin_window(id, &surface)?;
            }
            Ok(())
        })();
        if let Err(error) = transition {
            self.plugin_surface_hosts
                .retain(|key, _| key.plugin_id != id);
            self.active_shell_package_id = old;
            return Err(error);
        }
        if old != id {
            self.retire_preview_plugin_state();
            self.keyboard_gesture_leases.clear();
            self.keyboard_host
                .application_mut()
                .recipient_changed(false);
            for (key, (_, host)) in &self.plugin_surface_hosts {
                if key.plugin_id == old
                    && let Err(error) = host.application().retire_surface()
                {
                    tracing::warn!(plugin = old, %error, "old shell surface could not unmount");
                }
            }
            self.plugin_surface_hosts
                .retain(|key, _| key.plugin_id != old);
            self.plugin_window_placement_overrides
                .retain(|key, _| key.plugin_id != old);
        }
        if self.keyboard_visible {
            if self.active_shell_declares("keyboard") {
                self.set_default_shell_surface_visible("keyboard", true);
            } else {
                self.set_keyboard_visible(false);
            }
        }
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        Ok(true)
    }

    fn shell_surface_effect_owner(&self, owner: &str) -> String {
        let mut current = Some(self.active_shell_package_id.clone());
        for _ in 0..nickel_core::package_composition::MAX_COMPOSITION_DEPTH {
            let Some(id) = current else {
                break;
            };
            if id == owner {
                return self.active_shell_package_id.clone();
            }
            current = self
                .plugin_registry
                .get(&id)
                .and_then(|entry| entry.manifest.composition.as_ref())
                .and_then(|composition| composition.extends.clone());
        }
        owner.into()
    }

    fn active_shell_declares(&self, surface: &str) -> bool {
        self.package_runtimes
            .contains_key(&self.active_shell_package_id)
            && self
                .external_plugin_packages
                .get(&self.active_shell_package_id)
                .is_some_and(|source| {
                    source
                        .manifest
                        .surfaces
                        .iter()
                        .any(|declaration| declaration.id == surface)
                })
    }

    fn show_volume_osd(&mut self) {
        if self.locked || !self.active_shell_declares("volume-osd") {
            return;
        }
        self.set_default_shell_surface_visible("volume-osd", true);
        if self.default_shell_surface_visible("volume-osd") {
            self.volume_osd_until = Some(Instant::now() + Duration::from_millis(1500));
        }
    }

    fn hide_volume_osd(&mut self) -> bool {
        let pending = self.volume_osd_until.take().is_some();
        (self.active_shell_declares("volume-osd")
            && self.set_default_shell_surface_visible("volume-osd", false))
            || pending
    }

    fn default_shell_surface_visible(&self, surface: &str) -> bool {
        self.plugin_surface_hosts
            .contains_key(&self.active_shell_surface_key(surface))
    }

    fn set_default_shell_surface_visible(&mut self, surface: &str, visible: bool) -> bool {
        let result = if visible {
            self.show_plugin_window(&self.active_shell_package_id.clone(), surface)
        } else {
            self.close_plugin_window(&self.active_shell_surface_key(surface))
        };
        match result {
            Ok(changed) => changed,
            Err(error) => {
                tracing::warn!(surface,%error,"shell package surface visibility failed");
                false
            }
        }
    }

    pub(crate) fn active_launcher_surface_key(
        &self,
    ) -> Option<nickel_core::plugins::PluginSurfaceKey> {
        let key = self.active_shell_surface_key("launcher");
        self.plugin_surface_hosts.contains_key(&key).then_some(key)
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

    /// Active ordinary package declarations for compositor-owned output surfaces.
    pub(crate) fn shell_panel_surfaces(
        &self,
    ) -> Vec<(
        nickel_core::plugins::PluginSurfaceKey,
        nickel_core::plugins::PluginSurface,
    )> {
        let mut panels = Vec::new();
        panels.extend(self.plugin_panels());

        panels
    }

    pub(crate) fn shell_fixed_surface_keys(
        &self,
    ) -> HashSet<nickel_core::plugins::PluginSurfaceKey> {
        [self.active_shell_surface_key("window-preview")]
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
                let anchor = if *key == self.active_shell_surface_key("keyboard") {
                    if self.keyboard_dock_top {
                        nickel_core::plugins::PluginSurfaceAnchor::TopLeft
                    } else {
                        nickel_core::plugins::PluginSurfaceAnchor::BottomLeft
                    }
                } else {
                    surface.anchor
                };
                (
                    surface.kind,
                    surface.bottom_offset,
                    anchor,
                    surface.offset_x,
                    surface.offset_y,
                )
            })
    }

    pub(crate) fn plugin_panel_change_token(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<HostChangeToken> {
        let inspection = self.plugin_panel_host_ref(key)?.inspect();
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
        self.plugin_panel_change_token(key)
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
                        "maximized": window.state.maximized,
                        "fullscreen": window.state.fullscreen,
                        "canMoveToWorkspace": window.state.capabilities.move_workspace,
                        "canMoveToOutput": window.state.capabilities.move_display,
                        "canMinimize": window.state.capabilities.minimize,
                        "canMaximize": window.state.capabilities.maximize,
                        "canFullscreen": cfg!(target_os = "linux") && window.state.capabilities.fullscreen,
                        "canSnap": cfg!(target_os = "linux") && window.state.capabilities.maximize && !window.state.fullscreen,
                        "workspace": window.state.workspace.map(|workspace| workspace.to_string()),
                        "output": window.state.output,
                        "canActivate": window.state.capabilities.activate,
                        "canClose": window.state.capabilities.close,
                    })
                })
                .collect(),
        ))
    }

    fn plugin_window_destinations(&self, id: &str) -> Option<serde_json::Value> {
        self.external_plugin_windows(id)?;
        Some(
            serde_json::json!({"workspaces": self.workspaces.iter().take(128).map(|workspace| serde_json::json!({"id": workspace.id.to_string(), "name": format!("Workspace {}", workspace.id)})).collect::<Vec<_>>(), "outputs": self.window_feed.outputs().into_iter().take(128).collect::<Vec<_>>()}),
        )
    }

    fn plugin_window_menu(&self, id: &str) -> Option<serde_json::Value> {
        self.external_plugin_windows(id)?;
        Some(
            serde_json::json!({"targetId": self.window_menu.map(|window| window.0.to_string()), "generation": self.window_menu_generation.to_string()}),
        )
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
        Some(crate::notification::snapshot(
            self.notification
                .as_ref()
                .filter(|notification| !self.trusted_notification_id(notification.id)),
            &history,
        ))
    }

    fn external_plugin_applications(&mut self, plugin_id: &str) -> Option<serde_json::Value> {
        let package = self.external_plugin_packages.get(plugin_id)?;
        if !package
            .manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::ApplicationsRead)
        {
            return None;
        }
        // Running-only entries include window titles/identities; retain the
        // separate WindowsRead grant rather than expanding ApplicationsRead.
        let include_running = package
            .manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::WindowsRead);
        let index = usize::from(include_running);
        let windows = if include_running {
            self.windows.as_slice()
        } else {
            &[]
        };
        let revision = self.launcher.taskbar_revision();
        if let Some(cached) = self.application_catalog_cache[index].as_ref()
            && Arc::ptr_eq(&cached.launcher_revision, revision)
            && cached.windows.as_slice() == windows
        {
            return Some(cached.value.clone());
        }
        let value = crate::application_capabilities::include_running(&self.launcher, windows);
        #[cfg(test)]
        {
            self.application_catalog_builds = self.application_catalog_builds.saturating_add(1);
        }
        self.application_catalog_cache[index] = Some(ApplicationCatalogCache {
            launcher_revision: Arc::clone(revision),
            windows: windows.to_vec(),
            value: value.clone(),
        });
        Some(value)
    }

    fn plugin_application_search(&self, plugin_id: &str) -> Option<serde_json::Value> {
        let package = self.external_plugin_packages.get(plugin_id)?;
        if !package
            .manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::ApplicationsRead)
        {
            return None;
        }
        let mut snapshot = self.application_search.snapshot(&self.launcher, plugin_id);
        snapshot["status"] = self
            .launcher_status
            .as_ref()
            .map(|status| status.chars().take(160).collect::<String>())
            .into();
        snapshot["pinSaveFailed"] = self
            .launcher_status
            .as_deref()
            .is_some_and(|status| status.starts_with("Launcher preferences could not be saved:"))
            .into();
        Some(snapshot)
    }

    fn plugin_application_images(
        &mut self,
        catalog: Option<&serde_json::Value>,
        search: Option<&serde_json::Value>,
    ) -> crate::plugin_panel::PluginImages {
        let mut images = crate::plugin_panel::PluginImages::new();
        let items = catalog
            .and_then(serde_json::Value::as_array)
            .into_iter()
            .flatten()
            .chain(
                search
                    .and_then(|snapshot| snapshot["results"].as_array())
                    .into_iter()
                    .flatten(),
            );
        for item in items {
            let Some(id) = item["id"].as_str() else {
                continue;
            };
            let icon = if let Some(application) = self
                .launcher
                .applications()
                .find(|application| application.id() == id)
            {
                self.launcher_icons
                    .resolve(application)
                    .unwrap_or_else(launcher_placeholder_icon)
            } else if let Some(window) = self.windows.iter().find(|window| {
                window
                    .application_id
                    .as_ref()
                    .is_some_and(|application| application.as_str() == id)
            }) {
                self.window_feed
                    .icon(window.id)
                    .map(|image| {
                        self.launcher_icons
                            .resolve_window_icon(window.id, Arc::new(image))
                    })
                    .unwrap_or_else(launcher_placeholder_icon)
            } else {
                continue;
            };
            images.insert(crate::application_capabilities::icon_asset(id), icon);
        }
        images
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

    fn plugin_session(&self, plugin_id: &str) -> Option<serde_json::Value> {
        use nickel_core::plugins::PluginCapability;
        let entry = self.plugin_registry.get(plugin_id)?;
        if !entry.desired_enabled
            || !entry.manifest.capabilities.iter().any(|grant| {
                matches!(
                    grant,
                    PluginCapability::SessionControl | PluginCapability::SessionLogoutRequest
                )
            })
        {
            return None;
        }
        serde_json::to_value(crate::session_capabilities::snapshot(
            self.locked,
            &entry.manifest.capabilities,
        ))
        .ok()
    }

    fn wallpaper_chooser_identity(
        &self,
        plugin_id: &str,
    ) -> Option<nickel_core::plugins::PluginManifest> {
        use nickel_core::plugins::PluginCapability;
        if self.locked
            || self
                .plugin_registry
                .get(plugin_id)
                .is_none_or(|entry| !entry.desired_enabled)
        {
            return None;
        }
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
            .contains(&PluginCapability::WallpaperRead)
            || !manifest
                .capabilities
                .contains(&PluginCapability::WallpaperControl)
        {
            return None;
        }
        Some(manifest.clone())
    }

    fn record_wallpaper_chooser_result(&mut self, plugin_id: String, result: serde_json::Value) {
        if self.wallpaper_chooser_results.len() >= 128
            && !self.wallpaper_chooser_results.contains_key(&plugin_id)
        {
            self.wallpaper_chooser_results.pop_first();
        }
        self.wallpaper_chooser_results.insert(plugin_id, result);
    }

    fn begin_wallpaper_chooser(
        &mut self,
        plugin_id: String,
        effect: crate::appearance_capabilities::AppearanceEffect,
    ) -> bool {
        let Some(identity) = self.wallpaper_chooser_identity(&plugin_id) else {
            return false;
        };
        if self.wallpaper_chooser.is_some() {
            return false;
        }
        if effect
            .validate(&self.appearance_capabilities.refresh("wallpaper"))
            .is_err()
        {
            self.record_wallpaper_chooser_result(
                plugin_id,
                serde_json::json!({"status":"rejected","reason":"Wallpaper changed; choose again"}),
            );
            return true;
        }
        let (sender, receiver) = std::sync::mpsc::channel();
        match nickel_platform::choose_image_file(Box::new(move |outcome| {
            let _ = sender.send(outcome);
        })) {
            Ok(()) => {
                self.wallpaper_chooser_results.remove(&plugin_id);
                self.wallpaper_chooser = Some(WallpaperChooserRequest {
                    plugin_id,
                    identity,
                    activation: self.plugin_activation_generation,
                    effect,
                    receiver,
                });
            }
            Err(_) => {
                self.record_wallpaper_chooser_result(plugin_id, serde_json::json!({"status":"failed","reason":"Native image chooser could not be opened"}));
            }
        }
        true
    }

    fn poll_wallpaper_chooser(&mut self) -> bool {
        let Some(request) = &self.wallpaper_chooser else {
            return false;
        };
        let outcome = match request.receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return false,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                nickel_platform::FileDialogOutcome::Failed(String::new())
            }
        };
        let request = self.wallpaper_chooser.take().unwrap();
        let authorized = self.wallpaper_chooser_identity(&request.plugin_id).as_ref()
            == Some(&request.identity)
            && self.plugin_activation_generation == request.activation;
        let result = if !authorized {
            serde_json::json!({"status":"rejected","reason":"Wallpaper permission or package identity changed"})
        } else {
            match outcome {
                nickel_platform::FileDialogOutcome::Cancelled => {
                    serde_json::json!({"status":"cancelled"})
                }
                nickel_platform::FileDialogOutcome::Failed(_) => {
                    serde_json::json!({"status":"failed","reason":"Native image chooser failed"})
                }
                nickel_platform::FileDialogOutcome::Selected(path) => match self
                    .appearance_capabilities
                    .commit_chosen(&request.effect, path, || {
                        if authorized {
                            Ok(())
                        } else {
                            Err("Wallpaper permission changed".into())
                        }
                    }) {
                    Ok(settings) => {
                        self.refresh_configured_wallpaper(settings.image);
                        self.desktop_application_dirty = true;
                        self.appearance_capabilities.wallpaper_reconciled();
                        serde_json::json!({"status":"applied"})
                    }
                    Err(_) => {
                        serde_json::json!({"status":"rejected","reason":"Image is unavailable, invalid, or wallpaper changed; choose again"})
                    }
                },
            }
        };
        self.record_wallpaper_chooser_result(request.plugin_id, result);
        true
    }

    fn feature_snapshot(&self) -> serde_json::Value {
        let keyboard = self.session_host.keyboard_snapshot().ok();
        let override_active = self.keyboard_override
            != nickel_core::on_screen_keyboard::KeyboardOverride::None
            || keyboard
                .as_ref()
                .is_some_and(|keyboard| keyboard.environment_override);
        let runtime = self.launcher.codex_projection().map(|projection| {
            nickel_core::optional_features::OptionalFeatureRuntime {
                codex_generation: projection.generation,
                codex_support: projection.support,
                codex_installation: projection.installation,
                codex_health: projection.health,
                diagnostic: projection.reason.clone(),
                ..Default::default()
            }
        });
        self.feature_client.read(
            keyboard.as_ref(),
            override_active,
            cfg!(any(target_os = "linux", target_os = "windows")),
            if cfg!(target_os = "linux") {
                runtime.as_ref()
            } else {
                None
            },
        )
    }
    fn plugin_features(&self, id: &str, shortcuts: bool) -> Option<serde_json::Value> {
        let manifest = self
            .external_plugin_packages
            .get(id)
            .map(|package| &package.manifest)
            .or_else(|| self.plugin_registry.get(id).map(|entry| &entry.manifest))?;
        let capability = if shortcuts {
            nickel_core::plugins::PluginCapability::ShortcutsRead
        } else {
            nickel_core::plugins::PluginCapability::FeaturesRead
        };
        manifest.capabilities.contains(&capability).then(|| {
            if shortcuts {
                crate::shortcut_capabilities::snapshot(
                    self.shortcut_capability_observed
                        && self.shortcut_capability_status.is_none()
                        && !self.locked,
                    self.shortcut_capability_status.as_deref(),
                )
            } else {
                let mut snapshot = self.feature_snapshot();
                if self.locked
                    || !self.session_host.feature_preference_writes_allowed()
                    || !manifest
                        .capabilities
                        .contains(&nickel_core::plugins::PluginCapability::FeaturesControl)
                {
                    snapshot["operations"] = serde_json::json!({});
                }
                snapshot
            }
        })
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
        if wallpaper {
            snapshot["chooser"] = serde_json::json!({"available": cfg!(any(target_os="linux", target_os="windows")), "pending":self.wallpaper_chooser.is_some(), "result":self.wallpaper_chooser_results.get(plugin_id)});
        }
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
            let snapshot = if wifi {
                crate::connectivity_capabilities::wifi_snapshot(&self.network)
            } else {
                crate::connectivity_capabilities::bluetooth_snapshot(&self.bluetooth)
            };
            let control = if wifi {
                nickel_core::plugins::PluginCapability::NetworkControl
            } else {
                nickel_core::plugins::PluginCapability::BluetoothControl
            };
            crate::connectivity_capabilities::restrict_controls(
                snapshot,
                !self.locked && manifest.capabilities.contains(&control),
            )
        })
    }

    fn plugin_management(&self, plugin_id: &str) -> Option<serde_json::Value> {
        use nickel_core::plugins::PluginCapability;
        let manifest = &self.plugin_registry.get(plugin_id)?.manifest;
        manifest
            .capabilities
            .contains(&PluginCapability::PluginsRead)
            .then(|| {
                let mut snapshot = crate::plugins_capabilities::snapshot(
                    &self.plugin_status_snapshot(),
                    manifest
                        .capabilities
                        .contains(&PluginCapability::PluginsControl)
                        && !self.locked,
                    self.plugins_results.get(plugin_id),
                );
                if let Some(plugins) = snapshot["plugins"].as_array_mut() {
                    for plugin in plugins {
                        let id = plugin["id"].as_str().unwrap_or("").to_owned();
                        plugin["shell"] = serde_json::json!(self.is_shell_package(&id));
                        plugin["selected"] = serde_json::json!(id == self.active_shell_package_id);
                    }
                }
                snapshot["selectedShell"] = serde_json::json!(self.active_shell_package_id);
                snapshot["shellPreview"] = self.shell_preview_snapshot(plugin_id);
                snapshot
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
            .then(|| crate::audio_capabilities::snapshot(&self.audio, self.locked))
    }

    fn plugin_keyboard_snapshot(&self, id: &str) -> Option<serde_json::Value> {
        let manifest = self
            .external_plugin_packages
            .get(id)
            .map(|package| &package.manifest)
            .or_else(|| self.plugin_registry.get(id).map(|entry| &entry.manifest))?;
        manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::OnScreenKeyboardRead)
            .then(|| {
                let mut snapshot = self.keyboard_service_snapshot();
                let granted = self.public_native_granted(id, nickel_core::plugins::PluginCapability::OnScreenKeyboardInput) && snapshot["available"] == true;
                snapshot["operations"] = serde_json::json!({"press":granted && snapshot["recipientAvailable"] == true,"hide":granted,"toggleDock":granted,"holdModifiers":granted,"resize":granted});
                snapshot
            })
    }

    fn plugin_workspace_snapshot(&self, id: &str) -> Option<serde_json::Value> {
        let manifest = self
            .external_plugin_packages
            .get(id)
            .map(|package| &package.manifest)
            .or_else(|| self.plugin_registry.get(id).map(|entry| &entry.manifest))?;
        manifest
            .capabilities
            .contains(&nickel_core::plugins::PluginCapability::WorkspacesRead)
            .then(|| {
                crate::workspace_capabilities::snapshot(
                    &self.workspaces,
                    cfg!(target_os = "linux")
                        && !self.locked
                        && manifest
                            .capabilities
                            .contains(&nickel_core::plugins::PluginCapability::WorkspacesSwitch),
                )
            })
    }
    fn plugin_desktop_snapshot(&self, id: &str) -> Option<serde_json::Value> {
        let manifest = self
            .external_plugin_packages
            .get(id)
            .map(|package| &package.manifest)
            .or_else(|| self.plugin_registry.get(id).map(|entry| &entry.manifest))?;
        manifest.capabilities.contains(&nickel_core::plugins::PluginCapability::DesktopControl).then(||serde_json::json!({"available":cfg!(target_os="linux"),"operations":{"toggleShowDesktop":cfg!(target_os="linux")&&!self.locked}}))
    }
    fn public_native_granted(
        &self,
        id: &str,
        capability: nickel_core::plugins::PluginCapability,
    ) -> bool {
        !self.locked
            && self.plugin_registry.get(id).is_some_and(|entry| {
                entry.desired_enabled
                    && entry.health == nickel_core::plugins::PluginHealth::Running
                    && entry.manifest.capabilities.contains(&capability)
            })
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
                Ok(outputs) => serde_json::json!({"available": true, "outputs": outputs,
                    "revision": crate::display_capabilities::revision(&outputs),
                    "projectionModes": if !self.control_host.application().view_state().trusted_visible() {crate::display_capabilities::projection_modes(&outputs)} else {serde_json::json!([])},
                    "application_scale": self.application_scale_service.snapshot(),
                    "operations": {"setOrientation": true, "setApplicationScale": true, "identify": true},
                    "pending_confirmation": self.display_preview.is_some(),
                    "can_confirm": self.display_preview.as_ref().is_some_and(|preview| preview.owner == plugin_id && Instant::now() < preview.deadline && output_layout_from_snapshot(&outputs) == preview.applied),
                    "can_revert": self.display_preview.as_ref().is_some_and(|preview| preview.owner == plugin_id && (output_layout_from_snapshot(&outputs) == preview.applied || output_layout_from_snapshot(&outputs) == preview.previous)),
                    "transforms": ["normal","rotate90","rotate180","rotate270","flipped","flipped90","flipped180","flipped270"]}),
                Err(error) => {
                    serde_json::json!({"available": false, "reason": error, "outputs": [], "application_scale": self.application_scale_service.snapshot(), "operations": {"setApplicationScale": true, "identify": false}})
                }
            });
        }
        #[cfg(target_os = "windows")]
        {
            let read = crate::windows_plugin_display::read(plugin_id);
            return Some(serde_json::json!({
                "available": read.available,
                "reason": read.reason,
                "outputs": read.outputs,
                "projectionModes": [],
                "revision": read.revision,
                "pending_confirmation": read.pending_confirmation,
                "can_confirm": read.can_confirm,
                "can_revert": read.can_revert,
                "application_scale": self.application_scale_service.snapshot(),
                "operations": {"setOrientation": read.available, "setApplicationScale": true, "identify": false},
                "transforms": ["normal","rotate90","rotate180","rotate270"],
            }));
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        Some(serde_json::json!({
            "available": false,
            "reason": "display layout control is unavailable on this platform",
            "outputs": [],
        }))
    }

    fn plugin_navigation(&self, id: &str) -> Option<serde_json::Value> {
        self.plugin_registry
            .get(id)
            .filter(|entry| {
                entry
                    .manifest
                    .capabilities
                    .contains(&nickel_core::plugins::PluginCapability::SettingsRead)
            })
            .and_then(|_| self.settings_navigation.clone())
    }

    fn plugin_system_metadata() -> serde_json::Value {
        serde_json::json!({"available":true,"version":env!("CARGO_PKG_VERSION"),"platform":std::env::consts::OS,"architecture":std::env::consts::ARCH})
    }

    fn plugin_owner_resource_fields(&mut self, id: &str) -> Vec<(&'static str, serde_json::Value)> {
        let tray = self
            .plugin_registry
            .get(id)
            .filter(|entry| {
                entry
                    .manifest
                    .capabilities
                    .contains(&nickel_core::plugins::PluginCapability::TrayRead)
            })
            .map(|_| {
                serde_json::Value::Array(self.tray.iter().take(128).map(|item|
                serde_json::json!({"id":item.id,"title":item.title,"icon":false})).collect())
            });
        [
            ("clock", Some(crate::clock_capabilities::snapshot())),
            ("run", self.plugin_run_snapshot(id)),
            ("windows", self.external_plugin_windows(id)),
            ("windowMenu", self.plugin_window_menu(id)),
            ("windowDestinations", self.plugin_window_destinations(id)),
            ("windowPreviews", self.plugin_window_previews(id)),
            ("applications", self.external_plugin_applications(id)),
            ("applicationSearch", self.plugin_application_search(id)),
            ("notifications", self.external_plugin_notifications(id)),
            ("audio", self.plugin_audio(id)),
            ("tray", tray),
            ("associations", self.plugin_associations(id)),
            ("plugins", self.plugin_management(id)),
            ("preferences", self.plugin_preferences(id)),
            ("appearance", self.plugin_appearance(id, false)),
            ("wallpaper", self.plugin_appearance(id, true)),
            ("session", self.plugin_session(id)),
            ("features", self.plugin_features(id, false)),
            ("shortcuts", self.plugin_features(id, true)),
            ("system", Some(Self::plugin_system_metadata())),
            ("navigation", self.plugin_navigation(id)),
            ("wifi", self.plugin_connectivity(id, true)),
            ("bluetooth", self.plugin_connectivity(id, false)),
            ("displays", self.plugin_displays(id)),
            ("keyboard", self.plugin_keyboard_snapshot(id)),
            ("workspaces", self.plugin_workspace_snapshot(id)),
            ("desktop", self.plugin_desktop_snapshot(id)),
        ]
        .into_iter()
        .filter_map(|(name, value)| value.map(|value| (name, value)))
        .collect()
    }

    pub(crate) fn plugin_panel_scene(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        width: u32,
        height: u32,
    ) -> Option<Vec<PaintCommand>> {
        self.plugin_panel_scene_in_viewport(key, None, width, height)
    }

    fn plugin_panel_scene_in_viewport(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        output: Option<&str>,
        width: u32,
        height: u32,
    ) -> Option<Vec<PaintCommand>> {
        let replicated_surface = self.plugin_surface_hosts.get(key).map(|(surface, _)| {
            (
                surface.output == nickel_core::plugins::PluginOutputScope::All,
                surface.width,
                surface.height,
            )
        })?;
        let desktop = self.desktop_host.application();
        let output = output.or(Some(desktop.active_output.as_str()));
        let available = desktop
            .outputs
            .iter()
            .find(|candidate| Some(candidate.id.as_str()) == output);
        let viewport = if replicated_surface.0 {
            serde_json::json!({
                "width": replicated_surface.1, "height": replicated_surface.2,
                "output": null, "availableWidth": null, "availableHeight": null,
            })
        } else {
            serde_json::json!({
                "width": width, "height": height, "output": output,
                "availableWidth": available.map(|output| output.work_area.width),
                "availableHeight": available.map(|output| output.work_area.height),
            })
        };
        let surface_authority = if replicated_surface.0 {
            (None, None, None)
        } else {
            (
                output.map(str::to_owned),
                available.map(|output| (output.work_area.width, output.work_area.height)),
                available.map(|output| output.scale),
            )
        };
        let dependency_ids = self
            .plugin_panel_host_for(key)?
            .application()
            .composition_dependency_ids();
        let dependency_fields = dependency_ids
            .into_iter()
            .map(|id| {
                let mut fields = self.plugin_owner_resource_fields(&id);
                fields.push(("viewport", viewport.clone()));
                (id, fields)
            })
            .collect::<std::collections::BTreeMap<_, _>>();
        let window_previews = self.plugin_window_previews(&key.plugin_id);
        let preview_images = self.plugin_window_preview_images(&key.plugin_id);
        let windows = self.external_plugin_windows(&key.plugin_id);
        let window_menu = self.plugin_window_menu(&key.plugin_id);
        let window_destinations = self.plugin_window_destinations(&key.plugin_id);
        let applications = self.external_plugin_applications(&key.plugin_id);
        let application_search = self.plugin_application_search(&key.plugin_id);
        let features = self.plugin_features(&key.plugin_id, false);
        let shortcuts = self.plugin_features(&key.plugin_id, true);
        let mut application_images =
            self.plugin_application_images(applications.as_ref(), application_search.as_ref());
        application_images.insert("codex".into(), (u16::MAX, self.codex_icon.clone()));
        let clock = crate::clock_capabilities::snapshot();
        let notifications = self.external_plugin_notifications(&key.plugin_id);
        let tray = self.plugin_registry.get(&key.plugin_id)
            .filter(|entry| entry.manifest.capabilities.contains(&nickel_core::plugins::PluginCapability::TrayRead))
            .map(|_| serde_json::Value::Array(self.tray.iter().take(128).map(|item| serde_json::json!({"id":item.id,"title":item.title,"icon":false})).collect()));
        let audio = self.plugin_audio(&key.plugin_id);
        let associations = self.plugin_associations(&key.plugin_id);
        let plugins = self.plugin_management(&key.plugin_id);
        let preferences = self.plugin_preferences(&key.plugin_id);
        let appearance = self.plugin_appearance(&key.plugin_id, false);
        let wallpaper = self.plugin_appearance(&key.plugin_id, true);
        let session = self.plugin_session(&key.plugin_id);
        if wallpaper.is_some() {
            application_images.extend(self.appearance_capabilities.wallpaper_images.clone());
        }
        let system = Self::plugin_system_metadata();
        let navigation = self.plugin_navigation(&key.plugin_id);
        let wifi = self.plugin_connectivity(&key.plugin_id, true);
        let bluetooth = self.plugin_connectivity(&key.plugin_id, false);
        let displays = self.plugin_displays(&key.plugin_id);
        let workspaces = self.plugin_workspace_snapshot(&key.plugin_id);
        let desktop = self.plugin_desktop_snapshot(&key.plugin_id);
        let keyboard_data = self.plugin_keyboard_snapshot(&key.plugin_id);
        let result = (|| {
            let host = self.plugin_panel_host_for(key)?;
            let projected = (|| -> Result<bool, String> {
                // Geometry is published first. Focus is a distinct UiHost fact,
                // so hooks never observe a new focus paired with stale output
                // or scale state from this presentation pass.
                let geometry_changed = host.application_mut().sync_surface_geometry(
                    surface_authority.0.as_deref(),
                    surface_authority.1,
                    surface_authority.2,
                    Some(true),
                )?;
                let window_focused = host.inspect().window_focused;
                let focus_changed = host.application_mut().sync_surface_focus(window_focused)?;
                application_images.extend(preview_images);
                let fields = [
                    ("viewport", Some(&viewport)),
                    ("clock", Some(&clock)),
                    ("keyboard", keyboard_data.as_ref()),
                    ("windows", windows.as_ref()),
                    ("windowMenu", window_menu.as_ref()),
                    ("windowDestinations", window_destinations.as_ref()),
                    ("windowPreviews", window_previews.as_ref()),
                    ("applications", applications.as_ref()),
                    ("applicationSearch", application_search.as_ref()),
                    ("features", features.as_ref()),
                    ("shortcuts", shortcuts.as_ref()),
                    ("notifications", notifications.as_ref()),
                    ("audio", audio.as_ref()),
                    ("tray", tray.as_ref()),
                    ("associations", associations.as_ref()),
                    ("plugins", plugins.as_ref()),
                    ("preferences", preferences.as_ref()),
                    ("appearance", appearance.as_ref()),
                    ("wallpaper", wallpaper.as_ref()),
                    ("session", session.as_ref()),
                    ("system", Some(&system)),
                    ("navigation", navigation.as_ref()),
                    ("wifi", wifi.as_ref()),
                    ("bluetooth", bluetooth.as_ref()),
                    ("displays", displays.as_ref()),
                    ("workspaces", workspaces.as_ref()),
                    ("desktop", desktop.as_ref()),
                ]
                .into_iter()
                .filter_map(|(field, value)| value.map(|value| (field, value)))
                .collect::<Vec<_>>();
                let resource_changed = host.application_mut().sync_host_data_fields(&fields)?;
                let dependency_changed = host
                    .application_mut()
                    .sync_composition_dependency_fields(&dependency_fields)?;
                let application_images_changed =
                    if applications.is_some() || window_previews.is_some() {
                        host.application_mut()
                            .sync_application_images(application_images)
                    } else {
                        false
                    };
                let surface_changed = host.application_mut().reconcile_surface_authority()?;
                if resource_changed || dependency_changed || application_images_changed {
                    tracing::warn!(
                        surface = %key.surface_id,
                        resource_changed,
                        dependency_changed,
                        application_images_changed,
                        "plugin projection change source"
                    );
                }
                Ok(geometry_changed
                    || focus_changed
                    || surface_changed
                    || resource_changed
                    || dependency_changed
                    || application_images_changed)
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
                        application_changed: projected,
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
        self.plugin_panel_scene_in_viewport(key, output, width, height)
    }

    pub(crate) fn plugin_surface_scene_for_output(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        output: Option<&str>,
        width: u32,
        height: u32,
    ) -> Option<Vec<PaintCommand>> {
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
        // Surface visibility does not determine package lifetime. The shared
        // runtime remains owned until explicit disable or runtime failure.
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
        if self.is_shell_package(id) && !self.shell_package_selected(id) {
            return Err("shell package is not selected".into());
        }
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
                        nickel_core::plugins::PluginSurfaceKind::Panel
                            | nickel_core::plugins::PluginSurfaceKind::Dock
                            | nickel_core::plugins::PluginSurfaceKind::Window
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
            .package_runtimes
            .get(id)
            .cloned()
            .ok_or_else(|| format!("plugin {id:?} has no live package runtime"))?;
        let application = match runtime {
            RetainedPackageRuntime::Ordinary(runtime) => {
                crate::plugin_panel::PluginPanelApplication::from_package_surface_with_runtime(
                    &package,
                    &settings,
                    &surface,
                    crate::plugin_panel::package_images(&package)?,
                    Some(runtime),
                )?
            }
            RetainedPackageRuntime::Composed(host) if !self.is_shell_package(id) => {
                let owner = host.borrow().resolution().active.clone();
                let runtime = host.borrow().shared_owner_runtime(&owner)?;
                crate::plugin_panel::PluginPanelApplication::from_package_surface_with_runtime(
                    &package,
                    &settings,
                    &surface,
                    crate::plugin_panel::package_images(&package)?,
                    Some(runtime),
                )?
            }
            RetainedPackageRuntime::Composed(host) => {
                let catalog = self.composition_catalog(id, &package)?;
                let snapshots = self.composition_snapshots(&catalog, &surface);
                crate::plugin_panel::PluginPanelApplication::from_composed_surface(
                    &catalog,
                    id,
                    &snapshots,
                    &surface,
                    Some(host),
                )?
            }
        };
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
                        .composition
                        .as_ref()
                        .map(|composition| {
                            composition
                                .contributions
                                .iter()
                                .map(|entry| {
                                    format!("Contributes {} to {}", entry.id, entry.collection)
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
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

    // A provider may have no visible surface. Execute through the ordinary
    // package application so callback requests retain manifest effect validation.
    // No native host or visible surface is installed for this evaluation.
    fn invoke_hidden_package_setting(
        &mut self,
        provider: &str,
        id: &str,
        value: &serde_json::Value,
    ) -> Result<Vec<crate::plugin_panel::PluginEffect>, String> {
        let manifest = self
            .plugin_registry
            .get(provider)
            .filter(|entry| entry.desired_enabled)
            .ok_or("Settings provider is unavailable")?
            .manifest
            .clone();
        let runtime = self
            .package_settings_runtimes
            .get(provider)
            .cloned()
            .ok_or("Settings provider runtime is unavailable")?;
        let fields = self.plugin_owner_resource_fields(provider);
        let mut data: serde_json::Value = runtime
            .borrow_mut()
            .eval_json("JSON.stringify(nickel.data)")?;
        let object = data
            .as_object_mut()
            .ok_or("provider snapshot must be an object")?;
        object.remove("__componentProps");
        for (name, value) in fields {
            object.insert(name.into(), value);
        }
        runtime.borrow_mut().set_data(&data.to_string())?;
        if let Some(RetainedPackageRuntime::Composed(host)) =
            self.package_runtimes.get(&self.active_shell_package_id)
        {
            let mut host = host.borrow_mut();
            let owner = host
                .participating_owners()
                .find(|owner| owner.id == provider)
                .cloned();
            if let Some(owner) = owner {
                if std::rc::Rc::ptr_eq(&runtime, &host.shared_owner_runtime(&owner)?) {
                    host.update_snapshot(&owner, &data)?;
                }
            }
        }
        runtime.borrow_mut().begin_transaction()?;
        let result = (|| {
            runtime.borrow_mut().invoke_setting(provider, id, value)?;
            let effects = runtime.borrow_mut().take_effects()?;
            crate::plugin_panel::PluginPanelApplication::validate_provider_effects(
                &manifest,
                runtime.clone(),
                effects,
                &data,
            )
        })();
        runtime.borrow_mut().finish_transaction(result.is_ok())?;
        result
    }

    fn refresh_package_settings(&mut self) {
        let mut runtimes = std::collections::BTreeMap::new();
        for (id, retained) in &self.package_runtimes {
            runtimes.extend(retained.contexts(id));
        }
        for (key, (_, host)) in &self.plugin_surface_hosts {
            runtimes.insert(key.plugin_id.clone(), host.application().shared_runtime());
        }
        // The selected shell's contexts own its provider registrations. An
        // inactive shell's duplicate dependency instance cannot replace them.
        if let Some(retained) = self.package_runtimes.get(&self.active_shell_package_id) {
            runtimes.extend(retained.contexts(&self.active_shell_package_id));
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
            self.plugins_results.remove(&id);
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
            self.plugins_results.remove(id);
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

    fn propagate_composed_settings(
        &mut self,
        id: &str,
        values: &std::collections::BTreeMap<String, serde_json::Value>,
    ) -> Result<(), String> {
        let composed = self
            .package_runtimes
            .values()
            .filter_map(|runtime| {
                let RetainedPackageRuntime::Composed(host) = runtime else {
                    return None;
                };
                let owner = host
                    .borrow()
                    .participating_owners()
                    .find(|owner| owner.id == id)
                    .cloned()?;
                Some((host.clone(), owner))
            })
            .collect::<Vec<_>>();
        for (host, owner) in &composed {
            let mut host = host.borrow_mut();
            let mut data = host.snapshot(owner)?.clone();
            data["settings"] = serde_json::to_value(&values).map_err(|error| error.to_string())?;
            host.update_snapshot(owner, &data)?;
        }
        for (_, host) in self.plugin_surface_hosts.values_mut() {
            if host
                .application()
                .shared_composition_runtime()
                .is_some_and(|runtime| {
                    composed
                        .iter()
                        .any(|(candidate, _)| std::rc::Rc::ptr_eq(candidate, &runtime))
                })
            {
                let changed = host.application_mut().refresh_composition_snapshots()?;
                host.step(HostBatch {
                    application_changed: changed,
                    ..HostBatch::default()
                });
            }
        }
        Ok(())
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
        if matches!(
            self.package_runtimes.get(id),
            Some(RetainedPackageRuntime::Composed(_))
        ) {
            #[cfg(not(test))]
            nickel_core::plugins::PluginPreferences::update_default(&manifest, key, value)
                .map_err(|error| format!("could not save plugin setting: {error}"))?;
            self.plugin_settings.insert(id.to_owned(), values.clone());
            self.propagate_composed_settings(id, &values)?;
            self.plugin_activation_generation =
                self.plugin_activation_generation.wrapping_add(1).max(1);
            self.maybe_publish_plugin_status();
            return Ok(true);
        }
        let mut replacement_runtime = None;
        let replacement = if entry.desired_enabled {
            self.external_plugin_packages
                .get(id)
                .map(|descriptor| {
                    let package = descriptor.load()?;
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
                            .or_else(|| package.manifest.surfaces.first())
                            .ok_or("installed plugin has no declared surface")?;
                        let runtime = crate::plugin_panel::PluginPanelApplication::shared_package_runtime(
                            &package,
                            &values,
                            first_surface,
                        )?;
                        replacement_runtime = Some(runtime.clone());
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
                })
                .transpose()?
        } else {
            None
        };
        #[cfg(not(test))]
        nickel_core::plugins::PluginPreferences::update_default(&manifest, key, value)
            .map_err(|error| format!("could not save plugin setting: {error}"))?;
        self.plugin_settings.insert(id.to_owned(), values.clone());
        self.propagate_composed_settings(id, &values)?;
        if let Some(runtime) = replacement_runtime {
            self.package_runtimes
                .insert(id.to_owned(), RetainedPackageRuntime::Ordinary(runtime));
        }
        let mut replaced_panels = false;
        if let Some(replacements) = replacement {
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
                }
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
        if self.is_shell_package(id) {
            self.cancel_keyboard_gestures();
        }
        if !self.external_plugin_packages.contains_key(id)
            || !self
                .plugin_registry
                .get(id)
                .is_some_and(|entry| entry.health == nickel_core::plugins::PluginHealth::Running)
        {
            return false;
        }
        tracing::warn!(plugin = id, %error, "installed plugin runtime failed");
        let _ = self.plugin_registry.mark_failed(id, error);
        if id == self.active_shell_package_id {
            self.retire_preview_plugin_state();
        }
        self.package_runtimes.remove(id);
        self.retire_installed_composition_owner(id);
        if let Err(error) = self.reconcile_installed_contributors() {
            tracing::warn!(%error,"failed contributor retirement refresh failed");
        }
        self.application_search.retire(id);
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
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        if self
            .shell_selection_preview
            .as_ref()
            .is_some_and(|preview| preview.selected == id || preview.owner == id)
        {
            self.recover_pending_shell("The preview shell or its requesting provider failed.");
        }
        self.maybe_publish_plugin_status();
        true
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
        self.application_search.retire(id);
        retire(self);
        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        self.maybe_publish_plugin_status();
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
        } else {
            return self.fail_installed_plugin_runtime(id, error);
        };
        self.fail_bundled_plugin_runtime(id, error, retire);
        true
    }

    fn retire_preview_plugin_state(&mut self) {
        let key = self.active_shell_surface_key("window-preview");
        self.plugin_surface_hosts.remove(&key);
        self.plugin_panel_memory.remove(&key);
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

    /// Starts or retires a plugin instance after Settings has shown its grants.
    fn composition_catalog(
        &self,
        active: &str,
        package: &nickel_core::plugins::PluginPackage,
    ) -> Result<std::collections::BTreeMap<String, nickel_core::plugins::PluginPackage>, String>
    {
        let mut catalog = std::collections::BTreeMap::from([(active.to_owned(), package.clone())]);
        let mut next = package
            .manifest
            .composition
            .as_ref()
            .and_then(|composition| composition.extends.clone());
        while let Some(base) = next {
            if catalog.contains_key(&base) {
                return Err("shell inheritance cycle".into());
            }
            if catalog.len() >= nickel_core::package_composition::MAX_COMPOSITION_DEPTH {
                return Err("shell inheritance exceeds package limit".into());
            }
            let entry = self
                .plugin_registry
                .get(&base)
                .ok_or("shell base package is not installed")?;
            if !entry.desired_enabled {
                return Err(format!("shell base package {base:?} is disabled"));
            }
            let dependency = self
                .external_plugin_packages
                .get(&base)
                .ok_or("shell base package is not an installed Twinkle package")?
                .load()?;
            next = dependency
                .manifest
                .composition
                .as_ref()
                .and_then(|composition| composition.extends.clone());
            catalog.insert(base, dependency);
        }
        let approvals =
            nickel_core::plugins::PluginActivationSettings::load_default().unwrap_or_default();
        for entry in self.plugin_registry.entries() {
            if !entry.desired_enabled
                || entry.health != nickel_core::plugins::PluginHealth::Running
                || catalog.contains_key(&entry.manifest.id)
            {
                continue;
            }
            let Some(descriptor) = self.external_plugin_packages.get(&entry.manifest.id) else {
                continue;
            };
            if descriptor.manifest.composition.is_none() {
                continue;
            }
            #[cfg(not(test))]
            if !approvals.approval_current(&descriptor.manifest, &descriptor.source_digest) {
                continue;
            }
            catalog.insert(entry.manifest.id.clone(), descriptor.load()?);
        }
        let _ = approvals;
        Ok(catalog)
    }

    fn retire_installed_composition_owner(&mut self, id: &str) {
        // Providers can own a context without owning a native surface.
        for retained in self.package_runtimes.values() {
            if let RetainedPackageRuntime::Composed(host) = retained {
                let owners = host
                    .borrow()
                    .participating_owners()
                    .filter(|owner| owner.id == id)
                    .cloned()
                    .collect::<Vec<_>>();
                for owner in owners {
                    host.borrow_mut().retire(&owner);
                }
            }
        }
        for (_, host) in self.plugin_surface_hosts.values_mut() {
            host.application_mut().retire_composition_owner(id);
        }
    }

    fn composition_contexts(
        &self,
    ) -> std::collections::BTreeMap<
        nickel_core::package_composition::PackageIdentity,
        nickel_plugin_runtime::composition_runtime::ProviderContext,
    > {
        let mut contexts = std::collections::BTreeMap::new();
        for retained in self.package_runtimes.values() {
            if let RetainedPackageRuntime::Composed(host) = retained {
                let host = host.borrow();
                for owner in host.participating_owners() {
                    if let Ok(context) = host.provider_context(owner) {
                        contexts.entry(owner.clone()).or_insert(context);
                    }
                }
            }
        }
        contexts
    }

    fn reconcile_installed_contributors(&mut self) -> Result<(), String> {
        let contexts = self.composition_contexts();
        let hosts = self
            .package_runtimes
            .iter()
            .filter_map(|(id, retained)| match retained {
                RetainedPackageRuntime::Composed(host) => Some((id.clone(), host.clone())),
                _ => None,
            })
            .collect::<Vec<_>>();
        for (active, host) in hosts {
            let Some(descriptor) = self.external_plugin_packages.get(&active) else {
                continue;
            };
            let package = descriptor.load()?;
            let catalog = self.composition_catalog(&active, &package)?;
            host.borrow_mut().sync_contributors(&catalog, &contexts)?;
            for (key, (_, surface)) in &mut self.plugin_surface_hosts {
                if key.plugin_id == active {
                    surface
                        .application_mut()
                        .refresh_composition_catalog(&catalog)?;
                    surface.step(HostBatch {
                        application_changed: true,
                        ..Default::default()
                    });
                }
            }
        }
        Ok(())
    }

    fn composition_snapshots(
        &self,
        catalog: &std::collections::BTreeMap<String, nickel_core::plugins::PluginPackage>,
        surface: &nickel_core::plugins::PluginSurface,
    ) -> std::collections::BTreeMap<
        nickel_core::package_composition::PackageIdentity,
        serde_json::Value,
    > {
        catalog
            .values()
            .filter_map(|package| {
                let composition = package.manifest.composition.as_ref()?;
                let owner = nickel_core::package_composition::PackageIdentity {
                    id: composition.id.clone(),
                    version: composition.version.parse().ok()?,
                };
                let values = self
                    .plugin_settings
                    .get(&package.manifest.id)
                    .cloned()
                    .unwrap_or_else(|| {
                        package
                            .manifest
                            .settings
                            .iter()
                            .map(|setting| (setting.id.clone(), setting.kind.default_value()))
                            .collect()
                    });
                let data = crate::plugin_panel::PluginPanelApplication::package_surface_data(
                    package, &values, surface,
                );
                Some((
                    owner,
                    serde_json::from_str(&data).expect("package surface data is JSON"),
                ))
            })
            .collect()
    }

    pub fn set_plugin_enabled(&mut self, id: &str, enabled: bool) -> Result<bool, String> {
        let Some(entry) = self.plugin_registry.get(id) else {
            return Err(format!("unknown plugin {id:?}"));
        };
        if entry.desired_enabled == enabled {
            return Ok(false);
        }
        let external_panel = if enabled {
            if let Some(descriptor) = self.external_plugin_packages.get(id) {
                let surfaces = &descriptor.manifest.surfaces;
                Some(
                    if surfaces.is_empty() && descriptor.manifest.composition.is_some() {
                        descriptor.load().and_then(|package| {
                            crate::plugin_panel::PluginPanelApplication::validate_provider_resources(&package)?;
                            let catalog=self.composition_catalog(id,&package)?;
                            let snapshots=catalog.values().filter_map(|package| {
                                let c=package.manifest.composition.as_ref()?;
                                let owner=nickel_core::package_composition::PackageIdentity {id:c.id.clone(),version:c.version.parse().ok()?};
                                let settings=self.plugin_settings.get(&c.id).cloned().unwrap_or_else(||package.manifest.settings.iter().map(|setting|(setting.id.clone(),setting.kind.default_value())).collect());
                                Some((owner,serde_json::json!({"settings":settings})))
                            }).collect();
                            let host=nickel_plugin_runtime::composition_runtime::ShellCompositionRuntime::new_with_contexts(&catalog,id,&snapshots,&self.composition_contexts())?;
                            Ok((RetainedPackageRuntime::Composed(std::rc::Rc::new(std::cell::RefCell::new(host))),Vec::new()))
                        })
                    } else if !surfaces.is_empty()
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
                            if package.manifest.composition.as_ref().is_some_and(|composition| composition.extends.is_some() || composition.exports.contains_key("shell")) {
                                let catalog = self.composition_catalog(id, &package)?;
                                let first = surfaces.iter().find(|surface| !matches!(surface.kind, nickel_core::plugins::PluginSurfaceKind::Dialog | nickel_core::plugins::PluginSurfaceKind::Overlay)).ok_or("composed package has no ordinary surface")?;
                                let snapshots = self.composition_snapshots(&catalog, first);
                                let shared = std::rc::Rc::new(std::cell::RefCell::new(nickel_plugin_runtime::composition_runtime::ShellCompositionRuntime::new_with_contexts(&catalog, id, &snapshots, &self.composition_contexts())?));
                                let mut applications = Vec::new();
                                for surface in surfaces.iter().filter(|surface| surface.initially_open && (!self.is_shell_package(id) || self.shell_package_selected(id)) && !matches!(surface.kind,
                                    nickel_core::plugins::PluginSurfaceKind::Dialog | nickel_core::plugins::PluginSurfaceKind::Overlay)) {
                                    let snapshots = self.composition_snapshots(&catalog, surface);
                                    let application = crate::plugin_panel::PluginPanelApplication::from_composed_surface(&catalog, id, &snapshots, surface, Some(shared.clone()))?;
                                    let resolved = application.resolved_surface(surface)?;
                                    applications.push((application, resolved));
                                }
                                return Ok((RetainedPackageRuntime::Composed(shared), applications));
                            }
                            if package.manifest.composition.is_some() {
                                crate::plugin_panel::PluginPanelApplication::validate_provider_resources(&package)?;
                                let catalog=self.composition_catalog(id,&package)?;
                                let first=surfaces.iter().find(|surface|!matches!(surface.kind,nickel_core::plugins::PluginSurfaceKind::Dialog|nickel_core::plugins::PluginSurfaceKind::Overlay)).ok_or("provider has no ordinary surface")?;
                                let snapshots=self.composition_snapshots(&catalog,first);
                                let shared=std::rc::Rc::new(std::cell::RefCell::new(nickel_plugin_runtime::composition_runtime::ShellCompositionRuntime::new_with_contexts(&catalog,id,&snapshots,&self.composition_contexts())?));
                                let owner=shared.borrow().resolution().active.clone();let runtime=shared.borrow().shared_owner_runtime(&owner)?;
                                let settings=self.plugin_settings.get(id).cloned().map(Ok).unwrap_or_else(||external_plugin_settings(&package.manifest))?;
                                let images=crate::plugin_panel::package_images(&package)?;
                                let panels=surfaces.iter().filter(|surface|surface.initially_open&&!matches!(surface.kind,nickel_core::plugins::PluginSurfaceKind::Dialog|nickel_core::plugins::PluginSurfaceKind::Overlay)).map(|surface|{
                                    let application=crate::plugin_panel::PluginPanelApplication::from_package_surface_with_runtime(&package,&settings,surface,images.clone(),Some(runtime.clone()))?;
                                    let resolved=application.resolved_surface(surface)?;Ok((application,resolved))
                                }).collect::<Result<Vec<_>,String>>()?;
                                return Ok((RetainedPackageRuntime::Composed(shared),panels));
                            }
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
                            let panels = surfaces
                                .iter()
                                .filter(|surface| {
                                    surface.initially_open && (!self.is_shell_package(id) || self.shell_package_selected(id)) && !matches!(
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
                                .collect::<Result<Vec<_>, _>>()?;
                            Ok((RetainedPackageRuntime::Ordinary(runtime), panels))
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
        if !enabled && self.is_shell_package(id) {
            self.cancel_keyboard_gestures();
        }
        if !enabled {
            self.retire_installed_composition_owner(id);
        }

        self.plugin_activation_generation =
            self.plugin_activation_generation.wrapping_add(1).max(1);
        if !enabled {
            if id == self.active_shell_package_id {
                self.retire_preview_plugin_state();
            }
            self.run_status.remove(id);
            self.package_runtimes.remove(id);
            self.application_search.retire(id);
            self.plugin_surface_hosts
                .retain(|key, _| key.plugin_id != id);
            self.plugin_panel_memory
                .retain(|key, _| key.plugin_id != id);
            self.plugin_window_placement_overrides
                .retain(|key, _| key.plugin_id != id);
            if id == self.primary_panel_key.plugin_id {
                self.primary_panel_key = crate::plugin_panel::surface_key();
            }
            if let Err(error) = self.reconcile_installed_contributors() {
                tracing::warn!(%error,"contributor retirement refresh failed");
            }
            if self
                .shell_selection_preview
                .as_ref()
                .is_some_and(|preview| preview.selected == id || preview.owner == id)
            {
                self.recover_pending_shell(
                    "The preview shell or its requesting provider was disabled.",
                );
            }
            self.maybe_publish_plugin_status();
            return Ok(true);
        }
        let started = if let Some(external_panel) = external_panel {
            external_panel.map(|(runtime, panels)| {
                self.package_runtimes.insert(id.to_owned(), runtime);
                for (application, surface) in panels {
                    if self.is_shell_package(id) && !self.shell_package_selected(id) {
                        continue;
                    }
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
        } else {
            Err(format!("plugin {id:?} has no runtime host"))
        };
        let result = match started {
            Ok(()) => self.plugin_registry.mark_running(id).map(|()| true),
            Err(error) => {
                self.plugin_registry.mark_failed(id, error.clone())?;
                Err(error)
            }
        };
        if result.is_ok() {
            if self.plugin_registry.get(id).is_some_and(|entry| {
                entry.manifest.surfaces.is_empty() && entry.manifest.composition.is_some()
            }) {
                let _ = self.plugin_registry.record_memory(
                    id,
                    nickel_core::plugins::PluginMemory {
                        native_ui_bytes: Some(0),
                        texture_bytes: Some(0),
                        ..Default::default()
                    },
                );
            }
            if let Err(error) = self.reconcile_installed_contributors() {
                tracing::warn!(%error,"contributor admission refresh failed");
            }
        }
        self.maybe_publish_plugin_status();
        result
    }

    pub(crate) fn notification_preferred_surface_size(
        &self,
        maximum: (u32, u32),
    ) -> Option<(u32, u32)> {
        if self.notification_history_visible {
            return None;
        }
        self.notification.as_ref().map(|notification| {
            crate::notification_view::preferred_notification_surface_size(
                notification,
                self.palette,
                maximum,
            )
        })
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
        push("clock", Some(self.clock_deadline));
        push(
            "shell-selection-preview",
            self.shell_selection_preview
                .as_ref()
                .map(|preview| preview.deadline),
        );
        push("launcher-preferences", self.launcher_preference_deadline);
        push("lock", self.lock_deadline);
        push("control", self.control_deadline);
        push(
            "plugin-surface",
            self.plugin_surface_hosts
                .values()
                .filter_map(|(_, host)| host.next_deadline())
                .min(),
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
            SurfaceRole::Taskbar => None,
            SurfaceRole::Panel => self
                .primary_panel_host_ref()
                .map(|host| host_token(host.inspect())),
            SurfaceRole::Lock => Some(self.lock_change_token),
            SurfaceRole::Launcher => None,
            SurfaceRole::ControlCenter => self
                .quick_settings_surface_active()
                .then(|| {
                    host_token(
                        self.plugin_panel_host_ref(
                            &self.active_shell_surface_key("quick-settings"),
                        )
                        .unwrap()
                        .inspect(),
                    )
                })
                .or(Some(self.control_change_token)),
            SurfaceRole::Notification => {
                let trusted = self.trusted_notification_visible();
                let inspection = self.notification_host.inspect();
                let mut token = host_token(inspection);
                if trusted {
                    token.frame_generation = token.frame_generation.wrapping_add(1_u64 << 63);
                    token.semantic_generation = token.semantic_generation.wrapping_add(1_u64 << 63);
                }
                Some(token)
            }
            SurfaceRole::VolumeOsd => None,
            SurfaceRole::WindowPreview => self
                .preview_plugin_active()
                .then(|| host_token(self.preview_plugin_host_ref().unwrap().inspect())),
            SurfaceRole::WindowContextMenu => None,
            SurfaceRole::Screenshot => Some(self.screenshot.change_token()),
            SurfaceRole::OnScreenKeyboard => Some(host_token(self.keyboard_host.inspect())),
            SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat => None,
            #[cfg(target_os = "windows")]
            SurfaceRole::TrustedControl => None,
        }
    }

    pub fn poll_host_deadlines(&mut self, now: Instant) -> Vec<SurfaceRole> {
        let mut changed = Vec::new();
        if now >= self.clock_deadline {
            self.clock_deadline = now + crate::clock_capabilities::until_next_minute();
            changed.push(SurfaceRole::Panel);
        }
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
        let due_plugin_surfaces = self
            .plugin_surface_hosts
            .iter()
            .filter(|(_, (_, host))| host.next_deadline().is_some_and(|deadline| now >= deadline))
            .map(|(key, (surface, _))| (key.clone(), (surface.width, surface.height)))
            .collect::<Vec<_>>();
        for (key, size) in due_plugin_surfaces {
            let outcome = self.plugin_surface_host_event(&key, HostEvent::Poll, size, None, None);
            if outcome.changed {
                changed.push(self.plugin_surface_redraw_role(&key));
            }
        }
        changed
    }

    fn plugin_surface_redraw_role(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> SurfaceRole {
        if self.taskbar_surface_key().as_ref() == Some(key) {
            return SurfaceRole::Taskbar;
        }
        match key.surface_id.as_str() {
            "launcher" => SurfaceRole::Launcher,
            "quick-settings" => SurfaceRole::ControlCenter,
            "notifications" => SurfaceRole::Notification,
            "volume-osd" => SurfaceRole::VolumeOsd,
            "window-preview" => SurfaceRole::WindowPreview,
            "window-menu" => SurfaceRole::WindowContextMenu,
            "keyboard" => SurfaceRole::OnScreenKeyboard,
            _ => SurfaceRole::Panel,
        }
    }

    pub fn poll_deadlines(&mut self, now: Instant) -> ShellDeadlineOutcome {
        let shell_recovered = self
            .shell_selection_preview
            .as_ref()
            .is_some_and(|preview| now >= preview.deadline)
            && self.recover_pending_shell("Shell preview expired without confirmation.");
        let settings_changed = self.dispatch_pending_desktop_settings();
        let mut outcome = ShellDeadlineOutcome {
            visibility_changed: settings_changed || shell_recovered,
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
            self.hide_volume_osd();
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

        self.sync_notification_host(width, height);
        let point = Point { x, y };
        let outcome = self.notification_host.step(HostBatch {
            events: vec![
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                HostEvent::Ui(UiEvent::PointerReleased(point)),
            ],
            ..HostBatch::default()
        });
        outcome.changed | self.apply_notification_effects()
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

        self.sync_notification_host(420, 180);
        let outcome = self.notification_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        self.apply_notification_effects();
        outcome.changed
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

    fn preview_plugin_host_ref(
        &self,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.plugin_panel_host_ref(&self.active_shell_surface_key("window-preview"))
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
        self.plugin_pointer_paint = None;
        let pointer_only = passive_pointer_batch(&batch);
        let result = {
            let Some(host) = self.plugin_panel_host_for(key) else {
                return false;
            };
            let restore_focus = batch
                .window_focused
                .is_some_and(|focused| focused)
                .then(|| host.inspect().keyboard_focus)
                .flatten();
            let focused = batch.window_focused;
            step_plugin_host(host, None, batch).and_then(|(mut outcome, _)| {
                if let Some(focused) = focused {
                    host.application_mut().sync_surface_focus(focused)?;
                    outcome.changed |= host.application_mut().reconcile_surface_authority()?;
                }
                if let Some(target) = restore_focus
                    && host.inspect().keyboard_focus.is_none()
                {
                    let target = host
                        .accessibility_nodes()
                        .iter()
                        .find(|node| {
                            node.id.as_str().rsplit('/').next()
                                == target.as_str().rsplit('/').next()
                        })
                        .map(|node| node.id.clone())
                        .unwrap_or(target);
                    host.request_focus(target);
                    outcome.changed = true;
                }
                let paint_only = pointer_only
                    && outcome.messages.is_empty()
                    && matches!(
                        outcome.invalidation,
                        nickel_ui::Invalidation::None | nickel_ui::Invalidation::Paint
                    );
                Ok((
                    outcome.changed,
                    paint_only,
                    host.application_mut().take_effects(),
                ))
            })
        };
        let (changed, paint_only, effects) = match result {
            Ok(result) => result,
            Err(error) => return self.fail_plugin_panel_runtime(&key.plugin_id, error),
        };
        if paint_only && effects.is_empty() {
            self.plugin_pointer_paint = Some(key.clone());
            return changed;
        }
        let root_changed = match self.reconcile_plugin_surface_root(key) {
            Ok(changed) => changed,
            Err(error) => return self.fail_plugin_panel_runtime(&key.plugin_id, error),
        };
        changed
            | root_changed
            | self.apply_plugin_effects_with_keyboard_epoch(effects, keyboard_epoch)
    }

    /// Consume immediately after routing a pointer batch, before any data update.
    pub(crate) fn take_plugin_pointer_paint(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<Vec<PaintCommand>> {
        (self.plugin_pointer_paint.take().as_ref() == Some(key))
            .then(|| {
                self.plugin_panel_host_ref(key)
                    .map(|host| host.commands().to_vec())
            })
            .flatten()
    }

    pub(crate) fn plugin_panel_host_input_for(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        if *key == self.active_shell_surface_key("keyboard")
            && !self.native_surface_visible(SurfaceRole::Panel, Some(key))
        {
            return false;
        }
        let keyboard_epoch = (*key == self.active_shell_surface_key("keyboard"))
            .then(|| self.keyboard_gesture_epoch(&input))
            .flatten();

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
        if *key == self.active_shell_surface_key("keyboard") && action == ControllerAction::Cancel {
            return self.set_keyboard_visible(false);
        }
        let keyboard_epoch = (*key == self.active_shell_surface_key("keyboard"))
            .then(|| {
                self.keyboard_recipient
                    .as_ref()
                    .map(|snapshot| snapshot.epoch)
            })
            .flatten();

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

    pub(crate) fn plugin_panel_host_window_focus_for(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        focused: bool,
        width: u32,
        height: u32,
    ) -> bool {
        if self
            .plugin_panel_host_ref(key)
            .is_some_and(|host| host.inspect().window_focused == focused)
        {
            let Some(host) = self.plugin_panel_host_for(key) else {
                return false;
            };
            let published =
                host.application_mut()
                    .sync_surface_focus(focused)
                    .and_then(|changed| {
                        host.application_mut()
                            .reconcile_surface_authority()
                            .map(|rendered| changed || rendered)
                    });
            return match published {
                Ok(changed) => changed,
                Err(error) => self.fail_plugin_panel_runtime(&key.plugin_id, error),
            };
        }
        self.step_generic_plugin_surface(
            key,
            HostBatch {
                surface_size: Some((width, height)),
                window_focused: Some(focused),
                ..HostBatch::default()
            },
            None,
        )
    }

    pub(crate) fn plugin_panel_host_ui_for(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        event: UiEvent,
        width: u32,
        height: u32,
    ) -> bool {
        let keyboard_epoch = (*key == self.active_shell_surface_key("keyboard")
            && matches!(&event, UiEvent::AccessibilityActivate(_)))
        .then(|| {
            self.keyboard_recipient
                .as_ref()
                .map(|snapshot| snapshot.epoch)
        })
        .flatten();

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
                crate::plugin_panel::PluginEffect::SetApplicationScale { plugin_id, effect } => {
                    let granted = self.plugin_display_control_granted(&plugin_id) && !self.locked;
                    if granted {
                        // The transaction is synchronous: this owner cannot process
                        // a grant change until every guarded journal/native step ends.
                        match self.application_scale_service.execute(&effect, || {
                            if granted {
                                Ok(())
                            } else {
                                Err("display control grant was retired".into())
                            }
                        }) {
                            Ok(()) => changed = true,
                            Err(error) => {
                                tracing::warn!(%error, "application scale capability rejected");
                                changed = true;
                            }
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::IdentifyDisplays {
                    plugin_id,
                    revision,
                } => {
                    if self.plugin_display_control_granted(&plugin_id) && !self.locked {
                        let snapshot = self
                            .plugin_displays(&plugin_id)
                            .unwrap_or(serde_json::Value::Null);
                        if snapshot["revision"].as_str() == Some(&revision)
                            && snapshot["operations"]["identify"] == true
                        {
                            #[cfg(target_os = "linux")]
                            {
                                changed |= self.send_session_command(
                                    "identify-plugin-displays",
                                    ShellCommand::IdentifyOutputs,
                                );
                            }
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::SetDisplayLayout {
                    plugin_id,
                    layout,
                    revision,
                } => {
                    if self.plugin_display_control_granted(&plugin_id) && !self.locked {
                        changed |= self.preview_plugin_display_layout(plugin_id, layout, &revision);
                    }
                }
                crate::plugin_panel::PluginEffect::ConfirmDisplayLayout { plugin_id } => {
                    if self.plugin_display_control_granted(&plugin_id) && !self.locked {
                        changed |= self.confirm_plugin_display_layout(&plugin_id);
                    }
                }
                crate::plugin_panel::PluginEffect::RevertDisplayLayout { plugin_id } => {
                    if self.plugin_display_control_granted(&plugin_id) && !self.locked {
                        changed |= self.revert_plugin_display_layout(Some(&plugin_id));
                    }
                }
                effect @ crate::plugin_panel::PluginEffect::Keyboard { .. } => {
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
                } => match self
                    .show_plugin_window(&self.shell_surface_effect_owner(&plugin_id), &surface_id)
                {
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
                        plugin_id: self.shell_surface_effect_owner(&plugin_id),
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
                } => match self
                    .focus_plugin_window(&self.shell_surface_effect_owner(&plugin_id), &surface_id)
                {
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
                    &self.shell_surface_effect_owner(&plugin_id),
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
                    let result = Some(self.invoke_hidden_package_setting(&provider, &id, &value));
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
                crate::plugin_panel::PluginEffect::RunExecute { plugin_id, execute } => {
                    let revision = crate::run_capabilities::revision(
                        &plugin_id,
                        self.plugin_activation_generation,
                    );
                    if !self.native_ui_service_granted(
                        &plugin_id,
                        nickel_core::plugins::PluginCapability::RunCommand,
                    ) || !execute.is_current(&revision)
                    {
                        continue;
                    }
                    match platform::execute_run_command(&execute.command) {
                        Ok(()) => {
                            self.run_status
                                .insert(plugin_id, "Command submitted".into());
                            changed = true;
                        }
                        Err(error) => {
                            self.run_status.insert(
                                plugin_id,
                                format!("Could not run command: {}", launch_error_summary(&error)),
                            );
                            changed = true;
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::ToggleLauncher => {
                    self.request_launcher_toggle();
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::ToggleControlCenter => {
                    self.set_control_visible(!self.default_shell_surface_visible("quick-settings"));
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::ShowControlCenter => {
                    self.control_host.application_mut().dismiss();
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
                crate::plugin_panel::PluginEffect::WindowOperation {
                    plugin_id,
                    operation,
                    restore_focus,
                    destination,
                    window,
                } => {
                    if self.native_ui_service_granted(
                        &plugin_id,
                        nickel_core::plugins::PluginCapability::WindowsContext,
                    ) {
                        if operation == "windows.dismissMenu" && !self.locked {
                            self.dismiss_window_menu_with_focus(restore_focus);
                            changed = true;
                        } else {
                            changed |=
                                self.apply_public_window_operation(&operation, window, destination);
                        }
                    }
                }
                crate::plugin_panel::PluginEffect::ToggleOnScreenKeyboard { plugin_id } => {
                    if self.native_ui_service_granted(
                        &plugin_id,
                        nickel_core::plugins::PluginCapability::OnScreenKeyboardShow,
                    ) && self.keyboard_enabled
                    {
                        changed |= self.set_keyboard_visible(!self.keyboard_visible);
                    }
                }
                crate::plugin_panel::PluginEffect::ProjectsVisibility { plugin_id, toggle } => {
                    if self.native_ui_service_granted(
                        &plugin_id,
                        nickel_core::plugins::PluginCapability::ProjectsMenuShow,
                    ) {
                        changed |= self.show_projects_menu(toggle);
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

                crate::plugin_panel::PluginEffect::ActivateTrayItem { id } => {
                    if self.tray.iter().take(128).any(|item| item.id == id) {
                        self.tray_feed.activate(&id);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ContextTrayItem { id } => {
                    if self.tray.iter().take(128).any(|item| item.id == id) {
                        self.tray_feed.context_menu(&id);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::Feature { plugin_id, effect } => {
                    let check = || -> Result<(), String> {
                        if self.locked
                            || !self.session_host.feature_preference_writes_allowed()
                            || !self.plugin_registry.get(&plugin_id).is_some_and(|entry| {
                                entry.desired_enabled
                                    && entry.health == nickel_core::plugins::PluginHealth::Running
                                    && entry.manifest.capabilities.contains(
                                        &nickel_core::plugins::PluginCapability::FeaturesControl,
                                    )
                            })
                        {
                            return Err("feature authority is unavailable".into());
                        }
                        effect.validate(&self.feature_snapshot())
                    };
                    let result = nickel_core::optional_features::settings_path()
                        .map_err(|error| error.to_string())
                        .and_then(|path| {
                            crate::feature_capabilities::FeatureClient::apply(
                                &effect,
                                path,
                                &self.feature_snapshot(),
                                check,
                            )
                        })
                        .and_then(|settings| {
                            self.session_host.optional_features_committed(
                                settings.codex_generation,
                                settings.on_screen_keyboard_generation,
                            ).map_err(|error|if effect.commits_preference(){format!("Saved; native runtime has not applied the preference: {error}")}else{error})
                        });
                    self.feature_client.record(result);
                    self.refresh_keyboard();
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::SearchApplications { plugin_id, query } => {
                    let granted = self.plugin_registry.get(&plugin_id).is_some_and(|entry| {
                        entry.desired_enabled
                            && entry.health == nickel_core::plugins::PluginHealth::Running
                            && entry
                                .manifest
                                .capabilities
                                .contains(&nickel_core::plugins::PluginCapability::ApplicationsRead)
                    });
                    if granted && !self.locked {
                        match self.application_search.set_query(&plugin_id, query) {
                            Ok(()) => changed = true,
                            Err(error) => tracing::warn!(%error, "application search rejected"),
                        }
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
                        self.toggle_application_pin(&id);
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::RetryApplicationPinSave => {
                    if self.launcher_status.as_deref().is_some_and(|status| {
                        status.starts_with("Launcher preferences could not be saved:")
                    }) {
                        self.persist_launcher_preferences();
                        changed = true;
                    }
                }

                crate::plugin_panel::PluginEffect::SessionOperation { plugin_id, request } => {
                    let admitted = self
                        .plugin_registry
                        .get(&plugin_id)
                        .filter(|entry| entry.desired_enabled)
                        .and_then(|entry| {
                            crate::session_capabilities::snapshot(
                                self.locked,
                                &entry.manifest.capabilities,
                            )
                            .validate(&request, &entry.manifest.capabilities)
                            .ok()
                        });
                    if let Some(action) = admitted {
                        if self.task_switcher.session().is_some() {
                            self.apply_task_switch_action(
                                nickel_core::hotkeys::HotkeyAction::CancelSwitch,
                            );
                        }
                        changed |= self.send_session_command(
                            "plugin-session-operation",
                            ShellCommand::SessionAction(action.native()),
                        );
                    }
                }

                crate::plugin_panel::PluginEffect::InvokeNotification { plugin_id, id, key } => {
                    if self.notification_action_granted(&plugin_id, id)
                        && self.notification_feed.history().iter().any(|item| {
                            item.id == id && item.actions.iter().any(|action| action.key == key)
                        })
                    {
                        self.notification_feed.invoke(id, &key);
                        self.notification_feed.dismiss(id);
                        if self.notification.as_ref().is_some_and(|item| item.id == id) {
                            self.notification = None;
                        }
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::DismissNotification { plugin_id, id } => {
                    if self.notification_action_granted(&plugin_id, id) {
                        self.notification_feed.dismiss(id);
                        if self.notification.as_ref().is_some_and(|item| item.id == id) {
                            self.notification = None;
                        }
                        changed = true;
                    }
                }
                crate::plugin_panel::PluginEffect::ShellPreviewDecision { plugin_id, effect } => {
                    let result = self
                        .plugin_registry
                        .get(&plugin_id)
                        .filter(|entry| {
                            entry.desired_enabled
                                && entry.health == nickel_core::plugins::PluginHealth::Running
                        })
                        .and_then(|_| self.plugin_management(&plugin_id))
                        .ok_or("shell preview grant is unavailable".to_owned())
                        .and_then(|snapshot| effect.validate(&snapshot))
                        .and_then(|()| {
                            if effect.confirm {
                                self.confirm_shell_preview(Some(&plugin_id), effect.token)
                            } else {
                                self.revert_shell_preview(
                                    Some(&plugin_id),
                                    effect.token,
                                    "The previous shell was restored.",
                                )
                            }
                        });
                    if let Err(error) = result {
                        self.plugins_results.insert(
                            plugin_id,
                            serde_json::json!({"status":"rejected","detail":error}),
                        );
                    }
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::ShellSelection { plugin_id, effect } => {
                    let result = self
                        .plugin_registry
                        .get(&plugin_id)
                        .filter(|entry| {
                            entry.desired_enabled
                                && entry.health == nickel_core::plugins::PluginHealth::Running
                        })
                        .and_then(|_| self.plugin_management(&plugin_id))
                        .ok_or("shell selection grant is unavailable".to_owned())
                        .and_then(|snapshot| effect.validate(&snapshot))
                        .and_then(|()| self.begin_shell_preview(&plugin_id, &effect.id));
                    self.plugins_results.insert(
                        plugin_id,
                        match result {
                            Ok(_) => {
                                serde_json::json!({"status":if self.shell_selection_preview.is_some(){"preview"}else{"confirmed"},"selectedShell":effect.id})
                            }
                            Err(error) => serde_json::json!({"status":"rejected","detail":error}),
                        },
                    );
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::PluginsSetting { plugin_id, effect } => {
                    let result = self
                        .plugin_registry
                        .get(&plugin_id)
                        .filter(|entry| {
                            entry.desired_enabled
                                && entry.health == nickel_core::plugins::PluginHealth::Running
                        })
                        .and_then(|_| self.plugin_management(&plugin_id))
                        .ok_or_else(|| "plugin management is unavailable".to_owned())
                        .and_then(|snapshot| effect.validate(&snapshot))
                        .and_then(|()| {
                            self.set_plugin_setting(&effect.id, &effect.key, effect.value)
                        });
                    let value = match result {
                        Ok(_) => {
                            serde_json::json!({"status":"applied","id":effect.id,"key":effect.key})
                        }
                        Err(error) => {
                            serde_json::json!({"status":"rejected","detail":error.chars().take(512).collect::<String>()})
                        }
                    };
                    self.plugins_results.insert(plugin_id, value);
                    changed = true;
                }
                crate::plugin_panel::PluginEffect::Plugins { plugin_id, effect } => {
                    let result = self
                        .plugin_registry
                        .get(&plugin_id)
                        .filter(|entry| {
                            entry.desired_enabled
                                && entry.health == nickel_core::plugins::PluginHealth::Running
                        })
                        .and_then(|_| self.plugin_management(&plugin_id))
                        .ok_or_else(|| "plugin management read grant is unavailable".to_owned())
                        .and_then(|snapshot| effect.validate(&snapshot))
                        .and_then(|()| self.set_plugin_enabled(&effect.id, effect.enabled));
                    let value = match result {
                        Ok(_changed_state) => {
                            serde_json::json!({"status":"applied","id":effect.id,"enabled":effect.enabled})
                        }
                        Err(error) => {
                            serde_json::json!({"status":"rejected","detail":error.chars().take(512).collect::<String>()})
                        }
                    };
                    self.plugins_results.insert(plugin_id, value);
                    changed = true;
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
                    if matches!(
                        effect,
                        crate::appearance_capabilities::AppearanceEffect::ChooseImage { .. }
                    ) {
                        changed |= self.begin_wallpaper_chooser(plugin_id, effect);
                        continue;
                    }
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
                crate::plugin_panel::PluginEffect::Workspace { plugin_id, effect } => {
                    if self.public_native_granted(&plugin_id, effect.capability())
                        && self
                            .plugin_workspace_snapshot(&plugin_id)
                            .is_some_and(|snapshot| effect.validate(&snapshot).is_ok())
                    {
                        let command = match effect.operation.as_str() {
                            "workspaces.switch" => {
                                ShellCommand::SwitchWorkspace(effect.id.unwrap())
                            }
                            "workspaces.create" => ShellCommand::CreateWorkspace,
                            "workspaces.remove" => {
                                ShellCommand::RemoveWorkspace(effect.id.unwrap())
                            }
                            _ => continue,
                        };
                        changed |= self.send_session_command("plugin-workspaces", command);
                        changed |= self.refresh();
                    }
                }
                crate::plugin_panel::PluginEffect::ToggleShowDesktop { plugin_id } => {
                    if cfg!(target_os = "linux")
                        && self.public_native_granted(
                            &plugin_id,
                            nickel_core::plugins::PluginCapability::DesktopControl,
                        )
                    {
                        changed |= self.send_session_command(
                            "plugin-show-desktop",
                            ShellCommand::ToggleShowDesktop,
                        );
                        changed |= self.refresh();
                    }
                }
                crate::plugin_panel::PluginEffect::PreviewDisplayProjection {
                    plugin_id,
                    mode,
                    revision,
                } => {
                    if self.plugin_display_control_granted(&plugin_id)
                        && !self.locked
                        && self.projection_chooser.pending().is_none()
                    {
                        #[cfg(target_os = "linux")]
                        if let Ok(outputs) = self.session_host.projection_outputs() {
                            if let Some(layout) =
                                crate::display_capabilities::projection_layout(&outputs, mode)
                            {
                                changed |= self
                                    .preview_plugin_display_layout(plugin_id, layout, &revision);
                            }
                        }
                        #[cfg(not(target_os = "linux"))]
                        let _ = (mode, revision);
                    }
                }
                crate::plugin_panel::PluginEffect::WindowPreviewRequest {
                    plugin_id,
                    revision,
                    action,
                } => {
                    if self
                        .plugin_window_previews(&plugin_id)
                        .and_then(|v| {
                            v.get("revision")
                                .and_then(serde_json::Value::as_str)
                                .map(str::to_owned)
                        })
                        .as_deref()
                        == Some(&revision)
                        && self.plugin_registry.get(&plugin_id).is_some_and(|entry| {
                            entry.manifest.capabilities.contains(&match action {
                                PreviewAction::Activate(_) => {
                                    nickel_core::plugins::PluginCapability::WindowsFocus
                                }
                                _ => nickel_core::plugins::PluginCapability::WindowsContext,
                            })
                        })
                        && self.preview_plugin_action_allowed(action)
                    {
                        self.apply_preview_action(action);
                        changed = true;
                    }
                }
            }
        }
        changed
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
        if self.quick_settings_surface_active() {
            return self.plugin_surface_host_event(
                &self.active_shell_surface_key("quick-settings"),
                event,
                size,
                limit,
                authority,
            );
        }
        if !self
            .control_host
            .application()
            .view_state()
            .trusted_visible()
        {
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
    pub(crate) fn plugin_surface_field_lease(
        &self,
        key: &nickel_core::plugins::PluginSurfaceKey,
    ) -> Option<(nickel_ui::UiId, u64)> {
        let inspection = self.plugin_panel_host_ref(key)?.inspect();
        Some((
            inspection.keyboard_focus?,
            inspection.keyboard_focus_generation,
        ))
    }

    #[cfg(target_os = "linux")]
    pub(crate) fn shell_field_lease(&self, role: SurfaceRole) -> Option<(nickel_ui::UiId, u64)> {
        let inspection = match role {
            SurfaceRole::ControlCenter => self
                .quick_settings_surface_active()
                .then(|| {
                    self.plugin_panel_host_ref(&self.active_shell_surface_key("quick-settings"))
                        .unwrap()
                        .inspect()
                })
                .or_else(|| {
                    self.control_host
                        .application()
                        .view_state()
                        .trusted_visible()
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
            SurfaceRole::Taskbar => false,
            // Plugin panels always carry a surface key and use
            // `plugin_panel_host_ui_for` at the compositor boundary.
            SurfaceRole::Panel => false,
            SurfaceRole::Launcher => false,
            SurfaceRole::ControlCenter => {
                if !self.control_visible || !self.control_surface_available() {
                    return false;
                }
                if self.quick_settings_surface_active() {
                    return self
                        .plugin_surface_host_event(
                            &self.active_shell_surface_key("quick-settings"),
                            HostEvent::Ui(event),
                            (width, height),
                            None,
                            None,
                        )
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
            SurfaceRole::WindowContextMenu => false,
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
            SurfaceRole::Launcher => false,
            SurfaceRole::WindowContextMenu => false,
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

    #[cfg(any(target_os = "linux", test))]
    fn semantic_panel_output(&self, requested: Option<&String>) -> Option<String> {
        requested.cloned().or_else(|| self.panel_output.clone())
    }

    #[cfg(any(target_os = "linux", test))]
    fn semantic_panel_host(
        &self,
        _output: &Option<String>,
    ) -> Option<&nickel_ui::UiHost<crate::plugin_panel::PluginPanelApplication>> {
        self.taskbar_surface_key()
            .as_ref()
            .and_then(|key| self.plugin_panel_host_ref(key))
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
                let plugin_key = self.active_shell_surface_key("keyboard");
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
            ShellSemanticTarget::PanelCodex { output } => {
                let output = self.semantic_panel_output(output.as_ref());
                let plugin_host = self.semantic_panel_host(&output)?;
                let bounds = taskbar_plugin_control_bounds(plugin_host, "taskbar-codex")?;
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
                let bounds = if self.quick_settings_surface_active() {
                    taskbar_plugin_control_bounds(
                        self.plugin_panel_host_ref(
                            &self.active_shell_surface_key("quick-settings"),
                        )?,
                        "session-lock",
                    )?
                } else {
                    return None;
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

            ShellSemanticTarget::PanelApplication { .. }
            | ShellSemanticTarget::WindowMenu { .. }
            | ShellSemanticTarget::Screenshot { .. } => None,
        }
    }

    #[cfg(any(target_os = "linux", test))]
    pub fn perform_screenshot_semantic_action(
        &mut self,
        action: nickel_session_protocol::ScreenshotTargetAction,
    ) -> bool {
        self.screenshot.perform_semantic_action(action)
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

    pub(crate) fn active_plugin_output(&self) -> Option<&str> {
        self.panel_output
            .as_deref()
            .or(Some(self.desktop_host.application().active_output.as_str()))
    }

    fn switch_panel_output(&mut self, output: Option<String>) {
        self.panel_output = output;
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

    pub fn preview_controller(&mut self, action: ControllerAction) -> bool {
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
                self.open_window_menu_at(window.0, x, self.panel_origin_y);
            }
        }
    }

    pub fn preview_key(&mut self, key: Option<KeyCode>) -> bool {
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

    fn apply_public_window_operation(
        &mut self,
        operation: &str,
        window: Option<crate::model::WindowId>,
        destination: Option<String>,
    ) -> bool {
        if self.locked {
            return false;
        }
        if operation == "windows.dismissMenu" {
            self.dismiss_window_menu();
            return true;
        }
        let Some(window) = window else {
            return false;
        };
        let Some(current) = self.windows.iter().find(|current| current.id == window) else {
            return false;
        };
        if operation == "windows.showMenu" {
            return self.open_window_menu_at(window.0, self.panel_origin_x, self.panel_origin_y);
        }
        let state = &current.state;
        let action = match operation {
            "windows.minimize" if state.capabilities.minimize && !state.minimized => {
                Some(WindowAction::Minimize)
            }
            "windows.maximize" if state.capabilities.maximize && !state.maximized => {
                Some(WindowAction::Maximize)
            }
            "windows.toggleMaximize" if state.capabilities.maximize => Some(WindowAction::Maximize),
            "windows.restore" if state.minimized && state.capabilities.activate => {
                Some(WindowAction::Activate)
            }
            "windows.restore"
                if state.fullscreen
                    && cfg!(target_os = "linux")
                    && state.capabilities.fullscreen =>
            {
                Some(WindowAction::Fullscreen)
            }
            "windows.restore" if state.maximized && state.capabilities.maximize => {
                Some(WindowAction::Maximize)
            }
            "windows.toggleFullscreen"
                if cfg!(target_os = "linux") && state.capabilities.fullscreen =>
            {
                Some(WindowAction::Fullscreen)
            }
            "windows.snapLeading"
                if cfg!(target_os = "linux")
                    && state.capabilities.maximize
                    && !state.fullscreen =>
            {
                Some(WindowAction::SnapLeading)
            }
            "windows.snapTrailing"
                if cfg!(target_os = "linux")
                    && state.capabilities.maximize
                    && !state.fullscreen =>
            {
                Some(WindowAction::SnapTrailing)
            }
            _ => None,
        };
        if let Some(action) = action {
            return self.try_send_window_action(window, action);
        }
        match (operation, destination) {
            ("windows.moveToWorkspace", Some(destination)) if state.capabilities.move_workspace => {
                let Ok(workspace) = destination.parse::<u64>() else {
                    return false;
                };
                if !self
                    .workspaces
                    .iter()
                    .any(|candidate| candidate.id == workspace)
                {
                    return false;
                }
                self.send_session_command(
                    "move-window-to-workspace",
                    ShellCommand::MoveWindowToWorkspace { window, workspace },
                )
            }
            ("windows.moveToOutput", Some(output)) if state.capabilities.move_display => {
                if !self.window_feed.outputs().contains(&output) {
                    return false;
                }
                self.send_session_command(
                    "move-window-to-display",
                    ShellCommand::MoveWindowToDisplay { window, output },
                )
            }
            _ => false,
        }
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
    }

    fn preview_origin_x(&self, _index: usize, width: u32) -> i32 {
        self.panel_origin_x - (width / 2) as i32
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
        if self.locked || !self.active_shell_declares("window-preview") {
            return;
        }
        let groups = self.panel_groups();
        if groups
            .get(index)
            .is_none_or(|group| group.windows.is_empty())
        {
            return;
        }
        self.set_default_shell_surface_visible("window-preview", true);
        if self.preview_plugin_host_ref().is_none() {
            return;
        }
        if self.preview_group == Some(index) {
            self.preview_pending = None;
            return;
        }
        self.preview_pending = None;
        self.preview_group = Some(index);
        self.preview_generation = self.preview_generation.wrapping_add(1).max(1);
        self.preview_images.clear();
        self.preview_refresh_deadline = None;
        self.preview_hovered = None;
        self.window_menu = None;
        self.window_menu_snapshot = None;
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
        self.set_default_shell_surface_visible("window-menu", false);
        self.preview_generation = self.preview_generation.wrapping_add(1).max(1);
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

        let _ =
            self.send_session_command("clear-window-highlight", ShellCommand::ClearWindowHighlight);
    }

    fn clear_preview_plugin_payload(&mut self) {
        if self.task_switcher_group.is_none() && self.preview_group.is_none() {
            self.set_default_shell_surface_visible("window-preview", false);
        }
    }

    fn dismiss_window_menu(&mut self) {
        self.dismiss_window_menu_with_focus(true);
    }

    fn dismiss_window_menu_with_focus(&mut self, restore_focus: bool) {
        self.set_default_shell_surface_visible("window-menu", false);
        let focused_menu = self.window_menu.is_some();
        self.close_window_preview();
        if focused_menu && restore_focus {
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
                self.apply_launcher_signal(!self.default_shell_surface_visible("launcher"));
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
                    if locked {
                        self.recover_pending_shell(
                            "The shell preview ended when the session locked.",
                        );
                    }
                    if locked {
                        self.hide_volume_osd();
                    }
                    let application = self.lock_host.application_mut();
                    application.password.zeroize();
                    application.status = None;
                    if locked {
                        self.desktop_host
                            .application_mut()
                            .dismiss_context_menu(desktop::DesktopMenuDismissReason::FocusDeparted);
                        self.set_default_shell_surface_visible("launcher", false);
                        self.set_default_shell_surface_visible("quick-settings", false);
                        self.launcher_visible = false;
                        self.control_visible = false;
                        self.codex_project_menu_visible = false;
                        self.close_window_preview();
                    }
                    true
                }
            }
            platform::GlobalShortcut::ShowRun => {
                self.set_default_shell_surface_visible("run", true)
            }
            platform::GlobalShortcut::OpenFiles => self.launch_named_application("Nickel File"),
            platform::GlobalShortcut::OpenSettings => self.launch_settings(None),
            platform::GlobalShortcut::ShowControlCenter => {
                self.control_host.application_mut().dismiss();
                self.set_control_visible(true);
                true
            }
            platform::GlobalShortcut::ShowNotifications => {
                if !self.trusted_notification_visible() {
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
                self.rollback_projection();
                self.revert_plugin_display_layout(None);
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
                    self.show_volume_osd();
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
                if available {
                    self.show_volume_osd();
                } else {
                    self.hide_volume_osd();
                }
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
        self.preview_generation = self.preview_generation.wrapping_add(1).max(1);
        if self.active_shell_declares("window-preview") && !self.locked {
            self.set_default_shell_surface_visible("window-preview", true);
        }
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
            || self.notification_host.pointer_interaction_active()
            || self.control_host.pointer_interaction_active()
            || self
                .plugin_panel_host_ref(&self.active_shell_surface_key("quick-settings"))
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
        let visible = !self.default_shell_surface_visible("launcher");
        self.set_launcher_visible(visible);
        self.default_shell_surface_visible("launcher") == visible
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
        if visible {
            self.set_default_shell_surface_visible("quick-settings", false);
        }
        self.set_default_shell_surface_visible("launcher", visible);
    }

    pub(crate) fn launcher_intent_visible(&self) -> bool {
        self.active_launcher_surface_key().is_some()
    }

    pub(crate) fn can_show_launcher(&self) -> bool {
        self.active_shell_declares("launcher")
    }

    fn set_control_visible(&mut self, visible: bool) {
        if !visible && self.shell_selection_preview.is_some() {
            self.recover_pending_shell("Shell preview dismissed.");
            return;
        }
        if !self
            .control_host
            .application()
            .view_state()
            .trusted_visible()
        {
            if visible {
                self.set_default_shell_surface_visible("launcher", false);
            }
            self.set_default_shell_surface_visible("quick-settings", visible);
            return;
        }

        if visible && !self.control_surface_available() {
            return;
        }
        #[cfg(target_os = "linux")]
        if !self.send_session_command(
            "control-center-focus",
            if visible {
                if self.quick_settings_surface_active() {
                    ShellCommand::FocusPluginSurface {
                        key: self.active_shell_surface_key("quick-settings"),
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
        if !visible && self.shell_selection_preview.is_some() {
            self.recover_pending_shell("Shell preview dismissed.");
            return;
        }
        self.control_visible = visible;
        if !visible {
            self.control_host.application_mut().dismiss();
        }
    }

    fn apply_launcher_signal(&mut self, visible: bool) {
        self.set_launcher_visible(visible);
    }

    pub(crate) fn apply_session_launcher_visibility(&mut self, visible: bool) {
        self.set_default_shell_surface_visible("launcher", visible);
    }

    pub fn control_click(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        if !self.control_surface_available() {
            return false;
        }
        if self.quick_settings_surface_active() {
            let point = Point { x, y };
            let pressed = self.plugin_surface_host_event(
                &self.active_shell_surface_key("quick-settings"),
                HostEvent::Ui(UiEvent::PointerPressed(point)),
                (width, height),
                None,
                None,
            );
            let released = self.plugin_surface_host_event(
                &self.active_shell_surface_key("quick-settings"),
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
        if self.quick_settings_surface_active() {
            return self
                .plugin_surface_host_event(
                    &self.active_shell_surface_key("quick-settings"),
                    HostEvent::Controller(action),
                    (width, height),
                    None,
                    None,
                )
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
        if self.quick_settings_surface_active() {
            let changed = self
                .plugin_surface_host_event(
                    &self.active_shell_surface_key("quick-settings"),
                    HostEvent::Controller(action),
                    (width, height),
                    None,
                    None,
                )
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
            SurfaceRole::ControlCenter if self.shell_selection_preview.is_some() => false,
            SurfaceRole::ControlCenter => {
                if self.quick_settings_surface_active() {
                    return self.set_default_shell_surface_visible("quick-settings", false);
                }
                self.control_host.application_mut().dismiss();
                std::mem::replace(&mut self.control_visible, false)
            }
            SurfaceRole::CodexProjectMenu => {
                std::mem::replace(&mut self.codex_project_menu_visible, false)
            }
            SurfaceRole::WindowContextMenu => {
                let visible = self.window_menu.is_some();
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
        self.request_settings_navigation(screen, None);
        self.set_default_shell_surface_visible("launcher", false);
        self.set_default_shell_surface_visible("settings", true)
    }

    fn request_settings_navigation(&mut self, screen: Option<&str>, output: Option<&str>) {
        let destination = match screen {
            Some("display") => "displays",
            Some("network") => "wifi",
            Some("nickel-bar") => "shell-preferences",
            Some("keyboard-shortcuts") => "keyboard-shortcuts",
            Some(other) => other,
            None => return,
        };
        self.settings_navigation_revision =
            self.settings_navigation_revision.wrapping_add(1).max(1);
        self.settings_navigation = Some(
            serde_json::json!({"destination":destination,"revision":self.settings_navigation_revision.to_string(),"output":output}),
        );
    }

    fn dispatch_pending_desktop_settings(&mut self) -> bool {
        let Some(destination) = self.desktop_host.application_mut().pending_settings.take() else {
            return false;
        };
        match destination {
            SettingsDestination::Appearance => self.launch_settings(Some("appearance")),
            SettingsDestination::Display { output } => {
                self.launch_settings(Some("displays"));
                self.request_settings_navigation(Some("displays"), Some(&output));
                true
            }
        }
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
        if self.locked || !self.active_shell_declares("window-menu") {
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
        if !self.set_default_shell_surface_visible("window-menu", true)
            && !self.default_shell_surface_visible("window-menu")
        {
            return false;
        }
        self.window_menu_generation = self.window_menu_generation.saturating_add(1);
        self.window_menu = Some(snapshot.id);
        self.window_menu_snapshot = Some(snapshot);
        let owner = self.active_shell_package_id.clone();
        let _ = self.set_plugin_window_placement(
            &owner,
            "window-menu",
            nickel_core::plugins::PluginSurfaceAnchor::TopLeft,
            x.clamp(-8192, 8192),
            y.clamp(-8192, 8192),
        );
        let _ = self.focus_plugin_window(&owner, "window-menu");
        true
    }

    fn launch_application(&mut self, application: Application) {
        #[cfg(target_os = "linux")]
        {
            let secure_storage_state = self.session_host.secure_storage_state().unwrap_or_else(
                |error| {
                    tracing::warn!(%error, "secure-storage query failed before application launch");
                    platform::SecureStorageState::ControlUnavailable
                },
            );
            if !platform::secure_storage_allows_application_launch(secure_storage_state) {
                if let Err(error) = self.session_host.request_secure_storage_retry() {
                    tracing::warn!(%error, "secure-storage retry command failed");
                }
                self.launcher_status = Some(format!(
                    "Secure storage is not ready. {} will remain blocked until your existing wallet is available.",
                    application.name()
                ));
                return;
            }
        }
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
            let history = self
                .notification_feed
                .history()
                .into_iter()
                .filter(|item| self.trusted_notification_id(item.id))
                .collect::<Vec<_>>();
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

    fn plugin_run_snapshot(&self, id: &str) -> Option<serde_json::Value> {
        self.native_ui_service_granted(id, nickel_core::plugins::PluginCapability::RunCommand).then(|| serde_json::json!({
            "available":true,"revision":crate::run_capabilities::revision(id,self.plugin_activation_generation),"status":self.run_status.get(id)
        }))
    }

    fn native_ui_service_granted(
        &self,
        plugin_id: &str,
        grant: nickel_core::plugins::PluginCapability,
    ) -> bool {
        !self.locked
            && self.plugin_registry.get(plugin_id).is_some_and(|entry| {
                entry.desired_enabled
                    && entry.health == nickel_core::plugins::PluginHealth::Running
                    && entry.manifest.capabilities.contains(&grant)
            })
    }

    fn show_projects_menu(&mut self, toggle: bool) -> bool {
        if !self.launcher.codex_available() || self.locked {
            return false;
        }
        let visible = !toggle || !self.codex_project_menu_visible;
        let changed = self.codex_project_menu_visible != visible;
        if visible {
            self.set_launcher_visible(false);
        }
        self.codex_project_menu_visible = visible;
        changed
    }

    fn notification_action_granted(&self, plugin_id: &str, id: u32) -> bool {
        !self.locked
            && !self.trusted_notification_id(id)
            && self
                .notification_feed
                .history()
                .iter()
                .any(|item| item.id == id)
            && self.plugin_registry.get(plugin_id).is_some_and(|entry| {
                entry.desired_enabled
                    && entry.health == nickel_core::plugins::PluginHealth::Running
                    && entry
                        .manifest
                        .capabilities
                        .contains(&nickel_core::plugins::PluginCapability::NotificationsAct)
            })
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
            &self.active_shell_surface_key("window-preview"),
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
        if self.locked || !self.preview_plugin_active() {
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
            PreviewAction::OpenMenu(_) => true,
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

    fn plugin_window_previews(&mut self, id: &str) -> Option<serde_json::Value> {
        if !self
            .plugin_registry
            .get(id)
            .is_some_and(|entry| entry.health == nickel_core::plugins::PluginHealth::Running)
            || !self
                .external_plugin_packages
                .get(id)?
                .manifest
                .capabilities
                .contains(&nickel_core::plugins::PluginCapability::WindowsRead)
        {
            return None;
        }
        if self.locked {
            return Some(serde_json::json!({"available":false,"windows":[]}));
        }
        let group = self.preview_plugin_group();
        let switcher = self.task_switcher_group.is_some();
        let selected = self.task_switcher.selected().copied();
        Some(crate::window_preview_capabilities::snapshot(
            group.as_ref(),
            switcher,
            selected,
            self.plugin_activation_generation,
            self.preview_generation,
        ))
    }

    fn plugin_window_preview_images(&mut self, id: &str) -> crate::plugin_panel::PluginImages {
        if self.locked || self.plugin_window_previews(id).is_none() {
            return Default::default();
        }
        let Some(group) = self.preview_plugin_group() else {
            return Default::default();
        };
        group
            .windows
            .iter()
            .take(if self.task_switcher_group.is_some() {
                5
            } else {
                12
            })
            .enumerate()
            .filter_map(|(index, window)| {
                self.preview_images.get(&window.id).map(|image| {
                    (
                        format!("window:{}", window.id.0),
                        (0x7000 + index as u16, Arc::clone(image)),
                    )
                })
            })
            .collect()
    }

    fn preview_plugin_event(
        &mut self,
        event: HostEvent,
        size: (u32, u32),
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> nickel_ui::HostEventOutcome {
        self.plugin_surface_host_event(
            &self.active_shell_surface_key("window-preview"),
            event,
            size,
            None,
            authority,
        )
    }

    pub fn set_global_shortcut_capability(
        &mut self,
        capability: &nickel_input::global::ShortcutCapability,
    ) {
        self.shortcut_capability_observed = true;
        self.shortcut_capability_status = shortcut_capability_status(capability);
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
        self.control_host
            .application_mut()
            .sync_projection_modes(&supported_projection_modes);
        self.step_control_host(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }

    fn quick_settings_surface_active(&self) -> bool {
        self.plugin_panel_host_ref(&self.active_shell_surface_key("quick-settings"))
            .is_some()
            && !self
                .control_host
                .application()
                .view_state()
                .trusted_visible()
    }

    pub(crate) fn control_surface_available(&self) -> bool {
        self.active_shell_declares("quick-settings")
            || self
                .control_host
                .application()
                .view_state()
                .trusted_visible()
    }

    pub(crate) fn plugin_surface_host_event(
        &mut self,
        key: &nickel_core::plugins::PluginSurfaceKey,
        event: HostEvent,
        size: (u32, u32),
        limit: Option<usize>,
        authority: Option<nickel_ui::NormalizedIngressAuthority>,
    ) -> nickel_ui::HostEventOutcome {
        let batch = HostBatch {
            surface_size: Some(size),
            clipboard_text_limit: limit,
            events: vec![event],
            normalized_authorities: authority.into_iter().collect(),
            ..HostBatch::default()
        };
        if !passive_pointer_batch(&batch) {
            let _ = self.plugin_panel_scene(key, size.0, size.1);
        }
        let Some(host) = self.plugin_panel_host_for(key) else {
            return nickel_ui::HostEventOutcome::default();
        };
        let mut outcome = host.step(batch);
        let effects = host.application_mut().take_effects();
        if let Some(error) = host.application_mut().take_runtime_failure() {
            self.fail_plugin_panel_runtime(&key.plugin_id, error);
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

    fn toggle_application_pin(&mut self, id: &str) {
        self.launcher.toggle_pin(id);
        self.persist_launcher_preferences();
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
            ControlAction::PreviewProjection(mode) => {
                if !self.preview_projection(mode) {
                    self.control_host
                        .application_mut()
                        .projection_preview_failed();
                }
            }
            ControlAction::ConfirmShellPreview(token) => {
                if let Err(error) = self.confirm_shell_preview(None, token) {
                    tracing::warn!(%error,"trusted shell confirmation failed");
                }
            }
            ControlAction::RevertShellPreview(token) => {
                if !self.locked {
                    let _ = self.revert_shell_preview(
                        None,
                        token,
                        "The previous shell was restored from trusted recovery.",
                    );
                }
            }
            ControlAction::ConfirmProjection => {
                self.projection_chooser.confirm();
                self.projection_rollback_deadline = None;
            }
            ControlAction::CancelProjection => self.rollback_projection(),
            _ => {}
        }
        let _ = self.refresh();
    }

    fn preview_projection(
        &mut self,
        mode: nickel_core::display_projection::ProjectionMode,
    ) -> bool {
        if self.shell_selection_preview.is_some() {
            return false;
        }
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
                        transform: None,
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
                        transform: None,
                    })
                    .collect(),
            };
            let _ = self
                .send_session_command("rollback-projection", ShellCommand::ApplyOutputs(layout));
        }
    }

    fn plugin_display_control_granted(&self, plugin_id: &str) -> bool {
        self.external_plugin_packages
            .get(plugin_id)
            .map(|package| &package.manifest)
            .or_else(|| {
                self.plugin_registry
                    .get(plugin_id)
                    .map(|entry| &entry.manifest)
            })
            .is_some_and(|manifest| {
                manifest
                    .capabilities
                    .contains(&nickel_core::plugins::PluginCapability::DisplayControl)
            })
    }

    fn preview_plugin_display_layout(
        &mut self,
        plugin_id: String,
        layout: nickel_session_protocol::OutputLayout,
        expected_revision: &str,
    ) -> bool {
        if self.shell_selection_preview.is_some() {
            return false;
        }
        #[cfg(target_os = "linux")]
        {
            if self
                .control_host
                .application()
                .view_state()
                .trusted_visible()
                || self.display_preview.is_some()
                || self.projection_rollback_deadline.is_some()
            {
                return false;
            }
            let Ok(outputs) = self.session_host.projection_outputs() else {
                return false;
            };
            if crate::display_capabilities::revision(&outputs) != expected_revision {
                return false;
            }
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
            match crate::windows_plugin_display::set_layout(&plugin_id, &layout, expected_revision)
            {
                Ok(()) => true,
                Err(error) => {
                    tracing::warn!(%error, "plugin display layout preview failed");
                    false
                }
            }
        }
        #[cfg(not(any(target_os = "linux", target_os = "windows")))]
        {
            let _ = (plugin_id, layout, expected_revision);
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
            #[cfg(target_os = "linux")]
            {
                let Ok(outputs) = self.session_host.projection_outputs() else {
                    return false;
                };
                if self
                    .display_preview
                    .as_ref()
                    .is_none_or(|preview| output_layout_from_snapshot(&outputs) != preview.applied)
                {
                    return false;
                }
            }
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

fn update_feed_status(current: &mut FeedStatus, next: FeedStatus, feed: &'static str) -> bool {
    if *current == next {
        return false;
    }
    tracing::info!(feed, status = ?next, "shell feed state changed");
    *current = next;
    true
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
