use std::{
    env,
    path::PathBuf,
    sync::{Arc, Mutex, mpsc},
    thread,
    time::Duration,
};

use crate::{
    desktop::Wallpaper,
    icons,
    launcher::Launcher,
    model::{Application, ApplicationId, OpenWindow, TrayItem, WindowId, WindowPreview},
    platform::{GlobalShortcut, ShellCommand, TraySource, WindowAction},
};

#[path = "linux_control.rs"]
mod linux_control;

pub fn wallpaper() -> Wallpaper {
    Wallpaper::default()
}

pub fn paste_text_if_requested(_: &str) -> Option<String> {
    None
}

pub fn capture_active_window() -> Result<(), String> {
    Err("active-window capture is not implemented on Linux".into())
}

pub fn capture_active_window_to_file() -> Result<(), String> {
    Err("active-window capture is not implemented on Linux".into())
}

pub fn capture_desktop() -> Result<super::DesktopCapture, String> {
    Err("desktop capture is not implemented on Linux".into())
}

pub fn copy_image_to_clipboard(_: &image::RgbaImage) -> Result<(), String> {
    Err("image clipboard support is not implemented on Linux".into())
}

pub fn copy_temp_image_path(_: &image::RgbaImage) -> Result<PathBuf, String> {
    Err("image clipboard support is not implemented on Linux".into())
}

pub fn network_status() -> super::NetworkStatus {
    linux_control::network_status()
}

pub fn set_wifi_enabled(enabled: bool) -> bool {
    linux_control::set_wifi_enabled(enabled)
}

pub fn activate_wifi_network(id: &str) -> bool {
    linux_control::activate_wifi_network(id)
}

pub fn bluetooth_status() -> super::BluetoothStatus {
    linux_control::bluetooth_status()
}

pub fn set_bluetooth_powered(powered: bool) -> bool {
    linux_control::set_bluetooth_powered(powered)
}

pub fn set_bluetooth_discovery(discovering: bool) -> bool {
    linux_control::set_bluetooth_discovery(discovering)
}

