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
use nickel_input::{InputEvent, KeyEdge, LogicalKey, NamedKey, PointerButton, PointerEvent};
use nickel_ui::{
    ActionLegend, ActionLegendEntry, AdapterOutcome, AnyView, Application, Button,
    ButtonPresentation, ChoiceCard, ChoiceCardGroup, ColorSwatch, ComponentBuilderExt, Container,
    ControllerInput, GlobalAction, HostAdapter, HostServices, Image, ImageFit, InputModality,
    Insets, NavigationItem, PageHeader, PreviewTile, ReadingDirection, ResponsiveNavigation,
    ResponsiveNavigationDestination, SelectField, SemanticColors, SemanticControllerAction,
    SemanticRole, SemanticSelector, SemanticTheme, SettingsCard, SettingsNavigation, SettingsRow,
    SettingsSearchEntry, SettingsSearchField, SettingsStatus, SettingsStatusKind, SliderField,
    Surface, SurfaceRole, Switch, TabList, TextAlign, UiHost, UiId, ViewContext, search_settings,
    ui,
};
use sdl3::event::Event;

#[cfg(test)]
use nickel_ui::{
    ControllerAction, InputSource, InteractionIntent, Rect as UiRect, UiEvent, UiFrame,
    UiStateStore,
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
        secondary_accent: palette.complement,
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
    enabled: bool,
}

