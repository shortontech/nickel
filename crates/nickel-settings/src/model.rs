use super::*;
use crate::persistence::{load_shell_settings, load_wallpaper_settings};

pub(super) struct SettingsApp {
    pub(super) localizer: Localizer,
    pub(super) redraw_requested: Cell<bool>,
    pub(super) running: bool,
    pub(super) displays: Vec<DisplayCard>,
    pub(super) selected: usize,
    pub(super) cursor: (i32, i32),
    pub(super) drag_offset: Option<(i32, i32)>,
    pub(super) applied: bool,
    pub(super) pixels_per_logical: f64,
    pub(super) status: String,
    pub(super) page: SettingsPage,
    pub(super) appearance_tab: AppearanceTab,
    pub(super) shell_settings: ShellSettings,
    pub(super) wallpaper_settings: WallpaperSettings,
    pub(super) appearance_save_deadline: Option<Instant>,
    pub(super) resize_deadline: Option<Instant>,
    pub(super) frame_interval: Duration,
    pub(super) network_adapters: Vec<NetworkAdapter>,
    pub(super) wifi_networks: Vec<WifiNetwork>,
    pub(super) network_available: bool,
    pub(super) wifi_enabled: bool,
    pub(super) wifi_status: String,
    pub(super) pending_wifi_profile: Option<String>,
    pub(super) next_wifi_refresh: Option<Instant>,
    pub(super) wifi_refreshes_left: u8,
    pub(super) bluetooth: BluetoothSnapshot,
    pub(super) next_bluetooth_refresh: Instant,
    pub(super) next_network_refresh: Instant,
    pub(super) ui: UiTree<SettingsMessage>,
    pub(super) ui_state: UiStateStore,
    pub(super) controller: ControllerInput,
    pub(super) navigation: PaneNavigation,
    pub(super) controller_page: SettingsPage,
}

impl Default for SettingsApp {
    fn default() -> Self {
        let localizer = Localizer::system();
        let status = localizer.text("settings-status-changes-not-applied");
        Self {
            localizer,
            redraw_requested: Cell::new(true),
            running: true,
            displays: vec![
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
                },
            ],
            selected: 1,
            cursor: (0, 0),
            drag_offset: None,
            applied: false,
            pixels_per_logical: 0.14,
            status,
            page: SettingsPage::Display,
            appearance_tab: AppearanceTab::General,
            shell_settings: load_shell_settings(),
            wallpaper_settings: load_wallpaper_settings(),
            appearance_save_deadline: None,
            resize_deadline: None,
            frame_interval: Duration::from_millis(16),
            network_adapters: Vec::new(),
            wifi_networks: Vec::new(),
            network_available: false,
            wifi_enabled: false,
            wifi_status: String::new(),
            pending_wifi_profile: None,
            next_wifi_refresh: None,
            wifi_refreshes_left: 0,
            bluetooth: BluetoothSnapshot::default(),
            next_bluetooth_refresh: Instant::now(),
            next_network_refresh: Instant::now(),
            ui: UiTree::default(),
            ui_state: UiStateStore::default(),
            controller: ControllerInput::new(),
            navigation: PaneNavigation::default(),
            controller_page: SettingsPage::Display,
        }
    }
}

impl SettingsApp {
    pub(super) fn with_initial_page(page: SettingsPage) -> Self {
        Self {
            page,
            controller_page: page,
            ..Self::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_page_also_sets_controller_navigation_page() {
        let app = SettingsApp::with_initial_page(SettingsPage::Appearance);

        assert_eq!(app.page, SettingsPage::Appearance);
        assert_eq!(app.controller_page, SettingsPage::Appearance);
    }
}