pub fn toggle_bluetooth_device(id: &str) -> bool {
    linux_control::toggle_bluetooth_device(id)
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

pub fn select_audio_device(_id: &str) -> bool {
    false
}

pub fn update_panel_fullscreen_state() {}

#[path = "../desktop_entries.rs"]
mod desktop_entries;

const SESSION_CONTROL_ENV: &str = "NICKEL_SESSION_CONTROL";
const STATUS_NOTIFIER_WATCHER: &str = "org.kde.StatusNotifierWatcher";
const STATUS_NOTIFIER_PATH: &str = "/StatusNotifierWatcher";
const STATUS_NOTIFIER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const TRAY_POLL_INTERVAL: Duration = Duration::from_millis(250);
const TRAY_RETRY_MAX: Duration = Duration::from_secs(60);

pub struct TrayFeed {
    items: Arc<Mutex<Vec<TrayItem>>>,
    actions: mpsc::Sender<(String, bool)>,
}

impl TrayFeed {
    pub fn new() -> Self {
        let items = Arc::new(Mutex::new(Vec::new()));
        let (actions, receiver) = mpsc::channel();
        let worker_items = items.clone();
        if let Err(error) = thread::Builder::new()
            .name("nickel-status-notifier".into())
            .spawn(move || tray_worker(worker_items, receiver))
        {
            tracing::warn!(%error, "failed to start status-notifier worker");
        }
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
        let _ = self.actions.send((id.to_owned(), false));
    }

    fn context_menu(&self, id: &str) {
        let _ = self.actions.send((id.to_owned(), true));
    }
}

fn tray_worker(items: Arc<Mutex<Vec<TrayItem>>>, actions: mpsc::Receiver<(String, bool)>) {
    let connection = match zbus::blocking::Connection::session() {
        Ok(connection) => connection,
        Err(error) => {
            tracing::warn!(%error, "status notifier could not connect to the session bus");
            return;
        }
    };
    let Ok(dbus) = zbus::blocking::fdo::DBusProxy::new(&connection) else {
        tracing::warn!("status notifier could not create a D-Bus daemon proxy");
        return;
    };
    let watcher_name: zbus::names::BusName<'_> = STATUS_NOTIFIER_WATCHER
        .try_into()
        .expect("status notifier watcher name is valid");
    let mut watcher = None;
    let mut failures = 0_u32;
    let mut next_attempt = std::time::Instant::now();
    loop {
        let now = std::time::Instant::now();
        if watcher.is_none() && now >= next_attempt {
            watcher = dbus
                .name_has_owner(watcher_name.clone())
                .ok()
                .filter(|owned| *owned)
                .and_then(|_| {
                    zbus::blocking::Proxy::new(
                        &connection,
                        STATUS_NOTIFIER_WATCHER,
                        STATUS_NOTIFIER_PATH,
                        STATUS_NOTIFIER_INTERFACE,
                    )
                    .ok()
                });
            if let Some(proxy) = &watcher {
                failures = 0;
                if let Some(name) = connection.unique_name()
                    && let Err(error) =
                        proxy.call_method("RegisterStatusNotifierHost", &(name.as_str()))
                {
                    tracing::warn!(%error, "failed to register Nickel as a status-notifier host");
                }
                tracing::info!("status-notifier watcher connected");
            } else {
                failures = failures.saturating_add(1);
                let delay = tray_retry_delay(failures);
                next_attempt = now + delay;
                if failures == 1 {
                    tracing::warn!(
                        retry_seconds = delay.as_secs(),
                        "status-notifier watcher is unavailable; tray will reconnect in the background"
                    );
                } else {
                    tracing::debug!(
                        retry_seconds = delay.as_secs(),
                        "status-notifier watcher remains unavailable"
                    );
                }
            }
        }
        if let Some(proxy) = &watcher {
            if let Some(snapshot) = read_tray_items(proxy) {
                if let Ok(mut current) = items.lock() {
                    *current = snapshot;
                }
            } else {
                watcher = None;
                failures = failures.saturating_add(1);
                next_attempt = now + tray_retry_delay(failures);
                if let Ok(mut current) = items.lock() {
                    current.clear();
                }
                tracing::warn!(
                    "status-notifier watcher stopped responding; tray will reconnect in the background"
                );
            }
        }
        while let Ok((id, context_menu)) = actions.try_recv() {
            activate_tray_item(&connection, &id, context_menu);
        }
        thread::sleep(TRAY_POLL_INTERVAL);
    }
}

fn tray_retry_delay(failures: u32) -> Duration {
    let exponent = failures.saturating_sub(1).min(6);
    Duration::from_secs(1_u64 << exponent).min(TRAY_RETRY_MAX)
}

fn read_tray_items(watcher: &zbus::blocking::Proxy<'_>) -> Option<Vec<TrayItem>> {
    let ids = watcher
        .get_property::<Vec<String>>("RegisteredStatusNotifierItems")
        .ok()?;
    Some(
        ids.into_iter()
            .filter_map(|id| read_tray_item(watcher.connection(), id))
            .collect(),
    )
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

fn activate_tray_item(connection: &zbus::blocking::Connection, id: &str, context_menu: bool) {
    let (service, path) = item_address(id);
    if let Ok(proxy) =
        zbus::blocking::Proxy::new(connection, service, path, "org.kde.StatusNotifierItem")
    {
        let method = if context_menu {
            "ContextMenu"
        } else {
            "Activate"
        };
        let _ = proxy.call_method(method, &(0_i32, 0_i32));
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
        ShellCommand::Show => "show-launcher".to_owned(),
        ShellCommand::Hide => "hide-launcher".to_owned(),
        ShellCommand::ShowContextMenu { x, width, height } => {
            format!("show-context-menu\t{x}\t{width}\t{height}")
        }
        ShellCommand::ShowPreview {
            x, width, height, ..
        } => {
            format!("show-preview\t{x}\t{width}\t{height}")
        }
        ShellCommand::HideContextMenu => "hide-context-menu".to_owned(),
        ShellCommand::HighlightWindow(window) => format!("highlight-window\t{}", window.0),
        ShellCommand::ClearWindowHighlight => "clear-window-highlight".to_owned(),
        ShellCommand::WindowAction { window, action } => match action {
            WindowAction::Activate => format!("activate-window\t{}", window.0),
            WindowAction::Close => format!("close-window\t{}", window.0),
            WindowAction::Maximize => format!("maximize-window\t{}", window.0),
            WindowAction::Minimize => format!("minimize-window\t{}", window.0),
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

pub fn show_window_system_menu(_: WindowId) -> bool {
    false
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

    pub fn launcher_visible(&self) -> Option<bool> {
        let socket = self.socket.as_ref()?;
        socket
            .send_to(b"launcher-visible", env::var_os(SESSION_CONTROL_ENV)?)
            .ok()?;
        let mut response = [0_u8; 1];
        (socket.recv(&mut response).ok()? == 1).then_some(response[0] == b'1')
    }

    pub fn preview(&self, window: WindowId) -> Option<WindowPreview> {
        let socket = self.socket.as_ref()?;
        socket
            .send_to(
                format!("get-preview\t{}", window.0).as_bytes(),
                env::var_os(SESSION_CONTROL_ENV)?,
            )
            .ok()?;
        let mut response = vec![0_u8; 256 * 144 * 4 + 12];
        for _ in 0..4 {
            let length = socket.recv(&mut response).ok()?;
            if length < 12 || u64::from_le_bytes(response[..8].try_into().ok()?) != window.0 {
                continue;
            }
            let width = u16::from_le_bytes([response[8], response[9]]) as u32;
            let height = u16::from_le_bytes([response[10], response[11]]) as u32;
            let image = image::RgbaImage::from_raw(width, height, response[12..length].to_vec())?;
            return Some(WindowPreview { window, image });
        }
        None
    }

    pub fn supports_previews(&self) -> bool {
        false
    }

    pub fn icon(&self, _: WindowId) -> Option<image::RgbaImage> {
        None
    }
}

pub fn launcher_hotkey_receiver() -> mpsc::Receiver<GlobalShortcut> {
    let (_sender, receiver) = mpsc::channel();
    receiver
}

pub fn handle_focused_shortcut(_: nickel_core::hotkeys::Hotkey, _: nickel_core::hotkeys::KeyEdge) {}

pub fn execute_run_command(command: &str) -> Result<(), super::LaunchError> {
    let mut process = std::process::Command::new("sh");
    process.arg("-c").arg(command);
    if let Some(home) = std::env::var_os("HOME") {
        process.current_dir(home);
    }
    process
        .spawn()
        .map(|_| ())
        .map_err(|error| super::LaunchError::Platform(error.to_string()))
}

pub fn launch_application(application: &Application) -> Result<Option<u32>, super::LaunchError> {
    application
        .launch()
        .map(|child| Some(child.id()))
        .map_err(|error| super::LaunchError::Platform(error.to_string()))
}

pub fn launcher_visibility_applied(_: bool) {}

pub fn launcher_has_foreground_focus() -> bool {
    false
}

pub fn application_icon(_: &str) -> Option<image::RgbaImage> {
    None
}

impl Drop for WindowFeed {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

fn parse_window(line: &str, launcher: &Launcher) -> Option<OpenWindow> {
    let mut fields = line.splitn(4, '\t');
    let id = WindowId(fields.next()?.parse().ok()?);
    let active = fields.next()? == "1";
    let native_app_id = fields.next()?;
    let title = fields.next().unwrap_or_default().to_owned();
    Some(OpenWindow {
        id,
        application_id: resolve_application_id(native_app_id, launcher),
        active,
        title,
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
    use std::time::Duration;

    use crate::{launcher::Launcher, model::Application};

    use super::{parse_window, pixmap_to_rgba, resolve_application_id, tray_retry_delay};

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

    #[test]
    fn status_notifier_retries_back_off_and_cap() {
        assert_eq!(tray_retry_delay(1), Duration::from_secs(1));
        assert_eq!(tray_retry_delay(2), Duration::from_secs(2));
        assert_eq!(tray_retry_delay(6), Duration::from_secs(32));
        assert_eq!(tray_retry_delay(7), Duration::from_secs(60));
        assert_eq!(tray_retry_delay(20), Duration::from_secs(60));
    }

    #[test]
    fn window_snapshot_keeps_title() {
        let launcher = Launcher::new(vec![Application::new(
            "org.kde.konsole.desktop".into(),
            "Konsole".into(),
            None,
            None,
            None,
        )]);
        let window = parse_window("7\t1\torg.kde.konsole\tProject shell", &launcher)
            .expect("valid window snapshot");
        assert_eq!(window.title, "Project shell");
        assert!(window.active);
    }
}
