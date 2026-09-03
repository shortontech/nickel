use std::{collections::HashMap, sync::Arc, time::Instant};

use nickel_core::theme::{Appearance, ThemeMode, ThemePalette};
use nickel_ui::{ControllerFamily, ReadingDirection, SemanticRole};
use nickel_ui_testkit::{
    DEFAULT_ACCESSIBILITY, DEFAULT_LOCALE, DEFAULT_SCALE, Fixture, FixtureMetadata,
    FixtureProvider, FixtureRegistry, FixtureSource, FixtureTheme, FixtureVariant, RegistryError,
    Selector, ViewportPreset,
};

use nickel_codex_ui::ChatApplication;

use crate::{
    control_view::ControlCenterApp,
    launcher::{Launcher, LauncherInput},
    launcher_view::{LauncherApplication, LauncherIconCache, LauncherViewState},
    live_shell::{DesktopApplication, LockApplication, PanelApplication},
    model::{WindowGroup, WindowId},
    notification::{NotificationAction, NotificationRequest, NotificationStore},
    notification_view::NotificationApp,
    platform::{AudioStatus, BluetoothStatus, NetworkStatus, WorkspaceSummary},
    screenshot::ScreenshotApp,
    window_preview::WindowPreviewApp,
};

pub struct ShellFixtureProvider;

const fn variant(id: &'static str, title: &'static str, width: u32, height: u32) -> FixtureVariant {
    FixtureVariant {
        id,
        title,
        viewport: ViewportPreset { id, width, height },
        theme: FixtureTheme::Dark,
        locale: DEFAULT_LOCALE,
        scale: DEFAULT_SCALE,
        controller_family: ControllerFamily::Generic,
        accessibility: DEFAULT_ACCESSIBILITY,
    }
}

const RUNTIME_VARIANTS: &[FixtureVariant] = &[
    variant("multi-output", "Multi-output", 960, 540),
    variant("surface-lifecycle", "Surface lifecycle", 800, 450),
];
const DESKTOP_VARIANTS: &[FixtureVariant] = &[
    variant("solid", "Solid background", 960, 540),
    variant("wallpaper", "Wallpaper", 960, 540),
];
const PANEL_VARIANTS: &[FixtureVariant] = &[
    variant("wide", "Wide", 1200, 56),
    variant("narrow", "Narrow", 640, 56),
    variant("fullscreen", "Fullscreen", 960, 56),
    variant(
        "status-items",
        "Tasks, Codex, and notification area",
        1200,
        56,
    ),
];
const NOTIFICATION_VARIANTS: &[FixtureVariant] = &[
    variant("no-actions", "No actions", 420, 180),
    variant("actions", "Actions", 420, 180),
    variant("long-body", "Long body", 420, 240),
];
const LOCK_VARIANTS: &[FixtureVariant] = &[
    variant("empty", "Empty", 960, 540),
    variant("password", "Password", 960, 540),
    variant("error", "Error", 960, 540),
];
const SCREENSHOT_VARIANTS: &[FixtureVariant] = &[
    variant("idle", "Idle", 960, 540),
    variant("selecting", "Selecting", 960, 540),
    variant("confirmed", "Confirmed", 960, 540),
    variant("error", "Error", 960, 540),
];
const PREVIEW_VARIANTS: &[FixtureVariant] = &[
    variant("one", "One window", 300, 214),
    variant("empty", "Empty", 300, 214),
    variant("many", "Many windows", 882, 214),
    variant("missing-preview", "Missing preview", 300, 214),
];
const CONTROL_VARIANTS: &[FixtureVariant] = &[
    variant("available", "Available", 380, 650),
    variant("unavailable", "Unavailable", 380, 650),
    variant("confirmation", "Confirmation", 380, 650),
    variant("scroll", "Scroll", 380, 420),
];
const PROJECT_VARIANTS: &[FixtureVariant] = &[
    variant("open", "Open", 920, 680),
    variant("search", "Search", 920, 680),
    variant("empty", "Empty", 920, 680),
];
const SEARCH_VARIANTS: &[FixtureVariant] = &[
    variant("empty-query", "Empty query", 920, 680),
    variant("results", "Results", 920, 680),
    variant("no-results", "No results", 920, 680),
    variant("scroll", "Scroll", 920, 680),
];

