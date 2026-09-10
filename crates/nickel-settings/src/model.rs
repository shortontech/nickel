use super::*;
use crate::persistence::{
    load_optional_feature_settings, load_shell_settings, load_wallpaper_settings,
};

pub(super) struct SettingsApp {
    pub(super) controller_family: nickel_ui::ControllerFamily,
    pub(super) localizer: Localizer,
    pub(super) redraw_requested: Cell<bool>,
    pub(super) displays: Vec<DisplayCard>,
    pub(super) selected: usize,
    pub(super) cursor: (i32, i32),
    pub(super) drag_offset: Option<(i32, i32)>,
    pub(super) drag_origin: Option<Rect>,
    pub(super) applied: bool,
    pub(super) pixels_per_logical: f64,
    pub(super) display_plane: Rect,
    pub(super) status: String,
    pub(super) page: SettingsPage,
    pub(super) sidebar_query: String,
    pub(super) pending_effects: Vec<SettingsEffect>,
    pub(super) active_destination: Option<SettingsPage>,
    pub(super) appearance_notice: Option<AppearanceNotice>,
    pub(super) terminal_settings: nickel_core::terminal_settings::TerminalSettings,
    pub(super) terminal_foreground_input: String,
    pub(super) terminal_background_input: String,
    pub(super) terminal_status: Option<String>,
    pub(super) persistence_enabled: bool,
    pub(super) wallpaper_position_select_expanded: bool,
    pub(super) animation_select_expanded: bool,
    pub(super) file_icon_provider_select_expanded: bool,
    pub(super) default_app_handler_query: String,
    pub(super) default_app_handler_scroll_offset: f32,
    pub(super) default_apps: Vec<DefaultAppRow>,
    pub(super) default_app_targets: Vec<nickel_platform::AssociationTarget>,
    pub(super) default_app_target_query: String,
    pub(super) default_app_target_family: Option<nickel_platform::AssociationFamily>,
    pub(super) default_app_catalog_scroll_offset: f32,
    pub(super) default_app_target_status: Option<String>,
    pub(super) default_apps_loading: bool,
    pub(super) default_apps_discovery_generation: u64,
    pub(super) default_apps_discovery_rx: Option<std::sync::mpsc::Receiver<DefaultAppsDiscovery>>,
    pub(super) next_default_apps_refresh: Instant,
    pub(super) optional_features: OptionalFeatureSettings,
    pub(super) keyboard_runtime: Option<nickel_session_protocol::OnScreenKeyboardSnapshot>,
    pub(super) keyboard_error: Option<String>,
    pub(super) keyboard_preview: String,
    pub(super) optional_feature_runtime: OptionalFeatureRuntime,
    pub(super) codex_feature: FeatureState,
    pub(super) codex_probe_rx:
        Option<std::sync::mpsc::Receiver<(u64, CodexSource, FeatureCapability)>>,
    pub(super) codex_source_select_expanded: bool,
    pub(super) codex_executable_path: String,
    pub(super) codex_disable_confirmation: bool,
    pub(super) remote_control_settings: nickel_remote_control::RemoteAiControlSettings,
    pub(super) remote_control_runtime: nickel_session_protocol::RemoteControlSnapshot,
    pub(super) remote_lease_custom_minutes: String,
    pub(super) remote_pairing: Option<nickel_session_protocol::RemotePairingSnapshot>,
    pub(super) remote_pairing_qr: Option<Arc<image::RgbaImage>>,
    pub(super) next_optional_feature_refresh: Instant,
    pub(super) shell_settings: ShellSettings,
    pub(super) shell_topology_generation: u64,
    pub(super) wallpaper_settings: WallpaperSettings,
    pub(super) wallpaper_preview: Option<Arc<image::RgbaImage>>,
    pub(super) wallpaper_dimensions: Option<(u32, u32)>,
    pub(super) wallpaper_status: Option<String>,
    pub(super) wallpaper_dialog_rx:
        Option<std::sync::mpsc::Receiver<nickel_platform::FileDialogOutcome>>,
    pub(super) wallpaper_poll_delay: Duration,
    pub(super) appearance_save_deadline: Option<Instant>,
    pub(super) desktop_count_save_deadline: Option<Instant>,
    pub(super) desktop_count_previous: Option<ShellSettings>,
    pub(super) session_events: Option<std::sync::mpsc::Receiver<nickel_session_protocol::Event>>,
    pub(super) network_adapters: Vec<NetworkAdapter>,
    pub(super) wifi_networks: Vec<WifiNetwork>,
    pub(super) network_available: bool,
    pub(super) wifi_enabled: bool,
    pub(super) wifi_status: String,
    pub(super) wifi_power_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    pub(super) pending_wifi_profile: Option<String>,
    pub(super) next_wifi_refresh: Option<Instant>,
    pub(super) wifi_refreshes_left: u8,
    pub(super) bluetooth: BluetoothSnapshot,
    pub(super) bluetooth_operation: Option<BluetoothOperation>,
    pub(super) bluetooth_operation_rx: Option<std::sync::mpsc::Receiver<Result<(), String>>>,
    pub(super) bluetooth_status: Option<String>,
    pub(super) peripheral_snapshot: Option<nickel_platform::PeripheralSnapshot>,
    pub(super) peripheral_status: Option<String>,
    pub(super) peripheral_address: String,
    pub(super) peripheral_rx: Option<std::sync::mpsc::Receiver<PeripheralTaskResult>>,
    pub(super) next_peripheral_refresh: Instant,
    pub(super) maintenance_snapshot: Option<nickel_platform::MaintenanceSnapshot>,
    pub(super) maintenance_status: Option<String>,
    pub(super) maintenance_rx: Option<std::sync::mpsc::Receiver<MaintenanceTaskResult>>,
    pub(super) next_bluetooth_refresh: Instant,
    pub(super) next_network_refresh: Instant,
    pub(super) confirmed_displays: Vec<DisplayCard>,
    pub(super) pending_display_revert: Option<(Instant, Vec<DisplayCard>)>,
    pub(super) reverting_display: bool,
    pub(super) application_scale_policy: ApplicationScalePolicy,
    pub(super) toolkit_scale_status: String,
    #[cfg(target_os = "linux")]
    pub(super) toolkit_scale_observed: Vec<(nickel_platform::ToolkitFamily, String)>,
    pub(super) next_toolkit_scale_refresh: Instant,
}

