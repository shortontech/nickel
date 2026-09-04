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
use persistence::{
    load_shell_settings, try_save_shell_settings, try_save_wallpaper_settings,
    try_update_optional_feature_settings,
};
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
    optional_features::{
        ApplyRequirement, CodexSource, FeatureCapability, FeatureEffectiveState, FeatureHealth,
        FeatureInstallation, FeaturePolicy, FeatureState, FeatureSupport, OptionalFeatureRuntime,
        OptionalFeatureSettings, codex_policy,
    },
    shell_settings::{AnimationLevel, FileIconPreference, ShellSettings, ThemePreference},
    theme::{Appearance, ThemeMode, ThemePalette, accent_from_hue},
    wallpaper_settings::{WallpaperPosition, WallpaperSettings},
};
use nickel_i18n::Localizer;
use nickel_input::{DeviceId, InputEvent, KeyEdge, LogicalKey, NamedKey};
use nickel_ui::{
    ActionLegend, ActionLegendEntry, AdapterOutcome, AnyView, Application, Button,
    ButtonPresentation, ChoiceCard, ChoiceCardGroup, ColorSwatch, DragGesture, DragPhase,
    GlobalAction, HostAdapter, HostServices, Image, ImageFit, InputModality, Insets,
    NavigationItem, PageHeader, PreviewTile, ReadingDirection, ResponsiveNavigation,
    ResponsiveNavigationDestination, SelectField, SemanticControllerAction, SemanticRole,
    SemanticSelector, SemanticTheme, SettingsCard, SettingsNavigation, SettingsRow,
    SettingsSearchEntry, SettingsSearchField, SettingsStatus, SettingsStatusKind, SliderField,
    Surface, SurfaceRole, Switch, SwitchState, TextAlign, UiHost, UiId, ViewContext,
    search_settings, ui,
};
use winit::{dpi::LogicalSize, event::WindowEvent};

#[cfg(test)]
use nickel_ui::{ControllerAction, Rect as UiRect, UiFrame};

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
    SemanticTheme::from_tokens(nickel_ui::SemanticTokenSet::standard(
        palette.background,
        palette.panel,
        palette.surface,
        palette.surface_hover,
        palette.surface_hover,
        palette.text,
        palette.muted,
        palette.accent,
        palette.accent_soft,
        palette.complement,
        palette.complement,
    ))
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

fn codex_feature_state(
    preference: &OptionalFeatureSettings,
    runtime: &OptionalFeatureRuntime,
) -> FeatureState {
    let (policy, policy_source) = codex_policy();
    FeatureState::resolve(
        preference.codex_enabled,
        preference.codex_generation,
        runtime.codex_generation,
        FeatureCapability {
            support: runtime.codex_support,
            installation: runtime.codex_installation,
            health: runtime.codex_health,
            policy,
            policy_source,
            required_permissions: Vec::new(),
            configuration_destination: Some("optional-features/codex".into()),
            apply_requirement: ApplyRequirement::Live,
            source_label: runtime.source_label.clone(),
            diagnostic: runtime.diagnostic.clone(),
        },
    )
}

fn probe_codex_capability(settings: &OptionalFeatureSettings) -> FeatureCapability {
    let (policy, policy_source) = codex_policy();
    let (installation, health, source_label, diagnostic) = match &settings.codex_source {
        CodexSource::ApprovedRemote => match nickel_codex::CodexSettings::load_default() {
            Ok(hosts) if hosts.selected_host().is_some() => (
                FeatureInstallation::Installed,
                FeatureHealth::Unknown,
                hosts
                    .selected_host()
                    .map(|host| host.name.clone())
                    .unwrap_or_default(),
                None,
            ),
            Ok(_) => (
                FeatureInstallation::Missing,
                FeatureHealth::Failed,
                "Approved remote host".into(),
                Some("No remote Codex host is selected".into()),
            ),
            Err(error) => (
                FeatureInstallation::Missing,
                FeatureHealth::Failed,
                "Approved remote host".into(),
                Some(error.to_string()),
            ),
        },
        source => {
            let choice = match source {
                CodexSource::CompatibleInstalled => nickel_codex::BackendChoice::Installed,
                CodexSource::Bundled => nickel_codex::BackendChoice::Bundled,
                CodexSource::Executable(path) => nickel_codex::BackendChoice::Path(path.clone()),
                CodexSource::ApprovedRemote => unreachable!(),
            };
            let selection = nickel_codex::Selector::platform_default().select(choice);
            if let Some(selected) = selection.selected {
                (
                    FeatureInstallation::Installed,
                    FeatureHealth::Unknown,
                    selected.path.display().to_string(),
                    None,
                )
            } else {
                let incompatible = selection.probes.iter().any(|probe| !probe.compatible);
                let diagnostic = selection
                    .probes
                    .last()
                    .map(|probe| probe.reason.clone())
                    .or_else(|| Some("Configured Codex source was not found".into()));
                (
                    if incompatible {
                        FeatureInstallation::Incompatible
                    } else {
                        FeatureInstallation::Missing
                    },
                    FeatureHealth::Failed,
                    format!("{:?}", source),
                    diagnostic,
                )
            }
        }
    };
    let required_permissions = match &settings.codex_source {
        CodexSource::ApprovedRemote => vec!["Network access to the configured Codex host".into()],
        _ => vec!["Permission to start the configured Codex app-server".into()],
    };
    FeatureCapability {
        support: FeatureSupport::Supported,
        installation,
        health,
        policy,
        policy_source,
        required_permissions,
        configuration_destination: Some("optional-features/codex".into()),
        apply_requirement: ApplyRequirement::Live,
        source_label,
        diagnostic,
    }
}

#[derive(Clone, Copy)]
enum SidebarIconKind {
    Search,
    Display,
    Bar,
    Appearance,
    Network,
    Bluetooth,
    DefaultApps,
    OptionalFeatures,
    Keyboard,
    About,
}

