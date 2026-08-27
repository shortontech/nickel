#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod effects;
mod model;
mod persistence;
mod platform;
mod view;

use model::SettingsApp;
use persistence::{save_shell_settings, save_wallpaper_settings};
use platform::*;

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::collections::HashMap;

#[cfg(target_os = "linux")]
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue},
};

use nickel_core::{
    shell_settings::{ShellSettings, ThemePreference},
    theme::{ThemeMode, ThemePalette},
    wallpaper_settings::{WallpaperPosition, WallpaperSettings},
};
use nickel_i18n::Localizer;
use nickel_ui::{
    AnyView, ControllerAction, ControllerInput, Insets, LinearGradient, NavigationPane,
    PaintCommand, PaneNavigation, Point as UiPoint, Rect as UiRect, SdlCanvasPresenter,
    SemanticColors, SemanticTheme, TextAlign, UiStateStore, UiTree, ui,
};
use sdl3::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    mouse::{MouseButton, MouseWheelDirection},
};

const SIDEBAR_WIDTH: i32 = 190;
const DISPLAY_PLANE: Rect = Rect {
    x: 210,
    y: 96,
    w: 600,
    h: 340,
};

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn semantic_theme(palette: ThemePalette) -> SemanticTheme {
    SemanticTheme::new(SemanticColors {
        window: palette.background,
        sidebar: palette.panel,
        card: palette.surface,
        raised: palette.surface_hover,
        hover: palette.surface_hover,
        primary_text: palette.text,
        secondary_text: palette.muted,
        accent: palette.accent,
        accent_soft: palette.accent_soft,
        positive: palette.complement,
    })
}

struct OutputSnapshot {
    name: String,
    model: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    physical_width: i32,
    physical_height: i32,
    primary: bool,
}

impl Rect {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

struct DisplayCard {
    connector: String,
    name: String,
    detail: String,
    logical_width: i32,
    logical_height: i32,
    rect: Rect,
    primary: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct BluetoothDevice {
    id: String,
    name: String,
    paired: bool,
    connected: bool,
    battery_percent: Option<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct BluetoothSnapshot {
    available: bool,
    powered: bool,
    discovering: bool,
    adapter_name: String,
    devices: Vec<BluetoothDevice>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsPage {
    Display,
    Bar,
    Appearance,
    Network,
    Bluetooth,
}

impl SettingsPage {
    fn previous(self) -> Self {
        match self {
            Self::Display => Self::Display,
            Self::Bar => Self::Display,
            Self::Appearance => Self::Bar,
            Self::Network => Self::Appearance,
            Self::Bluetooth => Self::Network,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Display => Self::Bar,
            Self::Bar => Self::Appearance,
            Self::Appearance => Self::Network,
            Self::Network => Self::Bluetooth,
            Self::Bluetooth => Self::Bluetooth,
        }
    }
}

struct NetworkAdapter {
    name: String,
    description: String,
    connected: bool,
    speed: u64,
}

struct WifiNetwork {
    id: String,
    profile: String,
    signal: u32,
    connected: bool,
    saved: bool,
    secure: bool,
    #[cfg(target_os = "windows")]
    interface: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingsMessage {
    Navigate(SettingsPage),
    BluetoothPower,
    BluetoothDiscovery,
    BluetoothDevice(usize),
    BluetoothScroll,
    WifiPower,
    WifiNetwork(usize),
    NetworkScroll,
    AppearanceLight,
    AppearanceDark,
    SetAppearanceHue(u16),
    SetAppearanceIntensity(u8),
    WallpaperChoose,
    WallpaperPosition(WallpaperPosition),
    BarPrimaryDisplay,
    BarAllDisplays,
    BarDisplayWindows,
    BarAllWindows,
    SetDesktopCount(u8),
    DisplayIdentify,
    DisplayPrimary,
    DisplayApply,
}

fn desktop_count_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::SetDesktopCount(1 + (fraction.clamp(0.0, 1.0) * 7.0).round() as u8)
}

fn appearance_hue_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::SetAppearanceHue((fraction.clamp(0.0, 1.0) * 359.0).round() as u16)
}

fn appearance_intensity_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::SetAppearanceIntensity((fraction.clamp(0.0, 1.0) * 100.0).round() as u8)
}

impl SettingsApp {
    fn transient_scroll(&self, message: &SettingsMessage) -> f32 {
        self.ui
            .id_for_message(message)
            .and_then(|id| self.ui_state.state(id))
            .map(|state| state.scroll_offset)
            .unwrap_or(0.0)
    }