const RTL_LOCALE: nickel_ui_testkit::LocalePreset = nickel_ui_testkit::LocalePreset {
    id: "ar-SA",
    direction: nickel_ui_testkit::FixtureDirection::RightToLeft,
};
const SCALE_2X: nickel_ui_testkit::ScalePreset = nickel_ui_testkit::ScalePreset {
    id: "2x",
    factor: 2.0,
};
const HIGH_CONTRAST: nickel_ui_testkit::AccessibilityPreset =
    nickel_ui_testkit::AccessibilityPreset {
        id: "high-contrast",
        high_contrast: true,
        reduced_motion: false,
        reduced_transparency: true,
    };
const LAUNCHER_DASHBOARD_VARIANTS: &[FixtureVariant] = &[
    FixtureVariant {
        id: "populated-wide-ltr-dark-1x-pointer",
        title: "Populated pointer",
        viewport: ViewportPreset {
            id: "wide",
            width: 920,
            height: 680,
        },
        theme: FixtureTheme::Dark,
        locale: DEFAULT_LOCALE,
        scale: DEFAULT_SCALE,
        controller_family: ControllerFamily::Generic,
        accessibility: DEFAULT_ACCESSIBILITY,
    },
    FixtureVariant {
        id: "empty-narrow-rtl-light-2x-keyboard",
        title: "Empty keyboard",
        viewport: ViewportPreset {
            id: "narrow",
            width: 540,
            height: 680,
        },
        theme: FixtureTheme::Light,
        locale: RTL_LOCALE,
        scale: SCALE_2X,
        controller_family: ControllerFamily::Generic,
        accessibility: DEFAULT_ACCESSIBILITY,
    },
    FixtureVariant {
        id: "loading-wide-ltr-high-contrast-1x-controller-playstation",
        title: "Loading PlayStation",
        viewport: ViewportPreset {
            id: "wide",
            width: 920,
            height: 680,
        },
        theme: FixtureTheme::HighContrast,
        locale: DEFAULT_LOCALE,
        scale: DEFAULT_SCALE,
        controller_family: ControllerFamily::PlayStation,
        accessibility: HIGH_CONTRAST,
    },
    FixtureVariant {
        id: "partial-failure-narrow-rtl-dark-2x-a11y",
        title: "Partial failure accessibility",
        viewport: ViewportPreset {
            id: "narrow",
            width: 540,
            height: 680,
        },
        theme: FixtureTheme::Dark,
        locale: RTL_LOCALE,
        scale: SCALE_2X,
        controller_family: ControllerFamily::Generic,
        accessibility: DEFAULT_ACCESSIBILITY,
    },
    FixtureVariant {
        id: "populated-narrow-ltr-light-1x-controller-xbox",
        title: "Populated Xbox",
        viewport: ViewportPreset {
            id: "narrow",
            width: 540,
            height: 680,
        },
        theme: FixtureTheme::Light,
        locale: DEFAULT_LOCALE,
        scale: DEFAULT_SCALE,
        controller_family: ControllerFamily::Xbox,
        accessibility: DEFAULT_ACCESSIBILITY,
    },
    FixtureVariant {
        id: "empty-wide-rtl-high-contrast-2x-controller-switch",
        title: "Empty Switch",
        viewport: ViewportPreset {
            id: "wide",
            width: 920,
            height: 680,
        },
        theme: FixtureTheme::HighContrast,
        locale: RTL_LOCALE,
        scale: SCALE_2X,
        controller_family: ControllerFamily::Switch,
        accessibility: HIGH_CONTRAST,
    },
    FixtureVariant {
        id: "loading-narrow-ltr-dark-2x-pointer",
        title: "Loading pointer",
        viewport: ViewportPreset {
            id: "narrow",
            width: 540,
            height: 680,
        },
        theme: FixtureTheme::Dark,
        locale: DEFAULT_LOCALE,
        scale: SCALE_2X,
        controller_family: ControllerFamily::Generic,
        accessibility: DEFAULT_ACCESSIBILITY,
    },
    FixtureVariant {
        id: "partial-failure-wide-rtl-light-1x-keyboard",
        title: "Partial failure keyboard",
        viewport: ViewportPreset {
            id: "wide",
            width: 920,
            height: 680,
        },
        theme: FixtureTheme::Light,
        locale: RTL_LOCALE,
        scale: DEFAULT_SCALE,
        controller_family: ControllerFamily::Generic,
        accessibility: DEFAULT_ACCESSIBILITY,
    },
];

