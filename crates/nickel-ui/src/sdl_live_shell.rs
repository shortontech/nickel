use std::sync::Arc;

#[cfg(not(target_os = "macos"))]
use jiff::Zoned;
use nickel_components::{PaintCommand, Point, Rect, TextAlign};
use nickel_core::{
    shell_settings::ShellSettings,
    theme::{Appearance, ThemePalette},
    wallpaper_settings::WallpaperSettings,
};

use crate::{
    launcher::Launcher,
    model::{OpenWindow, TrayItem},
    notification::DesktopNotification,
    platform::{
        self, AudioStatus, BluetoothStatus, NetworkStatus, NotificationFeed, NotificationSource,
        ShellCommand, TrayFeed, TraySource, WindowAction, WindowFeed,
    },
    sdl_control_view::{ControlAction, ControlCenterFrame, ControlViewState, build_control_center},
    sdl_launcher_view::{
        LauncherAction, LauncherFrame, LauncherIconCache, LauncherViewState, build_launcher_frame,
    },
    sdl_shell::SurfaceRole,
};
use sdl3::keyboard::{Keycode, Mod};

const PANEL_ITEM_WIDTH: f32 = 52.0;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PanelHover {
    Launcher,
    Task(usize),
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
    notification: Option<DesktopNotification>,
    wallpaper: Option<Arc<image::RgbaImage>>,
    panel_icon: Arc<image::RgbaImage>,
    palette: ThemePalette,
    network: NetworkStatus,
    bluetooth: BluetoothStatus,
    audio: AudioStatus,
    launcher_visible: bool,
    control_visible: bool,
    panel_hover: Option<PanelHover>,
    control_state: ControlViewState,
    launcher_view: LauncherViewState,
    launcher_icons: LauncherIconCache,
    launcher_frame: Option<LauncherFrame>,
    launcher_status: Option<String>,
    secure_storage_override: Option<String>,
    secure_storage_state: platform::SecureStorageState,
}