    fn captured_message(&self) -> Option<&SettingsMessage> {
        self.ui_state
            .captured()
            .and_then(|id| self.ui.message_for_id(id))
    }

    fn hovered_message(&self) -> Option<&SettingsMessage> {
        self.ui_state
            .hovered()
            .and_then(|id| self.ui.message_for_id(id))
    }

    fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    fn palette(&self) -> ThemePalette {
        ThemePalette::from_appearance(
            self.shell_settings
                .resolve_appearance(nickel_platform::appearance()),
        )
    }

    fn ui_theme(&self) -> SemanticTheme {
        semantic_theme(self.palette())
    }

    fn pointer_pressed(&mut self) {
        let (x, y) = self.cursor;
        let point = UiPoint {
            x: x as f32,
            y: y as f32,
        };
        if let Some(SettingsMessage::SetDesktopCount(count)) = self.ui.message_at_owned(point) {
            let id = self
                .ui
                .id_for_message(&SettingsMessage::SetDesktopCount(count))
                .cloned();
            self.ui_state.set_capture(id);
            self.set_desktop_count(count);
            self.request_redraw();
            return;
        }
        if let Some(SettingsMessage::SetAppearanceHue(hue)) = self.ui.message_at_owned(point) {
            let id = self
                .ui
                .id_for_message(&SettingsMessage::SetAppearanceHue(hue))
                .cloned();
            self.ui_state.set_capture(id);
            self.set_appearance_hue(hue);
            self.request_redraw();
            return;
        }
        if let Some(SettingsMessage::SetAppearanceIntensity(intensity)) =
            self.ui.message_at_owned(point)
        {
            let id = self
                .ui
                .id_for_message(&SettingsMessage::SetAppearanceIntensity(intensity))
                .cloned();
            self.ui_state.set_capture(id);
            self.set_appearance_intensity(intensity);
            self.request_redraw();
            return;
        }
        let message = self.ui.message_at(point).cloned();
        if let Some(message) = message {
            match message {
                SettingsMessage::Navigate(page) => {
                    self.page = page;
                    match page {
                        SettingsPage::Network => self.load_linux_network(),
                        SettingsPage::Bluetooth => self.load_bluetooth(),
                        _ => {}
                    }
                }
                SettingsMessage::BluetoothPower => {
                    let _ = set_bluetooth_adapter_property("Powered", !self.bluetooth.powered);
                    self.next_bluetooth_refresh = Instant::now();
                }
                SettingsMessage::BluetoothDiscovery => {
                    let _ =
                        set_bluetooth_adapter_property("Discovering", !self.bluetooth.discovering);
                    self.next_bluetooth_refresh = Instant::now();
                }
                SettingsMessage::WifiPower => {
                    #[cfg(target_os = "linux")]
                    if let Err(error) = set_linux_wifi_enabled(!self.wifi_enabled) {
                        self.wifi_status = self.localizer.value(
                            "settings-network-connection-failed",
                            "error",
                            &error,
                        );
                    }
                    self.next_network_refresh = Instant::now();
                }
                SettingsMessage::BluetoothDevice(index) => {
                    if let Some(device) = self.bluetooth.devices.get(index) {
                        let _ = toggle_bluetooth_device(device);
                        self.next_bluetooth_refresh = Instant::now();
                    }
                }
                SettingsMessage::AppearanceLight => {
                    self.shell_settings.theme = ThemePreference::Light;
                    save_shell_settings(&self.shell_settings);
                }
                SettingsMessage::AppearanceDark => {
                    self.shell_settings.theme = ThemePreference::Dark;
                    save_shell_settings(&self.shell_settings);
                }
                SettingsMessage::WallpaperChoose => {
                    if let Some(path) = choose_wallpaper() {
                        self.wallpaper_settings.image = Some(path);
                        save_wallpaper_settings(&self.wallpaper_settings);
                    }
                }
                SettingsMessage::WallpaperPosition(position) => {
                    self.wallpaper_settings.position = position;
                    save_wallpaper_settings(&self.wallpaper_settings);
                }
                SettingsMessage::BarPrimaryDisplay => {
                    self.shell_settings.bar_on_all_displays = false;
                    save_shell_settings(&self.shell_settings);
                }
                SettingsMessage::BarAllDisplays => {
                    self.shell_settings.bar_on_all_displays = true;
                    save_shell_settings(&self.shell_settings);
                }
                SettingsMessage::BarDisplayWindows => {
                    self.shell_settings.all_windows_on_every_bar = false;
                    save_shell_settings(&self.shell_settings);
                }
                SettingsMessage::BarAllWindows => {
                    self.shell_settings.all_windows_on_every_bar = true;
                    save_shell_settings(&self.shell_settings);
                }
                SettingsMessage::DisplayIdentify => match session_request("identify-outputs") {
                    Ok(response) if response == "ok" => {
                        self.status = self.localizer.text("settings-status-identifying")
                    }
                    _ => self.status = self.localizer.text("settings-status-identify-failed"),
                },
                SettingsMessage::DisplayPrimary => {
                    for (index, display) in self.displays.iter_mut().enumerate() {
                        display.primary = index == self.selected;
                    }
                    self.applied = false;
                    self.status = self.localizer.text("settings-status-changes-not-applied");
                }
                SettingsMessage::DisplayApply => self.apply_layout(),
                SettingsMessage::WifiNetwork(index) => self.connect_windows_wifi(index),
                SettingsMessage::SetDesktopCount(_)
                | SettingsMessage::SetAppearanceHue(_)
                | SettingsMessage::SetAppearanceIntensity(_)
                | SettingsMessage::BluetoothScroll
                | SettingsMessage::NetworkScroll => {}
            }
            self.request_redraw();
            return;
        }
        if self.page != SettingsPage::Display {
            self.request_redraw();
            return;
        } else if let Some(index) = self
            .displays
            .iter()
            .rposition(|display| display.rect.contains(x, y))
        {
            self.selected = index;
            let rect = self.displays[index].rect;
            self.drag_offset = Some((x - rect.x, y - rect.y));
            self.applied = false;
            self.status = self.localizer.text("settings-status-changes-not-applied");
        }
        self.request_redraw();
    }