macro_rules! metadata {
    ($name:ident, $id:literal, $title:literal, $description:literal, $variants:ident, $tags:expr) => {
        static $name: FixtureMetadata = FixtureMetadata {
            id: $id,
            title: $title,
            description: $description,
            tags: $tags,
            source: FixtureSource {
                crate_name: "nickel-shell",
                file: file!(),
                line: line!(),
            },
            variants: $variants,
            assets: &[],
            simulated_effects: &[],
        };
    };
}

metadata!(
    RUNTIME_METADATA,
    "shell.runtime",
    "Shell runtime",
    "Production shell-owned UiHost surface lifecycle representative",
    RUNTIME_VARIANTS,
    &["shell", "runtime", "lifecycle", "noninteractive"]
);
metadata!(
    LAUNCHER_DASHBOARD_METADATA,
    "shell.launcher-dashboard",
    "Launcher dashboard",
    "Production launcher dashboard state, appearance, direction, scale, and modality matrix",
    LAUNCHER_DASHBOARD_VARIANTS,
    &["shell", "launcher", "dashboard", "matrix"]
);
metadata!(
    DESKTOP_METADATA,
    "shell.desktop",
    "Desktop",
    "Production desktop application",
    DESKTOP_VARIANTS,
    &["shell", "desktop", "noninteractive"]
);
metadata!(
    PANEL_METADATA,
    "shell.panel",
    "Panel",
    "Production panel application",
    PANEL_VARIANTS,
    &["shell", "panel", "controller"]
);
metadata!(
    NOTIFICATION_METADATA,
    "shell.notification",
    "Notification",
    "Production notification application",
    NOTIFICATION_VARIANTS,
    &["shell", "notification", "dialog", "variant-interactive"]
);
metadata!(
    LOCK_METADATA,
    "shell.lock",
    "Lock screen",
    "Production lock application",
    LOCK_VARIANTS,
    &["shell", "lock", "textbox", "input-only"]
);
metadata!(
    SCREENSHOT_METADATA,
    "shell.screenshot",
    "Screenshot",
    "Production screenshot application",
    SCREENSHOT_VARIANTS,
    &["shell", "screenshot", "selection", "variant-interactive"]
);
metadata!(
    PREVIEW_METADATA,
    "shell.window-preview",
    "Window preview",
    "Production window preview application",
    PREVIEW_VARIANTS,
    &["shell", "window", "preview"]
);
metadata!(
    CONTROL_METADATA,
    "shell.control-center",
    "Control Center",
    "Production control center application",
    CONTROL_VARIANTS,
    &["shell", "control-center"]
);
metadata!(
    PROJECT_METADATA,
    "shell.codex-project-menu",
    "Codex project menu",
    "Production launcher project surface used by the shell",
    PROJECT_VARIANTS,
    &["shell", "codex", "projects"]
);
metadata!(
    SEARCH_METADATA,
    "shell.launcher-search",
    "Launcher search",
    "Production launcher search surface",
    SEARCH_VARIANTS,
    &["shell", "launcher", "search"]
);

fn palette() -> ThemePalette {
    ThemePalette::from_appearance(Appearance::default())
}

pub struct RuntimeFixture;
pub struct DesktopFixture;
pub struct PanelFixture;
pub struct NotificationFixture;
pub struct LockFixture;
pub struct ScreenshotFixture;
pub struct WindowPreviewFixture;
pub struct ControlCenterFixture;
pub struct CodexProjectMenuFixture;
pub struct LauncherSearchFixture;
pub struct LauncherDashboardFixture;

fn fixture_palette(theme: FixtureTheme) -> ThemePalette {
    match theme {
        FixtureTheme::Light => ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Light,
            ..Appearance::default()
        }),
        FixtureTheme::Dark => palette(),
        FixtureTheme::HighContrast => ThemePalette {
            background: 0x000000,
            panel: 0x000000,
            surface: 0x111111,
            surface_hover: 0x222222,
            text: 0xffffff,
            muted: 0xd8d8d8,
            accent: 0xffff00,
            accent_soft: 0x333300,
            complement: 0x00ffff,
        },
    }
}

impl Fixture for RuntimeFixture {
    type App = DesktopApplication;
    fn metadata() -> &'static FixtureMetadata {
        &RUNTIME_METADATA
    }
    fn create() -> Self::App {
        DesktopApplication::fixture(None, palette())
    }
    fn surface_size() -> (u32, u32) {
        (960, 540)
    }
}