struct DisplayCard {
    connector: String,
    name: String,
    detail: String,
    logical_width: i32,
    logical_height: i32,
    rect: Rect,
    primary: bool,
    enabled: bool,
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

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SettingsPage {
    Display,
    Bar,
    Appearance,
    Network,
    Bluetooth,
    KeyboardShortcuts,
    About,
}

impl std::fmt::Display for SettingsPage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(match self {
            Self::Display => "display",
            Self::Bar => "bar",
            Self::Appearance => "appearance",
            Self::Network => "network",
            Self::Bluetooth => "bluetooth",
            Self::KeyboardShortcuts => "keyboard-shortcuts",
            Self::About => "about",
        })
    }
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
    ShowNavigation,
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
    SelectDisplay(usize),
    DisplayPrimary,
    DisplayEnabled(bool),
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
    #[cfg(test)]
    fn dispatch_ui_event(&mut self, event: UiEvent) {
        let source = event.input_source();
        let outcome = self
            .ui
            .transition(&mut self.ui_state, source, InteractionIntent::Event(event))
            .expect("event transitions do not perform fallible semantic lookup");
        for message in outcome.messages {
            self.handle_settings_message(message);
        }
        if outcome.invalidation != nickel_ui::Invalidation::None {
            self.request_redraw();
        }
    }

    #[cfg(test)]
    fn dispatch_semantic_action(
        &mut self,
        target: UiId,
        source: InputSource,
        action: nickel_ui::SemanticAction,
    ) {
        let outcome = self
            .ui
            .transition(
                &mut self.ui_state,
                source,
                InteractionIntent::Invoke { target, action },
            )
            .expect("semantic action target must advertise the requested action");
        for message in outcome.messages {
            self.handle_settings_message(message);
        }
        if outcome.invalidation != nickel_ui::Invalidation::None {
            self.request_redraw();
        }
    }

    #[cfg(test)]
    fn transient_scroll(&self, message: &SettingsMessage) -> f32 {
        self.ui
            .unique_semantic_target_for_message(message)
            .ok()
            .and_then(|target| self.ui_state.state(&target.id))
            .map(|state| state.scroll_offset)
            .unwrap_or(0.0)
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

    #[cfg(test)]
    fn apply_pending_focus(&mut self, ui: &UiFrame<SettingsMessage>) {
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
            let _ = ui.transition(
                &mut self.ui_state,
                InputSource::Programmatic,
                InteractionIntent::Event(UiEvent::AccessibilityFocus(id)),
            );
        }
    }

    fn handle_settings_message(&mut self, message: SettingsMessage) {
        match message {
            SettingsMessage::Navigate(page) => {
                self.page = page;
                self.active_destination = Some(page);
                match page {
                    SettingsPage::Network => self.load_linux_network(),
                    SettingsPage::Bluetooth => self.load_bluetooth(),
                    _ => {}
                }
            }
            SettingsMessage::NavigateTarget(page, target) => {
                self.page = page;
                self.active_destination = Some(page);
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
            SettingsMessage::ShowNavigation => self.active_destination = None,
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
                        self.wallpaper_poll_delay = Duration::from_millis(16);
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
            SettingsMessage::SelectDisplay(index) => {
                if index < self.displays.len() {
                    self.selected = index;
                }
            }
            SettingsMessage::DisplayPrimary => {
                self.displays[self.selected].enabled = true;
                for (index, display) in self.displays.iter_mut().enumerate() {
                    display.primary = index == self.selected;
                }
                self.applied = false;
                self.status = self.localizer.text("settings-status-changes-not-applied");
            }
            SettingsMessage::DisplayEnabled(enabled) => {
                if !enabled
                    && self
                        .displays
                        .iter()
                        .filter(|display| display.enabled)
                        .count()
                        <= 1
                {
                    self.status = "At least one display must remain enabled.".into();
                } else {
                    self.displays[self.selected].enabled = enabled;
                    if !enabled && self.displays[self.selected].primary {
                        self.displays[self.selected].primary = false;
                        if let Some(display) =
                            self.displays.iter_mut().find(|display| display.enabled)
                        {
                            display.primary = true;
                        }
                    }
                    self.applied = false;
                    self.status = self.localizer.text("settings-status-changes-not-applied");
                }
            }
            SettingsMessage::DisplayApply => self.apply_layout(),
            SettingsMessage::WifiNetwork(index) => self.connect_windows_wifi(index),
            SettingsMessage::BluetoothScroll
            | SettingsMessage::NetworkScroll
            | SettingsMessage::AppearanceScroll => {}
        }
        self.request_redraw();
    }

    fn begin_display_drag(&mut self, index: usize, x: i32, y: i32) {
        if self.page != SettingsPage::Display || index >= self.displays.len() {
            return;
        }
        self.selected = index;
        let rect = self.displays[index].rect;
        self.drag_offset = Some((x - rect.x, y - rect.y));
        self.drag_origin = Some(rect);
    }

    fn move_display_drag(&mut self, x: i32, y: i32) {
        self.cursor = (x, y);
        let Some((offset_x, offset_y)) = self.drag_offset else {
            return;
        };
        let mut rect = self.displays[self.selected].rect;
        rect.x = x - offset_x;
        rect.y = y - offset_y;
        rect = constrain_center(rect, self.display_plane);
        rect = self
            .displays
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.selected)
            .fold(rect, |moving, (_, other)| snap_rect(moving, other.rect, 42));
        self.displays[self.selected].rect = rect;
        if self.drag_origin != Some(rect) {
            self.applied = false;
            self.status = self.localizer.text("settings-status-changes-not-applied");
        }
        self.request_redraw();
    }

    fn finish_drag(&mut self) {
        if self.page != SettingsPage::Display {
            return;
        }
        if self.drag_offset.take().is_none() {
            return;
        }
        let Some(origin) = self.drag_origin.take() else {
            return;
        };
        let selected = self.displays[self.selected].rect;
        if selected == origin {
            self.request_redraw();
            return;
        }
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
        if snapped != origin {
            self.applied = false;
            self.status = self.localizer.text("settings-status-changes-not-applied");
        }
        self.request_redraw();
    }

    #[cfg(test)]
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

    #[cfg(test)]
    fn scroll_page(&mut self, message: SettingsMessage, delta: f32) {
        let Ok(target) = self.ui.unique_semantic_target_for_message(&message) else {
            return;
        };
        let maximum = self
            .ui
            .scroll_extent(&message)
            .map(|extent| (extent.content.height - extent.viewport.height).max(0.0))
            .unwrap_or(0.0);
        if self.ui_state.scroll_by(target.id, delta, maximum) != nickel_ui::Invalidation::None {
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
        self.appearance_save_deadline = Some(Instant::now() + Duration::from_millis(16));
    }

    fn set_appearance_intensity(&mut self, intensity: u8) {
        if self.shell_settings.accent_intensity == Some(intensity) {
            return;
        }
        self.shell_settings.accent_intensity = Some(intensity);
        self.appearance_save_deadline = Some(Instant::now() + Duration::from_millis(16));
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

    #[cfg(test)]
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

impl Application for SettingsApp {
    type Message = SettingsMessage;

    fn update(&mut self, message: Self::Message) {
        self.handle_settings_message(message);
    }

    fn view(&self, context: ViewContext) -> impl nickel_ui::View<Self::Message> {
        self.settings_view(
            context.viewport.size.width,
            context.viewport.size.height,
            context.modality,
        )
    }

    fn poll(&mut self) -> bool {
        self.tick();
        self.redraw_requested.replace(false)
    }

    fn poll_interval(&self) -> Option<std::time::Duration> {
        let now = Instant::now();
        let mut deadlines = Vec::with_capacity(4);
        deadlines.extend(self.appearance_save_deadline);
        deadlines.extend(self.next_wifi_refresh);
        if self.page == SettingsPage::Bluetooth {
            deadlines.push(self.next_bluetooth_refresh);
        }
        if self.page == SettingsPage::Network {
            deadlines.push(self.next_network_refresh);
        }
        if self.wallpaper_dialog_rx.is_some() {
            deadlines.push(now + self.wallpaper_poll_delay);
        }
        deadlines
            .into_iter()
            .min()
            .map(|deadline| deadline.saturating_duration_since(now))
    }

    fn title(&self) -> &str {
        "Nickel Settings"
    }

    fn initial_size(&self) -> (u32, u32) {
        (850, 580)
    }
}

struct SettingsHostAdapter {
    input: nickel_input::sdl::Adapter,
    sync_requested: bool,
}

impl Default for SettingsHostAdapter {
    fn default() -> Self {
        Self {
            input: nickel_input::sdl::Adapter::default(),
            sync_requested: true,
        }
    }
}

impl SettingsHostAdapter {
    fn sync_display_plane(host: &mut UiHost<SettingsApp>) {
        let Ok(node) = host.query_unique(&SemanticSelector::Name("Display arrangement".into()))
        else {
            return;
        };
        let resolved = Rect {
            x: node.bounds.origin.x.round() as i32,
            y: node.bounds.origin.y.round() as i32,
            w: node.bounds.size.width.round() as i32,
            h: node.bounds.size.height.round() as i32,
        };
        let app = host.application_mut();
        if resolved.w > 0 && resolved.h > 0 && resolved != app.display_plane {
            app.display_plane = resolved;
            center_display_rects(&mut app.displays, resolved);
        }
    }

    fn apply_pending_focus(host: &mut UiHost<SettingsApp>) {
        let pending = host.application_mut().pending_focus.take();
        let Some(target) = pending else { return };
        let suffix = format!("/{}", target.as_str());
        if let Some(id) = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id == target || node.id.as_str().ends_with(&suffix))
            .map(|node| node.id)
        {
            host.request_focus(id);
        }
    }
}

impl HostAdapter<SettingsApp> for SettingsHostAdapter {
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.sync_requested.then_some(now)
    }

    fn started(
        &mut self,
        host: &mut UiHost<SettingsApp>,
        mut services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        services.window().set_minimum_size(850, 580)?;
        let app = host.application_mut();
        app.load_outputs();
        app.load_bluetooth();
        app.load_linux_network();
        #[cfg(target_os = "windows")]
        {
            app.load_windows_outputs(services.video());
            app.load_windows_network();
            app.load_windows_wifi();
        }
        Ok(AdapterOutcome::changed())
    }

    fn event(
        &mut self,
        host: &mut UiHost<SettingsApp>,
        event: &Event,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        let Some(input) = self.input.normalize(event) else {
            return Ok(AdapterOutcome::default());
        };
        self.sync_requested = true;
        match input {
            InputEvent::Key(ref key)
                if key.edge == KeyEdge::Pressed
                    && key.logical == LogicalKey::Named(NamedKey::Escape)
                    && host.inspect().keyboard_focus.is_none() =>
            {
                return Ok(AdapterOutcome::exit());
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Primary,
                edge: KeyEdge::Pressed,
                position: Some(position),
                ..
            }) => {
                let count = host.application_mut().displays.len();
                for index in 0..count {
                    let Ok(target) = host
                        .unique_semantic_target_for_message(&SettingsMessage::SelectDisplay(index))
                    else {
                        continue;
                    };
                    let bounds = target.bounds;
                    let x = position.x as f32;
                    let y = position.y as f32;
                    if x >= bounds.origin.x
                        && y >= bounds.origin.y
                        && x < bounds.origin.x + bounds.size.width
                        && y < bounds.origin.y + bounds.size.height
                    {
                        host.application_mut().begin_display_drag(
                            index,
                            position.x.round() as i32,
                            position.y.round() as i32,
                        );
                        break;
                    }
                }
            }
            InputEvent::Pointer(PointerEvent::Motion { position, .. }) => {
                host.application_mut()
                    .move_display_drag(position.x.round() as i32, position.y.round() as i32);
            }
            InputEvent::Pointer(PointerEvent::Button {
                button: PointerButton::Primary,
                edge: KeyEdge::Released,
                ..
            }) => host.application_mut().finish_drag(),
            _ => {}
        }
        Ok(AdapterOutcome::default())
    }

    fn poll(
        &mut self,
        host: &mut UiHost<SettingsApp>,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        self.sync_requested = false;
        Self::sync_display_plane(host);
        Self::apply_pending_focus(host);
        Ok(AdapterOutcome::default())
    }

    fn global_action(
        &mut self,
        _host: &mut UiHost<SettingsApp>,
        action: GlobalAction,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        match action {
            GlobalAction::ToggleLauncher => {
                session_request(SessionRequest::Command(SessionCommand::ToggleLauncher))?;
            }
        }
        Ok(AdapterOutcome::default())
    }
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
    let _log_path = nickel_logging::init("nickel-settings").ok();
    sdl3::hint::set("SDL_APP_ID", "nickel-settings");
    nickel_ui::run_with_adapter(
        SettingsApp::with_initial_page(initial_page),
        SettingsHostAdapter::default(),
    )
}

