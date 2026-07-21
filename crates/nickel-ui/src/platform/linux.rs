use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use crate::{
    icons,
    launcher::Launcher,
    model::{Application, ApplicationId, OpenWindow, TrayItem, WindowId},
    platform::{ShellCommand, TraySource, WindowAction},
};

#[path = "../desktop_entries.rs"]
mod desktop_entries;

const SESSION_CONTROL_ENV: &str = "NICKEL_SESSION_CONTROL";

pub struct TrayFeed {
    items: Arc<Mutex<Vec<TrayItem>>>,
    actions: mpsc::Sender<String>,
}

impl TrayFeed {
    pub fn new() -> Self {
        let items = Arc::new(Mutex::new(Vec::new()));
        let (actions, receiver) = mpsc::channel();
        let worker_items = items.clone();
        let _ = thread::Builder::new()
            .name("nickel-status-notifier".into())
            .spawn(move || tray_worker(worker_items, receiver));
        Self { items, actions }
    }
}

impl TraySource for TrayFeed {
    fn snapshot(&self) -> Vec<TrayItem> {
        self.items
            .lock()
            .map(|items| items.clone())
            .unwrap_or_default()
    }

    fn activate(&self, id: &str) {
        let _ = self.actions.send(id.to_owned());
    }
}

fn tray_worker(items: Arc<Mutex<Vec<TrayItem>>>, actions: mpsc::Receiver<String>) {
    let Ok(connection) = zbus::blocking::Connection::session() else {
        return;
    };
    if let (Some(name), Ok(watcher)) = (
        connection.unique_name(),
        zbus::blocking::Proxy::new(
            &connection,
            "org.kde.StatusNotifierWatcher",
            "/StatusNotifierWatcher",
            "org.kde.StatusNotifierWatcher",
        ),
    ) {
        let _ = watcher.call_method("RegisterStatusNotifierHost", &(name.as_str()));
    }
    loop {
        let snapshot = read_tray_items(&connection);
        if let Ok(mut current) = items.lock() {
            *current = snapshot;
        }
        while let Ok(id) = actions.try_recv() {
            activate_tray_item(&connection, &id);
        }
        thread::sleep(Duration::from_millis(250));
    }
}

fn read_tray_items(connection: &zbus::blocking::Connection) -> Vec<TrayItem> {
    let Ok(watcher) = zbus::blocking::Proxy::new(
        connection,
        "org.kde.StatusNotifierWatcher",
        "/StatusNotifierWatcher",
        "org.kde.StatusNotifierWatcher",
    ) else {
        return Vec::new();
    };
    let Ok(ids) = watcher.get_property::<Vec<String>>("RegisteredStatusNotifierItems") else {
        return Vec::new();
    };
    ids.into_iter()
        .filter_map(|id| read_tray_item(connection, id))
        .collect()
}

fn item_address(id: &str) -> (&str, &str) {
    id.find('/')
        .map_or((id, "/StatusNotifierItem"), |at| (&id[..at], &id[at..]))
}

fn read_tray_item(connection: &zbus::blocking::Connection, id: String) -> Option<TrayItem> {
    let (service, path) = item_address(&id);
    let proxy =
        zbus::blocking::Proxy::new(connection, service, path, "org.kde.StatusNotifierItem").ok()?;
    let title = proxy.get_property::<String>("Title").unwrap_or_default();
    let icon = proxy
        .get_property::<Vec<(i32, i32, Vec<u8>)>>("IconPixmap")
        .ok()
        .and_then(|pixmaps| {
            pixmaps
                .into_iter()
                .max_by_key(|(width, height, _)| width * height)
        })
        .and_then(pixmap_to_rgba)
        .or_else(|| {
            let name = proxy.get_property::<String>("IconName").ok()?;
            ["breeze-dark", "breeze", "hicolor", "Adwaita"]
                .into_iter()
                .find_map(|theme| {
                    freedesktop_icons::lookup(&name)
                        .with_size(32)
                        .with_theme(theme)
                        .with_cache()
                        .find()
                })
                .and_then(|path| icons::load(&path))
        })?;
    drop(proxy);
    Some(TrayItem { id, title, icon })
}

fn activate_tray_item(connection: &zbus::blocking::Connection, id: &str) {
    let (service, path) = item_address(id);
    if let Ok(proxy) =
        zbus::blocking::Proxy::new(connection, service, path, "org.kde.StatusNotifierItem")
    {
        let _ = proxy.call_method("Activate", &(0_i32, 0_i32));
    }
}

fn pixmap_to_rgba((width, height, bytes): (i32, i32, Vec<u8>)) -> Option<image::RgbaImage> {
    let (width, height) = (u32::try_from(width).ok()?, u32::try_from(height).ok()?);
    if bytes.len() != width as usize * height as usize * 4 {
        return None;
    }
    let mut rgba = Vec::with_capacity(bytes.len());
    for pixel in bytes.chunks_exact(4) {
        rgba.extend_from_slice(&[pixel[1], pixel[2], pixel[3], pixel[0]]);
    }
    image::RgbaImage::from_raw(width, height, rgba)
}

pub fn applications() -> Vec<Application> {
    desktop_entries::load_applications()
}

pub fn send_shell_command(command: ShellCommand) -> bool {
    use std::os::unix::net::UnixDatagram;

    let Some(path) = env::var_os(SESSION_CONTROL_ENV) else {
        return false;
    };
    let command = match command {
        ShellCommand::Toggle => "toggle-launcher".to_owned(),
        ShellCommand::Show => "show-launcher".to_owned(),
        ShellCommand::Hide => "hide-launcher".to_owned(),
        ShellCommand::ShowContextMenu { x } => format!("show-context-menu\t{x}"),
        ShellCommand::HideContextMenu => "hide-context-menu".to_owned(),
        ShellCommand::WindowAction { window, action } => match action {
            WindowAction::Close => format!("close-window\t{}", window.0),
        },
    };
    UnixDatagram::unbound()
        .and_then(|socket| socket.send_to(command.as_bytes(), path))
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

    use super::{pixmap_to_rgba, resolve_application_id};

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

    #[test]
    fn status_notifier_argb_pixmap_becomes_rgba() {
        let image = pixmap_to_rgba((1, 1, vec![128, 10, 20, 30])).expect("valid pixmap");
        assert_eq!(image.get_pixel(0, 0).0, [10, 20, 30, 128]);
    }
}
