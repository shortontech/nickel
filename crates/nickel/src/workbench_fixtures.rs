use std::sync::Arc;

use nickel_core::theme::{Appearance, ThemeMode, ThemePalette};
use nickel_ui::{ActionKind, ControllerFamily, SemanticRole};
use nickel_ui_testkit::{
    DEFAULT_ACCESSIBILITY, DEFAULT_LOCALE, DEFAULT_SCALE, Fixture, FixtureMetadata,
    FixtureProvider, FixtureRegistry, FixtureSource, FixtureTheme, FixtureVariant, RegistryError,
    Selector, ViewportPreset,
};

use nickel_codex_ui::ChatApplication;

use crate::{
    control_view::ControlCenterApp,
    live_shell::{DesktopApplication, LockApplication},
    notification::{DesktopNotification, NotificationAction},
    platform::{AudioStatus, BluetoothStatus, NetworkStatus, WorkspaceSummary},
    plugin_panel::{
        NotificationPluginProjection, PluginImages, PluginPanelApplication, TaskbarPluginItem,
        TaskbarPluginProjection, TaskbarPluginTrayItem,
    },
    screenshot::ScreenshotApp,
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
    &["shell", "runtime", "lifecycle", "context-interactive"]
);

metadata!(
    DESKTOP_METADATA,
    "shell.desktop",
    "Desktop",
    "Production desktop application",
    DESKTOP_VARIANTS,
    &["shell", "desktop", "context-interactive"]
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
    "Bundled JSX notification presentation",
    NOTIFICATION_VARIANTS,
    &[
        "shell",
        "notification",
        "dialog",
        "jsx",
        "variant-interactive"
    ]
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

fn fixture_palette(theme: FixtureTheme) -> ThemePalette {
    match theme {
        FixtureTheme::Light => ThemePalette::from_appearance(Appearance {
            mode: ThemeMode::Light,
            ..Appearance::default()
        }),
        FixtureTheme::Dark => palette(),
        FixtureTheme::HighContrast => ThemePalette {
            background: 0x101114,
            panel: 0x17191d,
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
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(
            SemanticRole::ApplicationPresentation,
            "Desktop",
        ))
    }
    fn default_action() -> ActionKind {
        ActionKind::ContextMenu
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
    fn default_activation() -> Option<Selector> {
        Some(Selector::role_name(
            SemanticRole::ApplicationPresentation,
            "Desktop",
        ))
    }
    fn default_action() -> ActionKind {
        ActionKind::ContextMenu
    }
}

impl Fixture for PanelFixture {
    type App = PluginPanelApplication;
    fn metadata() -> &'static FixtureMetadata {
        &PANEL_METADATA
    }
    fn create() -> Self::App {
        Self::create_variant(&PANEL_VARIANTS[0])
    }
    fn create_variant(variant: &FixtureVariant) -> Self::App {
        let populated = variant.id == "status-items";
        let projection = TaskbarPluginProjection {
            items: if populated {
                vec![
                    TaskbarPluginItem {
                        index: 0,
                        id: "fixture.browser".into(),
                        name: "Fixture Browser".into(),
                        active: true,
                        pinned: true,
                        icon: true,
                    },
                    TaskbarPluginItem {
                        index: 1,
                        id: "fixture.editor".into(),
                        name: "Fixture Editor".into(),
                        active: false,
                        pinned: false,
                        icon: true,
                    },
                ]
            } else {
                Vec::new()
            },
            tray: if populated {
                vec![TaskbarPluginTrayItem {
                    id: "fixture-notification".into(),
                    title: "Fixture notification icon".into(),
                    icon: true,
                }]
            } else {
                Vec::new()
            },
            clock: "12:34 PM".into(),
            keyboard_enabled: populated,
            codex_available: populated,
        };
        let mut app = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::taskbar_manifest(),
            "main.js",
            projection.to_json(),
        )
        .expect("bundled taskbar fixture must compile");
        let mut images = PluginImages::new();
        for (key, id, color) in [
            ("logo", 2, [120, 90, 220, 255]),
            ("codex", 0x5000, [90, 190, 230, 255]),
            ("task:0", 0x6000, [40, 140, 240, 255]),
            ("task:1", 0x6001, [220, 90, 120, 255]),
            ("tray:fixture-notification", 0x6002, [80, 210, 140, 255]),
        ] {
            images.insert(
                key.into(),
                (
                    id,
                    Arc::new(image::RgbaImage::from_pixel(32, 32, image::Rgba(color))),
                ),
            );
        }
        app.sync_images(images);
        app
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
    type App = PluginPanelApplication;
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
        let notification =
            DesktopNotification::fixture(1, "Nickel", "Workbench notification", body, actions);
        let projection = NotificationPluginProjection::from_feed(Some(&notification), &[], false);
        PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::notification_manifest(),
            "main.js",
            projection.to_json(),
        )
        .expect("bundled notification fixture must compile")
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
    type App = PluginPanelApplication;
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
            .map(|index| {
                let title = format!("Workbench window {}", index + 1);
                serde_json::json!({
                    "id": (index + 1).to_string(),
                    "title": title,
                    "accessibleName": title,
                    "closable": true,
                    "index": index,
                    "imageWidth": 244,
                    "selected": index == 0,
                })
            })
            .collect::<Vec<_>>();
        let mut application = PluginPanelApplication::bundled_with_data(
            crate::plugin_panel::window_preview_manifest(),
            "main.js",
            serde_json::json!({"windows": windows}).to_string(),
        )
        .expect("bundled window preview fixture");
        if v.id != "missing-preview" {
            let thumbnail = Arc::new(image::RgbaImage::from_pixel(
                260,
                116,
                image::Rgba([40, 54, 82, 255]),
            ));
            let mut images = PluginImages::new();
            for index in 0..count {
                images.insert(
                    format!("window:{}", index + 1),
                    ((index + 1) as u16, Arc::clone(&thumbnail)),
                );
            }
            application.sync_images(images);
        }
        application
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
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
                "shell.lock",
                "shell.notification",
                "shell.panel",
                "shell.runtime",
                "shell.screenshot",
                "shell.window-preview",
            ]
        );
    }
}
