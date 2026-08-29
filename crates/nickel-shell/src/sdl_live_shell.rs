use std::sync::Arc;

#[cfg(not(target_os = "macos"))]
use jiff::Zoned;
use nickel_core::{
    shell_settings::ShellSettings,
    theme::{Appearance, ThemePalette},
    wallpaper_settings::WallpaperSettings,
};
use nickel_ui::{PaintCommand, Point, Rect, TextAlign};

use crate::{
    launcher::{
        DashboardAccount, DashboardProject, DashboardSection, Launcher, LauncherInput,
        LauncherInputOutcome, LauncherMode,
    },
    model::{Application, OpenWindow, TrayItem},
    notification::DesktopNotification,
    platform::{
        self, AudioStatus, BluetoothStatus, NetworkStatus, NotificationFeed, NotificationSource,
        ShellCommand, TrayFeed, TraySource, WindowAction, WindowFeed,
    },
    sdl_control_view::{ControlAction, ControlCenterFrame, ControlViewState, build_control_center},
    sdl_launcher_view::{
        LauncherAction, LauncherFrame, LauncherIconCache, LauncherShellEffect, LauncherViewState,
        build_launcher_frame, reduce_launcher_action,
    },
    sdl_shell::SurfaceRole,
};
use sdl3::keyboard::{Keycode, Mod};

const PANEL_ITEM_WIDTH: f32 = 52.0;
const PANEL_CLOCK_WIDTH: f32 = 96.0;
const PANEL_CONTROL_GAP: f32 = 8.0;
const PANEL_TRAY_WIDTH: f32 = 28.0;
const PANEL_TRAY_ICON_SIZE: u32 = 18;
const PANEL_CODEX_WIDTH: f32 = 36.0;
const PANEL_CODEX_ICON_SIZE: f32 = 28.0;

#[derive(Clone, Copy, Debug, PartialEq)]
struct PanelStatusLayout {
    control_start: f32,
    tray_start: f32,
    codex_start: f32,
}

impl PanelStatusLayout {
    fn codex_icon_bounds(self) -> Rect {
        Rect::new(
            self.codex_start + (PANEL_CODEX_WIDTH - PANEL_CODEX_ICON_SIZE) / 2.0,
            (56.0 - PANEL_CODEX_ICON_SIZE) / 2.0,
            PANEL_CODEX_ICON_SIZE,
            PANEL_CODEX_ICON_SIZE,
        )
    }
}

