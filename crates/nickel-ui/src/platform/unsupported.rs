use crate::{
    launcher::Launcher,
    model::{Application, OpenWindow, TrayItem, WindowId, WindowPreview},
    platform::{GlobalShortcut, ShellCommand, TraySource},
};

pub fn applications() -> Vec<Application> {
    Vec::new()
}

pub fn launcher_hotkey_receiver() -> std::sync::mpsc::Receiver<GlobalShortcut> {
    let (_sender, receiver) = std::sync::mpsc::channel();
    receiver
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