impl SidebarIconKind {
    const ALL: [Self; 10] = [
        Self::Search,
        Self::Display,
        Self::Bar,
        Self::Appearance,
        Self::Network,
        Self::Bluetooth,
        Self::DefaultApps,
        Self::OptionalFeatures,
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
            Self::DefaultApps => '\u{f2d0}',
            Self::OptionalFeatures => '\u{f12e}',
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
            Self::DefaultApps => include_bytes!("../../../assets/icons/start-menu/about.svg"),
            Self::OptionalFeatures => include_bytes!("../../../assets/icons/start-menu/about.svg"),
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
    static ICONS: OnceLock<[Arc<image::RgbaImage>; 10]> = OnceLock::new();
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum BluetoothOperation {
    SetPower(bool),
    SetDiscovery(bool),
    ToggleDevice(String),
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
enum SettingsPage {
    Display,
    Bar,
    Appearance,
    Network,
    Bluetooth,
    DefaultApps,
    OptionalFeatures,
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
            Self::DefaultApps => "default-apps",
            Self::OptionalFeatures => "optional-features",
            Self::KeyboardShortcuts => "keyboard-shortcuts",
            Self::About => "about",
        })
    }
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct DefaultAppRow {
    label: String,
    target: nickel_platform::AssociationTarget,
    snapshot: Option<nickel_platform::AssociationSnapshot>,
    status: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingsMessage {
    Navigate(SettingsPage),
    NavigateTarget(SettingsPage, String),
    ShowNavigation,
    SidebarSearchChanged(String),
    SetBluetoothPower(bool),
    BluetoothDiscovery,
    BluetoothDevice(usize),
    BluetoothScroll,
    SetWifiPower(bool),
    WifiNetwork(usize),
    NetworkScroll,
    DefaultAppsScroll,
    DefaultAppTargetChanged(String),
    AddDefaultAppTarget,
    ToggleDefaultAppSelect(usize),
    RequestDefaultAppConsent(usize),
    SetPreferredTerminal(String),
    SetPreferredFileManager(String),
    SetDefaultApp {
        row: usize,
        handler_id: String,
    },
    SetCodexEnabled(bool),
    ConfirmDisableCodex,
    CancelDisableCodex,
    ToggleCodexSourceSelect,
    SetCodexSource(CodexSource),
    CodexExecutablePathChanged(String),
    ApplyCodexExecutable,
    RetryCodexProbe,
    AppearanceLight,
    AppearanceDark,
    AppearanceSystem,
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
    ToggleFileIconProviderSelect,
    SetFileIconProvider(FileIconPreference),
    SetFileIconTheme(String),
    AppearanceScroll,
    BarPrimaryDisplay,
    BarAllDisplays,
    BarDisplayWindows,
    BarAllWindows,
    SetDesktopCount(u8),
    DisplayIdentify,
    SelectDisplay(usize),
    DisplayDrag {
        index: usize,
        phase: DragPhase,
        x: i32,
        y: i32,
    },
    DisplayPrimary,
    DisplayEnabled(bool),
    DisplayApply,
}

fn display_drag_message(seed: SettingsMessage, gesture: DragGesture) -> SettingsMessage {
    let SettingsMessage::SelectDisplay(index) = seed else {
        unreachable!("display drag targets use SelectDisplay as their typed seed")
    };
    SettingsMessage::DisplayDrag {
        index,
        phase: gesture.phase,
        x: gesture.position.x.round() as i32,
        y: gesture.position.y.round() as i32,
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingsEffect {
    FocusControl(String),
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

fn wifi_power_message(value: bool) -> SettingsMessage {
    SettingsMessage::SetWifiPower(value)
}

fn bluetooth_power_message(value: bool) -> SettingsMessage {
    SettingsMessage::SetBluetoothPower(value)
}

fn sidebar_search_message(value: String) -> SettingsMessage {
    SettingsMessage::SidebarSearchChanged(value)
}

fn default_app_categories() -> Vec<DefaultAppRow> {
    [
        (
            "Web browser (HTTPS scheme)",
            nickel_platform::AssociationTarget::scheme("https"),
        ),
        (
            "Mail (mailto scheme)",
            nickel_platform::AssociationTarget::scheme("mailto"),
        ),
        (
            "Text editor (text/plain)",
            nickel_platform::AssociationTarget::mime("text/plain"),
        ),
        (
            "Image viewer (image/png)",
            nickel_platform::AssociationTarget::mime("image/png"),
        ),
        (
            "Audio player (audio/mpeg)",
            nickel_platform::AssociationTarget::mime("audio/mpeg"),
        ),
        (
            "Video player (video/mp4)",
            nickel_platform::AssociationTarget::mime("video/mp4"),
        ),
        (
            "PDF viewer (application/pdf)",
            nickel_platform::AssociationTarget::mime("application/pdf"),
        ),
    ]
    .into_iter()
    .map(|(label, target)| DefaultAppRow {
        label: label.into(),
        target,
        snapshot: None,
        status: None,
    })
    .collect()
}

impl SettingsApp {
    fn load_default_apps(&mut self) {
        let service = nickel_platform::association_service();
        for row in &mut self.default_apps {
            match service.inspect(&row.target) {
                Ok(snapshot) => {
                    row.snapshot = Some(snapshot);
                    row.status = None;
                }
                Err(error) => {
                    row.status = Some(error.to_string());
                }
            }
        }
        self.next_default_apps_refresh = Instant::now() + Duration::from_secs(2);
    }

    fn change_default_app(&mut self, row_index: usize, handler_id: &str) {
        let Some(row) = self.default_apps.get_mut(row_index) else {
            return;
        };
        let previous = row.snapshot.clone();
        let service = nickel_platform::association_service();
        match service.request_change(&row.target, handler_id) {
            Ok(nickel_platform::ChangeOutcome::Confirmed(snapshot)) => {
                row.snapshot = Some(snapshot);
                row.status = Some("Default confirmed by the operating system".into());
            }
            Ok(nickel_platform::ChangeOutcome::NativeConsentRequired { detail })
            | Ok(nickel_platform::ChangeOutcome::Rejected { detail }) => {
                row.snapshot = previous;
                row.status = Some(detail);
            }
            Err(error) => {
                row.snapshot = previous;
                row.status = Some(error.to_string());
            }
        }
        self.default_app_select_expanded = None;
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

    fn handle_settings_message(&mut self, message: SettingsMessage) {
        match message {
            SettingsMessage::Navigate(page) => {
                self.page = page;
                self.active_destination = Some(page);
                match page {
                    SettingsPage::Network => self.load_linux_network(),
                    SettingsPage::Bluetooth => self.load_bluetooth(),
                    SettingsPage::DefaultApps => self.load_default_apps(),
                    SettingsPage::Bar => self.refresh_workspace_state(),
                    SettingsPage::OptionalFeatures => {
                        self.refresh_optional_feature_state();
                        self.start_codex_probe();
                    }
                    _ => {}
                }
            }
            SettingsMessage::NavigateTarget(page, target) => {
                self.page = page;
                self.active_destination = Some(page);
                self.sidebar_query.clear();
                self.pending_effects
                    .push(SettingsEffect::FocusControl(target));
                match page {
                    SettingsPage::Network => self.load_linux_network(),
                    SettingsPage::Bluetooth => self.load_bluetooth(),
                    SettingsPage::DefaultApps => self.load_default_apps(),
                    SettingsPage::Bar => self.refresh_workspace_state(),
                    SettingsPage::OptionalFeatures => {
                        self.refresh_optional_feature_state();
                        self.start_codex_probe();
                    }
                    _ => {}
                }
            }
            SettingsMessage::SidebarSearchChanged(value) => self.sidebar_query = value,
            SettingsMessage::ShowNavigation => self.active_destination = None,
            SettingsMessage::SetBluetoothPower(enabled) => {
                if !self.bluetooth.available
                    || self.bluetooth_operation_rx.is_some()
                    || enabled == self.bluetooth.powered
                {
                    return;
                }
                self.start_bluetooth_operation(BluetoothOperation::SetPower(enabled), move || {
                    set_bluetooth_adapter_property("Powered", enabled)
                });
            }
            SettingsMessage::BluetoothDiscovery => {
                if !self.bluetooth.available
                    || !self.bluetooth.powered
                    || self.bluetooth_operation_rx.is_some()
                {
                    return;
                }
                let enabled = !self.bluetooth.discovering;
                self.start_bluetooth_operation(
                    BluetoothOperation::SetDiscovery(enabled),
                    move || set_bluetooth_adapter_property("Discovering", enabled),
                );
            }
            SettingsMessage::SetWifiPower(enabled) => {
                if !self.network_available
                    || !cfg!(target_os = "linux")
                    || self.wifi_power_rx.is_some()
                    || enabled == self.wifi_enabled
                {
                    return;
                }
                #[cfg(target_os = "linux")]
                {
                    let (sender, receiver) = mpsc::channel();
                    std::thread::spawn(move || {
                        let _ = sender.send(set_linux_wifi_enabled(enabled));
                    });
                    self.wifi_power_rx = Some(receiver);
                }
                self.next_network_refresh = Instant::now();
            }
            SettingsMessage::SetCodexEnabled(enabled) => {
                self.request_codex_enabled(enabled, false);
            }
            SettingsMessage::ConfirmDisableCodex => self.request_codex_enabled(false, true),
            SettingsMessage::CancelDisableCodex => self.codex_disable_confirmation = false,
            SettingsMessage::ToggleCodexSourceSelect => {
                self.codex_source_select_expanded = !self.codex_source_select_expanded;
            }
            SettingsMessage::SetCodexSource(source) => {
                self.codex_source_select_expanded = false;
                self.update_codex_source(source);
            }
            SettingsMessage::CodexExecutablePathChanged(path) => self.codex_executable_path = path,
            SettingsMessage::ApplyCodexExecutable => {
                let path = std::path::PathBuf::from(self.codex_executable_path.trim());
                if path.is_absolute() {
                    self.update_codex_source(CodexSource::Executable(path));
                } else {
                    self.codex_feature.effective = FeatureEffectiveState::Rejected;
                    self.codex_feature.capability.diagnostic =
                        Some("Select an absolute executable path".into());
                }
            }
            SettingsMessage::RetryCodexProbe => self.start_codex_probe(),
            SettingsMessage::BluetoothDevice(index) => {
                let Some(device) = self.bluetooth.devices.get(index).cloned() else {
                    return;
                };
                if !self.bluetooth.available
                    || !self.bluetooth.powered
                    || self.bluetooth_operation_rx.is_some()
                {
                    return;
                }
                let id = device.id.clone();
                self.start_bluetooth_operation(BluetoothOperation::ToggleDevice(id), move || {
                    toggle_bluetooth_device(&device)
                });
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
            SettingsMessage::ToggleFileIconProviderSelect => {
                self.file_icon_provider_select_expanded = !self.file_icon_provider_select_expanded;
            }
            SettingsMessage::SetFileIconProvider(provider) => {
                self.shell_settings.file_icon_provider = provider;
                if provider == FileIconPreference::System {
                    self.shell_settings.file_icon_theme = None;
                }
                self.file_icon_provider_select_expanded = false;
                self.persist_appearance();
            }
            SettingsMessage::SetFileIconTheme(theme) => {
                self.shell_settings.file_icon_provider = FileIconPreference::System;
                self.shell_settings.file_icon_theme = Some(theme);
                self.file_icon_provider_select_expanded = false;
                self.persist_appearance();
            }
            SettingsMessage::BarPrimaryDisplay => {
                self.flush_pending_desktop_count();
                let previous = self.shell_settings.clone();
                self.shell_settings.bar_on_all_displays = false;
                self.persist_shell_behavior(previous);
            }
            SettingsMessage::BarAllDisplays => {
                self.flush_pending_desktop_count();
                let previous = self.shell_settings.clone();
                self.shell_settings.bar_on_all_displays = true;
                self.persist_shell_behavior(previous);
            }
            SettingsMessage::BarDisplayWindows => {
                self.flush_pending_desktop_count();
                let previous = self.shell_settings.clone();
                self.shell_settings.all_windows_on_every_bar = false;
                self.persist_shell_behavior(previous);
            }
            SettingsMessage::BarAllWindows => {
                self.flush_pending_desktop_count();
                let previous = self.shell_settings.clone();
                self.shell_settings.all_windows_on_every_bar = true;
                self.persist_shell_behavior(previous);
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
            SettingsMessage::DisplayDrag { index, phase, x, y } => match phase {
                DragPhase::Started => self.begin_display_drag(index, x, y),
                DragPhase::Moved => self.move_display_drag(x, y),
                DragPhase::Ended => {
                    self.move_display_drag(x, y);
                    self.finish_drag();
                }
                DragPhase::Cancelled => self.cancel_drag(),
            },
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
            | SettingsMessage::DefaultAppsScroll
            | SettingsMessage::AppearanceScroll => {}
            SettingsMessage::DefaultAppTargetChanged(value) => {
                self.default_app_target_query = value;
            }
            SettingsMessage::AddDefaultAppTarget => {
                let query = self.default_app_target_query.trim();
                let target = query
                    .strip_prefix("scheme:")
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(nickel_platform::AssociationTarget::scheme)
                    .or_else(|| {
                        query
                            .strip_prefix("x-scheme-handler/")
                            .filter(|value| !value.is_empty())
                            .map(nickel_platform::AssociationTarget::scheme)
                    })
                    .or_else(|| {
                        query
                            .contains('/')
                            .then(|| nickel_platform::AssociationTarget::mime(query))
                    });
                if let Some(target) = target {
                    if !self.default_apps.iter().any(|row| row.target == target) {
                        let snapshot = nickel_platform::association_service().inspect(&target);
                        self.default_apps.push(DefaultAppRow {
                            label: format!("Advanced — {}", target.platform_key()),
                            target,
                            snapshot: snapshot.as_ref().ok().cloned(),
                            status: snapshot.err().map(|error| error.to_string()),
                        });
                    }
                    self.default_app_target_query.clear();
                } else {
                    self.status = "Enter a MIME type such as text/markdown or scheme:https".into();
                }
            }
            SettingsMessage::ToggleDefaultAppSelect(index) => {
                self.default_app_select_expanded =
                    (self.default_app_select_expanded != Some(index)).then_some(index);
            }
            SettingsMessage::SetDefaultApp { row, handler_id } => {
                self.change_default_app(row, &handler_id);
            }
            SettingsMessage::RequestDefaultAppConsent(row) => {
                self.change_default_app(row, "");
            }
            SettingsMessage::SetPreferredTerminal(value) => {
                let previous = self.shell_settings.clone();
                self.shell_settings.preferred_terminal =
                    (!value.trim().is_empty()).then(|| value.trim().to_owned());
                self.persist_shell_behavior(previous);
            }
            SettingsMessage::SetPreferredFileManager(value) => {
                let previous = self.shell_settings.clone();
                self.shell_settings.preferred_file_manager =
                    (!value.trim().is_empty()).then(|| value.trim().to_owned());
                self.persist_shell_behavior(previous);
            }
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

    fn cancel_drag(&mut self) {
        self.drag_offset = None;
        let Some(origin) = self.drag_origin.take() else {
            return;
        };
        self.displays[self.selected].rect = origin;
        self.request_redraw();
    }

    fn set_desktop_count(&mut self, count: u8) {
        let count = count.clamp(1, 8);
        if count == self.shell_settings.desktop_count {
            return;
        }
        if self.desktop_count_previous.is_none() {
            self.desktop_count_previous = Some(self.shell_settings.clone());
        }
        self.shell_settings.desktop_count = count;
        self.shell_settings.active_desktop = self
            .shell_settings
            .active_desktop
            .min(count.saturating_sub(1));
        self.desktop_count_save_deadline = Some(Instant::now() + Duration::from_millis(100));
        self.status = "Desktop count pending".into();
        self.request_redraw();
    }

    fn persist_shell_behavior(&mut self, previous: ShellSettings) {
        if !self.persistence_enabled {
            return;
        }
        let result = try_save_shell_settings(&self.shell_settings).and_then(|()| {
            match session_request(SessionRequest::Command(SessionCommand::ReloadShellSettings)) {
                Ok(ServerMessage::Ack) => Ok(()),
                Ok(_) => Err("the session returned an unexpected response".to_owned()),
                Err(error) => Err(error.to_string()),
            }
        });
        if let Err(error) = result {
            self.shell_settings = previous.clone();
            let _ = try_save_shell_settings(&previous);
            let error = error.chars().take(240).collect::<String>();
            self.status = format!("Could not apply shell setting: {error}");
        } else {
            self.status = "Nickel Bar settings applied".into();
        }
    }

    fn flush_pending_desktop_count(&mut self) {
        self.desktop_count_save_deadline = None;
        if let Some(previous) = self.desktop_count_previous.take() {
            self.persist_shell_behavior(previous);
        }
    }

    fn start_codex_probe(&mut self) {
        let settings = self.optional_features.clone();
        let generation = settings.codex_generation;
        let source = settings.codex_source.clone();
        let (sender, receiver) = mpsc::channel();
        std::thread::Builder::new()
            .name("nickel-codex-feature-probe".into())
            .spawn(move || {
                let capability = probe_codex_capability(&settings);
                let _ = sender.send((generation, source, capability));
            })
            .ok();
        self.codex_probe_rx = Some(receiver);
        self.codex_feature.capability.health = FeatureHealth::Loading;
    }

    fn poll_codex_probe(&mut self) {
        let Some(receiver) = self.codex_probe_rx.as_ref() else {
            return;
        };
        match receiver.try_recv() {
            Ok((generation, source, capability)) => {
                self.codex_probe_rx = None;
                if generation != self.optional_features.codex_generation
                    || source != self.optional_features.codex_source
                {
                    self.start_codex_probe();
                    return;
                }
                self.codex_feature.capability = capability;
                self.codex_feature = FeatureState::resolve(
                    self.optional_features.codex_enabled,
                    self.optional_features.codex_generation,
                    self.optional_feature_runtime.codex_generation,
                    self.codex_feature.capability.clone(),
                );
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                self.codex_probe_rx = None;
                self.codex_feature.effective = FeatureEffectiveState::Rejected;
                self.codex_feature.capability.health = FeatureHealth::Failed;
                self.codex_feature.capability.diagnostic = Some("Codex probe stopped".into());
            }
            Err(mpsc::TryRecvError::Empty) => {}
        }
    }

    fn refresh_optional_feature_state(&mut self) {
        let disk = OptionalFeatureSettings::load_default();
        let runtime = OptionalFeatureRuntime::load_default();
        let external_change = self.persistence_enabled && disk != self.optional_features;
        if external_change {
            self.optional_features = disk;
        }
        let runtime_change = runtime != self.optional_feature_runtime;
        self.optional_feature_runtime = runtime;
        let (policy, policy_source) = codex_policy();
        let policy_change = policy != self.codex_feature.capability.policy
            || policy_source != self.codex_feature.capability.policy_source;
        self.codex_feature.capability.policy = policy;
        self.codex_feature.capability.policy_source = policy_source;
        if external_change {
            self.start_codex_probe();
        }
        if external_change || runtime_change || policy_change {
            self.codex_feature.requested_enabled = match self.codex_feature.capability.policy {
                FeaturePolicy::ForceEnabled => true,
                FeaturePolicy::ForceDisabled => false,
                FeaturePolicy::Editable => self.optional_features.codex_enabled,
            };
            self.codex_feature.generation = self.optional_features.codex_generation;
            self.codex_feature.acknowledged_generation =
                self.optional_feature_runtime.codex_generation;
            self.codex_feature.capability.support = self.optional_feature_runtime.codex_support;
            self.codex_feature.capability.installation =
                self.optional_feature_runtime.codex_installation;
            self.codex_feature.capability.health = self.optional_feature_runtime.codex_health;
            self.codex_feature.capability.diagnostic =
                self.optional_feature_runtime.diagnostic.clone();
            self.codex_feature = FeatureState::resolve(
                self.optional_features.codex_enabled,
                self.optional_features.codex_generation,
                self.optional_feature_runtime.codex_generation,
                self.codex_feature.capability.clone(),
            );
        }
    }

    fn request_codex_enabled(&mut self, enabled: bool, confirmed: bool) {
        if !self.codex_feature.editable() || self.optional_features.codex_enabled == enabled {
            return;
        }
        if !enabled && self.optional_feature_runtime.active_windows > 0 && !confirmed {
            self.codex_disable_confirmation = true;
            return;
        }
        self.codex_disable_confirmation = false;
        if self.persistence_enabled {
            match try_update_optional_feature_settings(|settings| {
                settings.codex_enabled = enabled;
                settings.codex_generation = settings.codex_generation.saturating_add(1);
            }) {
                Ok(settings) => self.optional_features = settings,
                Err(error) => {
                    self.codex_feature.effective = FeatureEffectiveState::Rejected;
                    self.codex_feature.capability.diagnostic = Some(error);
                    return;
                }
            }
        } else {
            self.optional_features.codex_enabled = enabled;
            self.optional_features.codex_generation =
                self.optional_features.codex_generation.saturating_add(1);
        }
        self.codex_feature = FeatureState::resolve(
            enabled,
            self.optional_features.codex_generation,
            self.optional_feature_runtime.codex_generation,
            self.codex_feature.capability.clone(),
        );
        if self.persistence_enabled
            && let Err(error) =
                session_request(SessionRequest::Command(SessionCommand::ReloadShellSettings))
        {
            self.codex_feature.capability.diagnostic = Some(format!(
                "Saved; waiting for the shell to observe the change: {error}"
            ));
        }
    }

    fn update_codex_source(&mut self, source: CodexSource) {
        if !self.codex_feature.editable() || self.optional_features.codex_source == source {
            return;
        }
        if self.optional_feature_runtime.active_windows > 0 {
            self.codex_feature.effective = FeatureEffectiveState::Rejected;
            self.codex_feature.capability.diagnostic =
                Some("Close built-in Codex windows before changing the backend source".into());
            return;
        }
        if self.persistence_enabled {
            match try_update_optional_feature_settings(|settings| {
                settings.codex_source = source.clone();
                settings.codex_generation = settings.codex_generation.saturating_add(1);
            }) {
                Ok(settings) => self.optional_features = settings,
                Err(error) => {
                    self.codex_feature.effective = FeatureEffectiveState::Rejected;
                    self.codex_feature.capability.diagnostic = Some(error);
                    return;
                }
            }
            if let Err(error) =
                session_request(SessionRequest::Command(SessionCommand::ReloadShellSettings))
            {
                self.codex_feature.capability.diagnostic = Some(format!(
                    "Saved; waiting for the shell to observe the change: {error}"
                ));
            }
        } else {
            self.optional_features.codex_source = source;
            self.optional_features.codex_generation =
                self.optional_features.codex_generation.saturating_add(1);
        }
        self.codex_feature.generation = self.optional_features.codex_generation;
        self.codex_feature.effective = FeatureEffectiveState::Enabling;
        self.start_codex_probe();
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
        if result.is_ok() {
            let _ = session_request(SessionRequest::Command(SessionCommand::ReloadShellSettings));
        }
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
        self.shell_settings.file_icon_provider = defaults.file_icon_provider;
        self.shell_settings.file_icon_theme = defaults.file_icon_theme;
        self.wallpaper_settings = WallpaperSettings::default();
        self.wallpaper_preview = None;
        self.wallpaper_dimensions = None;
        self.wallpaper_status = None;
    }
}

impl SettingsApp {
    fn tick(&mut self) {
        self.poll_wallpaper_dialog();
        self.poll_wifi_power();
        self.poll_bluetooth_operation();
        self.poll_codex_probe();
        let now = Instant::now();
        let session_events = self
            .session_events
            .as_ref()
            .map(|receiver| receiver.try_iter().collect::<Vec<_>>())
            .unwrap_or_default();
        for event in session_events {
            match event {
                nickel_session_protocol::Event::ShellSettingsChanged => {
                    if self.desktop_count_previous.is_none() {
                        self.shell_settings = load_shell_settings();
                    }
                    self.refresh_workspace_state();
                    self.request_redraw();
                }
                nickel_session_protocol::Event::Workspaces(workspaces) => {
                    self.apply_workspace_state(&workspaces);
                    self.request_redraw();
                }
                nickel_session_protocol::Event::Snapshot(_) => {
                    self.load_outputs();
                    self.refresh_workspace_state();
                    self.request_redraw();
                }
                _ => {}
            }
        }
        if self.page == SettingsPage::OptionalFeatures && now >= self.next_optional_feature_refresh
        {
            self.refresh_optional_feature_state();
            self.next_optional_feature_refresh = now + Duration::from_millis(250);
        }
        if self
            .appearance_save_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.persist_appearance();
            self.appearance_save_deadline = None;
        }
        if self
            .desktop_count_save_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            self.desktop_count_save_deadline = None;
            if let Some(previous) = self.desktop_count_previous.take() {
                self.persist_shell_behavior(previous);
            }
        }
        if self.page == SettingsPage::Bluetooth && now >= self.next_bluetooth_refresh {
            self.load_bluetooth();
        }
        if self.page == SettingsPage::Network && now >= self.next_network_refresh {
            self.load_linux_network();
        }
        if self.page == SettingsPage::DefaultApps && now >= self.next_default_apps_refresh {
            self.load_default_apps();
            self.request_redraw();
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

    fn refresh_workspace_state(&mut self) {
        let Ok(ServerMessage::Workspaces(workspaces)) =
            session_request(SessionRequest::Query(SessionQuery::Workspaces))
        else {
            return;
        };
        self.apply_workspace_state(&workspaces);
    }

    fn apply_workspace_state(&mut self, workspaces: &nickel_session_protocol::WorkspaceState) {
        if let Some(active) = workspaces
            .ordered
            .iter()
            .position(|workspace| workspace.id == workspaces.active)
        {
            self.shell_settings.active_desktop = u8::try_from(active).unwrap_or(u8::MAX);
        }
        if self.desktop_count_previous.is_none()
            && let Ok(count) = u8::try_from(workspaces.ordered.len())
            && (1..=8).contains(&count)
        {
            self.shell_settings.desktop_count = count;
        }
    }

    fn poll_wifi_power(&mut self) {
        let Some(receiver) = self.wifi_power_rx.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err("Wi-Fi request stopped".to_owned()),
        };
        self.wifi_power_rx = None;
        match result {
            Ok(()) => self.load_linux_network(),
            Err(error) => {
                self.wifi_status =
                    self.localizer
                        .value("settings-network-connection-failed", "error", &error);
                self.request_redraw();
            }
        }
    }

    fn start_bluetooth_operation(
        &mut self,
        operation: BluetoothOperation,
        request: impl FnOnce() -> Result<(), String> + Send + 'static,
    ) {
        let (sender, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(request());
        });
        self.bluetooth_operation = Some(operation);
        self.bluetooth_operation_rx = Some(receiver);
        self.bluetooth_status = None;
        self.request_redraw();
    }

    fn poll_bluetooth_operation(&mut self) {
        let Some(receiver) = self.bluetooth_operation_rx.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                Err("Bluetooth request stopped before completion".to_owned())
            }
        };
        self.bluetooth_operation_rx = None;
        self.bluetooth_operation = None;
        match result {
            Ok(()) => {
                self.bluetooth_status = None;
                self.load_bluetooth();
            }
            Err(error) => {
                self.bluetooth_status = Some(error);
                self.next_bluetooth_refresh = Instant::now() + Duration::from_secs(2);
                self.request_redraw();
            }
        }
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

    fn controller_family_changed(&mut self, family: nickel_ui::ControllerFamily) -> bool {
        if self.controller_family == family {
            false
        } else {
            self.controller_family = family;
            true
        }
    }

    fn take_focus_request(&mut self) -> Option<UiId> {
        self.pending_effects.pop().map(|effect| match effect {
            SettingsEffect::FocusControl(target) => UiId::from(target),
        })
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
        deadlines.extend(self.desktop_count_save_deadline);
        deadlines.extend(self.next_wifi_refresh);
        if self.page == SettingsPage::Bluetooth {
            deadlines.push(self.next_bluetooth_refresh);
        }
        if self.page == SettingsPage::Network {
            deadlines.push(self.next_network_refresh);
        }
        if self.page == SettingsPage::DefaultApps {
            deadlines.push(self.next_default_apps_refresh);
        }
        if self.wallpaper_dialog_rx.is_some() {
            deadlines.push(now + self.wallpaper_poll_delay);
        }
        if self.wifi_power_rx.is_some() {
            deadlines.push(now + Duration::from_millis(16));
        }
        if self.bluetooth_operation_rx.is_some() {
            deadlines.push(now + Duration::from_millis(16));
        }
        if self.page == SettingsPage::OptionalFeatures {
            deadlines.push(self.next_optional_feature_refresh);
        }
        if self.codex_probe_rx.is_some() {
            deadlines.push(now + Duration::from_millis(16));
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
    input: nickel_input::winit::Adapter,
    sync_requested: bool,
}

impl Default for SettingsHostAdapter {
    fn default() -> Self {
        Self {
            input: nickel_input::winit::Adapter::default(),
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
}

impl HostAdapter<SettingsApp> for SettingsHostAdapter {
    fn next_deadline(&self, now: Instant) -> Option<Instant> {
        self.sync_requested.then_some(now)
    }

    fn started(
        &mut self,
        host: &mut UiHost<SettingsApp>,
        services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        services
            .window()
            .set_min_inner_size(Some(LogicalSize::new(850, 580)));
        let app = host.application_mut();
        app.load_outputs();
        app.load_bluetooth();
        app.load_linux_network();
        if app.page == SettingsPage::DefaultApps {
            app.load_default_apps();
        }
        #[cfg(target_os = "windows")]
        {
            app.load_windows_outputs(services.window());
            app.load_windows_network();
            app.load_windows_wifi();
        }
        Ok(AdapterOutcome::changed())
    }

    fn event(
        &mut self,
        host: &mut UiHost<SettingsApp>,
        event: &WindowEvent,
        _services: HostServices<'_>,
    ) -> Result<AdapterOutcome, Box<dyn std::error::Error>> {
        self.sync_requested = true;
        for input in self.input.normalize(DeviceId(0), event) {
            match input {
                InputEvent::Key(ref key)
                    if key.edge == KeyEdge::Pressed
                        && key.logical == LogicalKey::Named(NamedKey::Escape)
                        && host.inspect().keyboard_focus.is_none() =>
                {
                    return Ok(AdapterOutcome::exit());
                }
                _ => {}
            }
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
    use nickel_ui::{Application, SemanticRole};

    use super::{
        BluetoothDevice, BluetoothOperation, CodexSource, ControllerAction, FeatureEffectiveState,
        FeatureHealth, FeatureInstallation, FeaturePolicy, FeatureSupport, FileIconPreference,
        NetworkAdapter, OptionalFeatureRuntime, OptionalFeatureSettings, Rect, SIDEBAR_WIDTH,
        SettingsApp, SettingsHostAdapter, SettingsMessage, SettingsPage, ThemePreference, UiHost,
        WallpaperSettings, WifiNetwork, attach_rect_centered, codex_feature_state,
        constrain_center, snap_rect,
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

    fn navigation_key(order: u64, physical: KeyCode, logical: NamedKey) -> InputEvent {
        InputEvent::Key(KeyEvent {
            device: DeviceId(1),
            order: EventOrder(order),
            physical: PhysicalKey::Code(physical),
            logical: LogicalKey::Named(logical),
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
    fn default_apps_page_exposes_confirmed_os_handlers_without_conflating_nickel_preferences() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::DefaultApps);
        app.default_apps[0].snapshot = Some(nickel_platform::AssociationSnapshot {
            target: nickel_platform::AssociationTarget::scheme("https"),
            effective: Some(nickel_platform::ApplicationHandler {
                id: "current.desktop".into(),
                name: "Current Browser".into(),
                icon: None,
                source: "fixture".into(),
            }),
            handlers: vec![nickel_platform::ApplicationHandler {
                id: "other.desktop".into(),
                name: "Other Browser".into(),
                icon: None,
                source: "fixture".into(),
            }],
            capability: nickel_platform::AssociationCapability::DirectUserChange,
            scope: nickel_platform::AssociationScope::User,
            detail: "User-level association".into(),
        });
        let tree = app.build_ui(850.0, 900.0);
        assert!(
            !tree
                .semantic_targets_for_message(&SettingsMessage::ToggleDefaultAppSelect(0))
                .is_empty()
        );

        app.default_app_select_expanded = Some(0);
        let expanded = app.build_ui(850.0, 900.0);
        assert!(
            !expanded
                .semantic_targets_for_message(&SettingsMessage::SetDefaultApp {
                    row: 0,
                    handler_id: "other.desktop".into(),
                })
                .is_empty()
        );
        assert!(!app.default_apps.iter().any(|row| {
            matches!(row.target, nickel_platform::AssociationTarget::Mime(ref mime) if mime == "inode/directory")
        }));
        app.update(SettingsMessage::DefaultAppTargetChanged(
            "text/markdown".into(),
        ));
        app.update(SettingsMessage::AddDefaultAppTarget);
        assert!(app.default_apps.iter().any(|row| {
            row.target == nickel_platform::AssociationTarget::mime("text/markdown")
        }));

        app.default_app_select_expanded = None;
        app.default_apps[0].snapshot.as_mut().unwrap().capability =
            nickel_platform::AssociationCapability::NativeConsent;
        let consent = app.build_ui(850.0, 900.0);
        assert!(
            !consent
                .semantic_targets_for_message(&SettingsMessage::RequestDefaultAppConsent(0))
                .is_empty(),
            "consent-only platforms must expose their supported system workflow"
        );
    }

    #[test]
    fn appearance_composition_exposes_shared_controls_and_scrolls() {
        let app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        let tree = app.build_ui(850.0, 580.0);

        assert!(
            tree.scroll_extent(&SettingsMessage::AppearanceScroll)
                .is_some_and(|extent| extent.can_scroll())
        );
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
    fn wifi_power_uses_one_truthful_semantic_switch() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Network);
        app.network_available = true;
        app.wifi_enabled = false;
        app.wifi_status = "Wi-Fi is disabled".to_owned();
        let tree = app.build_ui(850.0, 900.0);

        assert_eq!(
            tree.semantic_targets_for_message(&SettingsMessage::SetWifiPower(true))
                .len(),
            1,
            "one activation must issue exactly one typed power request"
        );
        assert!(
            tree.semantic_targets_for_message(&SettingsMessage::SetWifiPower(false))
                .is_empty(),
            "the rendered switch must request the opposite confirmed state"
        );
        let wifi = tree
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("network-wifi-power"))
            .expect("Wi-Fi power switch");
        assert_eq!(wifi.semantic_role, Some(SemanticRole::Switch));
        assert_eq!(wifi.label.as_deref(), Some("Wi-Fi"));
        assert_eq!(wifi.state.as_deref(), Some("off"));
        assert!(wifi.enabled);
    }

    #[test]
    fn unavailable_wifi_power_is_disabled_and_cannot_request_a_change() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Network);
        app.network_available = false;
        app.wifi_enabled = false;
        let tree = app.build_ui(850.0, 900.0);

        assert!(
            tree.semantic_targets_for_message(&SettingsMessage::SetWifiPower(true))
                .is_empty()
        );
        let wifi = tree
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("network-wifi-power"))
            .expect("unavailable Wi-Fi power switch remains represented");
        assert_eq!(wifi.semantic_role, Some(SemanticRole::Switch));
        assert_eq!(wifi.state.as_deref(), Some("off disabled"));
        assert!(!wifi.enabled);
    }

    #[test]
    fn pending_wifi_power_keeps_confirmed_value_and_rejects_repeat_activation() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Network);
        app.network_available = true;
        app.wifi_enabled = true;
        let (_sender, receiver) = std::sync::mpsc::channel();
        app.wifi_power_rx = Some(receiver);
        let tree = app.build_ui(850.0, 900.0);

        assert!(
            tree.semantic_targets_for_message(&SettingsMessage::SetWifiPower(false))
                .is_empty(),
            "pending requests must disable repeat activation"
        );
        let wifi = tree
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("network-wifi-power"))
            .expect("pending Wi-Fi power switch");
        assert_eq!(wifi.state.as_deref(), Some("on disabled"));
        assert!(!wifi.enabled);
    }

    #[test]
    fn failed_wifi_power_request_restores_the_confirmed_state_and_reports_error() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Network);
        app.network_available = true;
        app.wifi_enabled = false;
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Err("permission denied".to_owned())).unwrap();
        app.wifi_power_rx = Some(receiver);

        app.poll_wifi_power();

        assert!(!app.wifi_enabled, "failure must not mutate confirmed state");
        assert!(app.wifi_power_rx.is_none());
        assert!(app.wifi_status.contains("permission denied"));
        let tree = app.build_ui(850.0, 900.0);
        assert_eq!(
            tree.semantic_targets_for_message(&SettingsMessage::SetWifiPower(true))
                .len(),
            1,
            "rollback leaves the confirmed opposite request available"
        );
    }

    #[test]
    fn bluetooth_power_uses_the_same_semantic_switch_pattern() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Bluetooth);
        app.bluetooth.available = true;
        app.bluetooth.powered = true;
        app.bluetooth.adapter_name = "Test adapter".to_owned();
        let tree = app.build_ui(850.0, 900.0);

        assert_eq!(
            tree.semantic_targets_for_message(&SettingsMessage::SetBluetoothPower(false))
                .len(),
            1
        );
        let bluetooth = tree
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("bluetooth-power"))
            .expect("Bluetooth power switch");
        assert_eq!(bluetooth.semantic_role, Some(SemanticRole::Switch));
        assert_eq!(bluetooth.state.as_deref(), Some("on"));
    }

    #[test]
    fn pending_bluetooth_operation_preserves_confirmed_state_and_disables_commands() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Bluetooth);
        app.bluetooth.available = true;
        app.bluetooth.powered = true;
        app.bluetooth.discovering = false;
        app.bluetooth.devices.push(BluetoothDevice {
            id: "/test/device".into(),
            name: "Headphones".into(),
            paired: true,
            connected: false,
            battery_percent: Some(80),
        });
        let (_sender, receiver) = std::sync::mpsc::channel();
        app.bluetooth_operation = Some(BluetoothOperation::SetPower(false));
        app.bluetooth_operation_rx = Some(receiver);

