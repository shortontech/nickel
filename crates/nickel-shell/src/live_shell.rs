use std::{
    collections::{HashMap, VecDeque},
    sync::Arc,
    time::{Duration, Instant},
};

use nickel_core::task_switcher::{SwitchWindow, TaskSwitchEffect, TaskSwitcher};
use nickel_core::{
    launcher_preferences::LauncherPreferences,
    shell_settings::ShellSettings,
    theme::{Appearance, ThemePalette},
    wallpaper_settings::WallpaperSettings,
};
use nickel_file::desktop::{DesktopOutput, Point as DesktopPoint};
use nickel_session_protocol::ShellRole;
#[cfg(any(target_os = "linux", test))]
use nickel_session_protocol::{
    AnchorSide, Geometry, PointerInteraction, PreviewTargetAction, ResolvedShellTarget,
    ShellPopoverAnchor, ShellSemanticTarget, WindowMenuTargetAction,
};
use nickel_ui::Rect;
use nickel_ui::backend::PaintCommand;
use nickel_ui::{
    Application as UiApplication, Button, Column, Container, ControllerAction, HostBatch,
    HostChangeToken, HostEvent, Insets, Layer, Point, SemanticRole, Shortcut, Size, Spacer, Text,
    TextAlign, TextField, UiEvent, UiHostViewport, ViewContext,
};

use crate::{
    control_view::{ControlAction, ControlCenterApp, ControlCenterHost},
    launcher::{DashboardAccount, DashboardProject, DashboardSection, Launcher},
    launcher_view::{
        LauncherAction, LauncherApplication, LauncherIconCache, LauncherShellEffect,
        LauncherViewState, reduce_launcher_action,
    },
    model::{Application, OpenWindow, TrayItem, WindowGroup},
    notification::DesktopNotification,
    notification_view::{NotificationApp, NotificationEffect, NotificationHost},
    platform::{
        self, AudioStatus, BluetoothStatus, FeedState, FeedStatus, NetworkStatus, NotificationFeed,
        NotificationSource, ShellCommand, TrayFeed, TraySource, WindowAction, WindowFeed,
    },
    screenshot::ScreenshotTool,
    window_preview::{
        ApplicationMenuAction, ApplicationMenuApp, ApplicationMenuTarget, MENU_WIDTH, MenuAction,
        PreviewAction, TaskbarPreviewAnchor, WindowMenuApp, WindowPreviewFrame,
        application_menu_entries, build_preview_frame, menu_height, menu_height_for_rows,
        preview_dimensions, semantic_theme_from_palette, validated_application_close_targets,
        window_menu_action_is_current, window_menu_max_rows,
    },
    winit_shell::SurfaceRole,
};
use nickel_input::KeyCode;
use zeroize::{Zeroize, Zeroizing};

const RUN_COMMAND_LIMIT: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq)]
enum RunAction {
    SetCommand(String),
    Submit,
    Dismiss,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum RunEffect {
    Submit(String),
    Dismiss,
}

struct RunApplication {
    command: String,
    status: Option<String>,
    palette: ThemePalette,
    effects: Vec<RunEffect>,
    dirty: bool,
}

impl RunApplication {
    fn new(palette: ThemePalette) -> Self {
        Self {
            command: String::new(),
            status: None,
            palette,
            effects: Vec::new(),
            dirty: false,
        }
    }

    fn take_effects(&mut self) -> Vec<RunEffect> {
        std::mem::take(&mut self.effects)
    }
}

impl UiApplication for RunApplication {
    type Message = RunAction;

    fn update(&mut self, message: Self::Message) {
        match message {
            RunAction::SetCommand(command) => {
                self.command = command.chars().take(RUN_COMMAND_LIMIT).collect();
                self.status = None;
                self.dirty = true;
            }
            RunAction::Submit if !self.command.trim().is_empty() => {
                self.effects
                    .push(RunEffect::Submit(self.command.trim().to_owned()));
            }
            RunAction::Submit => {}
            RunAction::Dismiss => self.effects.push(RunEffect::Dismiss),
        }
    }

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        match shortcut {
            Shortcut::Submit => self.update(RunAction::Submit),
            Shortcut::Escape => self.update(RunAction::Dismiss),
            _ => return false,
        }
        true
    }

    fn poll(&mut self) -> bool {
        std::mem::take(&mut self.dirty)
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let mut content = Column::new()
            .gap(10.0)
            .child(
                Text::new("Run")
                    .scale(22.0)
                    .color(self.palette.text)
                    .bold(true),
            )
            .child(
                TextField::on_change_with_placeholder(
                    &self.command,
                    "Enter a command",
                    RunAction::SetCommand,
                )
                .id("run-command")
                .accessibility_label("Command")
                .color(self.palette.text),
            )
            .child(Button::new(RunAction::Submit, "Run").id("run-submit"));
        if let Some(status) = &self.status {
            content = content.child(Text::new(status).color(self.palette.complement));
        }
        Container::new()
            .id("run-dialog")
            .semantic_role(SemanticRole::Dialog)
            .accessibility_label("Run command")
            .width(context.viewport.size.width)
            .height(context.viewport.size.height)
            .padding(Insets::all(18.0))
            .background(self.palette.panel)
            .child(content)
    }
}

fn launcher_controller_host_event(action: ControllerAction, overlay_open: bool) -> HostEvent {
    if action == ControllerAction::Cancel && !overlay_open {
        HostEvent::Shortcut(Shortcut::Escape)
    } else {
        HostEvent::Controller(action)
    }
}

const PANEL_ITEM_WIDTH: f32 = 52.0;
const PANEL_CLOCK_WIDTH: f32 = 96.0;
#[cfg(test)]
const PANEL_CONTROL_GAP: f32 = 8.0;
const PANEL_TRAY_WIDTH: f32 = 28.0;
const PANEL_TRAY_ICON_SIZE: u32 = 18;
const PANEL_CODEX_WIDTH: f32 = 36.0;
const PANEL_CODEX_ICON_SIZE: f32 = 28.0;
const PREVIEW_LEAVE_DELAY: Duration = Duration::from_millis(500);
const PREVIEW_HOVER_DELAY: Duration = Duration::from_millis(350);
const PREVIEW_REFRESH_INTERVAL: Duration = Duration::from_millis(500);
#[cfg(target_os = "linux")]
const RECURRING_DIAGNOSTIC_INTERVAL: Duration = Duration::from_secs(30);
const WALLPAPER_MAX_WIDTH: u32 = 7680;
const WALLPAPER_MAX_HEIGHT: u32 = 4320;
const PREVIEW_CACHE_CAPACITY: usize = 32;

#[path = "live_shell/panel.rs"]
mod panel;
pub use panel::{PanelAction, PanelApplication};
use panel::{
    PanelHover, normalize_tray_items, panel_clock_text, panel_tray_icons, tint_panel_icon,
};
#[cfg(test)]
use panel::{panel_status_layout, visible_tray_item};

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
    effects: Vec<LockEffect>,
}

struct VolumeOsdApplication {
    label: String,
    percent: u8,
    palette: ThemePalette,
}

impl nickel_ui::Application for VolumeOsdApplication {
    type Message = ();

    fn update(&mut self, (): Self::Message) {}

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        let track_width = (width - 48.0).max(0.0);
        Container::new()
            .id("volume-osd")
            .semantic_role(SemanticRole::Status)
            .accessibility_label(self.label.clone())
            .width(width)
            .height(height)
            .background(self.palette.panel)
            .radius(14.0)
            .child(
                Layer::new()
                    .width(width)
                    .height(height)
                    .child(
                        Container::new()
                            .position(Point { x: 24.0, y: 14.0 })
                            .width(track_width)
                            .height(32.0)
                            .child(
                                Text::new(self.label.clone())
                                    .width(track_width)
                                    .height(32.0)
                                    .scale(1.15)
                                    .color(self.palette.text)
                                    .align(TextAlign::Center)
                                    .bold(true),
                            ),
                    )
                    .child(
                        Container::new()
                            .position(Point {
                                x: 24.0,
                                y: height - 28.0,
                            })
                            .width(track_width)
                            .height(8.0)
                            .background(self.palette.surface_hover)
                            .radius(4.0),
                    )
                    .child(
                        Container::new()
                            .position(Point {
                                x: 24.0,
                                y: height - 28.0,
                            })
                            .width(track_width * f32::from(self.percent) / 100.0)
                            .height(8.0)
                            .background(self.palette.accent)
                            .radius(4.0),
                    ),
            )
    }
}