    fn pointer_moved(&mut self, x: f32, y: f32) {
        self.cursor = (x.round() as i32, y.round() as i32);
        if matches!(
            self.captured_message(),
            Some(SettingsMessage::SetDesktopCount(_))
        ) {
            if let Some(fraction) = self.ui.horizontal_fraction_for_matching(x, |message| {
                matches!(message, SettingsMessage::SetDesktopCount(_))
            }) {
                self.set_desktop_count_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        if matches!(
            self.captured_message(),
            Some(SettingsMessage::SetAppearanceHue(_))
        ) {
            if let Some(fraction) = self.ui.horizontal_fraction_for_matching(x, |message| {
                matches!(message, SettingsMessage::SetAppearanceHue(_))
            }) {
                self.set_appearance_hue_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        if matches!(
            self.captured_message(),
            Some(SettingsMessage::SetAppearanceIntensity(_))
        ) {
            if let Some(fraction) = self.ui.horizontal_fraction_for_matching(x, |message| {
                matches!(message, SettingsMessage::SetAppearanceIntensity(_))
            }) {
                self.set_appearance_intensity_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        let hovered = self.ui.id_at(UiPoint { x, y }).cloned();
        if self.ui_state.set_hovered(hovered) != nickel_ui::Invalidation::None {
            self.request_redraw();
        }
        if matches!(self.page, SettingsPage::Bluetooth | SettingsPage::Network) {
            return;
        }
        if self.page != SettingsPage::Display {
            return;
        }
        if let Some((offset_x, offset_y)) = self.drag_offset {
            let mut rect = self.displays[self.selected].rect;
            rect.x = self.cursor.0 - offset_x;
            rect.y = self.cursor.1 - offset_y;
            rect = constrain_center(rect, DISPLAY_PLANE);
            rect = self
                .displays
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != self.selected)
                .fold(rect, |moving, (_, other)| snap_rect(moving, other.rect, 42));
            self.displays[self.selected].rect = rect;
            self.applied = false;
            self.status = self.localizer.text("settings-status-changes-not-applied");
            self.request_redraw();
        }
    }

    fn finish_drag(&mut self) {
        self.ui_state.set_pressed(None);
        self.ui_state.set_capture(None);
        if self.page != SettingsPage::Display {
            return;
        }
        if self.drag_offset.take().is_none() {
            return;
        }
        let selected = self.displays[self.selected].rect;
        let snapped = self
            .displays
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.selected)
            .map(|(_, other)| attach_rect_centered(selected, other.rect))
            .min_by_key(|rect| {
                let dx = rect.x - selected.x;
                let dy = rect.y - selected.y;
                dx * dx + dy * dy
            })
            .unwrap_or(selected);
        self.displays[self.selected].rect = snapped;
        self.request_redraw();
    }

    fn scroll_settings(&mut self, wheel_y: f32) {
        let delta = -wheel_y * 42.0;
        match self.page {
            SettingsPage::Network => {
                if let Some(id) = self
                    .ui
                    .id_for_message(&SettingsMessage::NetworkScroll)
                    .cloned()
                {
                    let maximum = self
                        .ui
                        .scroll_extent(&SettingsMessage::NetworkScroll)
                        .map(|extent| (extent.content.height - extent.viewport.height).max(0.0))
                        .unwrap_or(0.0);
                    if self.ui_state.scroll_by(id, delta, maximum) != nickel_ui::Invalidation::None
                    {
                        self.request_redraw();
                    }
                }
            }
            SettingsPage::Bluetooth => {
                if let Some(id) = self
                    .ui
                    .id_for_message(&SettingsMessage::BluetoothScroll)
                    .cloned()
                {
                    let maximum = self
                        .ui
                        .scroll_extent(&SettingsMessage::BluetoothScroll)
                        .map(|extent| (extent.content.height - extent.viewport.height).max(0.0))
                        .unwrap_or(0.0);
                    if self.ui_state.scroll_by(id, delta, maximum) != nickel_ui::Invalidation::None
                    {
                        self.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }

    fn set_desktop_count_from_fraction(&mut self, fraction: f32) {
        let SettingsMessage::SetDesktopCount(count) = desktop_count_message(fraction) else {
            unreachable!()
        };
        self.set_desktop_count(count);
    }

    fn set_desktop_count(&mut self, count: u8) {
        if count == self.shell_settings.desktop_count {
            return;
        }
        self.shell_settings.desktop_count = count;
        self.shell_settings.active_desktop = self
            .shell_settings
            .active_desktop
            .min(count.saturating_sub(1));
        save_shell_settings(&self.shell_settings);
    }

    fn set_appearance_hue_from_fraction(&mut self, fraction: f32) {
        let SettingsMessage::SetAppearanceHue(hue) = appearance_hue_message(fraction) else {
            unreachable!()
        };
        self.set_appearance_hue(hue);
    }

    fn set_appearance_hue(&mut self, hue: u16) {
        if self.shell_settings.accent_hue == Some(hue) {
            return;
        }
        self.shell_settings.accent_hue = Some(hue);
        self.appearance_save_deadline = Some(Instant::now() + self.frame_interval);
    }

    fn set_appearance_intensity_from_fraction(&mut self, fraction: f32) {
        let SettingsMessage::SetAppearanceIntensity(intensity) =
            appearance_intensity_message(fraction)
        else {
            unreachable!()
        };
        self.set_appearance_intensity(intensity);
    }

    fn set_appearance_intensity(&mut self, intensity: u8) {
        if self.shell_settings.accent_intensity == Some(intensity) {
            return;
        }
        self.shell_settings.accent_intensity = Some(intensity);
        self.appearance_save_deadline = Some(Instant::now() + self.frame_interval);
    }

    fn render(&mut self, presenter: &mut SdlCanvasPresenter) -> Result<(), String> {
        let appearance = self
            .shell_settings
            .resolve_appearance(nickel_platform::appearance());
        let palette = ThemePalette::from_appearance(appearance);
        let (logical_width, logical_height) = presenter.window().size();
        let pixel_width = presenter.window().size_in_pixels().0;
        let scale = pixel_width as f32 / logical_width.max(1) as f32;
        let ui = self.build_ui(logical_width as f32, logical_height as f32);
        ui.reconcile_state(&mut self.ui_state);
        self.ui = ui;
        let mut commands = self.ui.commands().to_vec();
        if self.page == SettingsPage::Display {
            for (index, display) in self.displays.iter().enumerate() {
                let rect = UiRect::new(
                    display.rect.x as f32,
                    display.rect.y as f32,
                    display.rect.w as f32,
                    display.rect.h as f32,
                );
                commands.push(PaintCommand::Fill {
                    rect,
                    color: if index == self.selected {
                        palette.accent_soft
                    } else {
                        palette.surface
                    },
                });
                commands.push(PaintCommand::Stroke {
                    rect,
                    color: if display.primary {
                        palette.accent
                    } else {
                        palette.muted
                    },
                    width: if display.primary { 4.0 } else { 2.0 },
                });
                commands.push(PaintCommand::Text {
                    bounds: UiRect::new(
                        (display.rect.x + 18) as f32,
                        (display.rect.y + 20) as f32,
                        (display.rect.w - 36) as f32,
                        32.0,
                    ),
                    text: display.name.clone(),
                    scale: 3.0,
                    color: palette.text,
                    align: TextAlign::Start,
                    bold: false,
                });
                commands.push(PaintCommand::Text {
                    bounds: UiRect::new(
                        (display.rect.x + 18) as f32,
                        (display.rect.y + 58) as f32,
                        (display.rect.w - 36) as f32,
                        24.0,
                    ),
                    text: display.detail.clone(),
                    scale: 2.0,
                    color: palette.muted,
                    align: TextAlign::Start,
                    bold: false,
                });
                if display.primary {
                    commands.push(PaintCommand::Text {
                        bounds: UiRect::new(
                            (display.rect.x + 18) as f32,
                            (display.rect.y + display.rect.h - 30) as f32,
                            (display.rect.w - 36) as f32,
                            24.0,
                        ),
                        text: "PRIMARY".into(),
                        scale: 2.0,
                        color: palette.accent,
                        align: TextAlign::Start,
                        bold: true,
                    });
                }
            }
        }
        presenter.present_accelerated(&commands, scale)?;
        Ok(())
    }
}

impl SettingsApp {
    fn finish_resize_if_due(&mut self) {
        let Some(deadline) = self.resize_deadline else {
            return;
        };
        if Instant::now() >= deadline {
            self.resize_deadline = None;
            self.request_redraw();
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Quit { .. }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            }
            | Event::Window {
                win_event: WindowEvent::CloseRequested,
                ..
            } => self.running = false,
            Event::Window {
                win_event: WindowEvent::Exposed,
                ..
            } => self.request_redraw(),
            Event::Window {
                win_event: WindowEvent::MouseLeave,
                ..
            } => {
                if self.ui_state.set_hovered(None) != nickel_ui::Invalidation::None {
                    self.request_redraw();
                }
            }
            Event::Window {
                win_event: WindowEvent::Resized(_, _) | WindowEvent::PixelSizeChanged(_, _),
                ..
            } => {
                self.resize_deadline = Some(Instant::now() + Duration::from_millis(24));
            }
            Event::MouseMotion { x, y, .. } => self.pointer_moved(x, y),
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                self.pointer_moved(x, y);
                self.pointer_pressed();
            }
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                self.pointer_moved(x, y);
                self.finish_drag();
            }
            Event::MouseWheel {
                y,
                direction,
                mouse_x,
                mouse_y,
                ..
            } => {
                self.pointer_moved(mouse_x, mouse_y);
                let wheel_y = if direction == MouseWheelDirection::Flipped {
                    -y
                } else {
                    y
                };
                self.scroll_settings(wheel_y);
            }
            _ => {}
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self
            .appearance_save_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            save_shell_settings(&self.shell_settings);
            self.appearance_save_deadline = None;
        }
        for action in self.controller.poll(now) {
            self.handle_controller_action(action);
        }
        if self.page == SettingsPage::Bluetooth && now >= self.next_bluetooth_refresh {
            self.load_bluetooth();
        }
        if self.page == SettingsPage::Network && now >= self.next_network_refresh {
            self.load_linux_network();
        }
        let Some(refresh_at) = self.next_wifi_refresh else {
            return;
        };
        if now < refresh_at {
            return;
        }

        self.load_windows_wifi();
        let connected = self.pending_wifi_profile.as_ref().is_some_and(|profile| {
            self.wifi_networks
                .iter()
                .any(|network| network.connected && network.profile.eq_ignore_ascii_case(profile))
        });
        if connected {
            let profile = self.pending_wifi_profile.take().unwrap_or_default();
            self.wifi_status =
                self.localizer
                    .value("settings-network-connected-to", "profile", &profile);
            self.next_wifi_refresh = None;
            self.wifi_refreshes_left = 0;
        } else if self.wifi_refreshes_left > 1 {
            self.wifi_refreshes_left -= 1;
            self.next_wifi_refresh = Some(Instant::now() + Duration::from_millis(400));
            if let Some(profile) = &self.pending_wifi_profile {
                self.wifi_status =
                    self.localizer
                        .value("settings-network-connecting", "profile", profile);
            }
        } else {
            let profile = self.pending_wifi_profile.take().unwrap_or_default();
            self.wifi_status =
                self.localizer
                    .value("settings-network-connection-timeout", "profile", &profile);
            self.next_wifi_refresh = None;
            self.wifi_refreshes_left = 0;
        }
        self.request_redraw();
    }
}

fn snap_rect(mut moving: Rect, fixed: Rect, threshold: i32) -> Rect {
    let horizontal_candidates = [fixed.x - moving.w, fixed.x + fixed.w];
    if let Some(x) = horizontal_candidates
        .into_iter()
        .min_by_key(|candidate| (moving.x - candidate).abs())
        .filter(|candidate| (moving.x - candidate).abs() <= threshold)
    {
        moving.x = x;
        let vertical_candidates = [fixed.y, fixed.y + fixed.h - moving.h];
        if let Some(y) = vertical_candidates
            .into_iter()
            .min_by_key(|candidate| (moving.y - candidate).abs())
            .filter(|candidate| (moving.y - candidate).abs() <= threshold)
        {
            moving.y = y;
        }
        return moving;
    }

    let vertical_candidates = [fixed.y - moving.h, fixed.y + fixed.h];
    if let Some(y) = vertical_candidates
        .into_iter()
        .min_by_key(|candidate| (moving.y - candidate).abs())
        .filter(|candidate| (moving.y - candidate).abs() <= threshold)
    {
        moving.y = y;
        let horizontal_alignment = [fixed.x, fixed.x + fixed.w - moving.w];
        if let Some(x) = horizontal_alignment
            .into_iter()
            .min_by_key(|candidate| (moving.x - candidate).abs())
            .filter(|candidate| (moving.x - candidate).abs() <= threshold)
        {
            moving.x = x;
        }
    }
    moving
}

#[cfg(target_os = "linux")]
fn choose_wallpaper() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("kdialog")
        .args([
            "--getopenfilename",
            "",
            "image/png image/jpeg image/webp image/bmp",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.into())
}

#[cfg(not(target_os = "linux"))]
fn choose_wallpaper() -> Option<std::path::PathBuf> {
    None
}

fn attach_rect_centered(moving: Rect, fixed: Rect) -> Rect {
    let candidates = [
        Rect {
            x: fixed.x - moving.w,
            y: fixed.y + (fixed.h - moving.h) / 2,
            ..moving
        },
        Rect {
            x: fixed.x + fixed.w,
            y: fixed.y + (fixed.h - moving.h) / 2,
            ..moving
        },
        Rect {
            x: fixed.x + (fixed.w - moving.w) / 2,
            y: fixed.y - moving.h,
            ..moving
        },
        Rect {
            x: fixed.x + (fixed.w - moving.w) / 2,
            y: fixed.y + fixed.h,
            ..moving
        },
    ];
    candidates
        .into_iter()
        .min_by_key(|candidate| {
            let dx = candidate.x - moving.x;
            let dy = candidate.y - moving.y;
            dx * dx + dy * dy
        })
        .unwrap_or(moving)
}

fn logical_placements(displays: &[DisplayCard]) -> Vec<(i32, i32)> {
    if displays.is_empty() {
        return Vec::new();
    }
    let mut placements = vec![(0, 0); displays.len()];
    for index in 1..displays.len() {
        let moving = &displays[index];
        let fixed = &displays[index - 1];
        let (fixed_x, fixed_y) = placements[index - 1];
        let edge_distances = [
            (moving.rect.x + moving.rect.w - fixed.rect.x).abs(),
            (moving.rect.x - fixed.rect.x - fixed.rect.w).abs(),
            (moving.rect.y + moving.rect.h - fixed.rect.y).abs(),
            (moving.rect.y - fixed.rect.y - fixed.rect.h).abs(),
        ];
        let edge = edge_distances
            .iter()
            .enumerate()
            .min_by_key(|(_, distance)| *distance)
            .map(|(edge, _)| edge)
            .unwrap_or(1);
        placements[index] = match edge {
            0 => (
                fixed_x - moving.logical_width,
                fixed_y + (fixed.logical_height - moving.logical_height) / 2,
            ),
            1 => (
                fixed_x + fixed.logical_width,
                fixed_y + (fixed.logical_height - moving.logical_height) / 2,
            ),
            2 => (
                fixed_x + (fixed.logical_width - moving.logical_width) / 2,
                fixed_y - moving.logical_height,
            ),
            _ => (
                fixed_x + (fixed.logical_width - moving.logical_width) / 2,
                fixed_y + fixed.logical_height,
            ),
        };
    }
    let minimum_x = placements.iter().map(|(x, _)| *x).min().unwrap_or(0);
    let minimum_y = placements.iter().map(|(_, y)| *y).min().unwrap_or(0);
    for (x, y) in &mut placements {
        *x -= minimum_x;
        *y -= minimum_y;
    }
    placements
}

#[cfg(target_os = "windows")]
fn apply_windows_layout(
    displays: &[DisplayCard],
    placements: &[(i32, i32)],
    primary: &str,
) -> Result<(), i32> {
    use std::mem::size_of;
    use windows::{
        Win32::Graphics::Gdi::{
            CDS_NORESET, CDS_SET_PRIMARY, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW,
            DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_POSITION, ENUM_CURRENT_SETTINGS,
            EnumDisplaySettingsW,
        },
        core::PCWSTR,
    };

    let (primary_x, primary_y) = displays
        .iter()
        .zip(placements)
        .find(|(display, _)| display.connector == primary)
        .map(|(_, placement)| *placement)
        .unwrap_or((0, 0));

    for (display, &(x, y)) in displays.iter().zip(placements) {
        let device_name = format!(r"\\.\{}", display.connector);
        let device_wide: Vec<u16> = device_name.encode_utf16().chain([0]).collect();
        let device = PCWSTR(device_wide.as_ptr());
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: `device` and `mode` remain valid for the duration of each Win32 call.
        if !unsafe { EnumDisplaySettingsW(device, ENUM_CURRENT_SETTINGS, &raw mut mode) }.as_bool()
        {
            return Err(-1);
        }
        mode.dmFields |= DM_POSITION;
        mode.Anonymous1.Anonymous2.dmPosition.x = x - primary_x;
        mode.Anonymous1.Anonymous2.dmPosition.y = y - primary_y;
        let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
        if display.connector == primary {
            flags |= CDS_SET_PRIMARY;
        }
        // SAFETY: The device name and initialized DEVMODEW are valid for this synchronous call.
        let result =
            unsafe { ChangeDisplaySettingsExW(device, Some(&raw const mode), None, flags, None) };
        if result != DISP_CHANGE_SUCCESSFUL {
            return Err(result.0);
        }
    }

    // Commit all staged changes together to avoid transient intermediate layouts.
    // SAFETY: A null device and mode apply the previously staged changes.
    let result =
        unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE::default(), None) };
    if result == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(result.0)
    }
}

fn constrain_center(mut monitor: Rect, plane: Rect) -> Rect {
    monitor.x = monitor.x.clamp(plane.x, plane.x + plane.w - monitor.w);
    monitor.y = monitor.y.clamp(plane.y, plane.y + plane.h - monitor.h);
    monitor
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let _log_path = nickel_logging::init("nickel-settings").ok();
    sdl3::hint::set("SDL_APP_ID", "nickel-settings");
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let mut window = video
        .window("Nickel Settings", 850, 580)
        .position_centered()
        .resizable()
        .build()
        .map_err(|error| error.to_string())?;
    window.set_minimum_size(850, 580)?;
    video.text_input().start(&window);
    let mut events = sdl.event_pump()?;
    let frame_interval = window
        .get_display()
        .ok()
        .and_then(|display| display.get_mode().ok())
        .map(|mode| mode.refresh_rate)
        .filter(|refresh_rate| refresh_rate.is_finite() && *refresh_rate > 1.0)
        .map(|refresh_rate| Duration::from_secs_f64(1.0 / f64::from(refresh_rate)))
        .unwrap_or_else(|| Duration::from_millis(16));
    let mut app = SettingsApp {
        frame_interval,
        ..SettingsApp::default()
    };
    let mut presenter = SdlCanvasPresenter::new(window)?;
    app.render(&mut presenter)?;
    tracing::info!(
        target: "nickel",
        elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
        "Nickel Settings first frame presented"
    );
    app.load_outputs();
    app.load_bluetooth();
    app.load_linux_network();
    #[cfg(target_os = "windows")]
    {
        app.load_windows_outputs(&video);
        app.load_windows_network();
        app.load_windows_wifi();
    }
    let mut next_frame = Instant::now();

    while app.running {
        if let Some(event) = events.wait_event_timeout(app.frame_interval) {
            app.handle_event(event);
            for event in events.poll_iter() {
                app.handle_event(event);
            }
        }
        app.tick();
        app.finish_resize_if_due();
        let now = Instant::now();
        if app.redraw_requested.get() && now >= next_frame {
            app.redraw_requested.set(false);
            if let Err(error) = app.render(&mut presenter) {
                tracing::error!(%error, "failed to render Nickel Settings");
                app.running = false;
            }
            next_frame = Instant::now() + app.frame_interval;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        NetworkAdapter, Rect, SettingsApp, SettingsMessage, SettingsPage, attach_rect_centered,
        constrain_center, snap_rect,
    };

    #[test]
    fn display_remains_completely_inside_plane() {
        let plane = Rect {
            x: 40,
            y: 120,
            w: 770,
            h: 340,
        };
        let monitor = Rect {
            x: -500,
            y: -500,
            w: 300,
            h: 180,
        };

        let constrained = constrain_center(monitor, plane);
        assert_eq!(constrained.x, plane.x);
        assert_eq!(constrained.y, plane.y);
    }

    #[test]
    fn nearby_monitor_edges_snap_together_and_align() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 300,
            h: 180,
        };
        let moving = Rect {
            x: 105,
            y: 190,
            w: 270,
            h: 160,
        };

        let snapped = snap_rect(moving, fixed, 42);
        assert_eq!(snapped.x + snapped.w, fixed.x);
        assert_eq!(snapped.y, fixed.y);
    }

    #[test]
    fn released_monitor_touches_and_centers_on_nearest_edge() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 220,
            h: 125,
        };
        let moving = Rect {
            x: 150,
            y: 205,
            w: 220,
            h: 125,
        };