        let tree = app.build_ui(850.0, 900.0);
        assert!(
            tree.semantic_targets_for_message(&SettingsMessage::SetBluetoothPower(false))
                .is_empty()
        );
        assert!(
            tree.semantic_targets_for_message(&SettingsMessage::BluetoothDiscovery)
                .is_empty()
        );
        let power = tree
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("bluetooth-power"))
            .expect("pending Bluetooth power switch");
        assert_eq!(power.state.as_deref(), Some("on disabled"));
        assert!(!power.enabled);
        assert!(
            app.bluetooth.powered,
            "pending state is not effective state"
        );
    }

    #[test]
    fn failed_bluetooth_operation_preserves_snapshot_and_exposes_error() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Bluetooth);
        app.bluetooth.available = true;
        app.bluetooth.powered = false;
        let (sender, receiver) = std::sync::mpsc::channel();
        sender.send(Err("permission denied".to_owned())).unwrap();
        app.bluetooth_operation = Some(BluetoothOperation::SetPower(true));
        app.bluetooth_operation_rx = Some(receiver);

        app.poll_bluetooth_operation();

        assert!(!app.bluetooth.powered);
        assert!(app.bluetooth_operation.is_none());
        assert!(app.bluetooth_operation_rx.is_none());
        assert_eq!(app.bluetooth_status.as_deref(), Some("permission denied"));
    }

    #[test]
    fn settings_low_level_click_targets_are_limited_to_documented_composites() {
        let source = include_str!("view/pages.rs");
        let click_targets = source
            .lines()
            .filter(|line| line.contains("on_press={"))
            .collect::<Vec<_>>();
        assert_eq!(click_targets.len(), 7, "{click_targets:#?}");
        for required in [
            "SelectDisplay",
            "WifiNetwork",
            "BluetoothDevice",
            "BarPrimaryDisplay",
            "BarAllDisplays",
            "BarDisplayWindows",
            "BarAllWindows",
        ] {
            assert!(
                click_targets.iter().any(|line| line.contains(required)),
                "missing documented custom composite for {required}: {click_targets:#?}"
            );
        }
    }

    #[test]
    fn every_settings_page_builds_across_layout_theme_and_capability_variants() {
        let pages = [
            SettingsPage::Display,
            SettingsPage::Bar,
            SettingsPage::Appearance,
            SettingsPage::Network,
            SettingsPage::Bluetooth,
            SettingsPage::DefaultApps,
            SettingsPage::OptionalFeatures,
            SettingsPage::KeyboardShortcuts,
            SettingsPage::About,
        ];
        for page in pages {
            for (width, height) in [(560.0, 580.0), (1424.0, 1200.0)] {
                let mut app = SettingsApp::with_initial_page(page);
                app.shell_settings.theme = ThemePreference::Light;
                app.network_available = true;
                app.bluetooth.available = true;
                app.bluetooth.powered = true;
                let tree = app.build_ui(width, height);
                assert!(
                    !tree.accessibility_nodes().is_empty(),
                    "empty semantic tree for {page:?} at {width}x{height}"
                );
            }
        }
        for (page, dark, available) in [
            (SettingsPage::Appearance, true, true),
            (SettingsPage::Network, false, false),
            (SettingsPage::Bluetooth, false, false),
        ] {
            let mut app = SettingsApp::with_initial_page(page);
            app.shell_settings.theme = if dark {
                ThemePreference::Dark
            } else {
                ThemePreference::Light
            };
            app.network_available = available;
            app.bluetooth.available = available;
            assert!(!app.build_ui(850.0, 900.0).accessibility_nodes().is_empty());
        }
    }

    #[test]
    fn documented_custom_setting_cards_expose_button_semantics_and_state() {
        let display = SettingsApp::with_initial_page(SettingsPage::Display).build_ui(850.0, 900.0);
        let display_card = display
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("display-card-0"))
            .expect("display card");
        assert_eq!(display_card.semantic_role, Some(SemanticRole::Button));
        assert!(
            display_card
                .label
                .as_deref()
                .is_some_and(|label| !label.is_empty())
        );
        assert!(display_card.state.is_some());

        let mut network = SettingsApp::with_initial_page(SettingsPage::Network);
        network.wifi_networks.push(WifiNetwork {
            id: "test".into(),
            profile: "Test network".into(),
            signal: 75,
            connected: true,
            saved: true,
            secure: true,
            #[cfg(target_os = "windows")]
            interface: 0,
        });
        let network = network.build_ui(850.0, 900.0);
        let wifi = network
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("wifi-network-0"))
            .expect("Wi-Fi network card");
        assert_eq!(wifi.semantic_role, Some(SemanticRole::Button));
        assert_eq!(wifi.state.as_deref(), Some("connected"));

        let mut bluetooth = SettingsApp::with_initial_page(SettingsPage::Bluetooth);
        bluetooth.bluetooth.available = true;
        bluetooth.bluetooth.powered = true;
        bluetooth.bluetooth.devices.push(BluetoothDevice {
            id: "test".into(),
            name: "Headphones".into(),
            paired: true,
            connected: false,
            battery_percent: Some(75),
        });
        let bluetooth = bluetooth.build_ui(850.0, 900.0);
        let device = bluetooth
            .accessibility_nodes()
            .iter()
            .find(|node| node.id.as_str().ends_with("bluetooth-device-0"))
            .expect("Bluetooth device card");
        assert_eq!(device.semantic_role, Some(SemanticRole::Button));
        assert_eq!(device.state.as_deref(), Some("not connected"));
    }

    #[test]
    fn unavailable_named_file_icon_theme_remains_visible_and_accessible() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.file_icon_provider = FileIconPreference::System;
        app.shell_settings.file_icon_theme = Some("missing-nickel-test-theme".to_owned());
        let host = UiHost::new(app, 850, 900);

        assert!(host.semantic_nodes().iter().any(|node| {
            node.name.as_deref() == Some("File artwork")
                && node.value
                    == Some(nickel_ui::SemanticValueSnapshot::Text(
                        "System — missing-nickel-test-theme (unavailable)".to_owned(),
                    ))
        }));
        assert_eq!(
            host.application().shell_settings.file_icon_theme.as_deref(),
            Some("missing-nickel-test-theme")
        );
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
            let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(
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
            let target_dir = std::env::var_os("CARGO_TARGET_DIR")
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| {
                    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")
                });
            let output = target_dir
                .join("nickel-ui-snapshots")
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

        let mut scenario = nickel_ui_testkit::Scenario::new(app, 850, 580);
        let search_result = scenario
            .host()
            .unique_semantic_target_for_message(&message)
            .expect("search result has a production semantic target")
            .id;
        scenario
            .accessibility_action(
                &nickel_ui_testkit::Selector::id(search_result),
                nickel_ui::ActionKind::Activate,
            )
            .expect("search result follows production accessibility routing");
        assert!(
            scenario
                .host()
                .inspect()
                .keyboard_focus
                .as_ref()
                .is_some_and(|id| id.as_str().ends_with("/appearance-mode-system"))
        );
        assert!(scenario.host().application().sidebar_query.is_empty());
    }

    #[test]
    fn sidebar_search_omits_unimplemented_appearance_destinations() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.sidebar_query = "fonts".into();
        let tree = app.build_ui(850.0, 580.0);
        assert!(!tree.accessibility_nodes().iter().any(|node| {
            node.label
                .as_deref()
                .is_some_and(|label| label.contains("Fonts"))
        }));
    }

    #[test]
    fn navigation_activation_converges_across_keyboard_controller_and_accessibility() {
        for modality in ["keyboard", "controller", "accessibility"] {
            let mut host = UiHost::new(
                SettingsApp::with_initial_page(SettingsPage::Display),
                850,
                580,
            );
            let id = host
                .unique_semantic_target_for_message(&SettingsMessage::Navigate(
                    SettingsPage::Appearance,
                ))
                .unwrap()
                .id;
            match modality {
                "keyboard" => {
                    host.request_focus(id);
                    host.handle_input(&enter_event(), None);
                }
                "controller" => {
                    host.perform_controller_semantic_action(
                        id,
                        nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                    );
                }
                "accessibility" => {
                    host.perform_accessibility_action(
                        id,
                        nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
                    );
                }
                _ => unreachable!(),
            }
            assert_eq!(
                host.application().page,
                SettingsPage::Appearance,
                "{modality}"
            );
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
        assert!(
            selected.as_str().contains("appearance-mode-card"),
            "{selected:?}"
        );
        assert_eq!(
            host.inspect().modality,
            nickel_ui::InputModality::Controller
        );
        let selected_background = host.application().ui_theme().surfaces.hover;
        assert!(host.commands().iter().any(|command| matches!(
            command,
            nickel_ui::backend::PaintCommand::RoundedFill { color, .. }
                if *color == selected_background
        )));
    }

    #[test]
    fn sidebar_semantic_activation_selects_destination() {
        let mut host = UiHost::new(
            SettingsApp::with_initial_page(SettingsPage::Appearance),
            850,
            580,
        );
        let destination = host
            .unique_semantic_target_for_message(&SettingsMessage::Navigate(SettingsPage::Display))
            .expect("display destination semantics")
            .id;
        host.perform_accessibility_action(
            destination,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
        );
        assert_eq!(host.application().page, SettingsPage::Display);
    }

    #[test]
    fn controller_reaches_offscreen_appearance_sliders_and_adjusts_them() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.accent_intensity = Some(50);
        let mut host = UiHost::new(app, 850, 580);
        let slider = host
            .semantic_nodes()
            .into_iter()
            .find(|node| {
                node.id.as_str().contains("/appearance-intensity/") && node.value.is_some()
            })
            .unwrap()
            .id;
        host.perform_controller_semantic_action(
            slider,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Increment),
        );
        assert_eq!(host.application().shell_settings.accent_intensity, Some(55));
        assert_eq!(
            host.inspect().modality,
            nickel_ui::InputModality::Controller
        );
    }

    #[test]
    fn directional_controller_navigation_reaches_appearance_controls() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.accent_intensity = Some(50);
        let mut host = UiHost::new(app, 850, 580);
        host.handle_controller_action(ControllerAction::PreviousPane);
        host.handle_controller_action(ControllerAction::Down);
        host.handle_controller_action(ControllerAction::NextPane);
        for _ in 0..3 {
            host.handle_controller_action(ControllerAction::Down);
        }
        let target = host.inspect().controller_target;
        assert!(
            target
                .as_ref()
                .is_some_and(|id| id.as_str().ends_with("/appearance-interface-card")),
            "{target:?}"
        );
        host.handle_controller_action(ControllerAction::Right);

        let mut reached_slider = false;
        let mut reached_dropdown = false;
        for _ in 0..8 {
            if let Some(selected) = host.inspect().controller_target
                && let Some(node) = host
                    .semantic_nodes()
                    .into_iter()
                    .find(|node| node.id == selected)
            {
                reached_dropdown |= node.id.as_str().contains("/appearance-animations/");
                if node.role == Some(nickel_ui::SemanticRole::Slider)
                    && node.id.as_str().contains("/appearance-intensity/")
                {
                    reached_slider = true;
                    host.handle_controller_action(ControllerAction::Confirm);
                    host.handle_controller_action(ControllerAction::Right);
                    assert!(host.inspect().controller_editing);
                    host.handle_controller_action(ControllerAction::Cancel);
                }
            }
            if reached_slider && reached_dropdown {
                break;
            }
            host.handle_controller_action(ControllerAction::Down);
        }

        assert!(
            reached_slider,
            "D-pad traversal skipped the appearance sliders"
        );
        assert!(
            reached_dropdown,
            "D-pad traversal skipped the animations dropdown"
        );
        assert_eq!(host.application().shell_settings.accent_intensity, Some(55));
    }

    #[test]
    fn normalized_keyboard_proxies_the_settings_navigation_tree() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Appearance);
        app.shell_settings.accent_intensity = Some(50);
        let mut host = UiHost::new(app, 850, 580);
        let mut order = 10;
        let mut press = |host: &mut UiHost<SettingsApp>, physical, logical| {
            order += 1;
            host.handle_input(&navigation_key(order, physical, logical), None)
        };

        for _ in 0..4 {
            press(&mut host, KeyCode::ArrowDown, NamedKey::ArrowDown);
        }
        assert!(
            host.inspect()
                .controller_target
                .as_ref()
                .is_some_and(|id| id.as_str().ends_with("/appearance-interface-card"))
        );
        press(&mut host, KeyCode::ArrowRight, NamedKey::ArrowRight);
        press(&mut host, KeyCode::ArrowDown, NamedKey::ArrowDown);
        assert!(
            host.inspect()
                .controller_target
                .as_ref()
                .is_some_and(|id| id.as_str().contains("/appearance-intensity/"))
        );
        press(&mut host, KeyCode::Enter, NamedKey::Enter);
        press(&mut host, KeyCode::ArrowRight, NamedKey::ArrowRight);
        assert_eq!(host.application().shell_settings.accent_intensity, Some(55));
        press(&mut host, KeyCode::Backspace, NamedKey::Backspace);
        assert!(!host.inspect().controller_editing);
        press(&mut host, KeyCode::Backspace, NamedKey::Backspace);
        assert!(
            host.inspect()
                .controller_target
                .as_ref()
                .is_some_and(|id| { id.as_str().ends_with("/appearance-interface-card") })
        );
        assert_eq!(host.inspect().modality, nickel_ui::InputModality::Keyboard);
    }

    #[test]
    fn settings_legend_uses_the_active_playstation_controller_family() {
        let mut host = UiHost::new(
            SettingsApp::with_initial_page(SettingsPage::Appearance),
            850,
            580,
        );
        assert!(host.set_controller_family(nickel_ui::ControllerFamily::PlayStation));
        host.handle_controller_action(ControllerAction::NextPane);

        let labels = host
            .accessibility_nodes()
            .iter()
            .filter_map(|node| node.label.as_deref())
            .collect::<Vec<_>>();
        assert!(labels.contains(&"Cross: Select"));
        assert!(labels.contains(&"L1: Navigation"));
        assert!(labels.contains(&"R1: Content"));
        assert!(labels.contains(&"Circle: Back"));
        assert!(!labels.contains(&"L: Navigation"));
        for label in [
            "Cross: Select",
            "L1: Navigation",
            "R1: Content",
            "Circle: Back",
        ] {
            let node = host
                .accessibility_nodes()
                .iter()
                .find(|node| node.label.as_deref() == Some(label))
                .expect("controller legend entry has accessibility geometry");
            assert!(
                node.rect.origin.y + node.rect.size.height <= 580.0,
                "{label} is outside the Settings viewport: {:?}",
                node.rect
            );
        }
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
        let navigation_states = pointer
            .accessibility_nodes()
            .iter()
            .filter_map(|node| Some((node.label.as_deref()?, node.state.as_deref()?)))
            .collect::<std::collections::HashMap<_, _>>();
        assert_eq!(navigation_states.get("Appearance"), Some(&"selected"));
        assert_eq!(navigation_states.get("Display"), Some(&"unselected"));
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
        let mut host = UiHost::new(
            SettingsApp::with_initial_page(SettingsPage::Display),
            1200,
            760,
        );
        SettingsHostAdapter::sync_display_plane(&mut host);

        let app = host.application();
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
        let mut host = UiHost::new(
            SettingsApp::with_initial_page(SettingsPage::Display),
            1200,
            760,
        );
        let second = host
            .unique_semantic_target_for_message(&SettingsMessage::SelectDisplay(1))
            .expect("second display card exposes an activation action")
            .id;

        host.perform_accessibility_action(
            second,
            nickel_ui::SemanticAction::Invoke(nickel_ui::ActionKind::Activate),
        );

        assert_eq!(host.application().selected, 1);
    }

    #[test]
    fn display_drag_flows_through_declarative_capture_and_application_update() {
        let mut host = UiHost::new(
            SettingsApp::with_initial_page(SettingsPage::Display),
            1200,
            760,
        );
        SettingsHostAdapter::sync_display_plane(&mut host);
        let target = host
            .unique_semantic_target_for_message(&SettingsMessage::SelectDisplay(0))
            .expect("first display is a semantic drag target");
        let start = nickel_ui::Point {
            x: target.bounds.origin.x + 12.0,
            y: target.bounds.origin.y + 12.0,
        };
        let moved = nickel_ui::Point {
            x: start.x + 80.0,
            y: start.y + 55.0,
        };
        host.application_mut().applied = true;
        let origin = host.application().displays[0].rect;

        host.handle_event(nickel_ui::UiEvent::PointerPressed(start));
        assert_eq!(host.application().selected, 0);
        assert!(host.application().drag_offset.is_some());

        host.handle_event(nickel_ui::UiEvent::PointerMoved(moved));
        assert_ne!(host.application().displays[0].rect, origin);

        host.handle_event(nickel_ui::UiEvent::PointerReleased(moved));
        assert!(host.application().drag_offset.is_none());
        assert!(host.application().drag_origin.is_none());
        assert!(!host.application().applied);

        let settled = host.application().displays[0].rect;
        host.handle_event(nickel_ui::UiEvent::PointerPressed(start));
        host.handle_event(nickel_ui::UiEvent::PointerMoved(nickel_ui::Point {
            x: start.x - 45.0,
            y: start.y - 35.0,
        }));
        assert_ne!(host.application().displays[0].rect, settled);
        host.handle_event(nickel_ui::UiEvent::FocusLost);
        assert_eq!(host.application().displays[0].rect, settled);
        assert!(host.application().drag_offset.is_none());
        assert!(host.application().drag_origin.is_none());
    }

    #[test]
    fn optional_features_exposes_a_semantic_codex_switch() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::OptionalFeatures);
        app.codex_feature.capability.installation = FeatureInstallation::Installed;
        app.codex_feature.capability.support = FeatureSupport::Supported;
        let frame = app.build_ui(1100.0, 720.0);
        assert!(
            frame
                .semantic_targets_for_message(&SettingsMessage::SetCodexEnabled(
                    !app.optional_features.codex_enabled,
                ))
                .len()
                == 1
        );
    }

    #[test]
    fn runtime_projection_does_not_confuse_disabled_with_missing_installation() {
        let settings = OptionalFeatureSettings {
            codex_enabled: false,
            codex_generation: 5,
            ..Default::default()
        };
        let runtime = OptionalFeatureRuntime {
            codex_generation: 5,
            codex_effective: FeatureEffectiveState::Disabled,
            codex_health: FeatureHealth::Unknown,
            codex_installation: FeatureInstallation::Installed,
            codex_support: FeatureSupport::Supported,
            ..Default::default()
        };
        let state = codex_feature_state(&settings, &runtime);
        assert_eq!(state.effective, FeatureEffectiveState::Disabled);
        assert_eq!(
            state.capability.installation,
            FeatureInstallation::Installed
        );
    }

    #[test]
    fn codex_preference_and_effective_state_are_not_conflated() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::OptionalFeatures);
        app.persistence_enabled = false;
        app.codex_feature.capability.installation = FeatureInstallation::Installed;
        app.codex_feature.capability.support = FeatureSupport::Supported;
        let enabled = app.optional_features.codex_enabled;
        app.handle_settings_message(SettingsMessage::SetCodexEnabled(!enabled));
        assert_eq!(app.optional_features.codex_enabled, !enabled);
        assert_eq!(app.codex_feature.requested_enabled, !enabled);
        assert_eq!(
            app.codex_feature.effective,
            if enabled {
                FeatureEffectiveState::Disabled
            } else {
                FeatureEffectiveState::Enabling
            }
        );
    }

    #[test]
    fn active_codex_windows_require_explicit_disable_confirmation() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::OptionalFeatures);
        app.persistence_enabled = false;
        app.optional_features.codex_enabled = true;
        app.codex_feature.requested_enabled = true;
        app.codex_feature.capability.policy = FeaturePolicy::Editable;
        app.optional_feature_runtime.active_windows = 2;
        let generation = app.optional_features.codex_generation;
        app.handle_settings_message(SettingsMessage::SetCodexEnabled(false));
        assert!(app.codex_disable_confirmation);
        assert!(app.optional_features.codex_enabled);
        assert_eq!(app.optional_features.codex_generation, generation);
        app.handle_settings_message(SettingsMessage::ConfirmDisableCodex);
        assert!(!app.codex_disable_confirmation);
        assert!(!app.optional_features.codex_enabled);
        assert_eq!(app.optional_features.codex_generation, generation + 1);
    }

    #[test]
    fn policy_locked_feature_rejects_user_changes() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::OptionalFeatures);
        app.persistence_enabled = false;
        app.codex_feature.capability.policy = FeaturePolicy::ForceEnabled;
        let before = app.optional_features.clone();
        app.handle_settings_message(SettingsMessage::SetCodexEnabled(false));
        assert_eq!(app.optional_features, before);
    }

    #[test]
    fn changing_source_creates_one_new_pending_generation() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::OptionalFeatures);
        app.persistence_enabled = false;
        app.codex_probe_rx = None;
        let generation = app.optional_features.codex_generation;
        app.handle_settings_message(SettingsMessage::SetCodexSource(CodexSource::Bundled));
        assert_eq!(app.optional_features.codex_source, CodexSource::Bundled);
        assert_eq!(app.optional_features.codex_generation, generation + 1);
        assert_eq!(app.codex_feature.effective, FeatureEffectiveState::Enabling);
        app.handle_settings_message(SettingsMessage::SetCodexSource(CodexSource::Bundled));
        assert_eq!(app.optional_features.codex_generation, generation + 1);
    }
}