#[cfg(any(test, feature = "workbench-fixtures"))]
impl LockApplication {
    #[allow(dead_code)] // Binary and fixture library compile this shared module separately.
    pub fn fixture(password: &str, status: Option<String>) -> Self {
        Self {
            password: Zeroizing::new(password.to_owned()),
            status,
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

    fn shortcut(&mut self, shortcut: Shortcut) -> bool {
        if shortcut != Shortcut::Submit {
            return false;
        }
        self.effects
            .push(LockEffect::Authenticate(std::mem::take(&mut self.password)));
        true
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        let width = context.viewport.size.width;
        let height = context.viewport.size.height;
        let username = std::env::var("USER").unwrap_or_else(|_| "Session locked".into());
        let password_color = if self.password.is_empty() {
            0x8992a6
        } else {
            0xffffff
        };
        let mut content = Column::new()
            .width(width)
            .height(height)
            .child(Spacer::vertical(height * 0.38))
            .child(
                Text::new("Nickel")
                    .height(48.0)
                    .scale(30.0)
                    .color(0xffffff)
                    .align(TextAlign::Center)
                    .bold(true),
            )
            .child(Spacer::vertical(8.0))
            .child(
                Text::new(username)
                    .height(32.0)
                    .scale(18.0)
                    .color(0xb8c0d4)
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
                    .background(0x20283a)
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
                            "Password",
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
                    .color(0xff9a9a)
                    .align(TextAlign::Center),
            );
        }
        Container::new()
            .id("lock-screen")
            .accessibility_label("Session locked")
            .background(0x080b12)
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

pub struct LiveShell {
    host_runtime_samples: HostRuntimeSamples,
    launcher: Launcher,
    window_feed: WindowFeed,
    tray_feed: TrayFeed,
    notification_feed: NotificationFeed,
    windows: Vec<OpenWindow>,
    window_icons: HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    task_switcher: TaskSwitcher<crate::model::WindowId>,
    task_switcher_group: Option<WindowGroup>,
    workspaces: Vec<platform::WorkspaceSummary>,
    window_feed_status: FeedStatus,
    workspace_feed_status: FeedStatus,
    tray: Vec<TrayItem>,
    tray_icons: Vec<Arc<image::RgbaImage>>,
    notification: Option<DesktopNotification>,
    notification_history_visible: bool,
    wallpaper_path: Option<std::path::PathBuf>,
    wallpaper_source_fingerprint: Option<WallpaperSourceFingerprint>,
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
    panel_icon: Arc<image::RgbaImage>,
    codex_icon: Arc<image::RgbaImage>,
    palette: ThemePalette,
    network: NetworkStatus,
    bluetooth: BluetoothStatus,
    audio: AudioStatus,
    volume_osd_until: Option<Instant>,
    volume_osd_host: nickel_ui::UiHost<VolumeOsdApplication>,
    launcher_visible: bool,
    run_visible: bool,
    run_host: nickel_ui::UiHost<RunApplication>,
    locked: bool,
    lock_host: nickel_ui::UiHost<LockApplication>,
    lock_change_token: HostChangeToken,
    lock_deadline: Option<Instant>,
    control_visible: bool,
    codex_project_menu_visible: bool,
    panel_hover: Option<PanelHover>,
    panel_hover_output: Option<String>,
    panel_host: nickel_ui::UiHost<PanelApplication>,
    panel_change_token: HostChangeToken,
    panel_deadline: Option<Instant>,
    panel_output: Option<String>,
    pending_popover_anchor: Option<PendingPopoverAnchor>,
    all_windows_on_every_bar: bool,
    preview_group: Option<usize>,
    preview_pending: Option<(usize, Instant)>,
    preview_focus_requested: bool,
    preview_pointer_inside: bool,
    preview_leave_deadline: Option<Instant>,
    preview_hovered: Option<crate::model::WindowId>,
    preview_images: HashMap<crate::model::WindowId, Arc<image::RgbaImage>>,
    preview_refresh_deadline: Option<Instant>,
    preview_frame: Option<WindowPreviewFrame>,
    window_menu: Option<crate::model::WindowId>,
    window_menu_snapshot: Option<OpenWindow>,
    window_menu_anchor_x: Option<i32>,
    window_menu_host: Option<nickel_ui::UiHost<WindowMenuApp>>,
    application_menu_target: Option<ApplicationMenuTarget>,
    application_menu_host: Option<nickel_ui::UiHost<ApplicationMenuApp>>,
    notification_host: NotificationHost,
    panel_origin_x: i32,
    control_host: ControlCenterHost,
    control_change_token: HostChangeToken,
    control_deadline: Option<Instant>,
    projection_chooser: nickel_core::display_projection::ProjectionChooser,
    projection_rollback_deadline: Option<Instant>,
    launcher_view: LauncherViewState,
    launcher_icons: LauncherIconCache,
    launcher_host: nickel_ui::UiHost<LauncherApplication>,
    launcher_status: Option<String>,
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
    screenshot: ScreenshotTool,
    keyboard_host: nickel_ui::UiHost<nickel_ui::on_screen_keyboard::KeyboardApp>,
    keyboard_visible: bool,
    keyboard_enabled: bool,
    keyboard_dock_top: bool,
    keyboard_height: u32,
    keyboard_resize: Option<(nickel_input::DeviceId, Option<nickel_input::TouchId>, f64)>,
    #[cfg(target_os = "linux")]
    keyboard_override: nickel_core::on_screen_keyboard::KeyboardOverride,
    keyboard_deadline: Instant,
    keyboard_gesture_leases: HashMap<(nickel_input::DeviceId, Option<nickel_input::TouchId>), u64>,
    keyboard_recipient: Option<nickel_session_protocol::OnScreenKeyboardSnapshot>,
}

struct DesktopSurfaceViewport {
    host: UiHostViewport<desktop::DesktopMessage>,
    application: desktop::DesktopViewportState,
    change_token: HostChangeToken,
    deadline: Option<Instant>,
    overlay_pointer_capture: Option<nickel_input::PointerButton>,
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

    pub fn take_requested_codex_project(&mut self) -> Option<String> {
        if self.launcher.codex_available() {
            self.requested_codex_project.take()
        } else {
            self.requested_codex_project = None;
            None
        }
    }
    pub fn new() -> Result<Self, String> {
        let shell_settings = ShellSettings::load_default();
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
        let palette =
            ThemePalette::from_appearance(shell_settings.resolve_appearance(Appearance::default()));
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
        let window_feed = WindowFeed::new();
        let tray_feed = TrayFeed::new();
        let notification_feed = NotificationFeed::new()?;
        let windows = Vec::new();
        let workspaces = Vec::new();
        let tray = normalize_tray_items(tray_feed.snapshot());
        let tray_icons = panel_tray_icons(&tray);
        let network = platform::network_status();
        let bluetooth = platform::bluetooth_status();
        let audio = platform::audio_status();
        let volume_osd_host = nickel_ui::UiHost::new(
            VolumeOsdApplication {
                label: String::new(),
                percent: audio.volume_percent.min(100),
                palette,
            },
            420,
            96,
        );
        #[cfg(target_os = "linux")]
        let (secure_storage_state, secure_storage_query_error) =
            match platform::secure_storage_state() {
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
            DesktopApplication::new(wallpaper.clone(), palette),
            1920,
            1080,
        );
        let lock_host = nickel_ui::UiHost::new(
            LockApplication {
                password: Zeroizing::new(String::new()),
                status: None,
                effects: Vec::new(),
            },
            1920,
            1080,
        );
        let launcher_view = LauncherViewState::default();
        let launcher_icons = LauncherIconCache::new();
        let launcher_host = nickel_ui::UiHost::new(
            LauncherApplication::new(
                launcher.clone(),
                launcher_view.clone(),
                launcher_icons.clone(),
                palette,
            ),
            920,
            680,
        );
        let run_host = nickel_ui::UiHost::new(RunApplication::new(palette), 620, 150);
        let (clock, date) = panel_clock_text();
        let panel_host = nickel_ui::UiHost::new(
            PanelApplication {
                keyboard_enabled: false,
                keyboard_visible: false,
                launcher: launcher.clone(),
                windows: windows.clone(),
                tray: tray.clone(),
                tray_icons: tray_icons.clone(),
                panel_icon: Arc::clone(&panel_icon),
                codex_icon: Arc::clone(&codex_icon),
                task_icons: Vec::new(),
                palette,
                panel_hover: None,
                launcher_visible: false,
                codex_project_menu_visible: false,
                control_visible: false,
                clock,
                date,
                effects: Vec::new(),
                task_drag: None,
            },
            1920,
            56,
        );
        Ok(Self {
            host_runtime_samples: HostRuntimeSamples::default(),
            launcher,
            window_feed,
            tray_feed,
            notification_feed,
            windows,
            window_icons: HashMap::new(),
            task_switcher: TaskSwitcher::default(),
            task_switcher_group: None,
            workspaces,
            window_feed_status: FeedStatus::Loading,
            workspace_feed_status: FeedStatus::Loading,
            tray,
            tray_icons,
            notification: None,
            notification_history_visible: false,
            wallpaper_path,
            wallpaper_source_fingerprint,
            wallpaper,
            wallpaper_size,
            desktop_host,
            desktop_viewports: HashMap::new(),
            desktop_active_viewport: "primary".into(),
            desktop_change_token: HostChangeToken::default(),
            desktop_deadline: None,
            desktop_application_dirty: false,
            desktop_overlay_pointer_capture: None,
            panel_icon,
            codex_icon,
            palette,
            network,
            bluetooth,
            audio,
            volume_osd_until: None,
            volume_osd_host,
            launcher_visible: false,
            run_visible: false,
            run_host,
            locked: false,
            lock_host,
            lock_change_token: HostChangeToken::default(),
            lock_deadline: None,
            control_visible: false,
            codex_project_menu_visible: false,
            panel_hover: None,
            panel_hover_output: None,
            panel_host,
            panel_change_token: HostChangeToken::default(),
            panel_deadline: None,
            panel_output: None,
            pending_popover_anchor: None,
            all_windows_on_every_bar: shell_settings.all_windows_on_every_bar,
            preview_group: None,
            preview_pending: None,
            preview_focus_requested: false,
            preview_pointer_inside: false,
            preview_leave_deadline: None,
            preview_hovered: None,
            preview_images: HashMap::new(),
            preview_refresh_deadline: None,
            preview_frame: None,
            window_menu: None,
            window_menu_snapshot: None,
            window_menu_anchor_x: None,
            window_menu_host: None,
            application_menu_target: None,
            application_menu_host: None,
            notification_host,
            panel_origin_x: 0,
            control_host,
            control_change_token: HostChangeToken::default(),
            control_deadline: Some(Instant::now()),
            projection_chooser: Default::default(),
            projection_rollback_deadline: None,
            launcher_view,
            launcher_icons,
            launcher_host,
            launcher_status: application_status.map(str::to_owned),
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
            screenshot: ScreenshotTool::default(),
            keyboard_host: nickel_ui::UiHost::new(
                nickel_ui::on_screen_keyboard::KeyboardApp::new(palette),
                1280,
                nickel_core::on_screen_keyboard::KEYBOARD_HEIGHT,
            ),
            keyboard_visible: false,
            keyboard_enabled: false,
            keyboard_dock_top: false,
            keyboard_height: nickel_core::on_screen_keyboard::KEYBOARD_HEIGHT,
            keyboard_resize: None,
            keyboard_deadline: Instant::now(),
            keyboard_gesture_leases: HashMap::new(),
            #[cfg(target_os = "linux")]
            keyboard_override: {
                let value = std::env::var(nickel_core::on_screen_keyboard::ENVIRONMENT_VARIABLE)
                    .unwrap_or_else(|_| "auto".into());
                nickel_core::on_screen_keyboard::KeyboardOverride::parse(&value).unwrap_or_else(|| {
                    eprintln!("Invalid NICKEL_ON_SCREEN_KEYBOARD value; using saved keyboard preference");
                    Default::default()
                })
            },
            keyboard_recipient: None,
        })
    }

    pub fn refresh(&mut self) -> bool {
        let fast = self.refresh_fast();
        let system = self.refresh_system();
        fast || system
    }

    pub fn image_cache_diagnostics(&self) -> ShellImageCacheDiagnostics {
        let launcher = self.launcher_icons.diagnostics();
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
            preview_entries: self.preview_images.len(),
            preview_bytes: self
                .preview_images
                .values()
                .map(|image| image.as_raw().len())
                .sum(),
        }
    }

    pub fn refresh_fast(&mut self) -> bool {
        let mut changed = false;
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
        let workspaces = self.window_feed.workspaces();
        if update_feed_status(
            &mut self.workspace_feed_status,
            workspaces.status(),
            "workspaces",
        ) {
            changed = true;
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
            changed = true;
        }
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
            let panel_windows = self.panel_windows();
            self.launcher
                .taskbar_applications(&panel_windows)
                .get(index)
                .map(|task| task.window_group())
        });
        let preview_refresh_now = Instant::now();
        if let Some(group) = preview_group
            && preview_refresh_due(self.preview_refresh_deadline, preview_refresh_now)
        {
            retain_preview_generation(&mut self.preview_images, &group.windows);
            for window in group.windows.iter().take(PREVIEW_CACHE_CAPACITY) {
                if let Some(preview) = self.window_feed.preview(window.id) {
                    let image = Arc::new(normalize_preview_image(&preview.image));
                    if self
                        .preview_images
                        .get(&window.id)
                        .is_none_or(|current| **current != *image)
                    {
                        self.preview_images.insert(window.id, image);
                        changed = true;
                    }
                }
            }
            self.preview_refresh_deadline = Some(preview_refresh_now + PREVIEW_REFRESH_INTERVAL);
        }
        let tray = normalize_tray_items(self.tray_feed.snapshot());
        if tray != self.tray {
            self.tray = tray;
            self.tray_icons = panel_tray_icons(&self.tray);
            changed = true;
        }
        let notification = self.notification_feed.snapshot();
        if !self.notification_history_visible && notification != self.notification {
            self.notification = notification;
            self.notification_host
                .application_mut()
                .sync(self.notification.as_ref(), self.palette);
            self.notification_host.step(HostBatch {
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            changed = true;
        }
        changed
    }

    pub fn refresh_system(&mut self) -> bool {
        let mut changed = false;
        #[cfg(target_os = "linux")]
        {
            let secure_storage_state = match platform::secure_storage_state() {
                Ok(state) => {
                    if self.secure_storage_query_error.take().is_some() {
                        tracing::info!("secure-storage session query recovered");
                    }
                    state
                }
                Err(error) => {
                    let now = Instant::now();
                    let should_log = self.secure_storage_query_error.as_ref().is_none_or(
                        |(previous, logged)| {
                            previous != &error
                                || now.duration_since(*logged) >= RECURRING_DIAGNOSTIC_INTERVAL
                        },
                    );
                    if should_log {
                        tracing::warn!(%error, "secure-storage query failed during shell refresh");
                        self.secure_storage_query_error = Some((error, now));
                    }
                    platform::SecureStorageState::ControlUnavailable
                }
            };
            if secure_storage_state != self.secure_storage_state {
                self.secure_storage_state = secure_storage_state;
                changed = true;
            }
            if self.launcher_status.is_some()
                && secure_storage_state == platform::SecureStorageState::Ready
            {
                self.launcher_status = None;
                self.secure_storage_override = None;
                changed = true;
            }
        }
        let shell_settings = ShellSettings::load_default();
        let wallpaper_settings = WallpaperSettings::load_default();
        if self.refresh_configured_wallpaper(wallpaper_settings.image) {
            changed = true;
        }
        self.launcher.set_places(crate::places::applications(
            shell_settings.preferred_file_manager.as_deref(),
        ));
        if self.all_windows_on_every_bar != shell_settings.all_windows_on_every_bar {
            self.all_windows_on_every_bar = shell_settings.all_windows_on_every_bar;
            self.close_window_preview();
            changed = true;
        }
        let palette =
            ThemePalette::from_appearance(shell_settings.resolve_appearance(Appearance::default()));
        if palette != self.palette {
            self.palette = palette;
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
        self.wallpaper_size = (0, 0);
        self.wallpaper = None;
        self.desktop_application_dirty = true;
        true
    }

    pub fn semantic_theme(&self) -> nickel_ui::SemanticTheme {
        semantic_theme_from_palette(self.palette)
    }

    pub fn scene(&mut self, role: SurfaceRole, width: u32, height: u32) -> Vec<PaintCommand> {
        match role {
            SurfaceRole::Desktop => self.desktop_scene(width, height),
            SurfaceRole::Panel => self.panel_scene(width, height),
            SurfaceRole::Launcher if self.run_visible => self.run_scene(width, height),
            SurfaceRole::Launcher => self.launcher_scene(width, height),
            SurfaceRole::ControlCenter => {
                self.sync_control_host(width, height);
                self.control_host.commands().to_vec()
            }
            SurfaceRole::Notification => {
                self.sync_notification_host(width, height);
                self.notification_host.commands().to_vec()
            }
            SurfaceRole::VolumeOsd => self.volume_osd_scene(width, height),
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
        }
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

    pub fn desktop_input(&mut self, event: nickel_input::InputEvent) -> bool {
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
                events: vec![HostEvent::Normalized {
                    input: event,
                    clipboard_text: None,
                }],
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
                if edge == nickel_input::KeyEdge::Pressed {
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
                let steps =
                    discrete.map_or(-delta.y as f32, |(_, vertical)| -vertical as f32 * 3.0);
                let (cell_width, _) = application.layout.grid();
                application.scroll_overflow(steps * cell_width)
            }
            _ => false,
        };
        let changed = changed | application.reveal_active();
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
                    application.activate(id);
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
            SurfaceRole::Desktop | SurfaceRole::Panel => true,
            SurfaceRole::Launcher => self.launcher_visible,
            SurfaceRole::ControlCenter => self.control_visible,
            SurfaceRole::Notification => {
                self.notification.is_some() || self.notification_history_visible
            }
            SurfaceRole::VolumeOsd => self.volume_osd_until.is_some(),
            SurfaceRole::WindowPreview => {
                self.preview_group.is_some() || self.task_switcher_group.is_some()
            }
            SurfaceRole::WindowContextMenu => {
                self.window_menu.is_some() || self.application_menu_target.is_some()
            }
            SurfaceRole::CodexProjectMenu => self.codex_project_menu_visible,
            SurfaceRole::Lock => self.locked,
            SurfaceRole::Screenshot => self.screenshot.visible(),
            SurfaceRole::OnScreenKeyboard => self.keyboard_visible,
            SurfaceRole::CodexChat => true,
        }
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
        push("panel", self.panel_deadline);
        push("lock", self.lock_deadline);
        push("control", self.control_deadline);
        push("screenshot", self.screenshot.next_deadline());
        push(
            "window-preview-host",
            self.preview_frame
                .as_ref()
                .and_then(WindowPreviewFrame::next_deadline),
        );
        push(
            "window-preview-open",
            self.preview_pending.map(|(_, deadline)| deadline),
        );
        push("window-preview-close", self.preview_leave_deadline);
        push("volume-osd", self.volume_osd_until);
        sources
    }

    pub fn scene_change_token(&self, role: SurfaceRole) -> Option<HostChangeToken> {
        let host_token = |inspection: nickel_ui::HostInspection| HostChangeToken {
            frame_generation: inspection.frame_generation,
            semantic_generation: inspection.semantic_generation,
        };
        match role {
            SurfaceRole::Desktop => Some(self.desktop_change_token),
            SurfaceRole::Panel => Some(self.panel_change_token),
            SurfaceRole::Lock => Some(self.lock_change_token),
            SurfaceRole::Launcher if self.run_visible => Some(host_token(self.run_host.inspect())),
            SurfaceRole::Launcher => Some(host_token(self.launcher_host.inspect())),
            SurfaceRole::ControlCenter => Some(self.control_change_token),
            SurfaceRole::Notification => Some(host_token(self.notification_host.inspect())),
            SurfaceRole::VolumeOsd => None,
            SurfaceRole::WindowPreview => {
                self.preview_frame.as_ref().map(|host| host.change_token())
            }
            SurfaceRole::WindowContextMenu => self
                .window_menu_host
                .as_ref()
                .map(|host| host_token(host.inspect()))
                .or_else(|| {
                    self.application_menu_host
                        .as_ref()
                        .map(|host| host_token(host.inspect()))
                }),
            SurfaceRole::Screenshot => Some(self.screenshot.change_token()),
            SurfaceRole::OnScreenKeyboard => Some(host_token(self.keyboard_host.inspect())),
            SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat => None,
        }
    }

    pub fn launcher_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        clipboard_text: Option<String>,
        width: u32,
        height: u32,
    ) -> nickel_ui::HostEventOutcome {
        if self.run_visible {
            let outcome = self.run_host.step(HostBatch {
                surface_size: Some((width, height)),
                events: vec![HostEvent::Normalized {
                    input,
                    clipboard_text,
                }],
                ..HostBatch::default()
            });
            self.apply_run_effects();
            self.host_runtime_samples.record(outcome.telemetry);
            return outcome;
        }
        let status = self.launcher_status_text();
        self.launcher_host
            .application_mut()
            .sync(&self.launcher, self.palette, status);
        let outcome = self.launcher_host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text,
            }],
            ..HostBatch::default()
        });
        let actions = self.launcher_host.application_mut().take_effects();
        for action in actions {
            self.apply_launcher_action(action);
        }
        self.host_runtime_samples.record(outcome.telemetry);
        outcome
    }

    pub fn launcher_host_controller(
        &mut self,
        action: ControllerAction,
        family: nickel_ui::ControllerFamily,
    ) -> bool {
        if self.run_visible {
            let event = launcher_controller_host_event(
                action,
                self.run_host.inspect().open_overlay.is_some(),
            );
            let outcome = self.run_host.step(HostBatch {
                events: vec![event],
                ..HostBatch::default()
            });
            if action == ControllerAction::Confirm
                && outcome.text_input_active
                && self.run_host.controller_targets_text_input()
            {
                self.set_keyboard_visible(true);
            }
            self.apply_run_effects();
            self.host_runtime_samples.record(outcome.telemetry);
            return outcome.changed;
        }
        let status = self.launcher_status_text();
        self.launcher_host
            .application_mut()
            .sync(&self.launcher, self.palette, status);
        self.launcher_host
            .application_mut()
            .set_controller_family(family);
        let event = launcher_controller_host_event(
            action,
            self.launcher_host.inspect().open_overlay.is_some(),
        );
        let outcome = self.launcher_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        if action == ControllerAction::Confirm
            && outcome.text_input_active
            && self.launcher_host.controller_targets_text_input()
        {
            self.set_keyboard_visible(true);
        }
        let actions = self.launcher_host.application_mut().take_effects();
        for action in actions {
            self.apply_launcher_action(action);
        }
        self.host_runtime_samples.record(outcome.telemetry);
        outcome.changed
    }

    pub fn set_launcher_controller_family(&mut self, family: nickel_ui::ControllerFamily) {
        self.launcher_host
            .application_mut()
            .set_controller_family(family);
        self.launcher_host.step(HostBatch {
            application_changed: true,
            ..HostBatch::default()
        });
    }

    pub fn poll_host_deadlines(&mut self, now: Instant) -> Vec<SurfaceRole> {
        let mut changed = Vec::new();

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
        if self.panel_deadline.is_some_and(|deadline| now >= deadline) {
            let outcome = self.panel_host.step(HostBatch {
                now: Some(now),
                events: vec![HostEvent::Poll],
                ..HostBatch::default()
            });
            self.panel_change_token = outcome.change_token;
            self.panel_deadline = outcome.next_deadline;
            if outcome.changed {
                changed.push(SurfaceRole::Panel);
            }
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
        if self
            .preview_frame
            .as_ref()
            .and_then(WindowPreviewFrame::next_deadline)
            .is_some_and(|deadline| now >= deadline)
            && let Some(frame) = self.preview_frame.as_mut()
            && frame
                .step(HostBatch {
                    now: Some(now),
                    events: vec![HostEvent::Poll],
                    ..HostBatch::default()
                })
                .changed
        {
            changed.push(SurfaceRole::WindowPreview);
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
                outcome.redraw.push(SurfaceRole::Panel);
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
        if self
            .preview_leave_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.preview_leave_deadline = None;
            let was_open = self.preview_group.is_some();
            self.close_window_preview();
            outcome.visibility_changed |= was_open;
        }
        outcome.capture_screenshot = self.screenshot.capture_ready_at(now);
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
        outcome
    }

    pub fn notification_click(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        if self.notification.is_none() && !self.notification_history_visible {
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
        if outcome.effects.is_empty() {
            self.notification_host.application_mut().request_dismiss();
        }
        self.apply_notification_effects()
    }

    pub fn notification_key(&mut self, key: Option<KeyCode>) -> bool {
        if self.notification.is_none() && !self.notification_history_visible {
            return false;
        }
        self.sync_notification_host(420, 180);
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
        self.notification_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        self.apply_notification_effects();
        true
    }

    pub fn notification_controller(&mut self, action: ControllerAction) -> bool {
        if self.notification.is_none() && !self.notification_history_visible {
            return false;
        }
        self.sync_notification_host(420, 180);
        let event = if action == ControllerAction::Cancel {
            HostEvent::Shortcut(Shortcut::Escape)
        } else {
            HostEvent::Controller(action)
        };
        let outcome = self.notification_host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        self.apply_notification_effects();
        outcome.changed
    }

    pub fn panel_click(&mut self, x: f32, width: u32, secondary: bool) -> bool {
        self.sync_panel_host();
        let events = if secondary {
            vec![HostEvent::Ui(UiEvent::PointerContext(Point { x, y: 28.0 }))]
        } else {
            vec![
                HostEvent::Ui(UiEvent::PointerPressed(Point { x, y: 28.0 })),
                HostEvent::Ui(UiEvent::PointerReleased(Point { x, y: 28.0 })),
            ]
        };
        let outcome = self.panel_host.step(HostBatch {
            surface_size: Some((width, 56)),
            events,
            ..HostBatch::default()
        });
        self.panel_change_token = outcome.change_token;
        self.panel_deadline = outcome.next_deadline;
        self.apply_panel_effects()
    }

    pub fn panel_controller(&mut self, action: ControllerAction, width: u32) -> bool {
        self.sync_panel_host();
        let outcome = self.panel_host.step(HostBatch {
            surface_size: Some((width, 56)),
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        self.panel_change_token = outcome.change_token;
        self.panel_deadline = outcome.next_deadline;
        self.apply_panel_effects();
        outcome.changed
    }

    fn apply_panel_action(&mut self, action: PanelAction) {
        let anchored_role = match &action {
            PanelAction::Codex => Some((ShellRole::ProjectMenu, "panel-codex")),
            PanelAction::Control => Some((ShellRole::ControlCenter, "panel-control")),
            _ => None,
        };
        if anchored_role.is_some() {
            self.pending_popover_anchor = None;
        }
        if let Some((role, control)) = anchored_role
            && let (Some(output), Some(target)) = (
                self.panel_output.clone(),
                self.panel_host
                    .semantic_targets_for_message(&action)
                    .into_iter()
                    .next(),
            )
        {
            self.pending_popover_anchor = Some(PendingPopoverAnchor {
                role,
                control: control.to_owned(),
                output,
                bounds: target.bounds,
            });
        }
        match action {
            PanelAction::Launcher => self.set_launcher_visible(!self.launcher_visible),
            PanelAction::OnScreenKeyboard => {
                self.set_keyboard_visible(!self.keyboard_visible);
            }
            PanelAction::Task(index) => {
                let panel_windows = self.panel_windows();
                let groups = self.launcher.taskbar_applications(&panel_windows);
                if groups
                    .get(index)
                    .is_some_and(|group| group.windows.len() > 1)
                {
                    self.open_window_preview(index);
                    self.preview_focus_requested = true;
                } else if let Some(window) =
                    groups.get(index).and_then(|group| group.windows.first())
                {
                    let _ = send_session_command(
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
            PanelAction::TaskContext(index) => {
                let panel_windows = self.panel_windows();
                let groups = self.launcher.taskbar_applications(&panel_windows);
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
                self.application_menu_target = Some(target);
                self.application_menu_host = None;
                let x = self
                    .panel_host
                    .semantic_targets_for_message(&PanelAction::Task(index))
                    .into_iter()
                    .next()
                    .map(|target| target.bounds.origin.x.round() as i32)
                    .unwrap_or((PANEL_ITEM_WIDTH * (index + 1) as f32).round() as i32);
                self.window_menu_anchor_x = Some(self.panel_origin_x + x);
                let _ = send_session_command(
                    "show-context-menu",
                    ShellCommand::ShowContextMenu {
                        x: self.panel_origin_x + x,
                        width: MENU_WIDTH as i32,
                        height: self.window_context_menu_height(),
                    },
                );
                #[cfg(target_os = "linux")]
                let _ = send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
            }
            PanelAction::ToggleTaskPin(id) => {
                self.launcher.toggle_pin(&id);
                self.persist_launcher_preferences();
            }
            PanelAction::MoveTaskPinLeft(id) => {
                if self.launcher.move_pin(&id, -1) {
                    self.persist_launcher_preferences();
                }
            }
            PanelAction::MoveTaskPinRight(id) => {
                if self.launcher.move_pin(&id, 1) {
                    self.persist_launcher_preferences();
                }
            }
            // Drag gestures are reduced by `PanelApplication` into a typed move action.
            PanelAction::TaskDrag(_, _) => {}
            PanelAction::Codex => {
                if !self.launcher.codex_available() {
                    self.codex_project_menu_visible = false;
                    return;
                }
                if self.launcher_visible {
                    self.set_launcher_visible(false);
                }
                self.codex_project_menu_visible = !self.codex_project_menu_visible;
            }
            PanelAction::Tray(id) => self.tray_feed.activate(&id),
            PanelAction::TrayContext(id) => self.tray_feed.context_menu(&id),
            PanelAction::Control => {
                if self.launcher_visible {
                    self.set_launcher_visible(false);
                }
                self.set_control_visible(!self.control_visible);
            }
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
                let target = self
                    .panel_host
                    .semantic_targets_for_message(&PanelAction::OnScreenKeyboard)
                    .into_iter()
                    .next()?;
                Some(ResolvedShellTarget {
                    role: ShellRole::Panel,
                    output: self.panel_output.clone(),
                    x: (target.bounds.origin.x + target.bounds.size.width / 2.0).round() as i32,
                    y: (target.bounds.origin.y + target.bounds.size.height / 2.0).round() as i32,
                    interaction: PointerInteraction::LeftClick,
                })
            }
            ShellSemanticTarget::PanelApplication {
                application_id,
                output,
                interaction,
            } => {
                let groups = self.launcher.taskbar_applications(&self.windows);
                let index = groups.iter().take(12).position(|group| {
                    group
                        .application_id
                        .as_ref()
                        .is_some_and(|id| id.as_str() == application_id)
                })?;
                let bounds = self
                    .panel_host
                    .semantic_targets_for_message(&PanelAction::Task(index))
                    .into_iter()
                    .next()?
                    .bounds;
                Some(ResolvedShellTarget {
                    role: ShellRole::Panel,
                    output: output.clone(),
                    x: (bounds.origin.x + bounds.size.width / 2.0).round() as i32,
                    y: (bounds.origin.y + bounds.size.height / 2.0).round() as i32,
                    interaction: *interaction,
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
                let bounds = self
                    .preview_frame
                    .as_ref()?
                    .semantic_bounds(preview_action)?;
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
                let bounds = self
                    .window_menu_host
                    .as_ref()?
                    .semantic_targets_for_message(&menu_action)
                    .into_iter()
                    .next()?
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

    pub fn panel_pointer_moved(&mut self, x: f32, width: u32) -> bool {
        self.sync_panel_host();
        self.panel_host.step(HostBatch {
            surface_size: Some((width, 56)),
            events: vec![HostEvent::Ui(UiEvent::PointerMoved(Point { x, y: 28.0 }))],
            ..HostBatch::default()
        });
        let hovered_action = self
            .panel_host
            .inspect()
            .pointer_hover
            .as_ref()
            .and_then(|target| self.panel_host.message_for_semantic_target(target))
            .cloned();
        let hovered = hovered_action
            .as_ref()
            .and_then(|action| self.panel_hover_for_action(action));
        let changed = hovered != self.panel_hover;
        self.panel_hover = hovered;
        self.panel_hover_output.clone_from(&self.panel_output);
        if let Some(PanelHover::Task(index)) = hovered {
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

    fn panel_hover_for_action(&self, action: &PanelAction) -> Option<PanelHover> {
        Some(match action {
            PanelAction::OnScreenKeyboard => PanelHover::OnScreenKeyboard,
            PanelAction::Launcher => PanelHover::Launcher,
            PanelAction::Task(index)
            | PanelAction::TaskContext(index)
            | PanelAction::TaskDrag(index, _) => PanelHover::Task(*index),
            PanelAction::ToggleTaskPin(_)
            | PanelAction::MoveTaskPinLeft(_)
            | PanelAction::MoveTaskPinRight(_) => return None,
            PanelAction::Codex => PanelHover::Codex,
            PanelAction::Tray(id) | PanelAction::TrayContext(id) => self
                .tray
                .iter()
                .rev()
                .take(4)
                .rev()
                .position(|item| item.id == id.as_str())
                .map(PanelHover::Tray)
                .unwrap_or(PanelHover::Tray(0)),
            PanelAction::Control => PanelHover::Control,
        })
    }

    pub fn set_panel_origin_x(&mut self, origin_x: i32) {
        self.panel_origin_x = origin_x;
    }

    pub fn set_panel_output(&mut self, output: impl Into<String>) {
        self.panel_output = Some(output.into());
    }

    fn visible_panel_hover(&self) -> Option<PanelHover> {
        (self.panel_hover_output == self.panel_output)
            .then_some(self.panel_hover)
            .flatten()
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
        if self.all_windows_on_every_bar {
            return self.windows.clone();
        }
        let Some(output) = self.panel_output.as_deref() else {
            return self.windows.clone();
        };
        self.windows
            .iter()
            .filter(|window| {
                window_belongs_to_panel(
                    false,
                    Some(output),
                    self.window_feed.window_output(window.id).as_deref(),
                )
            })
            .cloned()
            .collect()
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
        let Some(frame) = self.preview_frame.as_mut() else {
            return false;
        };
        let primed = action != ControllerAction::Cancel && frame.ensure_controller_selection();
        let outcome = frame.step(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        let actions = frame.take_actions();
        for action in actions {
            self.apply_preview_action(action);
        }
        outcome.changed || primed
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
                let _ = send_session_command(
                    "clear-window-highlight",
                    ShellCommand::ClearWindowHighlight,
                );
            }
        }
        true
    }

    pub fn preview_pointer_moved(&mut self, x: f32, y: f32) -> bool {
        let hovered = self
            .preview_frame
            .as_mut()
            .and_then(|frame| frame.transition_pointer_hover(Point { x, y }));
        if hovered == self.preview_hovered {
            return false;
        }
        self.preview_hovered = hovered;
        let command = hovered.map_or(
            ShellCommand::ClearWindowHighlight,
            ShellCommand::HighlightWindow,
        );
        let _ = send_session_command("highlight-preview-window", command);
        true
    }

    pub fn preview_click(&mut self, x: f32, y: f32, right_click: bool) -> bool {
        let Some(action) = self
            .preview_frame
            .as_mut()
            .and_then(|frame| frame.transition_pointer(Point { x, y }, right_click))
        else {
            return false;
        };
        self.apply_preview_action(action);
        true
    }

    pub fn preview_host_input(
        &mut self,
        input: nickel_input::InputEvent,
    ) -> nickel_ui::HostEventOutcome {
        let Some(frame) = self.preview_frame.as_mut() else {
            return nickel_ui::HostEventOutcome::default();
        };
        let outcome = frame.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text: None,
            }],
            ..HostBatch::default()
        });
        let actions = frame.take_actions();
        for action in actions {
            self.apply_preview_action(action);
        }
        outcome
    }

    fn apply_preview_action(&mut self, action: PreviewAction) {
        match action {
            PreviewAction::Activate(window) => {
                self.send_window_action(window, WindowAction::Activate);
                self.close_window_preview();
            }
            PreviewAction::Close(window) => {
                self.send_window_action(window, WindowAction::Close);
            }
            PreviewAction::OpenMenu(window) => {
                let x = self
                    .preview_group
                    .and_then(|index| {
                        let panel_windows = self.panel_windows();
                        let groups = self.launcher.taskbar_applications(&panel_windows);
                        let group = groups.get(index)?;
                        let (width, _) = preview_dimensions(group.windows.len());
                        let preview_origin = self.preview_origin_x(index, width);
                        let card = self
                            .preview_frame
                            .as_ref()?
                            .semantic_bounds(PreviewAction::Activate(window))?;
                        Some(preview_origin + card.origin.x.round() as i32)
                    })
                    .unwrap_or(self.panel_origin_x);
                self.application_menu_target = None;
                self.application_menu_host = None;
                self.window_menu = Some(window);
                self.window_menu_snapshot = self
                    .windows
                    .iter()
                    .find(|candidate| candidate.id == window)
                    .cloned();
                self.window_menu_host = None;
                self.window_menu_anchor_x = Some(x);
                let _ = send_session_command(
                    "show-context-menu",
                    ShellCommand::ShowContextMenu {
                        x,
                        width: MENU_WIDTH as i32,
                        height: self.window_context_menu_height(),
                    },
                );
                #[cfg(target_os = "linux")]
                let _ = send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
            }
            PreviewAction::Dismiss => self.close_window_preview(),
        }
    }

    pub fn preview_key(&mut self, key: Option<KeyCode>) -> bool {
        if self.window_menu.is_some() || self.application_menu_target.is_some() {
            return self.window_menu_host_key(key);
        }
        let Some(frame) = self.preview_frame.as_mut() else {
            return false;
        };
        if !matches!(key, Some(KeyCode::Escape) | None) {
            let _ = frame.ensure_controller_selection();
        }
        match key {
            Some(KeyCode::Escape) => {
                frame.step(HostBatch {
                    events: vec![HostEvent::Shortcut(Shortcut::Escape)],
                    ..HostBatch::default()
                });
                let dismissed = frame.take_actions().contains(&PreviewAction::Dismiss);
                if dismissed {
                    self.close_window_preview();
                }
                #[cfg(target_os = "linux")]
                let _ = send_session_command(
                    "restore-application-focus",
                    ShellCommand::RestoreApplicationFocus,
                );
            }
            Some(KeyCode::ArrowLeft | KeyCode::ArrowUp) => {
                frame.step(HostBatch {
                    events: vec![HostEvent::Controller(ControllerAction::Left)],
                    ..HostBatch::default()
                });
                self.preview_hovered = frame.controller_selected_window();
                if let Some(window) = self.preview_hovered {
                    let _ = send_session_command(
                        "highlight-preview-window",
                        ShellCommand::HighlightWindow(window),
                    );
                }
            }
            Some(KeyCode::ArrowRight | KeyCode::ArrowDown | KeyCode::Tab) => {
                frame.step(HostBatch {
                    events: vec![HostEvent::Controller(ControllerAction::Right)],
                    ..HostBatch::default()
                });
                self.preview_hovered = frame.controller_selected_window();
                if let Some(window) = self.preview_hovered {
                    let _ = send_session_command(
                        "highlight-preview-window",
                        ShellCommand::HighlightWindow(window),
                    );
                }
            }
            Some(KeyCode::Delete) => {
                if !frame.close_controller_selected() {
                    return false;
                }
                for action in frame.take_actions() {
                    self.apply_preview_action(action);
                }
            }
            Some(KeyCode::Enter | KeyCode::NumpadEnter | KeyCode::Space) => {
                frame.step(HostBatch {
                    events: vec![HostEvent::Controller(ControllerAction::Confirm)],
                    ..HostBatch::default()
                });
                let actions = frame.take_actions();
                for action in actions {
                    if let PreviewAction::Activate(window) = action {
                        self.send_window_action(window, WindowAction::Activate);
                        self.close_window_preview();
                    }
                }
            }
            _ => return false,
        }
        true
    }

    fn apply_window_menu_action(&mut self, action: MenuAction) {
        if !matches!(
            action,
            MenuAction::Dismiss
                | MenuAction::ShowWorkspaces
                | MenuAction::ShowDisplays
                | MenuAction::Back
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
        match action {
            MenuAction::Dismiss => self.dismiss_window_menu(),
            MenuAction::ShowWorkspaces | MenuAction::ShowDisplays | MenuAction::Back => {}
            MenuAction::Activate(window) => self.send_window_action(window, WindowAction::Activate),
            MenuAction::Close(window) => self.send_window_action(window, WindowAction::Close),
            MenuAction::MaximizeRestore(window) => {
                self.send_window_action(window, WindowAction::Maximize)
            }
            MenuAction::Minimize(window) => self.send_window_action(window, WindowAction::Minimize),
            MenuAction::FullscreenRestore(window) => {
                self.send_window_action(window, WindowAction::Fullscreen)
            }
            MenuAction::SnapLeading(window) => {
                self.send_window_action(window, WindowAction::SnapLeading)
            }
            MenuAction::SnapTrailing(window) => {
                self.send_window_action(window, WindowAction::SnapTrailing)
            }
            MenuAction::MoveToWorkspace(window, workspace) => {
                let _ = send_session_command(
                    "move-window-to-workspace",
                    ShellCommand::MoveWindowToWorkspace { window, workspace },
                );
            }
            MenuAction::MoveToDisplay(window, output) => {
                let _ = send_session_command(
                    "move-window-to-display",
                    ShellCommand::MoveWindowToDisplay { window, output },
                );
            }
        }
    }

    fn apply_application_menu_action(&mut self, action: ApplicationMenuAction) {
        match action {
            ApplicationMenuAction::Dismiss => self.dismiss_window_menu(),
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
                let dispatched = targets.into_iter().fold(false, |dispatched, window| {
                    self.try_send_window_action(window, WindowAction::Close) || dispatched
                });
                if dispatched {
                    self.dismiss_window_menu();
                }
            }
        }
    }

    pub fn window_menu_host_input(
        &mut self,
        input: nickel_input::InputEvent,
        width: u32,
        height: u32,
    ) -> bool {
        if self.application_menu_target.is_some() {
            if self.application_menu_host.is_none() {
                let _ = self.application_menu_scene();
            }
            let Some(host) = self.application_menu_host.as_mut() else {
                return false;
            };
            let outcome = host.step(HostBatch {
                surface_size: Some((width, height)),
                events: vec![HostEvent::Normalized {
                    input,
                    clipboard_text: None,
                }],
                ..HostBatch::default()
            });
            let actions = host.application_mut().take_effects();
            for action in actions {
                self.apply_application_menu_action(action);
                self.close_window_preview();
            }
            return outcome.changed;
        }
        if self.window_menu_host.is_none() {
            let _ = self.window_menu_scene();
        }
        let Some(host) = self.window_menu_host.as_mut() else {
            return false;
        };
        let outcome = host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text: None,
            }],
            ..HostBatch::default()
        });
        for failure in &outcome.failures {
            tracing::warn!(
                ?failure,
                "window context menu host reported recoverable failure"
            );
        }
        let actions = host.application_mut().take_effects();
        for action in actions {
            self.apply_window_menu_action(action);
            self.close_window_preview();
        }
        outcome.changed
    }

    pub fn window_menu_host_key(&mut self, key: Option<KeyCode>) -> bool {
        if self.application_menu_target.is_some() {
            if self.application_menu_host.is_none() {
                let _ = self.application_menu_scene();
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
            let Some(host) = self.application_menu_host.as_mut() else {
                return false;
            };
            let outcome = host.step(HostBatch {
                events: vec![event],
                ..HostBatch::default()
            });
            let actions = host.application_mut().take_effects();
            for action in actions {
                self.apply_application_menu_action(action);
                self.close_window_preview();
            }
            if key == Some(KeyCode::Escape) {
                self.dismiss_window_menu();
            }
            return outcome.changed;
        }
        if self.window_menu_host.is_none() {
            let _ = self.window_menu_scene();
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
        let Some(host) = self.window_menu_host.as_mut() else {
            return false;
        };
        let outcome = host.step(HostBatch {
            events: vec![event],
            ..HostBatch::default()
        });
        for failure in &outcome.failures {
            tracing::warn!(
                ?failure,
                "window context menu host reported recoverable failure"
            );
        }
        let actions = host.application_mut().take_effects();
        for action in actions {
            self.apply_window_menu_action(action);
            self.close_window_preview();
        }
        if key == Some(KeyCode::Escape) {
            self.dismiss_window_menu();
        }
        outcome.changed
    }

    pub fn window_menu_host_controller(&mut self, action: ControllerAction) -> bool {
        if self.application_menu_target.is_some() {
            if self.application_menu_host.is_none() {
                let _ = self.application_menu_scene();
            }
            let Some(host) = self.application_menu_host.as_mut() else {
                return false;
            };
            let outcome = host.step(HostBatch {
                events: vec![HostEvent::Controller(action)],
                ..HostBatch::default()
            });
            let effects = host.application_mut().take_effects();
            for effect in effects {
                self.apply_application_menu_action(effect);
                self.close_window_preview();
            }
            if action == ControllerAction::Cancel {
                self.dismiss_window_menu();
            }
            return outcome.changed;
        }
        if self.window_menu_host.is_none() {
            let _ = self.window_menu_scene();
        }
        let Some(host) = self.window_menu_host.as_mut() else {
            return false;
        };
        let outcome = host.step(HostBatch {
            events: vec![HostEvent::Controller(action)],
            ..HostBatch::default()
        });
        let effects = host.application_mut().take_effects();
        for effect in effects {
            self.apply_window_menu_action(effect);
            self.close_window_preview();
        }
        if action == ControllerAction::Cancel {
            self.dismiss_window_menu();
        }
        outcome.changed
    }

    pub fn sync_transient_overlays(&mut self) {
        if let Some(group) = &self.task_switcher_group {
            let windows = group
                .windows
                .iter()
                .map(|window| window.id)
                .collect::<Vec<_>>();
            let (width, height) = preview_dimensions(windows.len());
            let _ = send_session_command(
                "show-task-switcher",
                ShellCommand::ShowTaskSwitcher {
                    width: width as i32,
                    height: height as i32,
                    windows,
                },
            );
        } else if let Some(index) = self.preview_group {
            let panel_windows = self.panel_windows();
            let groups = self.launcher.taskbar_applications(&panel_windows);
            if let Some(group) = groups.get(index) {
                let windows = group
                    .windows
                    .iter()
                    .map(|window| window.id)
                    .collect::<Vec<_>>();
                let (width, height) = preview_dimensions(windows.len());
                let x = self.preview_origin_x(index, width);
                let _ = send_session_command(
                    "show-preview",
                    ShellCommand::ShowPreview {
                        x,
                        width: width as i32,
                        height: height as i32,
                        windows,
                    },
                );
                if self.preview_focus_requested {
                    #[cfg(target_os = "linux")]
                    let _ = send_session_command("focus-preview", ShellCommand::FocusPreview);
                    self.preview_focus_requested = false;
                }
            }
        }
        if self.window_menu.is_some() || self.application_menu_target.is_some() {
            let x = self.window_menu_anchor_x.unwrap_or(self.panel_origin_x);
            let _ = send_session_command(
                "show-context-menu",
                ShellCommand::ShowContextMenu {
                    x,
                    width: MENU_WIDTH as i32,
                    height: self.window_context_menu_height(),
                },
            );
        }
    }

    fn preview_origin_x(&self, index: usize, width: u32) -> i32 {
        let control_bounds = self
            .panel_host
            .semantic_targets_for_message(&PanelAction::Task(index))
            .into_iter()
            .next()
            .map(|target| target.bounds)
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
            return menu_height_for_rows(application_menu_entries(target, pinned).len()) as i32;
        }
        let Some(window) = self.window_menu_snapshot.as_ref().or_else(|| {
            self.window_menu
                .and_then(|id| self.windows.iter().find(|candidate| candidate.id == id))
        }) else {
            return menu_height(&self.workspaces) as i32;
        };
        let outputs = self.window_feed.outputs();
        menu_height_for_rows(window_menu_max_rows(window, &self.workspaces, &outputs)) as i32
    }

    fn send_window_action(&self, window: crate::model::WindowId, action: WindowAction) {
        let _ = self.try_send_window_action(window, action);
    }

    fn try_send_window_action(&self, window: crate::model::WindowId, action: WindowAction) -> bool {
        send_session_command(
            "window-action",
            ShellCommand::WindowAction { window, action },
        )
    }

    fn open_window_preview(&mut self, index: usize) {
        if self.preview_group == Some(index) {
            self.preview_pending = None;
            return;
        }
        let panel_windows = self.panel_windows();
        let groups = self.launcher.taskbar_applications(&panel_windows);
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
        self.window_menu_host = None;
        self.application_menu_target = None;
        self.application_menu_host = None;
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
        self.preview_frame = None;
        self.window_menu = None;
        self.window_menu_snapshot = None;
        self.window_menu_anchor_x = None;
        self.window_menu_host = None;
        self.application_menu_target = None;
        self.application_menu_host = None;
        let _ = send_session_command("clear-window-highlight", ShellCommand::ClearWindowHighlight);
        let _ = send_session_command("hide-context-menu", ShellCommand::HideContextMenu);
    }

    fn dismiss_window_menu(&mut self) {
        let focused_menu = self.window_menu.is_some() || self.application_menu_target.is_some();
        self.close_window_preview();
        if focused_menu {
            #[cfg(target_os = "linux")]
            let _ = send_session_command(
                "restore-window-menu-focus",
                ShellCommand::RestoreApplicationFocus,
            );
        }
    }

    pub fn global_shortcut(&mut self, shortcut: platform::GlobalShortcut) -> bool {
        match shortcut {
            platform::GlobalShortcut::ReloadShellSettings => self.refresh_system(),
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
            platform::GlobalShortcut::LockState { locked } => {
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
            platform::GlobalShortcut::ShowRun => self.set_run_visible(true),
            platform::GlobalShortcut::OpenFiles => self.launch_named_application("Nickel File"),
            platform::GlobalShortcut::OpenSettings => {
                self.launch_named_application("Nickel Settings")
            }
            platform::GlobalShortcut::ShowControlCenter => {
                self.control_host.application_mut().show_control_center();
                self.set_control_visible(true);
                true
            }
            platform::GlobalShortcut::ShowNotifications => {
                let history = self.notification_feed.history();
                self.notification_host
                    .application_mut()
                    .sync_history(&history, self.palette);
                self.notification = history.first().cloned();
                self.notification_history_visible = true;
                #[cfg(target_os = "linux")]
                let _ = send_session_command(
                    "focus-notifications",
                    ShellCommand::SetShellRoleVisible {
                        role: nickel_session_protocol::ShellRole::Notification,
                        visible: true,
                    },
                );
                true
            }
            platform::GlobalShortcut::ShowDesktop => {
                send_session_command("toggle-show-desktop", ShellCommand::ToggleShowDesktop)
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
                platform::handle_consumer_control(control);
                true
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
                if let Err(error) = platform::capture_active_window() {
                    tracing::warn!(%error, "failed to copy active window screenshot");
                    self.screenshot.show_error(error);
                    self.set_screenshot_focus(true);
                    return true;
                }
                false
            }
            platform::GlobalShortcut::Screenshot(
                platform::ScreenshotAction::ActiveWindowToFile,
            ) => {
                if let Err(error) = platform::capture_active_window_to_file() {
                    tracing::warn!(%error, "failed to capture active window to a temporary file");
                    self.screenshot.show_error(error);
                    self.set_screenshot_focus(true);
                    return true;
                }
                false
            }
            platform::GlobalShortcut::Screenshot(platform::ScreenshotAction::InteractiveRegion) => {
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
                    let _ = send_session_command(
                        "task-switcher-activate",
                        ShellCommand::WindowAction {
                            window,
                            action: WindowAction::Activate,
                        },
                    );
                }
                TaskSwitchEffect::HideFlip { .. } => {
                    self.task_switcher_group = None;
                    self.preview_frame = None;
                    self.preview_images.clear();
                }
                TaskSwitchEffect::ShowFlip { .. }
                | TaskSwitchEffect::RequestPreviews(_)
                | TaskSwitchEffect::SelectPreview(_) => {}
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
        self.preview_frame = None;
    }

    /// Requests a launcher toggle initiated by shell-owned input such as a controller.
    ///
    /// Linux compositor shortcut notifications use [`Self::global_shortcut`] after the
    /// compositor has already changed visibility. Shell-originated input must instead send the
    /// visibility request to the compositor before mirroring the resulting state.
    pub fn request_launcher_toggle(&mut self) -> bool {
        let visible = !self.launcher_visible;
        let command = if visible {
            ShellCommand::ShowFromController
        } else {
            ShellCommand::Hide
        };
        if !send_session_command("controller-launcher-visibility", command) {
            self.launcher_status = Some("Nickel could not update the launcher.".to_owned());
            return false;
        }
        self.apply_session_launcher_visibility(visible);
        platform::launcher_visibility_applied(visible);
        self.launcher_visible == visible
    }

    pub fn capture_screenshot(&mut self) -> bool {
        match platform::capture_desktop() {
            Ok(capture) => {
                self.screenshot.show(capture.image);
                self.set_screenshot_focus(true);
                true
            }
            Err(error) => {
                tracing::warn!(%error, "failed to capture desktop");
                self.screenshot.show_error(error);
                self.set_screenshot_focus(true);
                true
            }
        }
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
        let _ = send_session_command(
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
        if !send_session_command(
            "launcher-visibility",
            if visible {
                ShellCommand::Show
            } else {
                ShellCommand::Hide
            },
        ) {
            self.launcher_status = Some("Nickel could not update the launcher.".to_owned());
            return;
        }
        self.run_visible = false;
        self.apply_session_launcher_visibility(visible);
        platform::launcher_visibility_applied(visible);
    }

    fn set_run_visible(&mut self, visible: bool) -> bool {
        self.set_launcher_visible(visible);
        if self.launcher_visible != visible {
            return false;
        }
        self.run_visible = visible;
        if visible {
            self.run_host.application_mut().status = None;
            self.run_host.step(HostBatch {
                application_changed: true,
                ..HostBatch::default()
            });
            if let Ok(field) =
                self.run_host
                    .query_unique(&nickel_ui::SemanticSelector::Id(nickel_ui::UiId::new(
                        "run-command",
                    )))
            {
                let _ = self.run_host.request_focus(field.id);
            }
        }
        true
    }

    fn set_control_visible(&mut self, visible: bool) {
        #[cfg(target_os = "linux")]
        if !send_session_command(
            "control-center-focus",
            if visible {
                ShellCommand::FocusControlCenter
            } else {
                ShellCommand::RestoreApplicationFocus
            },
        ) {
            self.launcher_status = Some("Nickel could not update Quick Settings.".to_owned());
            return;
        }
        self.control_visible = visible;
        if !visible {
            self.control_host.application_mut().show_control_center();
        }
    }

    fn apply_launcher_signal(&mut self, visible: bool) {
        #[cfg(target_os = "linux")]
        self.apply_session_launcher_visibility(visible);
        #[cfg(not(target_os = "linux"))]
        self.set_launcher_visible(visible);
    }

    fn apply_session_launcher_visibility(&mut self, visible: bool) {
        self.launcher_visible = visible;
        if visible {
            self.control_visible = false;
            self.focus_launcher_search();
        } else {
            self.run_visible = false;
            self.launcher.clear();
        }
    }

    pub fn focus_launcher_search(&mut self) -> bool {
        let status = self.launcher_status_text();
        self.launcher_host
            .application_mut()
            .sync(&self.launcher, self.palette, status);
        self.launcher_host.step(HostBatch::default());
        let Ok(search) = self
            .launcher_host
            .query_unique(&nickel_ui::SemanticSelector::Role(SemanticRole::TextField))
        else {
            return false;
        };
        let outcome = self.launcher_host.request_focus(search.id);
        outcome.changed && outcome.failures.is_empty()
    }

    pub fn control_click(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
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
        if !self.control_visible {
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
        if !self.control_visible {
            return false;
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

    #[allow(dead_code)]
    pub fn hide_overlay(&mut self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Launcher if self.launcher_visible => {
                self.apply_session_launcher_visibility(false);
                let _ = send_session_command("hide-launcher", ShellCommand::Hide);
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

    fn open_active_window_menu(&mut self) -> bool {
        let Some(snapshot) = self.windows.iter().find(|window| window.active).cloned() else {
            return false;
        };
        self.window_menu = Some(snapshot.id);
        self.window_menu_snapshot = Some(snapshot);
        self.window_menu_host = None;
        self.window_menu_anchor_x = Some(self.panel_origin_x);
        let sent = send_session_command(
            "show-context-menu",
            ShellCommand::ShowContextMenu {
                x: self.panel_origin_x,
                width: MENU_WIDTH as i32,
                height: self.window_context_menu_height(),
            },
        );
        #[cfg(target_os = "linux")]
        let _ = send_session_command("focus-context-menu", ShellCommand::FocusContextMenu);
        sent
    }

    fn launch_application(&mut self, application: Application) {
        #[cfg(target_os = "linux")]
        if platform::application_requires_secure_storage(&application)
            && platform::secure_storage_state().unwrap_or_else(|error| {
                tracing::warn!(%error, "secure-storage query failed before application launch");
                platform::SecureStorageState::ControlUnavailable
            }) != platform::SecureStorageState::Ready
            && self.secure_storage_override.as_deref() != Some(application.id())
        {
            if let Err(error) = platform::request_secure_storage_retry() {
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
                self.persist_launcher_preferences();
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
        application.palette = self.palette;
        let icons_changed = application.prepare_icons();
        let application_changed =
            self.desktop_application_dirty || wallpaper_changed || palette_changed || icons_changed;
        let outcome = self.desktop_host.step(HostBatch {
            application_changed,
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.desktop_application_dirty = false;
        self.desktop_change_token = outcome.change_token;
        self.desktop_deadline = outcome.next_deadline;
        self.desktop_host.commands().to_vec()
    }

    fn volume_osd_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let percent = self.audio.volume_percent.min(100);
        let mut label = if self.audio.muted {
            "Muted".to_owned()
        } else {
            format!("Volume {percent}%")
        };
        let output = if self.locked {
            Some("Audio output")
        } else {
            self.audio
                .devices
                .iter()
                .find(|device| device.is_default)
                .map(|device| device.name.as_str())
        };
        if let Some(output) = output {
            label.push_str(" · ");
            label.push_str(output);
        }
        let application = self.volume_osd_host.application_mut();
        let changed = application.label != label
            || application.percent != percent
            || application.palette != self.palette;
        application.label = label;
        application.percent = percent;
        application.palette = self.palette;
        self.volume_osd_host.step(HostBatch {
            application_changed: changed,
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.volume_osd_host.commands().to_vec()
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
        let outcome = self.lock_host.step(HostBatch {
            events: vec![HostEvent::Normalized {
                input,
                clipboard_text: None,
            }],
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
                            if let Err(error) = platform::send_shell_command(ShellCommand::Unlock) {
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
        let Some(target) = wallpaper_cache_target(self.wallpaper_size, requested) else {
            return;
        };
        let Some(path) = self.wallpaper_path.as_deref() else {
            return;
        };
        let Ok(image) = image::open(path) else {
            return;
        };
        self.wallpaper = Some(Arc::new(image.thumbnail(target.0, target.1).into_rgba8()));
        self.wallpaper_size = target;
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
                    let _ = send_session_command(
                        "hide-notification-history",
                        ShellCommand::SetShellRoleVisible {
                            role: nickel_session_protocol::ShellRole::Notification,
                            visible: false,
                        },
                    );
                    #[cfg(target_os = "linux")]
                    let _ = send_session_command(
                        "restore-notification-focus",
                        ShellCommand::RestoreApplicationFocus,
                    );
                }
            }
        }
        handled
    }

    fn dismiss_notification_transport(&mut self, notification_id: u32) {
        if self.notification.as_ref().map(|item| item.id) != Some(notification_id) {
            return;
        }
        self.notification = None;
        self.notification_feed.dismiss(notification_id);
        self.notification_host
            .application_mut()
            .sync(None, self.palette);
        self.notification_host.step(HostBatch {
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
    }

    fn window_preview_scene(&mut self) -> Vec<PaintCommand> {
        let group = self.task_switcher_group.clone().or_else(|| {
            self.preview_group.and_then(|index| {
                let panel_windows = self.panel_windows();
                self.launcher
                    .taskbar_applications(&panel_windows)
                    .get(index)
                    .map(|task| task.window_group())
            })
        });
        let Some(group) = group else {
            self.preview_frame = None;
            return Vec::new();
        };
        let theme = self.semantic_theme();
        if let Some(frame) = self.preview_frame.as_mut() {
            frame.sync(&group, &self.preview_images, self.preview_hovered, theme);
        } else {
            self.preview_frame = Some(build_preview_frame(
                &group,
                &self.preview_images,
                self.preview_hovered,
                theme,
            ));
        }
        self.preview_frame.as_ref().map_or_else(Vec::new, |frame| {
            let _change_token = frame.change_token();
            frame.commands().to_vec()
        })
    }

    fn window_menu_scene(&mut self) -> Vec<PaintCommand> {
        if self.application_menu_target.is_some() {
            return self.application_menu_scene();
        }
        if self.window_menu.is_none() && self.window_menu_snapshot.is_none() {
            self.window_menu_host = None;
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
        let height =
            menu_height_for_rows(window_menu_max_rows(&snapshot, &self.workspaces, &outputs))
                .ceil()
                .max(1.0) as u32;
        let host = self.window_menu_host.get_or_insert_with(|| {
            nickel_ui::UiHost::new(
                WindowMenuApp::new(
                    snapshot.clone(),
                    self.workspaces.clone(),
                    outputs.clone(),
                    self.palette,
                ),
                MENU_WIDTH.ceil() as u32,
                height,
            )
        });
        host.application_mut()
            .sync(&snapshot, &self.workspaces, &outputs, self.palette);
        host.step(HostBatch {
            surface_size: Some((MENU_WIDTH.ceil() as u32, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        host.commands().to_vec()
    }

    fn application_menu_scene(&mut self) -> Vec<PaintCommand> {
        let Some(target) = self.application_menu_target.clone() else {
            self.application_menu_host = None;
            return Vec::new();
        };
        let pinned = target
            .application_id
            .as_ref()
            .is_some_and(|id| self.launcher.is_pinned(id.as_str()));
        let height = menu_height_for_rows(application_menu_entries(&target, pinned).len())
            .ceil()
            .max(1.0) as u32;
        let host = self.application_menu_host.get_or_insert_with(|| {
            nickel_ui::UiHost::new(
                ApplicationMenuApp::new(target, pinned, self.palette),
                MENU_WIDTH.ceil() as u32,
                height,
            )
        });
        host.application_mut().sync(pinned, self.palette);
        host.step(HostBatch {
            surface_size: Some((MENU_WIDTH.ceil() as u32, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        host.commands().to_vec()
    }

    fn launcher_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let status = self.launcher_status_text();
        self.launcher_host
            .application_mut()
            .sync(&self.launcher, self.palette, status);
        self.launcher_host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        for action in self.launcher_host.application_mut().take_effects() {
            self.apply_launcher_action(action);
        }
        self.launcher_host.commands().to_vec()
    }

    fn run_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        self.run_host.application_mut().palette = self.palette;
        self.run_host.step(HostBatch {
            surface_size: Some((width, height)),
            events: vec![HostEvent::Poll],
            ..HostBatch::default()
        });
        self.apply_run_effects();
        self.run_host.commands().to_vec()
    }

    fn apply_run_effects(&mut self) {
        for effect in self.run_host.application_mut().take_effects() {
            match effect {
                RunEffect::Submit(command) => match platform::execute_run_command(&command) {
                    Ok(()) => {
                        self.run_host.application_mut().command.clear();
                        self.set_launcher_visible(false);
                    }
                    Err(error) => {
                        let app = self.run_host.application_mut();
                        app.status = Some(format!(
                            "Could not run command: {}",
                            launch_error_summary(&error)
                        ));
                        app.dirty = true;
                    }
                },
                RunEffect::Dismiss => self.set_launcher_visible(false),
            }
        }
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
        let application_changed = self.sync_panel_host();
        let outcome = self.panel_host.step(HostBatch {
            application_changed,
            surface_size: Some((width, height)),
            ..HostBatch::default()
        });
        self.panel_change_token = outcome.change_token;
        self.panel_deadline = outcome.next_deadline;
        self.apply_panel_effects();
        self.panel_host.commands().to_vec()
    }

    fn sync_panel_host(&mut self) -> bool {
        let panel_windows = self.panel_windows();
        let groups = self.launcher.taskbar_applications(&panel_windows);
        let task_icons: Vec<Option<(u16, Arc<image::RgbaImage>)>> = groups
            .iter()
            .take(12)
            .map(|group| {
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
                        crate::icons::nickel_application(&group.application_name)
                            .map(|(id, image)| (id, Arc::new(image)))
                    })
                    .or_else(|| {
                        group.windows.first().and_then(|window| {
                            self.window_icons.get(&window.id).cloned().map(|icon| {
                                self.launcher_icons.resolve_window_icon(window.id, icon)
                            })
                        })
                    })
            })
            .collect();
        let visible_panel_hover = self.visible_panel_hover();
        let application = self.panel_host.application_mut();
        let keyboard_changed = application.keyboard_enabled != self.keyboard_enabled
            || application.keyboard_visible != self.keyboard_visible;
        application.keyboard_enabled = self.keyboard_enabled;
        application.keyboard_visible = self.keyboard_visible;
        let task_icons_changed = application.task_icons.len() != task_icons.len()
            || application
                .task_icons
                .iter()
                .zip(&task_icons)
                .any(|(current, next)| match (current, next) {
                    (Some((current_id, current)), Some((next_id, next))) => {
                        current_id != next_id || !Arc::ptr_eq(current, next)
                    }
                    (None, None) => false,
                    _ => true,
                });
        let application_changed = application.palette != self.palette
            || application.windows != panel_windows
            || application.tray != self.tray
            || application.panel_hover != visible_panel_hover
            || application.launcher_visible != self.launcher_visible
            || application.codex_project_menu_visible != self.codex_project_menu_visible
            || application.control_visible != self.control_visible
            || application.launcher.codex_available() != self.launcher.codex_available()
            || application.launcher.preferences() != self.launcher.preferences()
            || task_icons_changed
            || keyboard_changed;
        application.launcher.clone_from(&self.launcher);
        application.windows = panel_windows;
        application.tray.clone_from(&self.tray);
        application.tray_icons.clone_from(&self.tray_icons);
        application.panel_icon = Arc::clone(&self.panel_icon);
        application.codex_icon = Arc::clone(&self.codex_icon);
        application.task_icons = task_icons;
        application.palette = self.palette;
        application.panel_hover = visible_panel_hover;
        application.launcher_visible = self.launcher_visible;
        application.codex_project_menu_visible = self.codex_project_menu_visible;
        application.control_visible = self.control_visible;
        application_changed
    }

    fn apply_panel_effects(&mut self) -> bool {
        let effects = std::mem::take(&mut self.panel_host.application_mut().effects);
        let changed = !effects.is_empty();
        for action in effects {
            self.apply_panel_action(action);
        }
        changed
    }
}

fn supported_projection_modes() -> Vec<nickel_core::display_projection::ProjectionMode> {
    #[cfg(target_os = "linux")]
    {
        use nickel_core::display_projection::{ProjectionChooser, ProjectionOutput};
        let Ok(outputs) = platform::projection_outputs() else {
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
        let supported_projection_modes = supported_projection_modes();
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
        let Some(effect) =
            reduce_launcher_action(&mut self.launcher, &mut self.launcher_view, action)
        else {
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
            LauncherShellEffect::LaunchApplication(id) => self.launch_application_by_id(&id),
            LauncherShellEffect::OpenProject(id) => {
                self.set_launcher_visible(false);
                self.requested_codex_project = Some(id);
            }
            LauncherShellEffect::SeeAllProjects => {
                self.set_launcher_visible(false);
                self.codex_project_menu_visible = true;
            }
            LauncherShellEffect::OpenSettings(destination) => {
                let preferred = match destination {
                    crate::launcher::SettingsDestination::Nickel
                    | crate::launcher::SettingsDestination::KeyboardShortcuts
                    | crate::launcher::SettingsDestination::About => "Nickel Settings",
                };
                let id = self
                    .launcher
                    .applications()
                    .find(|application| application.name() == preferred)
                    .map(|application| application.id().to_owned());
                if let Some(id) = id {
                    let application = self
                        .launcher
                        .applications()
                        .find(|application| application.id() == id)
                        .cloned();
                    if let Some(application) = application {
                        let screen = match destination {
                            crate::launcher::SettingsDestination::Nickel => Some("appearance"),
                            crate::launcher::SettingsDestination::KeyboardShortcuts => {
                                Some("keyboard-shortcuts")
                            }
                            crate::launcher::SettingsDestination::About => Some("about"),
                        };
                        let application = match (screen, application.launch_command()) {
                            (Some(screen), Some(command)) => {
                                let mut command = command.to_vec();
                                command.extend(["--screen".into(), screen.into()]);
                                Application::new(
                                    application.id().to_owned(),
                                    application.name().to_owned(),
                                    application.icon().map(str::to_owned),
                                    application.icon_path().map(std::path::Path::to_owned),
                                    Some(command),
                                )
                            }
                            _ => application,
                        };
                        self.launch_application(application);
                    }
                }
            }
            LauncherShellEffect::OpenAccount => {
                self.set_control_visible(true);
                if self.control_visible {
                    self.set_launcher_visible(false);
                }
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
        #[cfg(test)]
        {
            self.launcher_persistence_attempts += 1;
        }
        #[cfg(test)]
        let result = self.launcher_preferences_path.as_ref().map_or_else(
            || self.launcher.preferences().save_default(),
            |path| self.launcher.preferences().save(path),
        );
        #[cfg(not(test))]
        let result = self.launcher.preferences().save_default();
        match result {
            Err(error) => {
                tracing::warn!(%error, "launcher preferences could not be saved");
                self.launcher_status = Some(format!(
                    "Launcher preferences could not be saved: {}",
                    error
                ));
            }
            Ok(())
                if self.launcher_status.as_deref().is_some_and(|status| {
                    status.starts_with("Launcher preferences could not be saved:")
                }) =>
            {
                self.launcher_status = None;
            }
            Ok(()) => {}
        }
    }

    fn apply_control_action(&mut self, action: ControlAction) {
        match action {
            ControlAction::ToggleWifiSection => {}
            ControlAction::SetWifiEnabled(enabled) => {
                log_control_result("set-wifi-enabled", platform::set_wifi_enabled(enabled));
            }
            ControlAction::ActivateWifi { id } => {
                log_control_result(
                    "activate-wifi-network",
                    platform::activate_wifi_network(&id),
                );
            }
            ControlAction::ToggleBluetoothSection => {}
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
            ControlAction::ToggleAudioSection => {}
            ControlAction::SetAudioVolume(volume) => {
                log_control_result("set-audio-volume", platform::set_audio_volume(volume));
            }
            ControlAction::SelectAudioDevice { id } => {
                log_control_result("select-audio-device", platform::select_audio_device(&id));
            }
            ControlAction::SwitchWorkspace(workspace) => {
                let _ = send_session_command(
                    "switch-workspace",
                    ShellCommand::SwitchWorkspace(workspace),
                );
            }
            ControlAction::CreateWorkspace => {
                let _ = send_session_command("create-workspace", ShellCommand::CreateWorkspace);
            }
            ControlAction::ToggleShowDesktop => {
                let _ =
                    send_session_command("toggle-show-desktop", ShellCommand::ToggleShowDesktop);
            }
            ControlAction::ShowNotifications => {
                self.global_shortcut(platform::GlobalShortcut::ShowNotifications);
            }
            ControlAction::RemoveWorkspace(workspace) => {
                let _ = send_session_command(
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
                let _ = send_session_command("session-action", ShellCommand::SessionAction(action));
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
            let Ok(outputs) = platform::projection_outputs() else {
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
                    })
                    .collect(),
            };
            if send_session_command("preview-projection", ShellCommand::ApplyOutputs(layout)) {
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
            if let Ok(outputs) = platform::projection_outputs() {
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
                    })
                    .collect(),
            };
            let _ = send_session_command("rollback-projection", ShellCommand::ApplyOutputs(layout));
        }
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

fn send_session_command(operation: &'static str, command: ShellCommand) -> bool {
    #[cfg(test)]
    {
        let _ = (operation, command);
        true
    }
    #[cfg(all(target_os = "linux", not(test)))]
    {
        match platform::send_shell_command(command) {
            Ok(()) => true,
            Err(error) => {
                tracing::warn!(operation, %error, "session command failed");
                false
            }
        }
    }
    #[cfg(all(not(target_os = "linux"), not(test)))]
    {
        let _ = operation;
        platform::send_shell_command(command)
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
) -> Option<&'static str> {
    match status {
        crate::model::ApplicationDiscoveryStatus::ReadyEmpty => Some("No applications found."),
        crate::model::ApplicationDiscoveryStatus::Ready => None,
        crate::model::ApplicationDiscoveryStatus::PartialFailure => {
            Some("Some applications could not be loaded.")
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

fn normalize_preview_image(image: &image::RgbaImage) -> image::RgbaImage {
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
mod run_application_tests {
    use super::*;

    fn application() -> RunApplication {
        RunApplication::new(ThemePalette::from_appearance(Appearance::default()))
    }

    #[test]
    fn command_input_is_unicode_safe_and_bounded() {
        let mut app = application();
        app.update(RunAction::SetCommand("🦀".repeat(RUN_COMMAND_LIMIT + 2)));

        assert_eq!(app.command.chars().count(), RUN_COMMAND_LIMIT);
        assert!(app.poll());
    }

    #[test]
    fn submit_and_escape_emit_typed_boundary_effects() {
        let mut app = application();
        app.update(RunAction::SetCommand("  cargo test  ".into()));

        assert!(app.shortcut(Shortcut::Submit));
        assert_eq!(app.take_effects(), [RunEffect::Submit("cargo test".into())]);
        assert!(app.shortcut(Shortcut::Escape));
        assert_eq!(app.take_effects(), [RunEffect::Dismiss]);
    }

    #[test]
    fn empty_command_does_not_cross_the_launch_boundary() {
        let mut app = application();
        app.update(RunAction::SetCommand("   ".into()));
        assert!(app.shortcut(Shortcut::Submit));
        assert!(app.take_effects().is_empty());
    }
}

#[cfg(test)]
#[path = "live_shell/tests.rs"]
mod tests;
