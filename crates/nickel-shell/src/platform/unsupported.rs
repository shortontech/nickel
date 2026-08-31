use crate::{
    desktop::Wallpaper,
    launcher::Launcher,
    model::{Application, ApplicationDiscovery, OpenWindow, TrayItem, WindowId, WindowPreview},
    platform::{FeedState, GlobalShortcut, NotificationSource, ShellCommand, TraySource},
};

pub fn wallpaper() -> Wallpaper {
    Wallpaper::default()
}

pub fn paste_text_if_requested(_: &str) -> Option<String> {
    None
}

pub fn network_status() -> super::NetworkStatus {
    super::NetworkStatus::default()
}

pub fn set_wifi_enabled(_enabled: bool) -> bool {
    false
}

pub fn activate_wifi_network(_id: &str) -> bool {
    false
}

pub fn bluetooth_status() -> super::BluetoothStatus {
    super::BluetoothStatus::default()
}

pub fn set_bluetooth_powered(_powered: bool) -> bool {
    false
}

pub fn set_bluetooth_discovery(_discovering: bool) -> bool {
    false
}

pub fn toggle_bluetooth_device(_id: &str) -> bool {
    false
}

pub fn audio_status() -> super::AudioStatus {
    super::AudioStatus::default()
}

pub fn set_audio_volume(_volume_percent: u8) -> bool {
    false
}

pub fn capture_pointer(_window: &sdl3::video::Window) -> bool {
    false
}

pub fn release_pointer() {}

pub fn configure_volume_osd_window(_window: &sdl3::video::Window) -> bool {
    true
}

pub fn select_audio_device(_id: &str) -> bool {
    false
}

pub fn update_panel_fullscreen_state() {}

pub fn applications() -> Vec<Application> {
    Vec::new()
}

pub fn application_discovery() -> ApplicationDiscovery {
    ApplicationDiscovery::ready(applications())
}

pub fn launcher_hotkey_receiver() -> super::GlobalShortcutFeed {
    super::GlobalShortcutFeed::unavailable(
        nickel_input::global::UnavailableReason::UnsupportedPlatform,
    )
}

pub fn handle_focused_shortcut(_: nickel_core::hotkeys::KeyCode, _: nickel_core::hotkeys::KeyEdge) {
}

pub fn execute_run_command(_: &str) -> Result<(), super::LaunchError> {
    Err(super::LaunchError::Platform(String::new()))
}

pub fn launch_application(_: &Application) -> Result<Option<u32>, super::LaunchError> {
    Err(super::LaunchError::Platform(String::new()))
}

pub fn launcher_visibility_applied(_: bool) {}

pub fn launcher_has_foreground_focus() -> bool {
    false
}

pub fn application_icon(_: &str) -> Option<image::RgbaImage> {
    None
}

pub struct TrayFeed;
impl TrayFeed {
    pub fn new() -> Self {
        Self
    }
}
impl TraySource for TrayFeed {
    fn snapshot(&self) -> Vec<TrayItem> {
        Vec::new()
    }
    fn activate(&self, _: &str) {}
    fn context_menu(&self, _: &str) {}
}

pub struct NotificationFeed;
impl NotificationFeed {
    pub fn new() -> Result<Self, String> {
        Ok(Self)
    }
}
impl NotificationSource for NotificationFeed {
    fn snapshot(&self) -> Option<crate::notification::DesktopNotification> {
        None
    }
    fn dismiss(&self, _: u32) {}
    fn invoke(&self, _: u32, _: &str) {}
}

pub fn send_shell_command(_: ShellCommand) -> bool {
    false
}

pub fn register_session_shell() -> Result<(), super::SessionRequestError> {
    Ok(())
}

pub struct WindowFeed;

pub fn show_window_system_menu(_: WindowId) -> bool {
    false
}

impl WindowFeed {
    pub fn launcher_visible(&self) -> Option<bool> {
        None
    }
    pub fn new() -> Self {
        Self
    }

    pub fn snapshot(&self, _: &Launcher) -> FeedState<Vec<OpenWindow>> {
        FeedState::Disconnected
    }

    pub fn workspaces(&self) -> FeedState<Vec<super::WorkspaceSummary>> {
        FeedState::Disconnected
    }

    pub fn preview(&self, _: WindowId) -> Option<WindowPreview> {
        None
    }

    pub fn supports_previews(&self) -> bool {
        false
    }

    pub fn icon(&self, _: WindowId) -> Option<image::RgbaImage> {
        None
    }
}