impl Fixture for DesktopFixture {
    type App = DesktopApplication;
    fn metadata() -> &'static FixtureMetadata {
        &DESKTOP_METADATA
    }
    fn create() -> Self::App {
        Self::create_variant(&DESKTOP_VARIANTS[0])
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        let wallpaper = (v.id == "wallpaper").then(|| {
            Arc::new(image::RgbaImage::from_fn(64, 64, |x, y| {
                let value = ((x / 8 + y / 8) % 2) as u8;
                image::Rgba([28 + value * 18, 34 + value * 12, 58 + value * 28, 255])
            }))
        });
        DesktopApplication::fixture(wallpaper, palette())
    }
    fn surface_size() -> (u32, u32) {
        (960, 540)
    }
}

impl Fixture for PanelFixture {
    type App = PanelApplication;
    fn metadata() -> &'static FixtureMetadata {
        &PANEL_METADATA
    }
    fn create() -> Self::App {
        PanelApplication::fixture(Launcher::default(), palette())
    }
    fn create_variant(variant: &FixtureVariant) -> Self::App {
        if variant.id == "status-items" {
            PanelApplication::populated_fixture(Launcher::default(), fixture_palette(variant.theme))
        } else {
            PanelApplication::fixture(Launcher::default(), fixture_palette(variant.theme))
        }
    }
    fn surface_size() -> (u32, u32) {
        (1200, 56)
    }
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(
            SemanticRole::Button,
            "Open Nickel Start",
        ))
    }
}

impl Fixture for NotificationFixture {
    type App = NotificationApp;
    fn metadata() -> &'static FixtureMetadata {
        &NOTIFICATION_METADATA
    }
    fn create() -> Self::App {
        Self::create_variant(&NOTIFICATION_VARIANTS[0])
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        let actions = if v.id == "no-actions" {
            vec![]
        } else {
            vec![NotificationAction {
                key: "open".into(),
                label: "Open".into(),
            }]
        };
        let body = if v.id == "long-body" {
            "A deterministic notification body that wraps across several lines without invoking the native notification transport.".repeat(2)
        } else {
            "The fixture is ready.".into()
        };
        let mut store = NotificationStore::default();
        store.notify(
            0,
            NotificationRequest {
                app_name: "Nickel".into(),
                summary: "Workbench notification".into(),
                body,
                actions,
                expire_timeout_ms: 0,
            },
            Instant::now(),
        );
        let mut app = NotificationApp::new(palette());
        let notification = store.newest();
        app.sync(notification.as_ref(), palette());
        app
    }
    fn surface_size() -> (u32, u32) {
        (420, 180)
    }
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(SemanticRole::Button, "Dismiss"))
    }
}

impl Fixture for LockFixture {
    type App = LockApplication;
    fn metadata() -> &'static FixtureMetadata {
        &LOCK_METADATA
    }
    fn create() -> Self::App {
        Self::create_variant(&LOCK_VARIANTS[0])
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        match v.id {
            "password" => LockApplication::fixture("nickel", None),
            "error" => LockApplication::fixture("", Some("Authentication failed".into())),
            _ => LockApplication::fixture("", None),
        }
    }
    fn surface_size() -> (u32, u32) {
        (960, 540)
    }
}

impl Fixture for ScreenshotFixture {
    type App = ScreenshotApp;
    fn metadata() -> &'static FixtureMetadata {
        &SCREENSHOT_METADATA
    }
    fn create() -> Self::App {
        ScreenshotApp::fixture(960, 540, "idle")
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        ScreenshotApp::fixture(v.viewport.width, v.viewport.height, v.id)
    }
    fn surface_size() -> (u32, u32) {
        (960, 540)
    }
}