fn panel_status_layout(width: u32, tray_count: usize) -> PanelStatusLayout {
    let control_start = panel_control_start(width);
    let tray_start = control_start - tray_count.min(4) as f32 * PANEL_TRAY_WIDTH;
    PanelStatusLayout {
        control_start,
        tray_start,
        codex_start: tray_start - PANEL_CODEX_WIDTH,
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PanelHover {
    Launcher,
    Task(usize),
    Codex,
    Tray(usize),
    #[cfg(not(target_os = "macos"))]
    Control,
}

pub struct LiveShell {
    launcher: Launcher,
    window_feed: WindowFeed,
    tray_feed: TrayFeed,
    notification_feed: NotificationFeed,
    windows: Vec<OpenWindow>,
    tray: Vec<TrayItem>,
    tray_icons: Vec<Arc<image::RgbaImage>>,
    notification: Option<DesktopNotification>,
    wallpaper_path: Option<std::path::PathBuf>,
    wallpaper: Option<Arc<image::RgbaImage>>,
    wallpaper_size: (u32, u32),
    panel_icon: Arc<image::RgbaImage>,
    codex_icon: Arc<image::RgbaImage>,
    palette: ThemePalette,
    network: NetworkStatus,
    bluetooth: BluetoothStatus,
    audio: AudioStatus,
    launcher_visible: bool,
    control_visible: bool,
    codex_project_menu_visible: bool,
    panel_hover: Option<PanelHover>,
    control_state: ControlViewState,
    launcher_view: LauncherViewState,
    launcher_icons: LauncherIconCache,
    launcher_frame: Option<LauncherFrame>,
    launcher_status: Option<String>,
    secure_storage_override: Option<String>,
    secure_storage_state: platform::SecureStorageState,
    requested_codex_project: Option<String>,
}

impl LiveShell {
    pub fn set_dashboard_projects(
        &mut self,
        projects: DashboardSection<Vec<DashboardProject>>,
    ) -> bool {
        self.launcher.set_dashboard_projects(projects)
    }

    pub fn take_requested_codex_project(&mut self) -> Option<String> {
        self.requested_codex_project.take()
    }
    pub fn new() -> Result<Self, String> {
        let mut launcher = Launcher::new(platform::applications());
        launcher.set_places(crate::places::applications());
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
        let wallpaper_path = wallpaper_settings.image;
        let shell_settings = ShellSettings::load_default();
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
        let windows = window_feed.snapshot(&launcher).unwrap_or_default();
        let tray = tray_feed.snapshot();
        let tray_icons = panel_tray_icons(&tray);
        let network = platform::network_status();
        let bluetooth = platform::bluetooth_status();
        let audio = platform::audio_status();
        #[cfg(target_os = "linux")]
        let secure_storage_state = platform::secure_storage_state();
        #[cfg(not(target_os = "linux"))]
        let secure_storage_state = platform::SecureStorageState::Ready;
        Ok(Self {
            launcher,
            window_feed,
            tray_feed,
            notification_feed,
            windows,
            tray,
            tray_icons,
            notification: None,
            wallpaper_path,
            wallpaper: None,
            wallpaper_size: (0, 0),
            panel_icon,
            codex_icon,
            palette,
            network,
            bluetooth,
            audio,
            launcher_visible: false,
            control_visible: false,
            codex_project_menu_visible: false,
            panel_hover: None,
            control_state: ControlViewState::default(),
            launcher_view: LauncherViewState::default(),
            launcher_icons: LauncherIconCache::new(),
            launcher_frame: None,
            launcher_status: None,
            secure_storage_override: None,
            secure_storage_state,
            requested_codex_project: None,
        })
    }

    pub fn refresh(&mut self) -> bool {
        let fast = self.refresh_fast();
        let system = self.refresh_system();
        fast || system
    }

    pub fn refresh_fast(&mut self) -> bool {
        let mut changed = false;
        #[cfg(target_os = "linux")]
        {
            let secure_storage_state = platform::secure_storage_state();
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
        #[cfg(target_os = "macos")]
        if self.launcher_visible || self.control_visible {
            return false;
        }
        if let Some(windows) = self.window_feed.snapshot(&self.launcher)
            && windows != self.windows
        {
            self.windows = windows;
            changed = true;
        }
        let tray = self.tray_feed.snapshot();
        if tray != self.tray {
            self.tray = tray;
            self.tray_icons = panel_tray_icons(&self.tray);
            changed = true;
        }
        let notification = self.notification_feed.snapshot();
        if notification != self.notification {
            self.notification = notification;
            changed = true;
        }
        changed
    }

    pub fn refresh_system(&mut self) -> bool {
        #[cfg(target_os = "macos")]
        if self.launcher_visible || self.control_visible {
            return false;
        }
        let mut changed = false;
        let palette = ThemePalette::from_appearance(
            ShellSettings::load_default().resolve_appearance(Appearance::default()),
        );
        if palette != self.palette {
            self.palette = palette;
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

    pub fn scene(&mut self, role: SurfaceRole, width: u32, height: u32) -> Vec<PaintCommand> {
        match role {
            SurfaceRole::Desktop => self.desktop_scene(width, height),
            SurfaceRole::Panel => self.panel_scene(width, height),
            SurfaceRole::Launcher => self.launcher_scene(width, height),
            SurfaceRole::ControlCenter => self.control_frame(width, height).commands,
            SurfaceRole::Notification => self.notification_scene(width, height),
            SurfaceRole::CodexProjectMenu | SurfaceRole::CodexChat => Vec::new(),
        }
    }

    pub fn surface_visible(&self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Desktop | SurfaceRole::Panel => true,
            SurfaceRole::Launcher => self.launcher_visible,
            SurfaceRole::ControlCenter => self.control_visible,
            SurfaceRole::Notification => self.notification.is_some(),
            SurfaceRole::CodexProjectMenu => self.codex_project_menu_visible,
            SurfaceRole::CodexChat => true,
        }
    }

    pub fn dismiss_notification(&mut self) -> bool {
        let Some(notification) = self.notification.take() else {
            return false;
        };
        self.notification_feed.dismiss(notification.id);
        true
    }

    pub fn panel_click(&mut self, x: f32, width: u32) -> bool {
        if x < PANEL_ITEM_WIDTH {
            self.set_launcher_visible(!self.launcher_visible);
            return true;
        }
        let groups = self.launcher.group_windows(&self.windows);
        let task_end = PANEL_ITEM_WIDTH + groups.len() as f32 * PANEL_ITEM_WIDTH;
        if x < task_end {
            let index = ((x - PANEL_ITEM_WIDTH) / PANEL_ITEM_WIDTH) as usize;
            if let Some(window) = groups.get(index).and_then(|group| group.windows.first()) {
                let _ = platform::send_shell_command(ShellCommand::WindowAction {
                    window: window.id,
                    action: WindowAction::Activate,
                });
            }
            return true;
        }
        let status = panel_status_layout(width, self.tray.len());
        #[cfg(not(target_os = "macos"))]
        if x >= status.control_start {
            if self.launcher_visible {
                self.set_launcher_visible(false);
            }
            self.control_visible = !self.control_visible;
            return true;
        }
        if x >= status.codex_start && x < status.tray_start {
            if self.launcher_visible {
                self.set_launcher_visible(false);
            }
            self.codex_project_menu_visible = !self.codex_project_menu_visible;
            return true;
        }
        if x >= status.tray_start {
            let index = ((x - status.tray_start) / PANEL_TRAY_WIDTH) as usize;
            if let Some(item) = visible_tray_item(&self.tray, index) {
                self.tray_feed.activate(&item.id);
            }
            return true;
        }
        false
    }

    pub fn panel_pointer_moved(&mut self, x: f32, width: u32) -> bool {
        let hovered = self.panel_hover_at(x, width);
        let changed = hovered != self.panel_hover;
        self.panel_hover = hovered;
        changed
    }

    pub fn panel_pointer_left(&mut self) -> bool {
        if self.panel_hover.is_none() {
            return false;
        }
        self.panel_hover = None;
        true
    }

    pub fn global_shortcut(&mut self, shortcut: platform::GlobalShortcut) -> bool {
        match shortcut {
            platform::GlobalShortcut::ShowLauncher => {
                self.apply_launcher_signal(true);
                true
            }
            platform::GlobalShortcut::HideLauncher => {
                self.apply_launcher_signal(false);
                true
            }
            platform::GlobalShortcut::ShowRun => {
                tracing::warn!("Nickel Run is not implemented in the SDL shell yet");
                false
            }
            _ => false,
        }
    }

    fn set_launcher_visible(&mut self, visible: bool) {
        self.apply_session_launcher_visibility(visible);
        let _ = platform::send_shell_command(if visible {
            ShellCommand::Show
        } else {
            ShellCommand::Hide
        });
        platform::launcher_visibility_applied(visible);
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
        } else {
            self.launcher.clear();
        }
    }

    pub fn launcher_click(&mut self, x: f32, y: f32) -> bool {
        let Some(action) = self
            .launcher_frame
            .as_ref()
            .and_then(|frame| frame.action_at(Point { x, y }))
            .cloned()
        else {
            return false;
        };
        self.apply_launcher_action(action);
        true
    }

    pub fn control_click(&mut self, x: f32, y: f32, width: u32, height: u32) -> bool {
        let frame = self.control_frame(width, height);
        if let Some(action) = frame.action_at(x, y) {
            self.apply_control_action(action);
        }
        true
    }

    #[allow(dead_code)]
    pub fn hide_overlay(&mut self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Launcher if self.launcher_visible => {
                self.apply_session_launcher_visibility(false);
                let _ = platform::send_shell_command(ShellCommand::Hide);
                true
            }
            SurfaceRole::ControlCenter if self.control_visible => {
                self.control_visible = false;
                true
            }
            SurfaceRole::CodexProjectMenu if self.codex_project_menu_visible => {
                self.codex_project_menu_visible = false;
                true
            }
            _ => false,
        }
    }

    pub fn pointer_moved(&mut self, x: f32, y: f32) -> bool {
        if !self.launcher_visible {
            return false;
        }
        let hovered = self
            .launcher_frame
            .as_ref()
            .and_then(|frame| frame.action_at(Point { x, y }))
            .cloned();
        let mut changed = hovered != self.launcher_view.hovered;
        if let Some(LauncherAction::ActivateResult(index)) = hovered {
            changed |= self.launcher.selected_index() != index;
            self.launcher.select(index);
        }
        if self.launcher.mode() == LauncherMode::Dashboard
            && let Some(action) = &hovered
            && let Some(index) = self.launcher_frame.as_ref().and_then(|frame| {
                frame
                    .navigable_actions
                    .iter()
                    .position(|item| item == action)
            })
        {
            changed |= self.launcher_view.dashboard_selected != index;
            self.launcher_view.dashboard_selected = index;
        }
        self.launcher_view.hovered = hovered;
        changed
    }

    pub fn scroll(&mut self, delta: f32) -> bool {
        if self.launcher_visible {
            let Some(frame) = &self.launcher_frame else {
                return false;
            };
            self.launcher_view.scroll(
                -(delta.round() as isize),
                self.launcher.result_count(),
                frame.columns,
                frame.visible_rows,
            );
            return true;
        }
        if self.control_visible {
            let frame = self.control_frame(800, 600);
            self.control_state.scroll_offset = (self.control_state.scroll_offset - delta * 36.0)
                .clamp(0.0, frame.maximum_scroll());
            return true;
        }
        false
    }

    pub fn insert_launcher_text(&mut self, value: &str) -> bool {
        if !self.launcher_visible {
            return false;
        }
        let previous = self.launcher.mode();
        let _ = self
            .launcher
            .reduce_input(LauncherInput::Text(value.to_owned()));
        self.launcher_view
            .transition_mode(previous, self.launcher.mode());
        self.launcher_view.reset_active_scroll(self.launcher.mode());
        true
    }

    pub fn launcher_is_dashboard(&self) -> bool {
        self.launcher.mode() == LauncherMode::Dashboard
    }

    pub fn set_launcher_preedit(&mut self, value: &str) -> bool {
        if !self.launcher_visible {
            return false;
        }
        let previous = self.launcher.mode();
        let _ = self
            .launcher
            .reduce_input(LauncherInput::Preedit(value.to_owned()));
        self.launcher_view
            .transition_mode(previous, self.launcher.mode());
        self.launcher_view.reset_active_scroll(self.launcher.mode());
        true
    }

    pub fn launcher_key(&mut self, key: Option<Keycode>, modifiers: Mod) -> bool {
        if !self.launcher_visible {
            return false;
        }
        let columns = self
            .launcher_frame
            .as_ref()
            .map(|frame| frame.columns)
            .unwrap_or(1)
            .max(1);
        if self.launcher.mode() == LauncherMode::Dashboard {
            let action_count = self
                .launcher_frame
                .as_ref()
                .map_or(0, |frame| frame.navigable_actions.len());
            match key {
                Some(Keycode::Escape) => self.set_launcher_visible(false),
                Some(Keycode::Tab) if modifiers.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) => {
                    self.launcher_view.select_dashboard_previous(action_count);
                }
                Some(Keycode::Down | Keycode::Right | Keycode::Tab) => {
                    self.launcher_view.select_dashboard_next(action_count);
                }
                Some(Keycode::Up | Keycode::Left) => {
                    self.launcher_view.select_dashboard_previous(action_count);
                }
                Some(Keycode::Return | Keycode::KpEnter) => {
                    let action = self.launcher_frame.as_ref().and_then(|frame| {
                        frame
                            .navigable_actions
                            .get(self.launcher_view.dashboard_selected)
                            .cloned()
                    });
                    if let Some(action) = action {
                        self.apply_launcher_action(action);
                    }
                }
                Some(Keycode::Backspace) => {}
                _ => return self.insert_launcher_key_text(key, modifiers),
            }
            return true;
        }
        match key {
            Some(Keycode::Escape) => {
                let previous = self.launcher.mode();
                if self.launcher.reduce_input(LauncherInput::Escape)
                    == LauncherInputOutcome::DismissRequested
                {
                    self.set_launcher_visible(false);
                } else {
                    self.launcher_view
                        .transition_mode(previous, self.launcher.mode());
                }
            }
            Some(Keycode::Backspace) => {
                let previous = self.launcher.mode();
                let _ = self.launcher.reduce_input(LauncherInput::Backspace);
                self.launcher_view
                    .transition_mode(previous, self.launcher.mode());
                if self.launcher.mode() == LauncherMode::Search {
                    self.launcher_view.reset_active_scroll(LauncherMode::Search);
                }
            }
            Some(Keycode::Down) => self.launcher.select_grid_down(columns),
            Some(Keycode::Up) => self.launcher.select_grid_up(columns),
            Some(Keycode::Left) => self.launcher.select_grid_left(columns),
            Some(Keycode::Right) => self.launcher.select_grid_right(columns),
            Some(Keycode::Return) => {
                let index = self.launcher.selected_index();
                self.launch_result(index);
            }
            Some(_) => return self.insert_launcher_key_text(key, modifiers),
            None => return false,
        }
        true
    }

    fn insert_launcher_key_text(&mut self, key: Option<Keycode>, modifiers: Mod) -> bool {
        let Some(key) = key else {
            return false;
        };
        let control_modifier = Mod::LCTRLMOD
            | Mod::RCTRLMOD
            | Mod::LALTMOD
            | Mod::RALTMOD
            | Mod::LGUIMOD
            | Mod::RGUIMOD;
        if modifiers.intersects(control_modifier) {
            return false;
        }
        #[cfg(target_os = "macos")]
        {
            let _ = key;
            false
        }
        #[cfg(not(target_os = "macos"))]
        {
            let name = key.name();
            let mut characters = name.chars();
            let Some(mut character) = characters.next() else {
                return false;
            };
            if characters.next().is_some() {
                if key == Keycode::Space {
                    character = ' ';
                } else {
                    return false;
                }
            } else if character.is_ascii_alphabetic() {
                let shifted = modifiers.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD);
                let caps = modifiers.contains(Mod::CAPSMOD);
                character = if shifted ^ caps {
                    character.to_ascii_uppercase()
                } else {
                    character.to_ascii_lowercase()
                };
            }
            let previous = self.launcher.mode();
            self.launcher.insert(&character.to_string());
            self.launcher_view
                .transition_mode(previous, self.launcher.mode());
            self.launcher_view.reset_active_scroll(LauncherMode::Search);
            true
        }
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

    fn launch_application(&mut self, application: Application) {
        #[cfg(target_os = "linux")]
        if platform::application_requires_secure_storage(&application)
            && platform::secure_storage_state() != platform::SecureStorageState::Ready
            && self.secure_storage_override.as_deref() != Some(application.id())
        {
            let _ = platform::request_secure_storage_retry();
            self.secure_storage_override = Some(application.id().to_owned());
            self.launcher_status = Some(format!(
                "Secure storage is not ready. Activate {} again to launch without credentials.",
                application.name()
            ));
            return;
        }
        self.secure_storage_override = None;
        self.launcher_status = None;
        let _ = platform::launch_application(&application);
        self.set_launcher_visible(false);
    }

    fn desktop_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        self.load_wallpaper_for(width, height);
        let bounds = Rect::new(0.0, 0.0, width as f32, height as f32);
        let mut commands = vec![PaintCommand::Fill {
            rect: bounds,
            color: self.palette.background,
        }];
        if let Some(wallpaper) = &self.wallpaper {
            commands.push(PaintCommand::Image {
                bounds,
                id: 1,
                image: wallpaper.clone(),
                high_density: None,
            });
        }
        commands
    }

    fn load_wallpaper_for(&mut self, width: u32, height: u32) {
        let requested = (width.max(1), height.max(1));
        if self.wallpaper.is_some()
            && requested.0 <= self.wallpaper_size.0
            && requested.1 <= self.wallpaper_size.1
        {
            return;
        }
        let Some(path) = self.wallpaper_path.as_deref() else {
            return;
        };
        let Ok(image) = image::open(path) else {
            return;
        };
        let target = (
            self.wallpaper_size.0.max(requested.0),
            self.wallpaper_size.1.max(requested.1),
        );
        self.wallpaper = Some(Arc::new(image.thumbnail(target.0, target.1).into_rgba8()));
        self.wallpaper_size = target;
    }

    fn notification_scene(&self, width: u32, height: u32) -> Vec<PaintCommand> {
        let Some(notification) = &self.notification else {
            return Vec::new();
        };
        let heading = if notification.summary.trim().is_empty() {
            &notification.app_name
        } else {
            &notification.summary
        };
        vec![
            PaintCommand::RoundedFill {
                rect: Rect::new(0.0, 0.0, width as f32, height as f32),
                color: self.palette.panel,
                radius: 16.0,
            },
            PaintCommand::Stroke {
                rect: Rect::new(0.5, 0.5, width as f32 - 1.0, height as f32 - 1.0),
                color: self.palette.surface_hover,
                width: 1.0,
            },
            text(
                Rect::new(20.0, 18.0, width as f32 - 40.0, 32.0),
                heading,
                20.0,
                self.palette.text,
                TextAlign::Start,
                true,
            ),
            text(
                Rect::new(20.0, 55.0, width as f32 - 40.0, height as f32 - 70.0),
                &notification.body,
                16.0,
                self.palette.muted,
                TextAlign::Start,
                false,
            ),
        ]
    }

    fn launcher_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let frame = build_launcher_frame(
            &self.launcher,
            &mut self.launcher_view,
            &mut self.launcher_icons,
            width,
            height,
            self.palette,
        );
        let mut commands = frame.commands.clone();
        let storage_status = self
            .launcher_status
            .as_deref()
            .or_else(|| secure_storage_status_label(self.secure_storage_state));
        if let Some(status) = storage_status {
            commands.push(text(
                Rect::new(250.0, 72.0, width.saturating_sub(280) as f32, 28.0),
                status,
                14.0,
                0xd98a32,
                TextAlign::Start,
                false,
            ));
        }
        self.launcher_frame = Some(frame);
        commands
    }

    fn panel_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let mut commands = vec![PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, width as f32, height as f32),
            color: self.palette.panel,
        }];
        if self.panel_hover == Some(PanelHover::Launcher) {
            commands.push(PaintCommand::RoundedFill {
                rect: Rect::new(6.0, 7.0, 44.0, 42.0),
                color: self.palette.surface_hover,
                radius: 8.0,
            });
        }
        commands.push(PaintCommand::Image {
            bounds: Rect::new(12.0, 12.0, 32.0, 32.0),
            id: 2,
            image: Arc::clone(&self.panel_icon),
            high_density: None,
        });
        let groups = self.launcher.group_windows(&self.windows);
        for (index, group) in groups.iter().take(12).enumerate() {
            let x = PANEL_ITEM_WIDTH + index as f32 * PANEL_ITEM_WIDTH;
            let hovered = self.panel_hover == Some(PanelHover::Task(index));
            if group.active() || hovered {
                commands.push(PaintCommand::RoundedFill {
                    rect: Rect::new(x + 4.0, 7.0, 44.0, 42.0),
                    color: if group.active() {
                        self.palette.accent_soft
                    } else {
                        self.palette.surface_hover
                    },
                    radius: 8.0,
                });
            }
            let icon = group
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
                });
            if let Some((id, image)) = icon {
                commands.push(PaintCommand::Image {
                    bounds: Rect::new(x + 10.0, 11.0, 32.0, 32.0),
                    id,
                    image,
                    high_density: None,
                });
            } else {
                let initial = group
                    .application_name
                    .chars()
                    .next()
                    .unwrap_or('?')
                    .to_uppercase()
                    .to_string();
                commands.push(text(
                    Rect::new(x + 5.0, 12.0, 42.0, 28.0),
                    &initial,
                    1.0,
                    self.palette.text,
                    TextAlign::Center,
                    true,
                ));
            }
            commands.push(PaintCommand::RoundedFill {
                rect: Rect::new(
                    x + if group.active() { 16.0 } else { 20.0 },
                    height as f32 - 6.0,
                    if group.active() { 20.0 } else { 12.0 },
                    3.0,
                ),
                color: if group.active() {
                    self.palette.accent
                } else {
                    self.palette.muted
                },
                radius: 1.5,
            });
            if group.windows.len() > 1 {
                commands.push(PaintCommand::RoundedFill {
                    rect: Rect::new(x + 38.0, 35.0, 5.0, 5.0),
                    color: self.palette.complement,
                    radius: 2.5,
                });
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            let now = Zoned::now();
            let clock = now.strftime("%-I:%M %p").to_string();
            let date = now.strftime("%-m/%-d/%Y").to_string();
            if self.panel_hover == Some(PanelHover::Control) {
                commands.push(PaintCommand::RoundedFill {
                    rect: Rect::new(
                        width as f32 - PANEL_CLOCK_WIDTH,
                        7.0,
                        PANEL_CLOCK_WIDTH - 8.0,
                        42.0,
                    ),
                    color: self.palette.surface_hover,
                    radius: 8.0,
                });
            }
            commands.push(text(
                Rect::new(
                    width as f32 - PANEL_CLOCK_WIDTH,
                    6.0,
                    PANEL_CLOCK_WIDTH,
                    22.0,
                ),
                &clock,
                1.0,
                self.palette.text,
                TextAlign::Center,
                false,
            ));
            commands.push(text(
                Rect::new(
                    width as f32 - PANEL_CLOCK_WIDTH,
                    28.0,
                    PANEL_CLOCK_WIDTH,
                    20.0,
                ),
                &date,
                0.72,
                self.palette.text,
                TextAlign::Center,
                false,
            ));
        }
        let status = panel_status_layout(width, self.tray.len());
        for (index, _) in self.tray.iter().rev().take(4).rev().enumerate() {
            let x = status.tray_start + index as f32 * PANEL_TRAY_WIDTH;
            if self.panel_hover == Some(PanelHover::Tray(index)) {
                commands.push(PaintCommand::RoundedFill {
                    rect: Rect::new(x + 1.0, 10.0, PANEL_TRAY_WIDTH - 2.0, 36.0),
                    color: self.palette.surface_hover,
                    radius: 7.0,
                });
            }
            if let Some(image) = self.tray_icons.get(index) {
                commands.push(PaintCommand::Image {
                    bounds: Rect::new(x + 5.0, 19.0, 18.0, 18.0),
                    id: 0x6000 + index as u16,
                    image: Arc::clone(image),
                    high_density: None,
                });
            }
        }
        let codex_x = status.codex_start;
        if self.panel_hover == Some(PanelHover::Codex) {
            commands.push(PaintCommand::RoundedFill {
                rect: Rect::new(codex_x + 2.0, 7.0, PANEL_CODEX_WIDTH - 4.0, 42.0),
                color: self.palette.surface_hover,
                radius: 8.0,
            });
        }
        commands.push(PaintCommand::Image {
            bounds: status.codex_icon_bounds(),
            id: 0x5000,
            image: Arc::clone(&self.codex_icon),
            high_density: None,
        });
        commands
    }

    fn panel_hover_at(&self, x: f32, width: u32) -> Option<PanelHover> {
        if x < PANEL_ITEM_WIDTH {
            return Some(PanelHover::Launcher);
        }
        let groups = self.launcher.group_windows(&self.windows);
        let task_count = groups.len().min(12);
        let task_end = PANEL_ITEM_WIDTH + task_count as f32 * PANEL_ITEM_WIDTH;
        if x < task_end {
            return Some(PanelHover::Task(
                ((x - PANEL_ITEM_WIDTH) / PANEL_ITEM_WIDTH) as usize,
            ));
        }
        let status = panel_status_layout(width, self.tray.len());
        #[cfg(not(target_os = "macos"))]
        if x >= status.control_start {
            return Some(PanelHover::Control);
        }
        if x >= status.tray_start {
            return Some(PanelHover::Tray(
                ((x - status.tray_start) / PANEL_TRAY_WIDTH) as usize,
            ));
        }
        if x >= status.codex_start {
            return Some(PanelHover::Codex);
        }
        None
    }

    fn control_frame(&self, width: u32, height: u32) -> ControlCenterFrame {
        build_control_center(
            &self.network,
            &self.bluetooth,
            &self.audio,
            self.control_state,
            (width as f32, height as f32),
        )
    }

    fn apply_launcher_action(&mut self, action: LauncherAction) {
        let Some(effect) =
            reduce_launcher_action(&mut self.launcher, &mut self.launcher_view, action)
        else {
            return;
        };
        match effect {
            LauncherShellEffect::Dismiss => {
                self.set_launcher_visible(false);
            }
            LauncherShellEffect::ActivateResult(index) => self.launch_result(index),
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
                    crate::launcher::SettingsDestination::System => "System Settings",
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
                            crate::launcher::SettingsDestination::System => None,
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
                self.set_launcher_visible(false);
                self.control_visible = true;
            }
            LauncherShellEffect::RequestLogout => {
                self.set_launcher_visible(false);
                self.control_visible = true;
                self.control_state.logout_confirmation = true;
            }
        }
    }

    fn apply_control_action(&mut self, action: ControlAction) {
        match action {
            ControlAction::ToggleWifiSection => {
                self.control_state.wifi_expanded = !self.control_state.wifi_expanded;
            }
            ControlAction::SetWifiEnabled(enabled) => {
                let _ = platform::set_wifi_enabled(enabled);
            }
            ControlAction::ActivateWifi { id } => {
                let _ = platform::activate_wifi_network(&id);
            }
            ControlAction::ToggleBluetoothSection => {
                self.control_state.bluetooth_expanded = !self.control_state.bluetooth_expanded;
            }
            ControlAction::SetBluetoothPowered(powered) => {
                let _ = platform::set_bluetooth_powered(powered);
            }
            ControlAction::SetBluetoothDiscovery(discovering) => {
                let _ = platform::set_bluetooth_discovery(discovering);
            }
            ControlAction::ToggleBluetoothDevice { id } => {
                let _ = platform::toggle_bluetooth_device(&id);
            }
            ControlAction::ToggleAudioSection => {
                self.control_state.audio_expanded = !self.control_state.audio_expanded;
            }
            ControlAction::SetAudioVolume(volume) => {
                let _ = platform::set_audio_volume(volume);
            }
            ControlAction::SelectAudioDevice { id } => {
                let _ = platform::select_audio_device(&id);
            }
            ControlAction::ToggleLogoutConfirmation => {
                self.control_state.logout_confirmation = !self.control_state.logout_confirmation;
            }
            ControlAction::LogOut => {
                let _ = platform::send_shell_command(ShellCommand::LogOut);
            }
        }
        let _ = self.refresh();
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
        platform::SecureStorageState::Ready => None,
    }
}

