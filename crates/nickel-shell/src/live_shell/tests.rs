use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use image::{Rgba, RgbaImage};
use nickel_input::KeyCode;
use nickel_session_protocol::{
    AnchorSide, PointerInteraction, PreviewTargetAction, ScreenshotTargetAction, ShellRole,
    ShellSemanticTarget, WindowMenuTargetAction,
};
use nickel_ui::{
    ActionKind, Application as _, ControllerAction, FrameOverlay, HostBatch, HostEvent,
    HostTelemetry, InputModality, OverlayAnchor, Point, Rect, SemanticAction, SemanticRole,
    SemanticSelector, SemanticValueInput, SemanticValueSnapshot, Shortcut, UiEvent, UiHost,
    ViewContext,
};
use nickel_ui_testkit::{Scenario, Selector};

use super::{
    ControlAction, HostRuntimeSamples, LiveShell, desktop_label_foreground, initial_wallpaper,
    panel_status_layout, panel_tray_icons,
    platform::{AudioStatus, FeedState, FeedStatus, GlobalShortcut, SecureStorageState},
    preview_refresh_due, retain_unchanged_desktop_icons, secure_storage_status_label,
    semantic_theme_from_palette, session_feed_status_label, shortcut_capability_status,
    visible_tray_item, window_belongs_to_panel,
};

include!("tests/wallpaper.rs");
include!("tests/shell_flows.rs");
include!("tests/panel_and_cache.rs");
include!("tests/desktop_interactions.rs");
