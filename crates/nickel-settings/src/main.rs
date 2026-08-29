#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

mod cli;
mod effects;
mod model;
mod persistence;
mod platform;
mod view;

use model::SettingsApp;
use nickel_session_protocol::{
    ClientEnvelope, Command as SessionCommand, OutputLayout as SessionOutputLayout,
    OutputPlacement as SessionOutputPlacement, Query as SessionQuery, Request as SessionRequest,
    ServerEnvelope, ServerMessage,
};
use persistence::{save_shell_settings, try_save_shell_settings, try_save_wallpaper_settings};
use platform::*;

use std::{
    cell::Cell,
    sync::{Arc, OnceLock, mpsc},
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
    shell_settings::{AnimationLevel, ShellSettings, ThemePreference},
    theme::{Appearance, ThemeMode, ThemePalette, accent_from_hue},
    wallpaper_settings::{WallpaperPosition, WallpaperSettings},
};
use nickel_i18n::Localizer;
use nickel_ui::{
    AnyView, Button, ButtonPresentation, ChoiceCard, ChoiceCardGroup, ColorSwatch,
    ComponentBuilderExt, Container, ControllerAction, ControllerInput, Image, ImageFit, Insets,
    NavigationItem, NavigationPane, PageHeader, PaintCommand, PaneNavigation, Point as UiPoint,
    PreviewTile, ReadingDirection, Rect as UiRect, SdlCanvasPresenter, SelectField, SemanticColors,
    SemanticTheme, SettingsCard, SettingsNarrowPane, SettingsNavigation, SettingsRow,
    SettingsSearchEntry, SettingsSearchField, SettingsShell, SettingsStatus, SettingsStatusKind,
    SliderField, Surface, SurfaceRole, Switch, TabList, TextAlign, UiEvent, UiId, UiStateStore,
    UiTree, search_settings, ui,
};
use sdl3::{
    event::{Event, WindowEvent},
    keyboard::{Keycode, Mod},
    mouse::{MouseButton, MouseWheelDirection},
};

const SIDEBAR_WIDTH: i32 = 280;
const DISPLAY_PLANE: Rect = Rect {
    x: 210,
    y: 96,
    w: 600,
    h: 340,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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

fn load_wallpaper_preview(
    settings: &WallpaperSettings,
) -> Result<Option<nickel_platform::DecodedPreview>, nickel_platform::PreviewDecodeError> {
    settings
        .image
        .as_deref()
        .map(nickel_platform::decode_image_preview)
        .transpose()
}

#[derive(Clone, Copy)]
enum SidebarIconKind {
    Search,
    Display,
    Bar,
    Appearance,
    Network,
    Bluetooth,
    Keyboard,
    About,
}

impl SidebarIconKind {
    const ALL: [Self; 8] = [
        Self::Search,
        Self::Display,
        Self::Bar,
        Self::Appearance,
        Self::Network,
        Self::Bluetooth,
        Self::Keyboard,
        Self::About,
    ];

    fn index(self) -> usize {
        self as usize
    }

    fn glyph(self) -> char {
        match self {
            Self::Search => '\u{f002}',
            Self::Display => '\u{f108}',
            Self::Bar => '\u{f0c9}',
            Self::Appearance => '\u{f1fc}',
            Self::Network => '\u{f0ac}',
            Self::Bluetooth => '\u{f294}',
            Self::Keyboard => '\u{f11c}',
            Self::About => '\u{f05a}',
        }
    }

    fn fallback(self) -> &'static [u8] {
        match self {
            Self::Search => include_bytes!("../../../assets/icons/settings/search.svg"),
            Self::Display => include_bytes!("../../../assets/icons/settings/display.svg"),
            Self::Bar => include_bytes!("../../../assets/icons/settings/bar.svg"),
            Self::Appearance => include_bytes!("../../../assets/icons/settings/appearance.svg"),
            Self::Network => include_bytes!("../../../assets/icons/settings/network.svg"),
            Self::Bluetooth => include_bytes!("../../../assets/icons/settings/bluetooth.svg"),
            Self::Keyboard => include_bytes!("../../../assets/icons/start-menu/keyboard.svg"),
            Self::About => include_bytes!("../../../assets/icons/start-menu/about.svg"),
        }
    }
}