fn panel_control_start(width: u32) -> f32 {
    #[cfg(target_os = "macos")]
    {
        width as f32
    }
    #[cfg(not(target_os = "macos"))]
    {
        width as f32 - PANEL_CLOCK_WIDTH - PANEL_CONTROL_GAP
    }
}

fn panel_tray_icons(items: &[TrayItem]) -> Vec<Arc<image::RgbaImage>> {
    items
        .iter()
        .rev()
        .take(4)
        .rev()
        .map(|item| {
            Arc::new(crate::icons::resized(
                &item.icon,
                PANEL_TRAY_ICON_SIZE,
                PANEL_TRAY_ICON_SIZE,
            ))
        })
        .collect()
}

fn visible_tray_item(items: &[TrayItem], visual_index: usize) -> Option<&TrayItem> {
    items.get(items.len().saturating_sub(4).checked_add(visual_index)?)
}

fn tint_panel_icon(mut icon: image::RgbaImage, color: u32) -> image::RgbaImage {
    let tint = [
        ((color >> 16) & 0xff) as u8,
        ((color >> 8) & 0xff) as u8,
        (color & 0xff) as u8,
    ];
    for pixel in icon.pixels_mut() {
        let coverage = u8::MAX - pixel.0[0];
        pixel.0[3] = ((u16::from(pixel.0[3]) * u16::from(coverage)) / u16::from(u8::MAX)) as u8;
        pixel.0[..3].copy_from_slice(&tint);
    }
    icon
}

