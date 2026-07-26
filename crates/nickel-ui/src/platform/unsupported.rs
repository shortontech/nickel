use crate::{
    desktop::Wallpaper,
    launcher::Launcher,
    model::{Application, OpenWindow, TrayItem, WindowId, WindowPreview},
    platform::{GlobalShortcut, ShellCommand, TraySource},
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

pub fn audio_status() -> super::AudioStatus {
    super::AudioStatus::default()
}

pub fn set_audio_volume(_volume_percent: u8) -> bool {
    false
}

pub fn capture_pointer(_window: &winit::window::Window) -> bool {
    false
}

pub fn release_pointer() {}

pub fn configure_volume_osd_window(_window: &winit::window::Window) -> bool {
    true
}

pub fn select_audio_device(_id: &str) -> bool {
    false
}

pub fn update_panel_fullscreen_state() {}

pub fn applications() -> Vec<Application> {
    Vec::new()
}

pub fn launcher_hotkey_receiver() -> std::sync::mpsc::Receiver<GlobalShortcut> {
    let (_sender, receiver) = std::sync::mpsc::channel();
    receiver
}

pub fn handle_focused_shortcut(_: nickel_core::hotkeys::Hotkey, _: nickel_core::hotkeys::KeyEdge) {}

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

pub fn send_shell_command(_: ShellCommand) -> bool {
    false
}

pub struct WindowFeed;

impl WindowFeed {
    pub fn new() -> Self {
        Self
    }

    pub fn snapshot(&self, _: &Launcher) -> Option<Vec<OpenWindow>> {
        None
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