        let attached = attach_rect_centered(moving, fixed);

        assert_eq!(attached.x + attached.w, fixed.x);
        assert_eq!(attached.y + attached.h / 2, fixed.y + fixed.h / 2);
    }

    #[test]
    fn distant_monitors_keep_freeform_position() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 300,
            h: 180,
        };
        let moving = Rect {
            x: 40,
            y: 430,
            w: 200,
            h: 120,
        };

        let snapped = snap_rect(moving, fixed, 42);
        assert_eq!((snapped.x, snapped.y), (moving.x, moving.y));
    }

    #[test]
    fn settings_lists_derive_scroll_extents_from_intrinsic_children() {
        let mut app = SettingsApp {
            page: SettingsPage::Network,
            network_adapters: (0..12)
                .map(|index| NetworkAdapter {
                    name: format!("Adapter {index}"),
                    description: "Synthetic adapter".into(),
                    connected: false,
                    speed: 0,
                })
                .collect(),
            ..SettingsApp::default()
        };
        let narrow = app.build_ui(560.0, 360.0);
        assert!(
            narrow
                .scroll_extent(&SettingsMessage::NetworkScroll)
                .is_some_and(|extent| extent.can_scroll())
        );

        app.page = SettingsPage::Bluetooth;
        let compact = app.build_ui(560.0, 360.0);
        assert!(compact.commands().iter().all(|command| match command {
            nickel_ui::PaintCommand::Fill { rect, .. }
            | nickel_ui::PaintCommand::OverlayFill { rect, .. }
            | nickel_ui::PaintCommand::Stroke { rect, .. }
            | nickel_ui::PaintCommand::OverlayStroke { rect, .. }
            | nickel_ui::PaintCommand::TopRoundedFill { rect, .. }
            | nickel_ui::PaintCommand::Gradient { rect, .. } => {
                rect.size.width >= 0.0 && rect.size.height >= 0.0
            }
            _ => true,
        }));

        let source = include_str!("main.rs");
        assert!(!source.contains(&["network_", "content_height"].concat()));
        assert!(!source.contains(&["bluetooth_", "content_height"].concat()));
        assert!(!source.contains(&["device_", "height"].concat()));
    }
}