fn text(
    bounds: Rect,
    value: &str,
    scale: f32,
    color: u32,
    align: TextAlign,
    bold: bool,
) -> PaintCommand {
    PaintCommand::Text {
        bounds,
        text: value.to_owned(),
        scale,
        color,
        align,
        bold,
        wrap: false,
    }
}

#[cfg(test)]
mod tests {
    use image::{Rgba, RgbaImage};
    use nickel_ui::Rect;

    use super::{
        panel_status_layout, panel_tray_icons, platform::SecureStorageState,
        secure_storage_status_label, visible_tray_item,
    };
    use crate::model::TrayItem;

    #[test]
    fn launcher_exposes_every_non_ready_secure_storage_state() {
        for (state, expected) in [
            (SecureStorageState::Starting, "Secure storage is starting…"),
            (SecureStorageState::Locked, "Secure storage is locked."),
            (
                SecureStorageState::PromptRequired,
                "Secure storage is waiting for its unlock prompt.",
            ),
            (
                SecureStorageState::Unavailable,
                "Secure storage is unavailable.",
            ),
        ] {
            assert_eq!(secure_storage_status_label(state), Some(expected));
        }
        assert_eq!(secure_storage_status_label(SecureStorageState::Ready), None);
    }

    #[test]
    fn right_panel_cluster_is_compact_and_grouped() {
        let layout = panel_status_layout(1920, 3);
        assert_eq!(layout.control_start, 1816.0);
        assert_eq!(layout.tray_start, 1732.0);
        assert_eq!(layout.codex_start, 1696.0);
        assert_eq!(
            layout.codex_icon_bounds(),
            Rect::new(1700.0, 14.0, 28.0, 28.0)
        );
    }

    #[test]
    fn tray_icons_are_normalized_without_letter_fallbacks() {
        let item = TrayItem {
            id: "status".into(),
            title: "Status Application".into(),
            icon: RgbaImage::from_pixel(32, 16, Rgba([10, 20, 30, 255])),
        };
        let icons = panel_tray_icons(&[item]);

        assert_eq!(icons.len(), 1);
        assert_eq!(icons[0].dimensions(), (18, 18));
        assert!(icons[0].pixels().any(|pixel| pixel.0[3] != 0));
    }

    #[test]
    fn visible_tray_indices_address_the_last_four_items() {
        let items = (0..5)
            .map(|index| TrayItem {
                id: index.to_string(),
                title: String::new(),
                icon: RgbaImage::new(1, 1),
            })
            .collect::<Vec<_>>();

        assert_eq!(visible_tray_item(&items, 0).unwrap().id, "1");
        assert_eq!(visible_tray_item(&items, 3).unwrap().id, "4");
        assert!(visible_tray_item(&items, 4).is_none());
    }
}
