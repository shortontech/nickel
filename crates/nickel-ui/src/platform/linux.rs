use std::{env, path::PathBuf, time::Duration};

use crate::{
    launcher::Launcher,
    model::{Application, ApplicationId, OpenWindow, WindowId},
    platform::ShellCommand,
};

#[path = "../desktop_entries.rs"]
mod desktop_entries;

const SESSION_CONTROL_ENV: &str = "NICKEL_SESSION_CONTROL";

pub fn applications() -> Vec<Application> {
    desktop_entries::load_applications()
}

pub fn send_shell_command(command: ShellCommand) -> bool {
    use std::os::unix::net::UnixDatagram;

    let Some(path) = env::var_os(SESSION_CONTROL_ENV) else {
        return false;
    };
    let command = match command {
        ShellCommand::Toggle => b"toggle-launcher".as_slice(),
        ShellCommand::Show => b"show-launcher".as_slice(),
        ShellCommand::Hide => b"hide-launcher".as_slice(),
    };
    UnixDatagram::unbound()
        .and_then(|socket| socket.send_to(command, path))
        .is_ok()
}

pub struct WindowFeed {
    socket: Option<std::os::unix::net::UnixDatagram>,
    path: PathBuf,
}

impl WindowFeed {
    pub fn new() -> Self {
        let path = env::temp_dir().join(format!("nickel-ui-{}-windows.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let socket = std::os::unix::net::UnixDatagram::bind(&path).ok();
        if let Some(socket) = &socket {
            let _ = socket.set_read_timeout(Some(Duration::from_millis(25)));
        }
        Self { socket, path }
    }

    pub fn snapshot(&self, launcher: &Launcher) -> Option<Vec<OpenWindow>> {
        let socket = self.socket.as_ref()?;
        socket
            .send_to(b"list-windows", env::var_os(SESSION_CONTROL_ENV)?)
            .ok()?;
        let mut response = [0_u8; 16 * 1024];
        let length = socket.recv(&mut response).ok()?;
        let text = std::str::from_utf8(&response[..length]).ok()?;
        Some(
            text.lines()
                .filter_map(|line| parse_window(line, launcher))
                .collect(),
        )
    }
}

impl Drop for WindowFeed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn parse_window(line: &str, launcher: &Launcher) -> Option<OpenWindow> {
    let mut fields = line.splitn(3, '\t');
    let id = WindowId(fields.next()?.parse().ok()?);
    let active = fields.next()? == "1";
    let native_app_id = fields.next()?;
    Some(OpenWindow {
        id,
        application_id: resolve_application_id(native_app_id, launcher),
        active,
    })
}

fn resolve_application_id(native_app_id: &str, launcher: &Launcher) -> Option<ApplicationId> {
    let native_app_id = native_app_id.trim_end_matches(".desktop");
    launcher
        .applications()
        .find(|application| {
            application
                .id()
                .trim_end_matches(".desktop")
                .eq_ignore_ascii_case(native_app_id)
        })
        .map(|application| application.application_id().clone())
}

#[cfg(test)]
mod tests {
    use crate::{launcher::Launcher, model::Application};

    use super::resolve_application_id;

    #[test]
    fn wayland_app_id_resolution_stays_in_linux_adapter() {
        let launcher = Launcher::new(vec![Application::new(
            "org.kde.konsole.desktop".into(),
            "Konsole".into(),
            None,
            None,
            None,
        )]);
        assert_eq!(
            resolve_application_id("org.kde.konsole", &launcher).map(|id| id.as_str().to_owned()),
            Some("org.kde.konsole.desktop".into())
        );
    }
}