#[cfg(test)]
mod tests {
    use nickel_input::{
        DeviceId, EventOrder, InputEvent, KeyCode, KeyEdge, KeyEvent, KeyLocation, LogicalKey,
        ModifierState, NamedKey, PhysicalKey, Point, PointerButton, PointerEvent,
    };
    use nickel_ui::Application;

    use super::{
        BluetoothDevice, ControllerAction, NetworkAdapter, Rect, SIDEBAR_WIDTH, SettingsApp,
        SettingsMessage, SettingsPage, ThemePreference, UiEvent, UiHost, WallpaperSettings,
        attach_rect_centered, constrain_center, snap_rect,
    };

    #[test]
    fn idle_settings_host_declares_no_poll_deadline() {
        let app = SettingsApp::with_initial_page(SettingsPage::Display);
        assert_eq!(Application::poll_interval(&app), None);
    }

    #[test]
    fn pending_wallpaper_dialog_uses_bounded_backoff() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Display);
        let (_sender, receiver) = std::sync::mpsc::channel();
        app.wallpaper_dialog_rx = Some(receiver);
        for _ in 0..8 {
            app.poll_wallpaper_dialog();
        }
        assert_eq!(
            app.wallpaper_poll_delay,
            std::time::Duration::from_millis(250)
        );
        assert!(Application::poll_interval(&app).is_some());
    }

    fn enter_event() -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(1),
            physical: PhysicalKey::Code(KeyCode::Enter),
            logical: LogicalKey::Named(NamedKey::Enter),
            location: KeyLocation::Standard,
            edge: KeyEdge::Pressed,
            repeat: false,
            modifiers: ModifierState::default(),
        })
    }

    fn primary_event(order: u64, edge: KeyEdge, x: f64, y: f64) -> InputEvent {
        InputEvent::Pointer(PointerEvent::Button {
            device: DeviceId(2),
            order: EventOrder(order),
            button: PointerButton::Primary,
            edge,
            position: Some(Point { x, y }),
        })
    }

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
    fn display_disable_keeps_one_active_output_and_transfers_primary() {
        let mut app = SettingsApp {
            selected: 1,
            ..SettingsApp::default()
        };

        app.handle_settings_message(SettingsMessage::DisplayEnabled(false));

        assert!(!app.displays[1].enabled);
        assert!(app.displays[0].enabled);
        assert!(app.displays[0].primary);

        app.selected = 0;
        app.handle_settings_message(SettingsMessage::DisplayEnabled(false));
        assert!(app.displays[0].enabled);
        assert_eq!(
            app.displays
                .iter()
                .filter(|display| display.enabled)
                .count(),
            1
        );
    }

    #[test]
    fn selecting_display_without_moving_it_keeps_layout_clean() {
        let mut app = SettingsApp {
            applied: true,
            ..SettingsApp::default()
        };
        let display = app.displays[0].rect;
        app.begin_display_drag(0, display.x + 20, display.y + 20);
        app.finish_drag();

        assert_eq!(app.selected, 0);
        assert_eq!(app.displays[0].rect, display);
        assert!(app.applied);
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
                !expanded.semantic_targets_for_message(&message).is_empty(),
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
                assert!(
                    tree.accessibility_nodes()
                        .iter()
                        .any(|node| { node.label.as_deref() == Some("المظهر") })
                );
            }
        }
    }

    #[test]
    fn appearance_activation_converges_across_supported_modalities() {
        fn activate(
            app: SettingsApp,
            message: SettingsMessage,
            via: nickel_ui_testkit::ActivationVia,
        ) -> nickel_ui_testkit::Scenario<SettingsApp> {
            let mut scenario = nickel_ui_testkit::Scenario::new(app, 1424, 1800);
            let target = scenario
                .host()
                .semantic_targets_for_message(&message)
                .into_iter()
                .next()
                .expect("settings action has a semantic identity");
            let selector = nickel_ui_testkit::Selector::id(target.id);
            if via == nickel_ui_testkit::ActivationVia::Controller {
                scenario
                    .controller_semantic_action(&selector, nickel_ui::ActionKind::Activate)
                    .expect("controller adapter semantic route");
            } else {
                scenario
                    .activate_via(via, &selector)
                    .expect("settings action is reachable through production host routing");
            }
            scenario
        }

        for via in [
            nickel_ui_testkit::ActivationVia::Pointer,
            nickel_ui_testkit::ActivationVia::Keyboard,
            nickel_ui_testkit::ActivationVia::Controller,
            nickel_ui_testkit::ActivationVia::Accessibility,
        ] {
            let mut scenario = activate(
                SettingsApp::with_initial_page(SettingsPage::Appearance),
                SettingsMessage::AppearanceDark,
                via,
            );
            assert_eq!(
                scenario.host_mut().application_mut().shell_settings.theme,
                ThemePreference::Dark,
                "{via:?}"
            );

            let mut scenario = activate(
                SettingsApp::with_initial_page(SettingsPage::Appearance),
                SettingsMessage::SetAccentHue(224),
                via,
            );
            assert_eq!(
                scenario
                    .host_mut()
                    .application_mut()
                    .shell_settings
                    .accent_hue,
                Some(224),
                "{via:?}"
            );

            let mut scenario = activate(
                SettingsApp::with_initial_page(SettingsPage::Appearance),
                SettingsMessage::SetReduceTransparency(true),
                via,
            );
            assert!(
                scenario
                    .host_mut()
                    .application_mut()
                    .shell_settings
                    .reduce_transparency,
                "{via:?}"
            );

            let mut scenario = activate(
                SettingsApp::with_initial_page(SettingsPage::Appearance),
                SettingsMessage::AppearanceTab(super::AppearanceTab::Theme),
                via,
            );
            assert_eq!(
                scenario.host_mut().application_mut().appearance_tab,
                super::AppearanceTab::Theme,
                "{via:?}"
            );
        }
    }

    #[test]
    fn appearance_sliders_use_host_semantic_values() {
        let app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        let labels = [
            app.localizer.text("settings-appearance-starting-hue"),
            app.localizer.text("settings-appearance-color-intensity"),
        ];
        let mut host = UiHost::new(app, 1424, 1800);
        for (label, expected) in labels.into_iter().zip([90_u16, 25_u16]) {
            let target = host
                .query_unique(&nickel_ui::SemanticSelector::RoleAndName {
                    role: nickel_ui::SemanticRole::Slider,
                    name: label,
                })
                .unwrap()
                .id;
            host.perform_semantic_action(
                target,
                nickel_ui::SemanticAction::SetValue(nickel_ui::SemanticValueInput::Number(0.25)),
            );
            if expected == 90 {
                assert!(
                    host.application()
                        .shell_settings
                        .accent_hue
                        .is_some_and(|value| value.abs_diff(expected) <= 1)
                );
            } else {
                assert_eq!(
                    host.application().shell_settings.accent_intensity,
                    Some(expected as u8)
                );
            }
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
        assert!(!tree.semantic_targets_for_message(&message).is_empty());
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.label
                .as_deref()
                .is_some_and(|label| label.contains("Automatic") && label.contains("Appearance"))
        }));

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
        assert!(tree.semantic_targets_for_message(&unavailable).is_empty());
        assert!(tree.accessibility_nodes().iter().any(|node| {
            node.state.as_deref() == Some("unavailable")
                && node
                    .label
                    .as_deref()
                    .is_some_and(|label| label.contains("Fonts"))
        }));
    }

    #[test]
    fn navigation_activation_converges_across_keyboard_controller_and_accessibility() {
        for modality in ["keyboard", "controller", "accessibility"] {
            let mut app = SettingsApp::with_initial_page(SettingsPage::Display);
            app.ui = app.build_ui(850.0, 580.0);
            app.ui.reconcile_state(&mut app.ui_state);
            let id = app
                .ui
                .unique_semantic_target_for_message(&SettingsMessage::Navigate(
                    SettingsPage::Appearance,
                ))
                .unwrap()
                .id;
            match modality {
                "keyboard" => {
                    app.dispatch_semantic_action(
                        id,
                        nickel_ui::InputSource::Keyboard,
                        nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                    );
                }
                "controller" => {
                    app.dispatch_semantic_action(
                        id,
                        nickel_ui::InputSource::Controller,
                        nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                    );
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
    fn shoulder_buttons_switch_controller_navigation_panes_and_paint_content_selection() {
        let mut host = UiHost::new(
            SettingsApp::with_initial_page(SettingsPage::Appearance),
            850,
            580,
        );
        host.handle_controller_action(ControllerAction::PreviousPane);
        host.handle_controller_action(ControllerAction::Down);
        host.handle_controller_action(ControllerAction::NextPane);

        let selected = host.inspect().controller_target.unwrap();
        let selected_node = host
            .semantic_nodes()
            .into_iter()
            .find(|node| node.id == selected)
            .unwrap();
        assert_ne!(
            selected_node.role,
            Some(nickel_ui::SemanticRole::NavigationItem)
        );
        assert_eq!(
            host.inspect().modality,
            nickel_ui::InputModality::Controller
        );
        assert!(selected_node.controller_selected);
    }

    #[test]
    fn sidebar_semantic_activation_selects_destination() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        let mut state = std::mem::take(&mut app.ui_state);
        app.ui = app.build_ui_with_state(850.0, 580.0, &mut state);
        app.ui_state = state;
        let destination = app
            .ui
            .unique_semantic_target_for_message(&SettingsMessage::Navigate(SettingsPage::Display))
            .expect("display destination semantics")
            .id;
        app.dispatch_ui_event(UiEvent::AccessibilityActivate(destination));
        assert_eq!(app.page, SettingsPage::Display);
    }

    #[test]
    fn controller_reaches_offscreen_appearance_sliders_and_adjusts_them() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.accent_intensity = Some(50);
        let mut state = std::mem::take(&mut app.ui_state);
        app.ui = app.build_ui_with_state(850.0, 580.0, &mut state);
        app.ui_state = state;
        let slider = app
            .ui
            .resolved_layout()
            .nodes()
            .iter()
            .find(|node| {
                node.id.as_str().contains("/appearance-intensity/")
                    && node.controller_value.is_some()
            })
            .unwrap()
            .id
            .clone();
        app.dispatch_semantic_action(
            slider,
            nickel_ui::InputSource::Controller,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Increment),
        );
        assert_eq!(app.shell_settings.accent_intensity, Some(55));
        assert_eq!(
            app.ui_state.input_modality(),
            nickel_ui::InputModality::Controller
        );
    }

    #[test]
    fn normalized_keyboard_and_pointer_reach_production_navigation_once() {
        let destination = SettingsMessage::Navigate(SettingsPage::Appearance);

        let mut keyboard = UiHost::new(
            SettingsApp::with_initial_page(SettingsPage::Display),
            850,
            580,
        );
        let id = keyboard
            .unique_semantic_target_for_message(&destination)
            .unwrap()
            .id;
        keyboard.request_focus(id);
        keyboard.handle_input(&enter_event(), None);
        assert_eq!(keyboard.application_mut().page, SettingsPage::Appearance);

        let mut pointer = UiHost::new(
            SettingsApp::with_initial_page(SettingsPage::Display),
            850,
            580,
        );
        let target = pointer
            .unique_semantic_target_for_message(&destination)
            .unwrap();
        let bounds = target.bounds;
        let x = f64::from(bounds.origin.x + bounds.size.width / 2.0);
        let y = f64::from(bounds.origin.y + bounds.size.height / 2.0);
        pointer.handle_input(&primary_event(1, KeyEdge::Pressed, x, y), None);
        pointer.handle_input(&primary_event(2, KeyEdge::Released, x, y), None);
        assert_eq!(pointer.application_mut().page, SettingsPage::Appearance);
    }

    #[test]
    fn narrow_settings_navigation_is_reversible_without_losing_location() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        let content = app.build_ui(560.0, 760.0);
        assert!(
            !content
                .semantic_targets_for_message(&SettingsMessage::ShowNavigation)
                .is_empty()
        );
        assert!(
            content
                .semantic_targets_for_message(&SettingsMessage::Navigate(SettingsPage::Display))
                .is_empty()
        );

        app.handle_settings_message(SettingsMessage::ShowNavigation);
        let navigation = app.build_ui(560.0, 760.0);
        assert!(
            !navigation
                .semantic_targets_for_message(&SettingsMessage::Navigate(SettingsPage::Display))
                .is_empty()
        );
        assert_eq!(app.page, SettingsPage::Appearance);

        app.handle_settings_message(SettingsMessage::Navigate(SettingsPage::Display));
        assert_eq!(app.active_destination, Some(SettingsPage::Display));
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

    #[test]
    fn display_card_selection_uses_the_declarative_semantic_route() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Display);
        app.ui = app.build_ui(1200.0, 760.0);
        app.ui.reconcile_state(&mut app.ui_state);
        let second = app
            .ui
            .unique_semantic_target_for_message(&SettingsMessage::SelectDisplay(1))
            .expect("second display card exposes an activation action")
            .id;

        app.dispatch_ui_event(UiEvent::AccessibilityActivate(second));

        assert_eq!(app.selected, 1);
    }
}