impl Default for SettingsApp {
    fn default() -> Self {
        let localizer = Localizer::system();
        let status = localizer.text("settings-status-changes-not-applied");
        let wallpaper_settings = load_wallpaper_settings();
        let terminal_settings = nickel_core::terminal_settings::TerminalSettings::load_default();
        let terminal_foreground_input = format!("#{:08x}", terminal_settings.foreground);
        let terminal_background_input = format!("#{:08x}", terminal_settings.background);
        let optional_features = load_optional_feature_settings();
        let optional_feature_runtime = OptionalFeatureRuntime::load_default();
        let codex_feature = codex_feature_state(&optional_features, &optional_feature_runtime);
        let codex_executable_path = match &optional_features.codex_source {
            CodexSource::Executable(path) => path.display().to_string(),
            _ => String::new(),
        };
        let remote_control_settings =
            nickel_remote_control::RemoteAiControlSettings::load_default().unwrap_or_default();
        let remote_control_runtime = session_request(nickel_session_protocol::Request::Query(
            nickel_session_protocol::Query::RemoteControl,
        ))
        .ok()
        .and_then(|message| match message {
            nickel_session_protocol::ServerMessage::RemoteControl(snapshot) => Some(snapshot),
            _ => None,
        })
        .unwrap_or(nickel_session_protocol::RemoteControlSnapshot {
            requested_enabled: remote_control_settings.requested_enabled,
            effective: nickel_session_protocol::RemoteControlEffectiveState::Rejected,
            generation: remote_control_settings.generation,
            acknowledged_generation: 0,
            endpoint: nickel_remote_control::MCP_ENDPOINT.into(),
            host_fingerprint: None,
            environment_override: false,
            diagnostic: Some(
                "Remote control status is unavailable; the session has not acknowledged its listener state."
                    .into(),
            ),
            pending_clients: Vec::new(),
            pending_leases: Vec::new(),
            active_leases: Vec::new(),
            granted_clients: Vec::new(),
            lease_audit: Vec::new(),
            lease_audit_evicted: 0,
            permission_audit: Vec::new(),
            permission_audit_evicted: 0,
            trace_audit: Vec::new(),
            trace_audit_evicted: 0,
            connection_audit: Vec::new(),
            connection_audit_evicted: 0,
        });
        let application_scale_policy = nickel_core::dpi::ApplicationScaleSettings::load_default()
            .unwrap_or_default()
            .policy;
        let (wallpaper_preview, wallpaper_dimensions, wallpaper_status) =
            match load_wallpaper_preview(&wallpaper_settings) {
                Ok(Some(preview)) => (
                    Some(preview.image),
                    Some((preview.source_width, preview.source_height)),
                    None,
                ),
                Ok(None) => (None, None, None),
                Err(error) => (None, None, Some(error.to_string())),
            };
        let displays = vec![
            DisplayCard {
                connector: "DVI-I-1".into(),
                name: "ASUS MB16ACV".into(),
                detail: "DISPLAYLINK  1920 X 1080".into(),
                logical_width: 1920,
                logical_height: 1080,
                rect: Rect {
                    x: 225,
                    y: 186,
                    w: 270,
                    h: 160,
                },
                primary: false,
                enabled: true,
                scale: nickel_core::dpi::Scale120::ONE,
            },
            DisplayCard {
                connector: "DP-3".into(),
                name: "DP-3".into(),
                detail: "NVIDIA  1920 X 1080".into(),
                logical_width: 1920,
                logical_height: 1080,
                rect: Rect {
                    x: 495,
                    y: 176,
                    w: 300,
                    h: 180,
                },
                primary: true,
                enabled: true,
                scale: nickel_core::dpi::Scale120::ONE,
            },
        ];
        let mut shell_settings = load_shell_settings();
        let mut shell_topology_generation = 0;
        if let Ok(nickel_session_protocol::ServerMessage::ShellBehavior(snapshot)) = session_request(
            nickel_session_protocol::Request::Query(nickel_session_protocol::Query::ShellBehavior),
        ) {
            shell_settings.bar_on_all_displays = snapshot.bar_on_all_displays;
            shell_settings.all_windows_on_every_bar = snapshot.all_windows_on_every_bar;
            shell_settings.desktop_count = snapshot.desktop_count;
            shell_topology_generation = snapshot.topology_generation;
        }
        Self {
            controller_family: nickel_ui::ControllerFamily::Generic,
            localizer,
            redraw_requested: Cell::new(true),
            displays: displays.clone(),
            selected: 1,
            cursor: (0, 0),
            drag_offset: None,
            drag_origin: None,
            applied: false,
            pixels_per_logical: 0.14,
            display_plane: DISPLAY_PLANE,
            status,
            page: SettingsPage::Display,
            sidebar_query: String::new(),
            pending_effects: Vec::new(),
            active_destination: Some(SettingsPage::Display),
            appearance_notice: None,
            terminal_settings,
            terminal_foreground_input,
            terminal_background_input,
            terminal_status: None,
            persistence_enabled: !cfg!(test),
            wallpaper_position_select_expanded: false,
            animation_select_expanded: false,
            file_icon_provider_select_expanded: false,
            default_app_handler_query: String::new(),
            default_app_handler_scroll_offset: 0.0,
            default_apps: default_app_categories(),
            default_app_targets: Vec::new(),
            default_app_target_query: String::new(),
            default_app_target_family: None,
            default_app_catalog_scroll_offset: 0.0,
            default_app_target_status: None,
            default_apps_loading: false,
            default_apps_discovery_generation: 0,
            default_apps_discovery_rx: None,
            next_default_apps_refresh: Instant::now(),
            optional_features,
            keyboard_runtime: None,
            keyboard_error: None,
            keyboard_preview: String::new(),
            optional_feature_runtime,
            codex_feature,
            codex_probe_rx: None,
            codex_source_select_expanded: false,
            codex_executable_path,
            codex_disable_confirmation: false,
            remote_control_settings,
            remote_control_runtime,
            remote_lease_custom_minutes: "20".into(),
            remote_pairing: None,
            remote_pairing_qr: None,
            next_optional_feature_refresh: Instant::now(),
            shell_settings,
            shell_topology_generation,
            wallpaper_settings,
            wallpaper_preview,
            wallpaper_dimensions,
            wallpaper_status,
            wallpaper_dialog_rx: None,
            wallpaper_poll_delay: Duration::from_millis(16),
            appearance_save_deadline: None,
            desktop_count_save_deadline: None,
            desktop_count_previous: None,
            session_events: if cfg!(test) {
                None
            } else {
                session_event_receiver()
            },
            network_adapters: Vec::new(),
            wifi_networks: Vec::new(),
            network_available: false,
            wifi_enabled: false,
            wifi_status: String::new(),
            wifi_power_rx: None,
            pending_wifi_profile: None,
            next_wifi_refresh: None,
            wifi_refreshes_left: 0,
            bluetooth: BluetoothSnapshot::default(),
            bluetooth_operation: None,
            bluetooth_operation_rx: None,
            bluetooth_status: None,
            peripheral_snapshot: None,
            peripheral_status: None,
            peripheral_address: String::new(),
            peripheral_rx: None,
            next_peripheral_refresh: Instant::now(),
            maintenance_snapshot: None,
            maintenance_status: None,
            maintenance_rx: None,
            next_bluetooth_refresh: Instant::now(),
            next_network_refresh: Instant::now(),
            confirmed_displays: displays,
            pending_display_revert: None,
            reverting_display: false,
            application_scale_policy,
            toolkit_scale_status: String::new(),
            #[cfg(target_os = "linux")]
            toolkit_scale_observed: Vec::new(),
            next_toolkit_scale_refresh: Instant::now(),
        }
    }
}