impl Fixture for WindowPreviewFixture {
    type App = WindowPreviewApp;
    fn metadata() -> &'static FixtureMetadata {
        &PREVIEW_METADATA
    }
    fn create() -> Self::App {
        Self::create_variant(&PREVIEW_VARIANTS[1])
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        let count = match v.id {
            "empty" => 0,
            "many" => 3,
            _ => 1,
        };
        let windows = (0..count)
            .map(|index| crate::model::OpenWindow {
                id: WindowId(index + 1),
                application_id: None,
                active: index == 0,
                title: format!("Workbench window {}", index + 1),
            })
            .collect::<Vec<_>>();
        let group = WindowGroup {
            application_id: None,
            application_name: "Workbench".into(),
            windows,
        };
        let previews = if v.id == "missing-preview" {
            HashMap::new()
        } else {
            group
                .windows
                .iter()
                .map(|window| {
                    (
                        window.id,
                        Arc::new(image::RgbaImage::from_pixel(
                            260,
                            116,
                            image::Rgba([40, 54, 82, 255]),
                        )),
                    )
                })
                .collect()
        };
        WindowPreviewApp::fixture(group, previews, palette())
    }
    fn surface_size() -> (u32, u32) {
        (882, 214)
    }
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(
            SemanticRole::Button,
            "Workbench window 1",
        ))
    }
}

impl Fixture for ControlCenterFixture {
    type App = ControlCenterApp;
    fn metadata() -> &'static FixtureMetadata {
        &CONTROL_METADATA
    }
    fn create() -> Self::App {
        Self::create_variant(&CONTROL_VARIANTS[0])
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        let available = v.id != "unavailable";
        let network = NetworkStatus {
            available,
            enabled: available,
            connected: available,
            name: "Nickel Wi-Fi".into(),
            signal_percent: 82,
            ..Default::default()
        };
        let bluetooth = BluetoothStatus {
            available,
            powered: available,
            ..Default::default()
        };
        let audio = AudioStatus {
            available,
            volume_percent: 64,
            ..Default::default()
        };
        let mut app = ControlCenterApp::new(
            network,
            bluetooth,
            audio,
            vec![
                WorkspaceSummary {
                    id: 1,
                    active: true,
                },
                WorkspaceSummary {
                    id: 2,
                    active: false,
                },
            ],
        );
        if v.id == "confirmation" {
            app.request_session_action(crate::platform::SessionAction::LogOut);
        }
        app
    }
    fn surface_size() -> (u32, u32) {
        (380, 650)
    }
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(SemanticRole::Switch, "wifi-power"))
    }
}

fn launcher_application(kind: &str) -> LauncherApplication {
    let mut launcher = Launcher::default();
    match kind {
        "search-results" => {
            launcher.reduce_input(LauncherInput::Text("fi".into()));
        }
        "search-none" => {
            launcher.reduce_input(LauncherInput::Text("no-such-application".into()));
        }
        "search-scroll" => {
            launcher.reduce_input(LauncherInput::Text("a".into()));
        }
        "search-empty" => {
            launcher.reduce_input(LauncherInput::Text(String::new()));
        }
        _ => {}
    }
    LauncherApplication::new(
        launcher,
        LauncherViewState::default(),
        LauncherIconCache::new(),
        palette(),
    )
}

impl Fixture for CodexProjectMenuFixture {
    type App = ChatApplication;
    fn metadata() -> &'static FixtureMetadata {
        &PROJECT_METADATA
    }
    fn create() -> Self::App {
        ChatApplication::fixture_shell_project_menu("open")
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        ChatApplication::fixture_shell_project_menu(v.id)
    }
    fn surface_size() -> (u32, u32) {
        (920, 680)
    }
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(SemanticRole::Button, "Retry"))
    }
}

impl Fixture for LauncherSearchFixture {
    type App = LauncherApplication;
    fn metadata() -> &'static FixtureMetadata {
        &SEARCH_METADATA
    }
    fn create() -> Self::App {
        launcher_application("search-results")
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        launcher_application(match v.id {
            "results" => "search-results",
            "no-results" => "search-none",
            "scroll" => "search-scroll",
            _ => "search-empty",
        })
    }
    fn surface_size() -> (u32, u32) {
        (920, 680)
    }
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(SemanticRole::GridCell, "firefox"))
    }
}

