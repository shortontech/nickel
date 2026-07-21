use crate::{
    launcher::Launcher,
    model::{Application, OpenWindow},
    platform::ShellCommand,
};

pub fn applications() -> Vec<Application> {
    Vec::new()
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
}