impl SettingsApp {
    pub(super) fn with_initial_page(page: SettingsPage) -> Self {
        let mut app = Self {
            page,
            active_destination: Some(page),
            ..Self::default()
        };
        if page == SettingsPage::OptionalFeatures {
            app.start_codex_probe();
        } else if page == SettingsPage::Bar {
            app.refresh_workspace_state();
        } else if page == SettingsPage::Security {
            app.load_maintenance();
        } else if page == SettingsPage::PrintersStorage {
            app.load_peripherals();
        }
        app
    }

    pub(super) fn poll_wallpaper_dialog(&mut self) {
        let Some(receiver) = self.wallpaper_dialog_rx.as_ref() else {
            return;
        };
        let outcome = match receiver.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => {
                self.wallpaper_poll_delay = self
                    .wallpaper_poll_delay
                    .saturating_mul(2)
                    .min(Duration::from_millis(250));
                return;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                nickel_platform::FileDialogOutcome::Failed(
                    self.localizer.text("settings-wallpaper-picker-failed"),
                )
            }
        };
        self.wallpaper_dialog_rx = None;
        self.wallpaper_poll_delay = Duration::from_millis(16);
        match outcome {
            nickel_platform::FileDialogOutcome::Cancelled => {
                self.wallpaper_status = None;
            }
            nickel_platform::FileDialogOutcome::Failed(error) => {
                self.wallpaper_status = Some(error);
            }
            nickel_platform::FileDialogOutcome::Selected(path) => {
                match nickel_platform::decode_image_preview(&path) {
                    Ok(preview) => {
                        self.wallpaper_settings.image = Some(path);
                        self.wallpaper_dimensions =
                            Some((preview.source_width, preview.source_height));
                        self.wallpaper_preview = Some(preview.image);
                        self.wallpaper_status = None;
                        self.persist_wallpaper();
                    }
                    Err(error) => {
                        self.wallpaper_status = Some(error.to_string());
                    }
                }
            }
        }
        self.redraw_requested.set(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_page_is_the_active_navigation_destination() {
        let app = SettingsApp::with_initial_page(SettingsPage::Appearance);

        assert_eq!(app.page, SettingsPage::Appearance);
        assert_eq!(app.active_destination, Some(SettingsPage::Appearance));
    }

    #[test]
    fn cancelled_and_invalid_wallpaper_choices_preserve_the_current_selection() {
        let mut app = SettingsApp::default();
        let original_settings = app.wallpaper_settings.clone();
        let original_dimensions = app.wallpaper_dimensions;

        let (sender, receiver) = std::sync::mpsc::channel();
        app.wallpaper_dialog_rx = Some(receiver);
        sender
            .send(nickel_platform::FileDialogOutcome::Cancelled)
            .unwrap();
        app.poll_wallpaper_dialog();
        assert_eq!(app.wallpaper_settings, original_settings);
        assert_eq!(app.wallpaper_dimensions, original_dimensions);

        let (sender, receiver) = std::sync::mpsc::channel();
        app.wallpaper_dialog_rx = Some(receiver);
        sender
            .send(nickel_platform::FileDialogOutcome::Selected(
                std::path::PathBuf::from("/definitely/missing/wallpaper.png"),
            ))
            .unwrap();
        app.poll_wallpaper_dialog();
        assert_eq!(app.wallpaper_settings, original_settings);
        assert_eq!(app.wallpaper_dimensions, original_dimensions);
        assert!(app.wallpaper_status.is_some());
    }

    #[test]
    fn appearance_preview_updates_immediately_and_persistence_is_debounced() {
        let mut app = SettingsApp::default();
        app.set_appearance_hue(123);
        assert_eq!(app.shell_settings.accent_hue, Some(123));
        assert!(app.appearance_save_deadline.is_some());
        app.set_appearance_intensity(77);
        assert_eq!(app.shell_settings.accent_intensity, Some(77));
        assert!(app.appearance_save_deadline.is_some());
    }

    #[test]
    fn desktop_count_preview_coalesces_and_commits_only_the_latest_value() {
        let mut app = SettingsApp::default();
        let original = app.shell_settings.clone();
        app.set_desktop_count(2);
        app.set_desktop_count(7);
        app.set_desktop_count(4);
        assert_eq!(app.shell_settings.desktop_count, 4);
        assert_eq!(app.desktop_count_previous.as_ref(), Some(&original));
        assert!(app.desktop_count_save_deadline.is_some());
        assert_eq!(app.status, "Desktop count pending");

        app.desktop_count_save_deadline = Some(Instant::now());
        app.tick();
        assert!(app.desktop_count_previous.is_none());
        assert!(app.desktop_count_save_deadline.is_none());
        assert_eq!(app.shell_settings.desktop_count, 4);
    }

    #[test]
    fn authoritative_workspace_broadcast_updates_count_and_active_control() {
        let mut app = SettingsApp::default();
        let state = nickel_session_protocol::WorkspaceState {
            active: nickel_session_protocol::WorkspaceId(7),
            active_output: Some("HDMI-A-1".into()),
            ordered: vec![
                nickel_session_protocol::WorkspaceSnapshot {
                    id: nickel_session_protocol::WorkspaceId(2),
                    windows: Vec::new(),
                    focused: None,
                },
                nickel_session_protocol::WorkspaceSnapshot {
                    id: nickel_session_protocol::WorkspaceId(7),
                    windows: Vec::new(),
                    focused: None,
                },
            ],
        };
        app.apply_workspace_state(&state);
        assert_eq!(app.shell_settings.desktop_count, 2);
        assert_eq!(app.shell_settings.active_desktop, 1);

        app.set_desktop_count(6);
        app.apply_workspace_state(&state);
        assert_eq!(
            app.shell_settings.desktop_count, 6,
            "pending preview is authoritative locally"
        );
        assert_eq!(app.shell_settings.active_desktop, 1);
    }

    #[test]
    fn appearance_persistence_failure_is_visible_without_reverting_session_state() {
        let mut app = SettingsApp::default();
        app.shell_settings.accent_hue = Some(211);
        app.record_appearance_persistence(Err("read-only filesystem".into()));
        assert_eq!(app.shell_settings.accent_hue, Some(211));
        assert!(matches!(
            app.appearance_notice,
            Some(AppearanceNotice::Error(ref message)) if message.contains("read-only filesystem")
        ));
    }

    #[test]
    fn appearance_reset_has_documented_scope_and_preserves_unrelated_shell_choices() {
        let mut app = SettingsApp::default();
        app.shell_settings.bar_on_all_displays = false;
        app.shell_settings.desktop_count = 7;
        app.shell_settings.theme = ThemePreference::Dark;
        app.shell_settings.accent_hue = Some(300);
        app.shell_settings.accent_intensity = Some(20);
        app.shell_settings.reduce_transparency = true;
        app.shell_settings.animations = AnimationLevel::Off;
        app.shell_settings.file_icon_provider = FileIconPreference::System;
        app.shell_settings.file_icon_theme = Some("removed-theme".to_owned());
        app.wallpaper_settings.image = Some("wallpaper.png".into());
        app.reset_appearance_values();
        assert!(!app.shell_settings.bar_on_all_displays);
        assert_eq!(app.shell_settings.desktop_count, 7);
        assert_eq!(app.shell_settings.theme, ThemePreference::System);
        assert_eq!(app.shell_settings.accent_hue, None);
        assert_eq!(app.shell_settings.accent_intensity, None);
        assert!(!app.shell_settings.reduce_transparency);
        assert_eq!(app.shell_settings.animations, AnimationLevel::Normal);
        assert_eq!(
            app.shell_settings.file_icon_provider,
            FileIconPreference::default()
        );
        assert_eq!(app.shell_settings.file_icon_theme, None);
        assert_eq!(app.wallpaper_settings, WallpaperSettings::default());
    }

    #[test]
    fn automatic_mode_tracks_platform_mode_while_retaining_system_accent_defaults() {
        let settings = ShellSettings {
            theme: ThemePreference::System,
            ..ShellSettings::default()
        };
        let system = Appearance {
            mode: ThemeMode::Dark,
            accent: [12, 34, 56],
            intensity: 63,
        };
        assert_eq!(settings.resolve_appearance(system), system);
    }
}