impl Fixture for LauncherDashboardFixture {
    type App = LauncherApplication;
    fn metadata() -> &'static FixtureMetadata {
        &LAUNCHER_DASHBOARD_METADATA
    }
    fn create() -> Self::App {
        Self::create_variant(&LAUNCHER_DASHBOARD_VARIANTS[0])
    }
    fn create_variant(v: &FixtureVariant) -> Self::App {
        let populated = v.id.starts_with("populated");
        let mut launcher = if populated {
            Launcher::default()
        } else {
            Launcher::new(Vec::new())
        };
        launcher.set_codex_available(true);
        use crate::launcher::{
            DashboardAccount, DashboardProject, DashboardSection, ProjectActivity,
        };
        if v.id.starts_with("partial-failure") {
            launcher.set_dashboard_projects(DashboardSection::Failed {
                message: "Project service unavailable".into(),
                recoverable: true,
            });
            launcher.set_dashboard_account(DashboardSection::Ready(DashboardAccount {
                display_name: "Local user".into(),
                supporting_text: "Offline".into(),
            }));
        } else if v.id.starts_with("loading") {
            launcher.set_dashboard_projects(DashboardSection::Loading);
            launcher.set_dashboard_account(DashboardSection::Loading);
        } else if v.id.starts_with("empty") {
            launcher.set_dashboard_projects(DashboardSection::Empty);
            launcher.set_dashboard_account(DashboardSection::Empty);
        } else {
            launcher.set_dashboard_projects(DashboardSection::Ready(vec![DashboardProject {
                id: "nickel".into(),
                name: "Nickel".into(),
                roots: Vec::new(),
                chat_count: Some(2),
                activity: ProjectActivity::Active,
                last_used_at: Some(1),
            }]));
            launcher.set_dashboard_account(DashboardSection::Ready(DashboardAccount {
                display_name: "Nickel user".into(),
                supporting_text: "Local session".into(),
            }));
        }
        let mut app = LauncherApplication::new(
            launcher,
            LauncherViewState::default(),
            LauncherIconCache::new(),
            fixture_palette(v.theme),
        );
        app.set_controller_family(v.controller_family);
        app.set_reading_direction(match v.locale.direction {
            nickel_ui_testkit::FixtureDirection::LeftToRight => ReadingDirection::LeftToRight,
            nickel_ui_testkit::FixtureDirection::RightToLeft => ReadingDirection::RightToLeft,
        });
        app
    }
    fn surface_size() -> (u32, u32) {
        (920, 680)
    }
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(SemanticRole::GridCell, "firefox"))
    }
}

impl FixtureProvider for ShellFixtureProvider {
    fn register(&self, registry: &mut FixtureRegistry) -> Result<(), RegistryError> {
        registry.register::<RuntimeFixture>()?;
        registry.register::<DesktopFixture>()?;
        registry.register::<PanelFixture>()?;
        registry.register::<NotificationFixture>()?;
        registry.register::<LockFixture>()?;
        registry.register::<ScreenshotFixture>()?;
        registry.register::<WindowPreviewFixture>()?;
        registry.register::<ControlCenterFixture>()?;
        registry.register::<CodexProjectMenuFixture>()?;
        registry
            .register::<LauncherSearchFixture>()
            .and_then(|()| registry.register::<LauncherDashboardFixture>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nickel_ui_testkit::{
        ReachabilityModality, ReachabilityPolicy, Scenario, audit_reachability,
    };

    #[test]
    fn provider_registers_every_shell_surface() {
        let mut registry = FixtureRegistry::new();
        ShellFixtureProvider.register(&mut registry).unwrap();
        let ids = registry
            .finish()
            .into_iter()
            .map(|entry| entry.metadata.id)
            .collect::<Vec<_>>();
        assert_eq!(
            ids,
            vec![
                "shell.codex-project-menu",
                "shell.control-center",
                "shell.desktop",
                "shell.launcher-dashboard",
                "shell.launcher-search",
                "shell.lock",
                "shell.notification",
                "shell.panel",
                "shell.runtime",
                "shell.screenshot",
                "shell.window-preview",
            ]
        );
    }

    #[test]
    fn populated_dashboard_replays_discover_accessibility_menu_actions() {
        let report = audit_reachability(
            || {
                Scenario::new(
                    LauncherDashboardFixture::create_variant(&LAUNCHER_DASHBOARD_VARIANTS[0]),
                    920,
                    680,
                )
            },
            &ReachabilityPolicy {
                modalities: [ReachabilityModality::Accessibility].into_iter().collect(),
                ..ReachabilityPolicy::default()
            },
        );

        for target in [
            "application-menu-discover/launch",
            "application-menu-discover/toggle-pin",
        ] {
            assert!(
                report.paths.iter().any(|path| {
                    path.target == target
                        && path.modality == ReachabilityModality::Accessibility
                        && path.reached
                }),
                "{target} was not replayed: {:?}",
                report.issues
            );
        }
        assert!(report.issues.is_empty(), "issues: {:?}", report.issues);
    }
}