fn rasterize_sidebar_icon(kind: SidebarIconKind) -> Arc<image::RgbaImage> {
    const SIZE: u32 = 24;
    let mut options = resvg::usvg::Options::default();
    let font_loaded = options
        .fontdb_mut()
        .load_font_file("/usr/share/fonts/opentype/font-awesome/FontAwesome.otf")
        .is_ok();
    let font_awesome = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{SIZE}" height="{SIZE}">
<text x="12" y="18" text-anchor="middle" font-family="FontAwesome" font-size="16" fill="#a8abb2">{}</text>
</svg>"##,
        kind.glyph()
    );
    let data = if font_loaded {
        font_awesome.as_bytes()
    } else {
        kind.fallback()
    };
    let image = resvg::usvg::Tree::from_data(data, &options)
        .ok()
        .and_then(|tree| {
            let mut pixmap = resvg::tiny_skia::Pixmap::new(SIZE, SIZE)?;
            resvg::render(
                &tree,
                resvg::tiny_skia::Transform::identity(),
                &mut pixmap.as_mut(),
            );
            image::RgbaImage::from_raw(SIZE, SIZE, pixmap.take())
        })
        .unwrap_or_else(|| image::RgbaImage::new(SIZE, SIZE));
    Arc::new(image)
}

fn sidebar_icon<Message>(kind: SidebarIconKind) -> Image<Message> {
    static ICONS: OnceLock<[Arc<image::RgbaImage>; 8]> = OnceLock::new();
    let icons = ICONS.get_or_init(|| SidebarIconKind::ALL.map(rasterize_sidebar_icon));
    Image::new(400 + kind.index() as u16, icons[kind.index()].clone())
        .fit(ImageFit::Contain)
        .width(20.0)
        .height(20.0)
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
    KeyboardShortcuts,
    About,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
enum AppearanceTab {
    #[default]
    General,
    Theme,
    Fonts,
    Icons,
    Cursors,
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum AppearanceNotice {
    Confirmation(String),
    Error(String),
}

impl SettingsPage {
    fn previous(self) -> Self {
        match self {
            Self::Display => Self::Display,
            Self::Bar => Self::Display,
            Self::Appearance => Self::Bar,
            Self::Network => Self::Appearance,
            Self::Bluetooth => Self::Network,
            Self::KeyboardShortcuts => Self::Bluetooth,
            Self::About => Self::KeyboardShortcuts,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Display => Self::Bar,
            Self::Bar => Self::Appearance,
            Self::Appearance => Self::Network,
            Self::Network => Self::Bluetooth,
            Self::Bluetooth => Self::KeyboardShortcuts,
            Self::KeyboardShortcuts => Self::About,
            Self::About => Self::About,
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
    NavigateTarget(SettingsPage, String),
    ToggleNavigation,
    SidebarSearchChanged(String),
    BluetoothPower,
    BluetoothDiscovery,
    BluetoothDevice(usize),
    BluetoothScroll,
    WifiPower,
    WifiNetwork(usize),
    NetworkScroll,
    AppearanceLight,
    AppearanceDark,
    AppearanceSystem,
    AppearanceTab(AppearanceTab),
    AppearanceReset,
    SetAccentHue(u16),
    SetAppearanceHue(u16),
    SetAppearanceIntensity(u8),
    WallpaperChoose,
    WallpaperRemove,
    ToggleWallpaperPositionSelect,
    WallpaperPosition(WallpaperPosition),
    SetReduceTransparency(bool),
    ToggleAnimationSelect,
    SetAnimationLevel(AnimationLevel),
    AppearanceScroll,
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

fn reduce_transparency_message(value: bool) -> SettingsMessage {
    SettingsMessage::SetReduceTransparency(value)
}

fn sidebar_search_message(value: String) -> SettingsMessage {
    SettingsMessage::SidebarSearchChanged(value)
}

impl SettingsApp {
    fn dispatch_ui_event(&mut self, event: UiEvent) {
        let outcome = self.ui.handle_event(&mut self.ui_state, event);
        for message in outcome.messages {
            self.handle_settings_message(message);
        }
        if outcome.invalidation != nickel_ui::Invalidation::None {
            self.request_redraw();
        }
    }

    fn transient_scroll(&self, message: &SettingsMessage) -> f32 {
        self.ui
            .id_for_message(message)
            .and_then(|id| self.ui_state.state(id))
            .map(|state| state.scroll_offset)
            .unwrap_or(0.0)
    }

    fn hovered_message(&self) -> Option<&SettingsMessage> {
        self.ui_state
            .hovered()
            .and_then(|id| self.ui.message_for_id(id))
    }

    fn focused_role(&self) -> Option<&str> {
        let focused = self.ui_state.focused()?;
        self.ui
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| &node.id == focused)
            .and_then(|node| node.accessibility_role.as_deref())
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

    fn apply_pending_focus(&mut self, ui: &UiTree<SettingsMessage>) {
        let Some(target) = self.pending_focus.take() else {
            return;
        };
        let suffix = format!("/{}", target.as_str());
        if let Some(id) = ui
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id == target || node.id.as_str().ends_with(&suffix))
            .map(|node| node.id.clone())
        {
            self.ui_state.set_focus(Some(id));
        }
    }

    fn handle_settings_message(&mut self, message: SettingsMessage) {
        match message {
            SettingsMessage::Navigate(page) => {
                self.page = page;
                self.narrow_navigation = false;
                match page {
                    SettingsPage::Network => self.load_linux_network(),
                    SettingsPage::Bluetooth => self.load_bluetooth(),
                    _ => {}
                }
            }
            SettingsMessage::NavigateTarget(page, target) => {
                self.page = page;
                self.narrow_navigation = false;
                self.sidebar_query.clear();
                self.pending_focus = Some(target.into());
                if page == SettingsPage::Appearance {
                    self.appearance_tab = AppearanceTab::General;
                }
                match page {
                    SettingsPage::Network => self.load_linux_network(),
                    SettingsPage::Bluetooth => self.load_bluetooth(),
                    _ => {}
                }
            }
            SettingsMessage::SidebarSearchChanged(value) => self.sidebar_query = value,
            SettingsMessage::ToggleNavigation => {
                self.narrow_navigation = !self.narrow_navigation;
            }
            SettingsMessage::BluetoothPower => {
                let _ = set_bluetooth_adapter_property("Powered", !self.bluetooth.powered);
                self.next_bluetooth_refresh = Instant::now();
            }
            SettingsMessage::BluetoothDiscovery => {
                let _ = set_bluetooth_adapter_property("Discovering", !self.bluetooth.discovering);
                self.next_bluetooth_refresh = Instant::now();
            }
            SettingsMessage::WifiPower => {
                #[cfg(target_os = "linux")]
                if let Err(error) = set_linux_wifi_enabled(!self.wifi_enabled) {
                    self.wifi_status =
                        self.localizer
                            .value("settings-network-connection-failed", "error", &error);
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
                self.persist_appearance();
            }
            SettingsMessage::AppearanceDark => {
                self.shell_settings.theme = ThemePreference::Dark;
                self.persist_appearance();
            }
            SettingsMessage::AppearanceSystem => {
                self.shell_settings.theme = ThemePreference::System;
                self.persist_appearance();
            }
            SettingsMessage::AppearanceTab(tab) => self.appearance_tab = tab,
            SettingsMessage::SetAccentHue(hue) => {
                self.shell_settings.accent_hue = Some(hue.min(359));
                self.persist_appearance();
            }
            SettingsMessage::AppearanceReset => self.reset_appearance(),
            SettingsMessage::SetDesktopCount(count) => self.set_desktop_count(count),
            SettingsMessage::SetAppearanceHue(hue) => self.set_appearance_hue(hue),
            SettingsMessage::SetAppearanceIntensity(intensity) => {
                self.set_appearance_intensity(intensity);
            }
            SettingsMessage::WallpaperChoose => {
                let (sender, receiver) = mpsc::channel();
                match nickel_platform::choose_image_file(Box::new(move |outcome| {
                    let _ = sender.send(outcome);
                })) {
                    Ok(()) => {
                        self.wallpaper_dialog_rx = Some(receiver);
                        self.wallpaper_status = None;
                    }
                    Err(error) => {
                        self.wallpaper_status = Some(error);
                    }
                }
            }
            SettingsMessage::WallpaperRemove => {
                self.wallpaper_settings.image = None;
                self.wallpaper_preview = None;
                self.wallpaper_dimensions = None;
                self.wallpaper_status = None;
                self.persist_wallpaper();
            }
            SettingsMessage::ToggleWallpaperPositionSelect => {
                self.wallpaper_position_select_expanded = !self.wallpaper_position_select_expanded;
            }
            SettingsMessage::WallpaperPosition(position) => {
                self.wallpaper_settings.position = position;
                self.wallpaper_position_select_expanded = false;
                self.persist_wallpaper();
            }
            SettingsMessage::SetReduceTransparency(value) => {
                self.shell_settings.reduce_transparency = value;
                self.persist_appearance();
            }
            SettingsMessage::SetAnimationLevel(level) => {
                self.shell_settings.animations = level;
                self.animation_select_expanded = false;
                self.persist_appearance();
            }
            SettingsMessage::ToggleAnimationSelect => {
                self.animation_select_expanded = !self.animation_select_expanded;
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
            SettingsMessage::DisplayIdentify => {
                match session_request(SessionRequest::Command(SessionCommand::IdentifyOutputs)) {
                    Ok(ServerMessage::Ack) => {
                        self.status = self.localizer.text("settings-status-identifying")
                    }
                    _ => self.status = self.localizer.text("settings-status-identify-failed"),
                }
            }
            SettingsMessage::DisplayPrimary => {
                for (index, display) in self.displays.iter_mut().enumerate() {
                    display.primary = index == self.selected;
                }
                self.applied = false;
                self.status = self.localizer.text("settings-status-changes-not-applied");
            }
            SettingsMessage::DisplayApply => self.apply_layout(),
            SettingsMessage::WifiNetwork(index) => self.connect_windows_wifi(index),
            SettingsMessage::BluetoothScroll
            | SettingsMessage::NetworkScroll
            | SettingsMessage::AppearanceScroll => {}
        }
        self.request_redraw();
    }

    fn pointer_pressed(&mut self) {
        let (x, y) = self.cursor;
        let point = UiPoint {
            x: x as f32,
            y: y as f32,
        };
        self.dispatch_ui_event(UiEvent::PointerPressed(point));
        if self.ui.message_at(point).is_some() {
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
        self.dispatch_ui_event(UiEvent::PointerMoved(UiPoint { x, y }));
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
            rect = constrain_center(rect, self.display_plane);
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
            SettingsPage::Appearance => {
                self.scroll_page(SettingsMessage::AppearanceScroll, delta);
            }
            SettingsPage::Network => {
                self.scroll_page(SettingsMessage::NetworkScroll, delta);
            }
            SettingsPage::Bluetooth => {
                self.scroll_page(SettingsMessage::BluetoothScroll, delta);
            }
            _ => {}
        }
    }

    fn scroll_page(&mut self, message: SettingsMessage, delta: f32) {
        let Some(id) = self.ui.id_for_message(&message).cloned() else {
            return;
        };
        let maximum = self
            .ui
            .scroll_extent(&message)
            .map(|extent| (extent.content.height - extent.viewport.height).max(0.0))
            .unwrap_or(0.0);
        if self.ui_state.scroll_by(id, delta, maximum) != nickel_ui::Invalidation::None {
            self.request_redraw();
        }
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

    fn set_appearance_hue(&mut self, hue: u16) {
        if self.shell_settings.accent_hue == Some(hue) {
            return;
        }
        self.shell_settings.accent_hue = Some(hue);
        self.appearance_save_deadline = Some(Instant::now() + self.frame_interval);
    }

    fn set_appearance_intensity(&mut self, intensity: u8) {
        if self.shell_settings.accent_intensity == Some(intensity) {
            return;
        }
        self.shell_settings.accent_intensity = Some(intensity);
        self.appearance_save_deadline = Some(Instant::now() + self.frame_interval);
    }

    fn persist_appearance(&mut self) {
        if !self.persistence_enabled {
            return;
        }
        self.record_appearance_persistence(try_save_shell_settings(&self.shell_settings));
    }

    fn persist_wallpaper(&mut self) {
        if !self.persistence_enabled {
            return;
        }
        self.record_appearance_persistence(try_save_wallpaper_settings(&self.wallpaper_settings));
    }

    fn record_appearance_persistence(&mut self, result: Result<(), String>) {
        self.appearance_notice = result.err().map(|error| {
            AppearanceNotice::Error(self.localizer.value(
                "settings-appearance-save-failed",
                "error",
                &error,
            ))
        });
    }

    fn reset_appearance(&mut self) {
        self.reset_appearance_values();
        if !self.persistence_enabled {
            self.appearance_notice = Some(AppearanceNotice::Confirmation(
                self.localizer
                    .text("settings-appearance-reset-confirmation"),
            ));
            return;
        }
        let result = try_save_shell_settings(&self.shell_settings)
            .and_then(|()| try_save_wallpaper_settings(&self.wallpaper_settings));
        if let Err(error) = result {
            self.record_appearance_persistence(Err(error));
        } else {
            self.appearance_notice = Some(AppearanceNotice::Confirmation(
                self.localizer
                    .text("settings-appearance-reset-confirmation"),
            ));
        }
    }

    fn reset_appearance_values(&mut self) {
        let defaults = ShellSettings::default();
        self.shell_settings.theme = defaults.theme;
        self.shell_settings.accent_hue = defaults.accent_hue;
        self.shell_settings.accent_intensity = defaults.accent_intensity;
        self.shell_settings.reduce_transparency = defaults.reduce_transparency;
        self.shell_settings.animations = defaults.animations;
        self.wallpaper_settings = WallpaperSettings::default();
        self.wallpaper_preview = None;
        self.wallpaper_dimensions = None;
        self.wallpaper_status = None;
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
        self.apply_pending_focus(&ui);
        self.ui = ui;
        self.sync_display_plane();
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
                    wrap: false,
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
                    wrap: false,
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
                        wrap: false,
                    });
                }
            }
        }
        presenter.present_accelerated(&commands, scale)?;
        Ok(())
    }

    fn sync_display_plane(&mut self) {
        if self.page != SettingsPage::Display {
            return;
        }
        let Some(node) = self
            .ui
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("/display-plane"))
        else {
            return;
        };
        let resolved = Rect {
            x: node.allocated.origin.x.round() as i32,
            y: node.allocated.origin.y.round() as i32,
            w: node.allocated.size.width.round() as i32,
            h: node.allocated.size.height.round() as i32,
        };
        if resolved.w <= 0 || resolved.h <= 0 || resolved == self.display_plane {
            return;
        }
        self.display_plane = resolved;
        center_display_rects(&mut self.displays, resolved);
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
            | Event::Window {
                win_event: WindowEvent::CloseRequested,
                ..
            } => self.running = false,
            Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            } => {
                if self.ui_state.focused().is_some() {
                    self.ui_state.set_focus(None);
                    self.sidebar_query.clear();
                    self.request_redraw();
                } else {
                    self.running = false;
                }
            }
            Event::KeyDown {
                keycode: Some(Keycode::Backspace),
                ..
            } => self.dispatch_ui_event(UiEvent::TextBackspace),
            Event::KeyDown {
                keycode: Some(Keycode::Tab),
                keymod,
                ..
            } if keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD) => {
                self.dispatch_ui_event(UiEvent::FocusPrevious)
            }
            Event::KeyDown {
                keycode: Some(Keycode::Tab),
                ..
            } => self.dispatch_ui_event(UiEvent::FocusNext),
            Event::KeyDown {
                keycode: Some(Keycode::Return | Keycode::Space),
                ..
            } => self.dispatch_ui_event(UiEvent::KeyboardActivate),
            Event::KeyDown {
                keycode: Some(Keycode::Up),
                ..
            } => self.dispatch_ui_event(UiEvent::FocusPrevious),
            Event::KeyDown {
                keycode: Some(Keycode::Down),
                ..
            } => self.dispatch_ui_event(UiEvent::FocusNext),
            Event::KeyDown {
                keycode: Some(Keycode::Left),
                keymod,
                ..
            } => self.dispatch_ui_event(if self.focused_role() == Some("tab") {
                UiEvent::FocusPrevious
            } else {
                UiEvent::TextMoveLeft {
                    extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                }
            }),
            Event::KeyDown {
                keycode: Some(Keycode::Right),
                keymod,
                ..
            } => self.dispatch_ui_event(if self.focused_role() == Some("tab") {
                UiEvent::FocusNext
            } else {
                UiEvent::TextMoveRight {
                    extend_selection: keymod.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
                }
            }),
            Event::KeyDown {
                keycode: Some(Keycode::Delete),
                ..
            } => self.dispatch_ui_event(UiEvent::TextDelete),
            Event::TextInput { text, .. } => self.dispatch_ui_event(UiEvent::TextInput(text)),
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
                self.dispatch_ui_event(UiEvent::PointerReleased(UiPoint { x, y }));
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
        self.poll_wallpaper_dialog();
        let now = Instant::now();
        if self
            .appearance_save_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.persist_appearance();
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

fn center_display_rects(displays: &mut [DisplayCard], plane: Rect) {
    let Some(first) = displays.first() else {
        return;
    };
    let minimum_x = displays
        .iter()
        .map(|display| display.rect.x)
        .min()
        .unwrap_or(first.rect.x);
    let minimum_y = displays
        .iter()
        .map(|display| display.rect.y)
        .min()
        .unwrap_or(first.rect.y);
    let maximum_x = displays
        .iter()
        .map(|display| display.rect.x + display.rect.w)
        .max()
        .unwrap_or(first.rect.x + first.rect.w);
    let maximum_y = displays
        .iter()
        .map(|display| display.rect.y + display.rect.h)
        .max()
        .unwrap_or(first.rect.y + first.rect.h);
    let arrangement_center_x = minimum_x + (maximum_x - minimum_x) / 2;
    let arrangement_center_y = minimum_y + (maximum_y - minimum_y) / 2;
    let target_center_x = plane.x + plane.w / 2;
    let target_center_y = plane.y + plane.h / 2;
    let delta_x = target_center_x - arrangement_center_x;
    let delta_y = target_center_y - arrangement_center_y;
    for display in displays {
        display.rect.x += delta_x;
        display.rect.y += delta_y;
        display.rect = constrain_center(display.rect, plane);
    }
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
    let initial_page = match cli::parse(std::env::args_os().skip(1)) {
        Ok(cli::Action::Run(page)) => page,
        Ok(cli::Action::Help) => {
            print!("{}", cli::HELP);
            return Ok(());
        }
        Err(error) => {
            eprintln!("error: {error}\n\n{}", cli::HELP);
            std::process::exit(2);
        }
    };
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
    let mut app = SettingsApp::with_initial_page(initial_page);
    app.frame_interval = frame_interval;
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
        BluetoothDevice, NetworkAdapter, PaintCommand, Rect, SIDEBAR_WIDTH, SettingsApp,
        SettingsMessage, SettingsPage, ThemePreference, UiEvent, UiPoint, WallpaperSettings,
        attach_rect_centered, constrain_center, snap_rect,
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
        app.bluetooth.available = true;
        app.bluetooth.powered = true;
        app.bluetooth.devices = (0..12)
            .map(|index| BluetoothDevice {
                id: format!("device-{index}"),
                name: format!("Headphones {index}"),
                paired: true,
                connected: index == 0,
                battery_percent: Some(80),
            })
            .collect();
        let compact = app.build_ui(560.0, 360.0);
        assert!(
            compact
                .scroll_extent(&SettingsMessage::BluetoothScroll)
                .is_some_and(|extent| extent.can_scroll()),
            "Bluetooth device rows must determine the scroll extent"
        );
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
    }

    #[test]
    fn appearance_composition_exposes_shared_controls_and_scrolls() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        let tree = app.build_ui(850.0, 580.0);

        assert!(
            tree.scroll_extent(&SettingsMessage::AppearanceScroll)
                .is_some_and(|extent| extent.can_scroll())
        );
        app.ui = tree;
        app.scroll_settings(-1.0);
        assert!(app.transient_scroll(&SettingsMessage::AppearanceScroll) > 0.0);
        let expanded = app.build_ui(850.0, 1600.0);
        for message in [
            SettingsMessage::AppearanceLight,
            SettingsMessage::AppearanceDark,
            SettingsMessage::AppearanceSystem,
            SettingsMessage::WallpaperChoose,
            SettingsMessage::WallpaperRemove,
            SettingsMessage::SetReduceTransparency(true),
            SettingsMessage::AppearanceReset,
        ] {
            assert!(
                expanded.message_rect(&message).is_some(),
                "missing Appearance control for {message:?}"
            );
        }
    }

    #[test]
    fn unavailable_appearance_tabs_explain_platform_authority_and_restart_scope() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.appearance_tab = super::AppearanceTab::Theme;
        let tree = app.build_ui_with_diagnostics(1000.0, 760.0);
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("status") && node.state.as_deref() == Some("unavailable")
        }));
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.role.as_deref() == Some("status")
                && node.state.as_deref() == Some("restart required")
        }));
    }

    #[test]
    fn appearance_reference_matrix_renders_locales_themes_scales_and_widths() {
        let cases = [
            (
                "dark-en",
                "en-US",
                ThemePreference::Dark,
                1424.0,
                1105.0,
                1.0,
                Some(305),
                Some(100),
            ),
            (
                "light-en-2x",
                "en-US",
                ThemePreference::Light,
                1000.0,
                760.0,
                2.0,
                Some(24),
                Some(82),
            ),
            (
                "automatic-en",
                "en-US",
                ThemePreference::System,
                1000.0,
                760.0,
                1.0,
                None,
                None,
            ),
            (
                "dark-de",
                "de-DE",
                ThemePreference::Dark,
                1424.0,
                1105.0,
                1.0,
                Some(305),
                Some(100),
            ),
            (
                "dark-zh",
                "zh-CN",
                ThemePreference::Dark,
                1000.0,
                760.0,
                1.0,
                Some(188),
                Some(90),
            ),
            (
                "dark-es-narrow",
                "es",
                ThemePreference::Dark,
                560.0,
                900.0,
                1.0,
                Some(340),
                Some(75),
            ),
            (
                "dark-ar-rtl",
                "ar",
                ThemePreference::Dark,
                1000.0,
                760.0,
                1.0,
                Some(78),
                Some(70),
            ),
        ];
        for (name, locale, preference, width, height, scale, hue, intensity) in cases {
            let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
            app.localizer = nickel_i18n::Localizer::for_locale(Some(locale));
            app.shell_settings.theme = preference;
            app.shell_settings.accent_hue = hue;
            app.shell_settings.accent_intensity = intensity;
            app.wallpaper_settings = WallpaperSettings::default();
            app.wallpaper_preview = None;
            app.wallpaper_dimensions = None;
            let tree = app.build_ui_with_diagnostics(width, height);
            let visible_diagnostics = tree
                .diagnostics()
                .iter()
                .filter(|diagnostic| {
                    diagnostic.kind != nickel_ui::DiagnosticKind::ClippedInteraction
                })
                .collect::<Vec<_>>();
            assert!(
                visible_diagnostics.is_empty(),
                "{name}: {visible_diagnostics:#?}"
            );
            assert!(
                tree.scroll_extent(&SettingsMessage::AppearanceScroll)
                    .is_some_and(|extent| extent.content.height >= extent.viewport.height),
                "{name}: Appearance content must remain reachable through its scroller"
            );
            let physical_width = (width * scale) as u32;
            let physical_height = (height * scale) as u32;
            let mut renderer = nickel_ui::SdlComponentRenderer::new_pixel_buffer(
                physical_width,
                physical_height,
                scale,
            );
            renderer.render(tree.commands());
            assert!(renderer.pixels().iter().any(|pixel| pixel.a > 0));
            let image = image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(
                physical_width,
                physical_height,
                |x, y| {
                    let pixel = renderer.pixels()[(y * physical_width + x) as usize];
                    image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
                },
            );
            let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/nickel-ui-snapshots")
                .join(format!("appearance-{name}.png"));
            std::fs::create_dir_all(output.parent().unwrap()).unwrap();
            image.save(output).unwrap();
            if locale == "ar" {
                assert!(tree.commands().iter().any(|command| matches!(
                    command,
                    PaintCommand::Text { text, .. } if text == "المظهر"
                )));
            }
        }
    }

    #[test]
    fn appearance_activation_converges_across_supported_modalities() {
        fn activate(app: &mut SettingsApp, message: SettingsMessage, modality: &str) {
            app.ui = app.build_ui(1424.0, 1800.0);
            app.ui.reconcile_state(&mut app.ui_state);
            let id = app.ui.id_for_message(&message).unwrap().clone();
            match modality {
                "pointer" => {
                    let rect = app.ui.message_rect(&message).unwrap();
                    let point = UiPoint {
                        x: rect.origin.x + rect.size.width / 2.0,
                        y: rect.origin.y + rect.size.height / 2.0,
                    };
                    app.dispatch_ui_event(UiEvent::PointerMoved(point));
                    app.dispatch_ui_event(UiEvent::PointerPressed(point));
                    app.dispatch_ui_event(UiEvent::PointerReleased(point));
                }
                "keyboard" => {
                    app.ui_state.set_focus(Some(id));
                    app.dispatch_ui_event(UiEvent::KeyboardActivate);
                }
                "controller" => {
                    app.ui_state.set_controller_selected(Some(id));
                    app.dispatch_ui_event(UiEvent::ControllerActivate);
                }
                "accessibility" => {
                    app.dispatch_ui_event(UiEvent::AccessibilityActivate(id));
                }
                _ => unreachable!(),
            }
        }

        for modality in ["pointer", "keyboard", "controller", "accessibility"] {
            let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
            activate(&mut app, SettingsMessage::AppearanceDark, modality);
            assert_eq!(
                app.shell_settings.theme,
                ThemePreference::Dark,
                "{modality}"
            );

            let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
            activate(&mut app, SettingsMessage::SetAccentHue(224), modality);
            assert_eq!(app.shell_settings.accent_hue, Some(224), "{modality}");

            let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
            activate(
                &mut app,
                SettingsMessage::SetReduceTransparency(true),
                modality,
            );
            assert!(app.shell_settings.reduce_transparency, "{modality}");

            let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
            activate(
                &mut app,
                SettingsMessage::AppearanceTab(super::AppearanceTab::Theme),
                modality,
            );
            assert_eq!(
                app.appearance_tab,
                super::AppearanceTab::Theme,
                "{modality}"
            );
        }
    }

    #[test]
    fn appearance_sliders_use_resolved_hit_geometry_and_typed_values() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.ui = app.build_ui(1424.0, 1800.0);
        app.ui.reconcile_state(&mut app.ui_state);
        for (suffix, expected) in [("appearance-hue", 90_u16), ("appearance-intensity", 25_u16)] {
            let node = app
                .ui
                .resolved_layout()
                .nodes()
                .iter()
                .find(|node| node.component == "Slider" && node.id.as_str().contains(suffix))
                .unwrap();
            let point = UiPoint {
                x: node.allocated.origin.x + node.allocated.size.width * 0.25,
                y: node.allocated.origin.y + node.allocated.size.height / 2.0,
            };
            app.dispatch_ui_event(UiEvent::PointerPressed(point));
            app.dispatch_ui_event(UiEvent::PointerReleased(point));
            if suffix == "appearance-hue" {
                assert!(
                    app.shell_settings
                        .accent_hue
                        .is_some_and(|value| value.abs_diff(expected) <= 1)
                );
            } else {
                assert_eq!(app.shell_settings.accent_intensity, Some(expected as u8));
            }
            app.ui = app.build_ui(1424.0, 1800.0);
            app.ui.reconcile_state(&mut app.ui_state);
        }
    }

    #[test]
    fn sidebar_search_disambiguates_controls_and_focuses_the_selected_destination() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.sidebar_query = "automatic".into();
        let tree = app.build_ui(850.0, 580.0);

        assert_eq!(app.page, SettingsPage::Appearance);
        let message = SettingsMessage::NavigateTarget(
            SettingsPage::Appearance,
            "appearance-mode-system".into(),
        );
        assert!(tree.message_rect(&message).is_some());
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. }
                if text.contains("Automatic") && text.contains("Appearance")
        )));

        app.handle_settings_message(message);
        let destination = app.build_ui(850.0, 900.0);
        destination.reconcile_state(&mut app.ui_state);
        app.apply_pending_focus(&destination);
        assert!(
            app.ui_state
                .focused()
                .is_some_and(|id| id.as_str().ends_with("/appearance-mode-system"))
        );
        assert!(app.sidebar_query.is_empty());
    }

    #[test]
    fn sidebar_search_renders_unavailable_destinations_without_activation() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.sidebar_query = "fonts".into();
        let tree = app.build_ui(850.0, 580.0);
        let unavailable = SettingsMessage::NavigateTarget(
            SettingsPage::Appearance,
            "appearance-tab-fonts".into(),
        );
        assert!(tree.message_rect(&unavailable).is_none());
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.state.as_deref() == Some("unavailable")
                && node
                    .label
                    .as_deref()
                    .is_some_and(|label| label.contains("Fonts"))
        }));
        assert!(tree.commands().iter().any(|command| matches!(
            command,
            PaintCommand::Text { text, .. } if text == "Unavailable"
        )));
    }

    #[test]
    fn navigation_activation_converges_across_keyboard_controller_and_accessibility() {
        for modality in ["keyboard", "controller", "accessibility"] {
            let mut app = SettingsApp::with_initial_page(SettingsPage::Display);
            app.ui = app.build_ui(850.0, 580.0);
            app.ui.reconcile_state(&mut app.ui_state);
            let id = app
                .ui
                .id_for_message(&SettingsMessage::Navigate(SettingsPage::Appearance))
                .unwrap()
                .clone();
            match modality {
                "keyboard" => {
                    app.ui_state.set_focus(Some(id));
                    app.dispatch_ui_event(UiEvent::KeyboardActivate);
                }
                "controller" => {
                    app.ui_state.set_controller_selected(Some(id));
                    app.dispatch_ui_event(UiEvent::ControllerActivate);
                }
                "accessibility" => {
                    app.dispatch_ui_event(UiEvent::AccessibilityActivate(id));
                }
                _ => unreachable!(),
            }
            assert_eq!(app.page, SettingsPage::Appearance, "{modality}");
        }
    }

    #[test]
    fn narrow_settings_navigation_is_reversible_without_losing_location() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        let content = app.build_ui(560.0, 760.0);
        assert!(
            content
                .message_rect(&SettingsMessage::ToggleNavigation)
                .is_some()
        );
        assert!(
            content
                .message_rect(&SettingsMessage::Navigate(SettingsPage::Display))
                .is_none()
        );

        app.handle_settings_message(SettingsMessage::ToggleNavigation);
        let navigation = app.build_ui(560.0, 760.0);
        assert!(
            navigation
                .message_rect(&SettingsMessage::Navigate(SettingsPage::Display))
                .is_some()
        );
        assert_eq!(app.page, SettingsPage::Appearance);

        app.handle_settings_message(SettingsMessage::Navigate(SettingsPage::Display));
        assert!(!app.narrow_navigation);
        assert_eq!(app.page, SettingsPage::Display);
    }

    #[test]
    fn display_cards_follow_the_resolved_canvas_instead_of_legacy_coordinates() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Display);
        app.ui = app.build_ui(1200.0, 760.0);
        app.sync_display_plane();

        let plane = app.display_plane;
        assert!(plane.x > SIDEBAR_WIDTH);
        for display in &app.displays {
            assert!(display.rect.x >= plane.x);
            assert!(display.rect.y >= plane.y);
            assert!(display.rect.x + display.rect.w <= plane.x + plane.w);
            assert!(display.rect.y + display.rect.h <= plane.y + plane.h);
        }
    }
}