impl LiveShell {
    pub fn new() -> Result<Self, String> {
        let mut launcher = Launcher::new(platform::applications());
        launcher.set_places(crate::places::applications());
        let wallpaper_settings = WallpaperSettings::load_default();
        let wallpaper = wallpaper_settings
            .image
            .as_deref()
            .and_then(|path| image::open(path).ok())
            .map(image::DynamicImage::into_rgba8)
            .map(Arc::new);
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
        let window_feed = WindowFeed::new();
        let tray_feed = TrayFeed::new();
        let notification_feed = NotificationFeed::new()?;
        let windows = window_feed.snapshot(&launcher).unwrap_or_default();
        let tray = tray_feed.snapshot();
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
            notification: None,
            wallpaper,
            panel_icon,
            palette,
            network,
            bluetooth,
            audio,
            launcher_visible: false,
            control_visible: false,
            panel_hover: None,
            control_state: ControlViewState::default(),
            launcher_view: LauncherViewState::default(),
            launcher_icons: LauncherIconCache::new(),
            launcher_frame: None,
            launcher_status: None,
            secure_storage_override: None,
            secure_storage_state,
        })
    }

    pub fn refresh(&mut self) -> bool {
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
            changed = true;
        }
        if let Some(visible) = self.window_feed.launcher_visible() {
            changed |= self.launcher_visible != visible;
            self.launcher_visible = visible;
            if visible {
                changed |= self.control_visible;
                self.control_visible = false;
            }
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
            changed = true;
        }
        let notification = self.notification_feed.snapshot();
        if notification != self.notification {
            self.notification = notification;
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
        }
    }

    pub fn surface_visible(&self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Desktop | SurfaceRole::Panel => true,
            SurfaceRole::Launcher => self.launcher_visible,
            SurfaceRole::ControlCenter => self.control_visible,
            SurfaceRole::Notification => self.notification.is_some(),
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
        let control_start = panel_control_start(width);
        #[cfg(not(target_os = "macos"))]
        if x >= control_start {
            self.control_visible = !self.control_visible;
            self.launcher_visible = false;
            return true;
        }
        let tray_start = control_start - self.tray.len() as f32 * 34.0;
        if x >= tray_start {
            let index = ((x - tray_start) / 34.0) as usize;
            if let Some(item) = self.tray.get(index) {
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
                self.set_launcher_visible(true);
                true
            }
            platform::GlobalShortcut::HideLauncher => {
                self.set_launcher_visible(false);
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
        self.launcher_visible = visible;
        if visible {
            self.control_visible = false;
        } else {
            self.launcher.clear();
        }
        let _ = platform::send_shell_command(if visible {
            ShellCommand::Show
        } else {
            ShellCommand::Hide
        });
        platform::launcher_visibility_applied(visible);
    }

    pub fn launcher_click(&mut self, x: f32, y: f32) -> bool {
        let action = self
            .launcher_frame
            .as_ref()
            .and_then(|frame| frame.action_at(Point { x, y }))
            .cloned()
            .unwrap_or(LauncherAction::Dismiss);
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
                self.launcher_visible = false;
                let _ = platform::send_shell_command(ShellCommand::Hide);
                true
            }
            SurfaceRole::ControlCenter if self.control_visible => {
                self.control_visible = false;
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
        self.launcher.insert(value);
        self.launcher_view.scroll_row = 0;
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
        match key {
            Some(Keycode::Escape) => self.launcher_visible = false,
            Some(Keycode::Backspace) => self.launcher.backspace(),
            Some(Keycode::Down) => self.launcher.select_grid_down(columns),
            Some(Keycode::Up) => self.launcher.select_grid_up(columns),
            Some(Keycode::Left) => self.launcher.select_grid_left(columns),
            Some(Keycode::Right) => self.launcher.select_grid_right(columns),
            Some(Keycode::Return) => {
                let index = self.launcher.selected_index();
                self.launch_result(index);
            }
            Some(key) => {
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
                    return false;
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
                    self.launcher.insert(&character.to_string());
                }
            }
            None => return false,
        }
        self.launcher_view.scroll_row = 0;
        true
    }

    fn launch_result(&mut self, index: usize) {
        let Some(application) = self.launcher.result_at(index).cloned() else {
            return;
        };
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
        self.launcher_visible = false;
        let _ = platform::send_shell_command(ShellCommand::Hide);
        self.launcher.clear();
    }

    fn desktop_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
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
            });
        }
        commands
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
                .and_then(|application| self.launcher_icons.resolve(application));
            if let Some((id, image)) = icon {
                commands.push(PaintCommand::Image {
                    bounds: Rect::new(x + 10.0, 11.0, 32.0, 32.0),
                    id,
                    image,
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
                    rect: Rect::new(width.saturating_sub(154) as f32, 7.0, 140.0, 42.0),
                    color: self.palette.surface_hover,
                    radius: 8.0,
                });
            }
            commands.push(text(
                Rect::new(width.saturating_sub(150) as f32, 6.0, 132.0, 22.0),
                &clock,
                0.78,
                self.palette.text,
                TextAlign::End,
                false,
            ));
            commands.push(text(
                Rect::new(width.saturating_sub(150) as f32, 28.0, 132.0, 20.0),
                &date,
                0.72,
                self.palette.text,
                TextAlign::End,
                false,
            ));
        }
        for (index, item) in self.tray.iter().rev().take(4).rev().enumerate() {
            let x = panel_control_start(width) - (self.tray.len().min(4) - index) as f32 * 34.0;
            if self.panel_hover == Some(PanelHover::Tray(index)) {
                commands.push(PaintCommand::RoundedFill {
                    rect: Rect::new(x + 1.0, 9.0, 28.0, 38.0),
                    color: self.palette.surface_hover,
                    radius: 7.0,
                });
            }
            commands.push(text(
                Rect::new(x, 14.0, 30.0, 26.0),
                &item.title.chars().next().unwrap_or('•').to_string(),
                0.8,
                self.palette.text,
                TextAlign::Center,
                false,
            ));
        }
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
        let control_start = panel_control_start(width);
        #[cfg(not(target_os = "macos"))]
        if x >= control_start {
            return Some(PanelHover::Control);
        }
        let tray_count = self.tray.len().min(4);
        let tray_start = control_start - tray_count as f32 * 34.0;
        if x >= tray_start {
            return Some(PanelHover::Tray(((x - tray_start) / 34.0) as usize));
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
        match action {
            LauncherAction::Dismiss => {
                self.launcher_visible = false;
                let _ = platform::send_shell_command(ShellCommand::Hide);
            }
            LauncherAction::FocusSearch => {}
            LauncherAction::SetView(view) => {
                self.launcher.set_view(view);
                self.launcher_view.scroll_row = 0;
            }
            LauncherAction::ActivateResult(index) => self.launch_result(index),
            LauncherAction::TogglePin(id) => self.launcher.toggle_pin(&id),
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
        width.saturating_sub(190) as f32
    }
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
    }
}

#[cfg(test)]
mod tests {
    use super::{platform::SecureStorageState, secure_storage_status_label};

    #[test]
    fn launcher_exposes_every_non_ready_secure_storage_state() {
        for state in [
            SecureStorageState::Starting,
            SecureStorageState::Locked,
            SecureStorageState::PromptRequired,
            SecureStorageState::Unavailable,
        ] {
            assert!(secure_storage_status_label(state).is_some());
        }
        assert_eq!(secure_storage_status_label(SecureStorageState::Ready), None);
    }
}
