use std::sync::Arc;

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
    platform::{
        self, AudioStatus, BluetoothStatus, NetworkStatus, ShellCommand, TrayFeed, TraySource,
        WindowAction, WindowFeed,
    },
    sdl_control_view::{ControlAction, ControlCenterFrame, ControlViewState, build_control_center},
    sdl_launcher_view::{
        LauncherAction, LauncherFrame, LauncherIconCache, LauncherViewState, build_launcher_frame,
    },
    sdl_shell::SurfaceRole,
};
use sdl3::keyboard::{Keycode, Mod};

const PANEL_ITEM_WIDTH: f32 = 52.0;

pub struct LiveShell {
    launcher: Launcher,
    window_feed: WindowFeed,
    tray_feed: TrayFeed,
    windows: Vec<OpenWindow>,
    tray: Vec<TrayItem>,
    wallpaper: Option<Arc<image::RgbaImage>>,
    panel_icon: Arc<image::RgbaImage>,
    palette: ThemePalette,
    network: NetworkStatus,
    bluetooth: BluetoothStatus,
    audio: AudioStatus,
    launcher_visible: bool,
    control_visible: bool,
    control_state: ControlViewState,
    launcher_view: LauncherViewState,
    launcher_icons: LauncherIconCache,
    launcher_frame: Option<LauncherFrame>,
}

impl LiveShell {
    pub fn new() -> Self {
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
        let windows = window_feed.snapshot(&launcher).unwrap_or_default();
        let tray = tray_feed.snapshot();
        let network = platform::network_status();
        let bluetooth = platform::bluetooth_status();
        let audio = platform::audio_status();
        Self {
            launcher,
            window_feed,
            tray_feed,
            windows,
            tray,
            wallpaper,
            panel_icon,
            palette,
            network,
            bluetooth,
            audio,
            launcher_visible: false,
            control_visible: false,
            control_state: ControlViewState::default(),
            launcher_view: LauncherViewState::default(),
            launcher_icons: LauncherIconCache::new(),
            launcher_frame: None,
        }
    }

    pub fn refresh(&mut self) -> bool {
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
        }
    }

    pub fn surface_visible(&self, role: SurfaceRole) -> bool {
        match role {
            SurfaceRole::Desktop | SurfaceRole::Panel => true,
            SurfaceRole::Launcher => self.launcher_visible,
            SurfaceRole::ControlCenter => self.control_visible,
        }
    }

    pub fn panel_click(&mut self, x: f32, width: u32) -> bool {
        if x < PANEL_ITEM_WIDTH {
            self.launcher_visible = !self.launcher_visible;
            self.control_visible = false;
            let _ = platform::send_shell_command(if self.launcher_visible {
                ShellCommand::Show
            } else {
                ShellCommand::Hide
            });
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
        let control_start = width.saturating_sub(190) as f32;
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
            None => return false,
        }
        self.launcher_view.scroll_row = 0;
        true
    }

    fn launch_result(&mut self, index: usize) {
        if let Some(application) = self.launcher.result_at(index) {
            let _ = platform::launch_application(application);
            self.launcher_visible = false;
            let _ = platform::send_shell_command(ShellCommand::Hide);
            self.launcher.clear();
        }
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

    fn launcher_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let frame = build_launcher_frame(
            &self.launcher,
            &mut self.launcher_view,
            &mut self.launcher_icons,
            width,
            height,
            self.palette,
        );
        let commands = frame.commands.clone();
        self.launcher_frame = Some(frame);
        commands
    }

    fn panel_scene(&mut self, width: u32, height: u32) -> Vec<PaintCommand> {
        let mut commands = vec![PaintCommand::Fill {
            rect: Rect::new(0.0, 0.0, width as f32, height as f32),
            color: self.palette.panel,
        }];
        commands.push(PaintCommand::Image {
            bounds: Rect::new(12.0, 12.0, 32.0, 32.0),
            id: 2,
            image: Arc::clone(&self.panel_icon),
        });
        let groups = self.launcher.group_windows(&self.windows);
        for (index, group) in groups.iter().take(12).enumerate() {
            let x = PANEL_ITEM_WIDTH + index as f32 * PANEL_ITEM_WIDTH;
            if group.active() {
                commands.push(PaintCommand::RoundedFill {
                    rect: Rect::new(x + 4.0, 7.0, 44.0, 42.0),
                    color: self.palette.accent_soft,
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
        }
        let now = Zoned::now();
        let clock = now.strftime("%-I:%M %p").to_string();
        let date = now.strftime("%-m/%-d/%Y").to_string();
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
        for (index, item) in self.tray.iter().rev().take(4).rev().enumerate() {
            let x = width.saturating_sub(190 + (self.tray.len().min(4) - index) as u32 * 34) as f32;
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
