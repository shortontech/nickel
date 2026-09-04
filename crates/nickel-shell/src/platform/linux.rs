use nickel_session_protocol::{
    ClientEnvelope, Command as SessionCommand, Event as SessionEvent, Geometry as SessionGeometry,
    Query as SessionQuery, Request as SessionRequest, SecureStorageState as SessionSecureStorage,
    ServerEnvelope, ServerMessage, ShellRole as SessionShellRole,
    WindowAction as SessionWindowAction, WindowId as SessionWindowId, decode as decode_session,
    encode as encode_session,
};
use std::{
    borrow::Cow,
    cell::RefCell,
    collections::HashMap,
    env,
    hash::{DefaultHasher, Hash, Hasher},
    io,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    desktop::Wallpaper,
    icons,
    launcher::Launcher,
    model::{
        Application, ApplicationDiscovery, ApplicationId, OpenWindow, TrayItem, WindowId,
        WindowPreview,
    },
    notification::{
        ClosedNotification, DesktopNotification, MAX_NOTIFICATION_ACTIONS, NotificationAction,
        NotificationRequest, NotificationStore,
    },
    platform::{
        FeedState, GlobalShortcut, NotificationSource, ScreenshotAction, SessionRequestError,
        ShellCommand, TraySource, WindowAction,
    },
};

#[path = "linux_audio.rs"]
mod linux_audio;
#[path = "linux_control.rs"]
mod linux_control;

pub fn wallpaper() -> Wallpaper {
    Wallpaper::default()
}

pub fn capture_active_window() -> Result<(), String> {
    let image = capture_active_window_image()?;
    copy_image_to_clipboard(&image)
}

pub fn capture_active_window_to_file() -> Result<(), String> {
    let image = capture_active_window_image()?;
    copy_temp_image_path(&image).map(|_| ())
}

pub fn capture_desktop() -> Result<super::DesktopCapture, String> {
    capture_output(None)
}

fn capture_output(output: Option<&str>) -> Result<super::DesktopCapture, String> {
    use std::os::unix::net::UnixDatagram;

    let server = env::var_os(SESSION_CONTROL_ENV)
        .ok_or_else(|| "Nickel session capture is unavailable".to_string())?;
    let token = env::var(SESSION_TOKEN_ENV)
        .map_err(|_| "Nickel session capture is not authorized".to_string())?;
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let sequence = SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let reply_path = runtime.join(format!(
        "nickel-shell-capture-reply-{}-{sequence}.sock",
        std::process::id()
    ));
    let image_path = runtime.join(format!(
        "nickel-shell-capture-{}-{sequence}.png",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&reply_path);
    let _ = std::fs::remove_file(&image_path);
    let result = (|| {
        let socket = UnixDatagram::bind(&reply_path)
            .map_err(|error| format!("could not create capture reply socket: {error}"))?;
        socket
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(|error| format!("could not configure capture timeout: {error}"))?;
        let request_id = SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let request = ClientEnvelope {
            token,
            request_id,
            request: SessionRequest::Command(SessionCommand::CaptureOutput {
                path: image_path.to_string_lossy().into_owned(),
                output: output.map(str::to_owned),
            }),
        };
        socket
            .send_to(
                &encode_session(&request).map_err(|error| error.to_string())?,
                server,
            )
            .map_err(|error| format!("could not request desktop capture: {error}"))?;
        let mut response = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
        let acknowledgement = receive_capture_response(&socket, &mut response, request_id)?;
        match acknowledgement {
            ServerMessage::Ack => {}
            ServerMessage::Error { message, .. } => return Err(message),
            _ => return Err("session returned an invalid capture acknowledgement".into()),
        }
        let completion = receive_capture_response(&socket, &mut response, request_id)?;
        match completion {
            ServerMessage::Event(SessionEvent::OutputCaptureCompleted {
                path,
                result: nickel_session_protocol::CaptureResult::Saved { .. },
            }) if Path::new(&path) == image_path => {}
            ServerMessage::Event(SessionEvent::OutputCaptureCompleted {
                result: nickel_session_protocol::CaptureResult::Failed { message },
                ..
            }) => return Err(message),
            _ => return Err("session returned an invalid capture completion".into()),
        }
        let image = image::open(&image_path)
            .map_err(|error| format!("could not read captured desktop pixels: {error}"))?
            .into_rgba8();
        Ok(super::DesktopCapture { image })
    })();
    let _ = std::fs::remove_file(reply_path);
    let _ = std::fs::remove_file(image_path);
    result
}

fn capture_active_window_image() -> Result<image::RgbaImage, String> {
    let snapshot = session_snapshot()?;
    let focused = snapshot
        .focused
        .ok_or_else(|| "Nickel has no focused application window".to_string())?;
    let window = snapshot
        .windows
        .iter()
        .find(|window| window.id == focused)
        .ok_or_else(|| "the focused application is no longer available".to_string())?;
    let geometry = window
        .geometry
        .ok_or_else(|| "the focused application has no capturable geometry".to_string())?;
    let output = snapshot
        .outputs
        .iter()
        .filter(|output| output.enabled)
        .max_by_key(|output| intersection_area(geometry, output.geometry))
        .filter(|output| intersection_area(geometry, output.geometry) > 0)
        .ok_or_else(|| "the focused application is not on a capturable output".to_string())?;
    let capture = capture_output(Some(&output.name))?;
    crop_output_geometry(capture.image, output.geometry, geometry)
}

fn session_snapshot() -> Result<nickel_session_protocol::Snapshot, String> {
    use std::os::unix::net::UnixDatagram;

    let server = env::var_os(SESSION_CONTROL_ENV)
        .ok_or_else(|| "Nickel session capture is unavailable".to_string())?;
    let token = env::var(SESSION_TOKEN_ENV)
        .map_err(|_| "Nickel session capture is not authorized".to_string())?;
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let sequence = SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let reply_path = runtime.join(format!(
        "nickel-shell-snapshot-reply-{}-{sequence}.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&reply_path);
    let result = (|| {
        let socket = UnixDatagram::bind(&reply_path)
            .map_err(|error| format!("could not create snapshot reply socket: {error}"))?;
        socket
            .set_read_timeout(Some(Duration::from_secs(5)))
            .map_err(|error| format!("could not configure snapshot timeout: {error}"))?;
        let request_id = SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
        let request = ClientEnvelope {
            token,
            request_id,
            request: SessionRequest::Query(SessionQuery::Snapshot),
        };
        socket
            .send_to(
                &encode_session(&request).map_err(|error| error.to_string())?,
                server,
            )
            .map_err(|error| format!("could not query active window: {error}"))?;
        let mut response = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
        match receive_capture_response(&socket, &mut response, request_id)? {
            ServerMessage::Snapshot(snapshot) => Ok(snapshot),
            ServerMessage::Error { message, .. } => Err(message),
            _ => Err("session returned an invalid snapshot response".into()),
        }
    })();
    let _ = std::fs::remove_file(reply_path);
    result
}

fn intersection_area(
    left: nickel_session_protocol::Geometry,
    right: nickel_session_protocol::Geometry,
) -> i64 {
    let width = (left.x + left.width).min(right.x + right.width) - left.x.max(right.x);
    let height = (left.y + left.height).min(right.y + right.height) - left.y.max(right.y);
    i64::from(width.max(0)) * i64::from(height.max(0))
}

fn crop_output_geometry(
    image: image::RgbaImage,
    output: nickel_session_protocol::Geometry,
    window: nickel_session_protocol::Geometry,
) -> Result<image::RgbaImage, String> {
    if output.width <= 0 || output.height <= 0 {
        return Err("capture output has invalid geometry".into());
    }
    let left = window.x.max(output.x);
    let top = window.y.max(output.y);
    let right = (window.x + window.width).min(output.x + output.width);
    let bottom = (window.y + window.height).min(output.y + output.height);
    if right <= left || bottom <= top {
        return Err("focused application does not intersect its capture output".into());
    }
    let scale_x = image.width() as f64 / output.width as f64;
    let scale_y = image.height() as f64 / output.height as f64;
    let x = (((left - output.x) as f64 * scale_x).round() as u32).min(image.width() - 1);
    let y = (((top - output.y) as f64 * scale_y).round() as u32).min(image.height() - 1);
    let width = ((right - left) as f64 * scale_x).round().max(1.0) as u32;
    let height = ((bottom - top) as f64 * scale_y).round().max(1.0) as u32;
    Ok(image::imageops::crop_imm(
        &image,
        x,
        y,
        width.min(image.width().saturating_sub(x)),
        height.min(image.height().saturating_sub(y)),
    )
    .to_image())
}

fn receive_capture_response(
    socket: &std::os::unix::net::UnixDatagram,
    buffer: &mut [u8],
    request_id: u64,
) -> Result<ServerMessage, String> {
    loop {
        let length = socket
            .recv(buffer)
            .map_err(|error| format!("desktop capture response failed: {error}"))?;
        let response = decode_session::<ServerEnvelope>(&buffer[..length])
            .map_err(|error| format!("desktop capture response was invalid: {error}"))?;
        if response.request_id == request_id {
            return Ok(response.message);
        }
    }
}

pub fn copy_image_to_clipboard(image: &image::RgbaImage) -> Result<(), String> {
    let mut clipboard = wayland_clipboard()?;
    clipboard
        .set_image(arboard::ImageData {
            width: image.width() as usize,
            height: image.height() as usize,
            bytes: Cow::Borrowed(image.as_raw()),
        })
        .map_err(|error| format!("could not copy screenshot pixels: {error}"))
}

pub fn copy_temp_image_path(image: &image::RgbaImage) -> Result<PathBuf, String> {
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let sequence = SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let path = runtime.join(format!(
        "nickel-screenshot-{}-{sequence}.png",
        std::process::id()
    ));
    image
        .save(&path)
        .map_err(|error| format!("could not save temporary screenshot: {error}"))?;
    let mut clipboard = wayland_clipboard()?;
    if let Err(error) = clipboard.set_text(path.to_string_lossy()) {
        let _ = std::fs::remove_file(&path);
        return Err(format!("could not copy temporary screenshot path: {error}"));
    }
    Ok(path)
}

fn wayland_clipboard() -> Result<std::sync::MutexGuard<'static, arboard::Clipboard>, String> {
    static CLIPBOARD: OnceLock<Result<Mutex<arboard::Clipboard>, String>> = OnceLock::new();
    let clipboard = CLIPBOARD.get_or_init(|| {
        arboard::Clipboard::new()
            .map(Mutex::new)
            .map_err(|error| format!("could not connect to the Wayland clipboard: {error}"))
    });
    clipboard
        .as_ref()
        .map_err(Clone::clone)?
        .lock()
        .map_err(|_| "Wayland clipboard state is unavailable".into())
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
    linux_audio::status()
}

pub fn set_audio_volume(_volume_percent: u8) -> bool {
    linux_audio::set_volume(_volume_percent)
}

pub fn prepare_audio_environment() {
    if std::env::var_os("SPA_PLUGIN_DIR").is_some() {
        return;
    }
    let candidates = [
        "/usr/lib/x86_64-linux-gnu/spa-0.2",
        "/usr/lib/aarch64-linux-gnu/spa-0.2",
        "/usr/lib64/spa-0.2",
        "/usr/lib/spa-0.2",
    ];
    if let Some(path) = candidates
        .into_iter()
        .find(|path| std::path::Path::new(path).is_dir())
    {
        // SAFETY: called at the beginning of main, before Nickel starts worker threads.
        unsafe { std::env::set_var("SPA_PLUGIN_DIR", path) };
    }
}

pub fn handle_consumer_control(control: nickel_session_protocol::ConsumerControl) {
    use nickel_session_protocol::ConsumerControl;
    tracing::info!(?control, "handling Linux consumer control");
    match control {
        ConsumerControl::VolumeUp => {
            let _ = linux_audio::adjust_volume(5);
        }
        ConsumerControl::VolumeDown => {
            let _ = linux_audio::adjust_volume(-5);
        }
        ConsumerControl::VolumeMute => {
            let _ = linux_audio::toggle_mute();
        }
        _ => linux_control::handle_consumer_control(control),
    }
}

pub fn capture_pointer(_window: &impl raw_window_handle::HasWindowHandle) -> bool {
    false
}

pub fn release_pointer() {}

pub fn select_audio_device(_id: &str) -> bool {
    linux_audio::select_output(_id)
}

pub fn update_panel_fullscreen_state() {}

#[path = "../desktop_entries.rs"]
mod desktop_entries;

const SESSION_CONTROL_ENV: &str = "NICKEL_SESSION_CONTROL";
const SESSION_TOKEN_ENV: &str = "NICKEL_SESSION_TOKEN";
const SHELL_TEST_CONTROL_ENV: &str = "NICKEL_SHELL_TEST_CONTROL";
static SESSION_REQUEST_ID: AtomicU64 = AtomicU64::new(1);
const STATUS_NOTIFIER_WATCHER: &str = "org.kde.StatusNotifierWatcher";
const STATUS_NOTIFIER_PATH: &str = "/StatusNotifierWatcher";
const STATUS_NOTIFIER_INTERFACE: &str = "org.kde.StatusNotifierWatcher";
const TRAY_POLL_INTERVAL: Duration = Duration::from_secs(1);
const TRAY_RETRY_MAX: Duration = Duration::from_secs(60);
const NOTIFICATION_SERVICE: &str = "org.freedesktop.Notifications";
const NOTIFICATION_PATH: &str = "/org/freedesktop/Notifications";

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

pub struct NotificationFeed {
    store: Arc<Mutex<NotificationStore>>,
    connection: zbus::blocking::Connection,
}

impl NotificationFeed {
    pub fn new() -> Result<Self, String> {
        let store = Arc::new(Mutex::new(NotificationStore::default()));
        let connection = zbus::blocking::connection::Builder::session()
            .map_err(|error| error.to_string())?
            .serve_at(
                NOTIFICATION_PATH,
                NotificationService {
                    store: store.clone(),
                },
            )
            .map_err(|error| error.to_string())?
            .build()
            .map_err(|error| error.to_string())?;
        let owns_name = notification_name_owned(connection.request_name_with_flags(
            NOTIFICATION_SERVICE,
            zbus::fdo::RequestNameFlags::DoNotQueue.into(),
        ))?;
        if owns_name {
            let worker_store = store.clone();
            let worker_connection = connection.clone();
            thread::Builder::new()
                .name("nickel-notification-expiry".into())
                .spawn(move || notification_expiry_worker(worker_store, worker_connection))
                .map_err(|error| error.to_string())?;
            tracing::info!("Nickel notification daemon owns org.freedesktop.Notifications");
        } else {
            tracing::warn!(
                "another notification daemon is active; continuing without the Nickel notification feed"
            );
        }
        Ok(Self { store, connection })
    }
}

fn notification_name_owned(
    result: Result<zbus::fdo::RequestNameReply, zbus::Error>,
) -> Result<bool, String> {
    match result {
        Ok(
            zbus::fdo::RequestNameReply::PrimaryOwner | zbus::fdo::RequestNameReply::AlreadyOwner,
        ) => Ok(true),
        Ok(zbus::fdo::RequestNameReply::Exists | zbus::fdo::RequestNameReply::InQueue)
        | Err(zbus::Error::NameTaken) => Ok(false),
        Err(error) => Err(error.to_string()),
    }
}

impl NotificationSource for NotificationFeed {
    fn snapshot(&self) -> Option<DesktopNotification> {
        self.store.lock().ok()?.newest()
    }

    fn dismiss(&self, id: u32) {
        let closed = self
            .store
            .lock()
            .ok()
            .and_then(|mut store| store.close(id, 2));
        if let Some(closed) = closed {
            emit_notification_closed(&self.connection, closed);
        }
    }

    fn invoke(&self, id: u32, action_key: &str) {
        let known = self
            .store
            .lock()
            .is_ok_and(|store| store.has_action(id, action_key));
        if !known {
            return;
        }
        if let Err(error) = self.connection.emit_signal(
            None::<&str>,
            NOTIFICATION_PATH,
            NOTIFICATION_SERVICE,
            "ActionInvoked",
            &(id, action_key),
        ) {
            tracing::warn!(%error, id, action_key, "failed to emit ActionInvoked");
        }
    }
}

struct NotificationService {
    store: Arc<Mutex<NotificationStore>>,
}

#[zbus::interface(name = "org.freedesktop.Notifications")]
impl NotificationService {
    fn get_capabilities(&self) -> Vec<String> {
        vec!["actions".into(), "body".into(), "persistence".into()]
    }

    fn get_server_information(&self) -> (String, String, String, String) {
        (
            "Nickel".into(),
            "Nickel".into(),
            env!("CARGO_PKG_VERSION").into(),
            "1.2".into(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    async fn notify(
        &self,
        app_name: String,
        replaces_id: u32,
        _app_icon: String,
        summary: String,
        body: String,
        actions: Vec<String>,
        _hints: std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
        expire_timeout: i32,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> u32 {
        let (id, discarded) = {
            let Ok(mut store) = self.store.lock() else {
                return 0;
            };
            store.notify(
                replaces_id,
                NotificationRequest {
                    app_name: bounded_notification_text(app_name, 256),
                    summary: bounded_notification_text(summary, 512),
                    body: bounded_notification_text(body, 4_096),
                    actions: notification_actions(actions),
                    expire_timeout_ms: expire_timeout,
                },
                std::time::Instant::now(),
            )
        };
        if let Some(discarded) = discarded
            && let Err(error) =
                Self::notification_closed(&emitter, discarded.id, discarded.reason).await
        {
            tracing::warn!(
                %error,
                id = discarded.id,
                "failed to emit NotificationClosed"
            );
        }
        id
    }

    async fn close_notification(
        &self,
        id: u32,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) {
        let closed = {
            self.store
                .lock()
                .ok()
                .and_then(|mut store| store.close(id, 3))
        };
        if let Some(closed) = closed
            && let Err(error) = Self::notification_closed(&emitter, closed.id, closed.reason).await
        {
            tracing::warn!(%error, id = closed.id, "failed to emit NotificationClosed");
        }
    }

    #[zbus(signal)]
    async fn notification_closed(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        reason: u32,
    ) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn action_invoked(
        emitter: &zbus::object_server::SignalEmitter<'_>,
        id: u32,
        action_key: &str,
    ) -> zbus::Result<()>;
}

fn notification_actions(values: Vec<String>) -> Vec<NotificationAction> {
    values
        .chunks_exact(2)
        .filter_map(|pair| {
            let key = pair[0].trim();
            let label = pair[1].trim();
            (!key.is_empty() && !label.is_empty()).then(|| NotificationAction {
                key: key.chars().take(128).collect(),
                label: label.chars().take(128).collect(),
            })
        })
        .take(MAX_NOTIFICATION_ACTIONS)
        .collect()
}

fn bounded_notification_text(value: String, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        value
    } else {
        value.chars().take(max_chars).collect()
    }
}

fn notification_expiry_worker(
    store: Arc<Mutex<NotificationStore>>,
    connection: zbus::blocking::Connection,
) {
    loop {
        let expired = store
            .lock()
            .map(|mut store| store.expire(std::time::Instant::now()))
            .unwrap_or_default();
        for closed in expired {
            emit_notification_closed(&connection, closed);
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn emit_notification_closed(connection: &zbus::blocking::Connection, closed: ClosedNotification) {
    if let Err(error) = connection.emit_signal(
        None::<&str>,
        NOTIFICATION_PATH,
        NOTIFICATION_SERVICE,
        "NotificationClosed",
        &(closed.id, closed.reason),
    ) {
        tracing::warn!(%error, id = closed.id, "failed to emit NotificationClosed");
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
    let mut icon_cache = HashMap::new();
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
            if let Some(snapshot) = read_tray_items(proxy, &mut icon_cache) {
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
                icon_cache.clear();
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

#[derive(Clone, Debug, Eq, PartialEq)]
enum TrayIconKey {
    Named(String),
    Application(String, String),
    Pixmap { width: i32, height: i32, hash: u64 },
}

struct CachedTrayIcon {
    key: TrayIconKey,
    icon: image::RgbaImage,
}

fn read_tray_items(
    watcher: &zbus::blocking::Proxy<'_>,
    icon_cache: &mut HashMap<String, CachedTrayIcon>,
) -> Option<Vec<TrayItem>> {
    let ids = watcher
        .get_property::<Vec<String>>("RegisteredStatusNotifierItems")
        .ok()?;
    let snapshot = ids
        .into_iter()
        .filter_map(|id| read_tray_item(watcher.connection(), id, icon_cache))
        .collect::<Vec<_>>();
    icon_cache.retain(|id, _| snapshot.iter().any(|item| item.id == *id));
    Some(snapshot)
}

fn item_address(id: &str) -> (&str, &str) {
    id.find('/')
        .map_or((id, "/StatusNotifierItem"), |at| (&id[..at], &id[at..]))
}

fn read_tray_item(
    connection: &zbus::blocking::Connection,
    id: String,
    icon_cache: &mut HashMap<String, CachedTrayIcon>,
) -> Option<TrayItem> {
    let (service, path) = item_address(&id);
    let proxy =
        zbus::blocking::Proxy::new(connection, service, path, "org.kde.StatusNotifierItem").ok()?;
    type StatusNotifierToolTip = (String, Vec<(i32, i32, Vec<u8>)>, String, String);

    let title = proxy.get_property::<String>("Title").unwrap_or_default();
    let tooltip_title = proxy
        .get_property::<StatusNotifierToolTip>("ToolTip")
        .map(|(_, _, title, _)| title)
        .unwrap_or_default();
    let icon_name = proxy.get_property::<String>("IconName").unwrap_or_default();
    let named_key = TrayIconKey::Named(icon_name.clone());
    let application_key = TrayIconKey::Application(title.clone(), tooltip_title.clone());
    let icon = cached_tray_icon(icon_cache, &id, &named_key)
        .or_else(|| {
            resolve_status_icon_name(&icon_name).inspect(|icon| {
                cache_tray_icon(icon_cache, &id, named_key.clone(), icon.clone());
            })
        })
        .or_else(|| cached_tray_icon(icon_cache, &id, &application_key))
        .or_else(|| {
            resolve_status_application_icon(&title, &tooltip_title).inspect(|icon| {
                cache_tray_icon(icon_cache, &id, application_key.clone(), icon.clone());
            })
        })
        .or_else(|| {
            let pixmap = proxy
                .get_property::<Vec<(i32, i32, Vec<u8>)>>("IconPixmap")
                .ok()?
                .into_iter()
                .max_by_key(|(width, height, _)| width * height)?;
            let mut hasher = DefaultHasher::new();
            pixmap.hash(&mut hasher);
            let key = TrayIconKey::Pixmap {
                width: pixmap.0,
                height: pixmap.1,
                hash: hasher.finish(),
            };
            cached_tray_icon(icon_cache, &id, &key).or_else(|| {
                pixmap_to_rgba(pixmap).inspect(|icon| {
                    cache_tray_icon(icon_cache, &id, key, icon.clone());
                })
            })
        })?;
    drop(proxy);
    Some(TrayItem { id, title, icon })
}

fn cached_tray_icon(
    cache: &HashMap<String, CachedTrayIcon>,
    id: &str,
    key: &TrayIconKey,
) -> Option<image::RgbaImage> {
    cache
        .get(id)
        .filter(|cached| cached.key == *key)
        .map(|cached| cached.icon.clone())
}

fn cache_tray_icon(
    cache: &mut HashMap<String, CachedTrayIcon>,
    id: &str,
    key: TrayIconKey,
    icon: image::RgbaImage,
) {
    cache.insert(id.to_owned(), CachedTrayIcon { key, icon });
}

fn resolve_status_icon_name(name: &str) -> Option<image::RgbaImage> {
    (!name.trim().is_empty())
        .then_some(())
        .and_then(|()| {
            ["breeze-dark", "breeze", "hicolor", "Adwaita"]
                .into_iter()
                .find_map(|theme| {
                    freedesktop_icons::lookup(name)
                        .with_size(32)
                        .with_theme(theme)
                        .with_cache()
                        .find()
                })
        })
        .and_then(|path| icons::load(&path))
}

fn resolve_status_application_icon(title: &str, tooltip_title: &str) -> Option<image::RgbaImage> {
    static APPLICATIONS: OnceLock<Vec<Application>> = OnceLock::new();
    let applications =
        APPLICATIONS.get_or_init(|| desktop_entries::load_applications().into_applications());
    [title, tooltip_title]
        .into_iter()
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .find_map(|label| {
            applications
                .iter()
                .find(|application| application.name().eq_ignore_ascii_case(label))
                .and_then(|application| application.icon_path().and_then(icons::load))
        })
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
    application_discovery().into_applications()
}

pub fn application_discovery() -> ApplicationDiscovery {
    desktop_entries::load_applications()
}

fn session_request_on(
    socket: &std::os::unix::net::UnixDatagram,
    request: SessionRequest,
) -> Result<ServerMessage, SessionRequestError> {
    let request_id = SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    session_request_on_with_id(socket, request, request_id)
}

fn session_request_on_with_id(
    socket: &std::os::unix::net::UnixDatagram,
    request: SessionRequest,
    request_id: u64,
) -> Result<ServerMessage, SessionRequestError> {
    let server =
        env::var_os(SESSION_CONTROL_ENV).ok_or(SessionRequestError::MissingControlSocket)?;
    let token =
        env::var(SESSION_TOKEN_ENV).map_err(|_| SessionRequestError::MissingSessionToken)?;
    let envelope = ClientEnvelope {
        token,
        request_id,
        request,
    };
    let encoded = encode_session(&envelope).map_err(|_| SessionRequestError::Encoding)?;
    socket
        .send_to(&encoded, server)
        .map_err(|_| SessionRequestError::Send)?;
    let mut response = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
    loop {
        let length = socket.recv(&mut response).map_err(session_receive_error)?;
        let response = decode_session::<ServerEnvelope>(&response[..length])
            .map_err(|_| SessionRequestError::Decoding)?;
        if let Some(message) = response_for_request(response, request_id) {
            return response_message(message);
        }
    }
}

fn session_receive_error(error: io::Error) -> SessionRequestError {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => SessionRequestError::ReceiveTimeout,
        _ => SessionRequestError::Receive,
    }
}

fn response_message(message: ServerMessage) -> Result<ServerMessage, SessionRequestError> {
    match message {
        ServerMessage::Error {
            code: nickel_session_protocol::ErrorCode::Unauthorized,
            message,
        } => Err(SessionRequestError::Authorization {
            message: bounded_protocol_message(message),
        }),
        ServerMessage::Error { code, message } => Err(SessionRequestError::ServerRejection {
            code,
            message: bounded_protocol_message(message),
        }),
        message => Ok(message),
    }
}

const MAX_PROTOCOL_ERROR_MESSAGE_CHARS: usize = 256;

fn bounded_protocol_message(message: String) -> String {
    let mut characters = message.chars().filter(|character| !character.is_control());
    let mut bounded = characters
        .by_ref()
        .take(MAX_PROTOCOL_ERROR_MESSAGE_CHARS)
        .collect::<String>();
    if characters.next().is_some() {
        bounded.push('…');
    }
    bounded
}

fn response_for_request(response: ServerEnvelope, request_id: u64) -> Option<ServerMessage> {
    (response.request_id == request_id).then_some(response.message)
}

fn one_shot_session_request(request: SessionRequest) -> Result<ServerMessage, SessionRequestError> {
    use std::os::unix::net::UnixDatagram;

    let operation = session_request_operation(&request);
    let request_id = SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let started = Instant::now();
    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let path = runtime.join(format!("nickel-{}-{}.sock", std::process::id(), request_id));
    let _ = std::fs::remove_file(&path);
    let result = (|| {
        let socket = UnixDatagram::bind(&path).map_err(|_| SessionRequestError::SocketBind)?;
        socket
            .set_read_timeout(Some(Duration::from_millis(250)))
            .map_err(|_| SessionRequestError::SocketConfiguration)?;
        session_request_on_with_id(&socket, request, request_id)
    })();
    let _ = std::fs::remove_file(path);
    let elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0;
    match &result {
        Ok(_) => tracing::debug!(
            operation,
            correlation_id = request_id,
            elapsed_ms,
            retry_disposition = "none",
            socket_identity = "session-control-env",
            "session request completed"
        ),
        Err(error) if session_request_failure_log_admitted(operation, error.stage()) => {
            tracing::warn!(
                operation,
                correlation_id = request_id,
                stage = error.stage(),
                elapsed_ms,
                retry_disposition = "caller-policy",
                socket_identity = "session-control-env",
                error = %error,
                "session request failed"
            );
        }
        Err(_) => {}
    }
    result
}

fn session_request_failure_log_admitted(operation: &'static str, stage: &'static str) -> bool {
    static LAST_FAILURES: OnceLock<Mutex<HashMap<(&'static str, &'static str), Instant>>> =
        OnceLock::new();
    let Ok(mut failures) = LAST_FAILURES
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
    else {
        return false;
    };
    let now = Instant::now();
    let admitted = failures
        .get(&(operation, stage))
        .is_none_or(|previous| now.duration_since(*previous) >= Duration::from_secs(30));
    if admitted {
        failures.insert((operation, stage), now);
    }
    admitted
}

fn session_request_operation(request: &SessionRequest) -> &'static str {
    match request {
        SessionRequest::RegisterShell { .. } => "register-shell",
        SessionRequest::Subscribe => "subscribe",
        SessionRequest::Query(query) => match query {
            SessionQuery::Snapshot => "query-snapshot",
            SessionQuery::Windows => "query-windows",
            SessionQuery::Outputs => "query-outputs",
            SessionQuery::ShellSurfaces => "query-shell-surfaces",
            SessionQuery::ShellReadiness => "query-shell-readiness",
            SessionQuery::LauncherVisibility => "query-launcher-visibility",
            SessionQuery::SecureStorage => "query-secure-storage",
            SessionQuery::IdleInhibition => "query-idle-inhibition",
            SessionQuery::CacheDiagnostics => "query-cache-diagnostics",
            SessionQuery::Workspaces => "query-workspaces",
            SessionQuery::Preview { .. } => "query-preview",
            SessionQuery::ShellSemanticTarget { .. } => "query-shell-semantic-target",
            SessionQuery::ShellRuntimeDiagnostics => "query-shell-runtime-diagnostics",
        },
        SessionRequest::Command(command) => match command {
            SessionCommand::ReloadShellSettings => "reload-shell-settings",
            SessionCommand::ToggleLauncher => "toggle-launcher",
            SessionCommand::SetLauncherVisible { .. } => "set-launcher-visible",
            SessionCommand::SetLauncherVisibleFromController { .. } => {
                "set-launcher-visible-from-controller"
            }
            SessionCommand::SetShellRoleVisible { .. } => "set-shell-role-visible",
            SessionCommand::ShowAnchoredShellRole { .. } => "show-anchored-shell-role",
            SessionCommand::LogOut => "log-out",
            SessionCommand::SessionAction { .. } => "session-action",
            SessionCommand::Unlock => "unlock",
            SessionCommand::RetrySecureStorage => "retry-secure-storage",
            SessionCommand::HideOverlay => "hide-overlay",
            SessionCommand::ShowOverlay { .. } => "show-overlay",
            SessionCommand::FocusShellRole { .. } => "focus-shell-role",
            SessionCommand::RestoreApplicationFocus => "restore-application-focus",
            SessionCommand::IdentifyOutputs => "identify-outputs",
            SessionCommand::CaptureOutput { .. } => "capture-output",
            SessionCommand::ApplyOutputs { .. } => "apply-outputs",
            SessionCommand::CreateWorkspace => "create-workspace",
            SessionCommand::RemoveWorkspace { .. } => "remove-workspace",
            SessionCommand::SwitchWorkspace { .. } => "switch-workspace",
            SessionCommand::MoveWindowToWorkspace { .. } => "move-window-to-workspace",
            SessionCommand::MoveWindowToOutput { .. } => "move-window-to-output",
            SessionCommand::HighlightWindow { .. } => "highlight-window",
            SessionCommand::WindowAction { .. } => "window-action",
            SessionCommand::TestInput { .. } => "test-input",
            SessionCommand::TestOutput { .. } => "test-output",
        },
    }
}

pub fn register_session_shell() -> Result<(), SessionRequestError> {
    match one_shot_session_request(SessionRequest::RegisterShell {
        pid: std::process::id(),
    })? {
        ServerMessage::Snapshot(_) => Ok(()),
        _ => Err(SessionRequestError::UnexpectedResponse {
            expected: "registration snapshot",
        }),
    }
}

pub fn configured_primary_output() -> Option<String> {
    match one_shot_session_request(SessionRequest::Query(SessionQuery::Outputs)) {
        Ok(ServerMessage::Outputs(outputs)) => outputs
            .into_iter()
            .find(|output| output.primary)
            .map(|output| output.name),
        _ => None,
    }
}

pub fn shell_readiness()
-> Result<nickel_session_protocol::ShellReadinessSnapshot, SessionRequestError> {
    match one_shot_session_request(SessionRequest::Query(SessionQuery::ShellReadiness))? {
        ServerMessage::ShellReadiness(snapshot) => Ok(snapshot),
        _ => Err(SessionRequestError::UnexpectedResponse {
            expected: "shell readiness snapshot",
        }),
    }
}

pub fn secure_storage_state() -> Result<super::SecureStorageState, SessionRequestError> {
    secure_storage_response(one_shot_session_request(SessionRequest::Query(
        SessionQuery::SecureStorage,
    ))?)
}

fn secure_storage_response(
    response: ServerMessage,
) -> Result<super::SecureStorageState, SessionRequestError> {
    match response {
        ServerMessage::SecureStorage { state, reason } => Ok(match state {
            SessionSecureStorage::Starting => super::SecureStorageState::Starting,
            SessionSecureStorage::Locked => super::SecureStorageState::Locked,
            SessionSecureStorage::PromptRequired => super::SecureStorageState::PromptRequired,
            SessionSecureStorage::Ready => super::SecureStorageState::Ready,
            SessionSecureStorage::Unavailable => reason.map_or(
                super::SecureStorageState::Unavailable,
                super::SecureStorageState::UnavailableReason,
            ),
        }),
        _ => Err(SessionRequestError::UnexpectedResponse {
            expected: "secure-storage state",
        }),
    }
}

pub fn request_secure_storage_retry() -> Result<(), SessionRequestError> {
    secure_storage_retry_response(one_shot_session_request(SessionRequest::Command(
        SessionCommand::RetrySecureStorage,
    ))?)
}

pub fn send_shell_command(command: ShellCommand) -> Result<(), SessionRequestError> {
    command_response(one_shot_session_request(SessionRequest::Command(
        shell_command_payload(command),
    ))?)
}

fn secure_storage_retry_response(response: ServerMessage) -> Result<(), SessionRequestError> {
    match response {
        ServerMessage::Ack => Ok(()),
        _ => Err(SessionRequestError::UnexpectedResponse {
            expected: "secure-storage retry acknowledgement",
        }),
    }
}

fn command_response(response: ServerMessage) -> Result<(), SessionRequestError> {
    match response {
        ServerMessage::Ack | ServerMessage::Workspaces(_) => Ok(()),
        _ => Err(SessionRequestError::UnexpectedResponse {
            expected: "shell command acknowledgement",
        }),
    }
}

fn shell_command_payload(command: ShellCommand) -> SessionCommand {
    match command {
        ShellCommand::Show => SessionCommand::SetLauncherVisible { visible: true },
        ShellCommand::ShowFromController => {
            SessionCommand::SetLauncherVisibleFromController { visible: true }
        }
        ShellCommand::Hide => SessionCommand::SetLauncherVisible { visible: false },
        ShellCommand::LogOut => SessionCommand::LogOut,
        ShellCommand::SessionAction(action) => match action {
            super::SessionAction::LogOut => SessionCommand::LogOut,
            super::SessionAction::RestartShell => SessionCommand::SessionAction {
                action: nickel_session_protocol::SessionAction::RestartShell,
            },
            super::SessionAction::Lock => SessionCommand::SessionAction {
                action: nickel_session_protocol::SessionAction::Lock,
            },
            super::SessionAction::Suspend => SessionCommand::SessionAction {
                action: nickel_session_protocol::SessionAction::Suspend,
            },
            super::SessionAction::Reboot => SessionCommand::SessionAction {
                action: nickel_session_protocol::SessionAction::Reboot,
            },
            super::SessionAction::PowerOff => SessionCommand::SessionAction {
                action: nickel_session_protocol::SessionAction::PowerOff,
            },
        },
        ShellCommand::Unlock => SessionCommand::Unlock,
        ShellCommand::ShowContextMenu { x, width, height } => SessionCommand::ShowOverlay {
            role: SessionShellRole::ContextMenu,
            geometry: SessionGeometry {
                x,
                y: 0,
                width,
                height,
            },
            windows: Vec::new(),
        },
        ShellCommand::ShowPreview {
            x,
            width,
            height,
            windows,
        } => SessionCommand::ShowOverlay {
            role: SessionShellRole::Preview,
            geometry: SessionGeometry {
                x,
                y: 0,
                width,
                height,
            },
            windows: windows
                .into_iter()
                .map(|window| SessionWindowId(window.0))
                .collect(),
        },
        ShellCommand::ShowTaskSwitcher {
            width,
            height,
            windows,
        } => SessionCommand::ShowOverlay {
            role: SessionShellRole::Preview,
            geometry: SessionGeometry {
                x: 0,
                y: 0,
                width,
                height,
            },
            windows: windows
                .into_iter()
                .map(|window| SessionWindowId(window.0))
                .collect(),
        },
        ShellCommand::FocusControlCenter => SessionCommand::FocusShellRole {
            role: SessionShellRole::ControlCenter,
        },
        ShellCommand::FocusPreview => SessionCommand::FocusShellRole {
            role: SessionShellRole::Preview,
        },
        ShellCommand::FocusContextMenu => SessionCommand::FocusShellRole {
            role: SessionShellRole::ContextMenu,
        },
        ShellCommand::FocusScreenshot => SessionCommand::FocusShellRole {
            role: SessionShellRole::Screenshot,
        },
        ShellCommand::RestoreApplicationFocus => SessionCommand::RestoreApplicationFocus,
        ShellCommand::SetShellRoleVisible { role, visible } => {
            SessionCommand::SetShellRoleVisible { role, visible }
        }
        ShellCommand::ShowAnchoredShellRole { role, anchor } => {
            SessionCommand::ShowAnchoredShellRole { role, anchor }
        }
        ShellCommand::HideContextMenu => SessionCommand::HideOverlay,
        ShellCommand::HighlightWindow(window) => SessionCommand::HighlightWindow {
            window: Some(SessionWindowId(window.0)),
        },
        ShellCommand::ClearWindowHighlight => SessionCommand::HighlightWindow { window: None },
        ShellCommand::WindowAction { window, action } => SessionCommand::WindowAction {
            window: SessionWindowId(window.0),
            action: match action {
                WindowAction::Activate => SessionWindowAction::Activate,
                WindowAction::Close => SessionWindowAction::Close,
                WindowAction::Maximize => SessionWindowAction::MaximizeRestore,
                WindowAction::Minimize => SessionWindowAction::Minimize,
                WindowAction::Fullscreen => SessionWindowAction::FullscreenRestore,
            },
        },
        ShellCommand::CreateWorkspace => SessionCommand::CreateWorkspace,
        ShellCommand::RemoveWorkspace(workspace) => SessionCommand::RemoveWorkspace {
            workspace: nickel_session_protocol::WorkspaceId(workspace),
        },
        ShellCommand::SwitchWorkspace(workspace) => SessionCommand::SwitchWorkspace {
            workspace: nickel_session_protocol::WorkspaceId(workspace),
            output: None,
        },
        ShellCommand::MoveWindowToWorkspace { window, workspace } => {
            SessionCommand::MoveWindowToWorkspace {
                window: SessionWindowId(window.0),
                workspace: nickel_session_protocol::WorkspaceId(workspace),
            }
        }
        ShellCommand::MoveWindowToDisplay { window, output } => {
            SessionCommand::MoveWindowToOutput {
                window: SessionWindowId(window.0),
                output,
            }
        }
    }
}

pub struct WindowFeed {
    socket: Option<std::os::unix::net::UnixDatagram>,
    path: PathBuf,
    outputs: RefCell<HashMap<WindowId, String>>,
    available_outputs: RefCell<Vec<String>>,
    primary_output: RefCell<Option<String>>,
}

pub fn show_window_system_menu(_: WindowId) -> bool {
    false
}

impl WindowFeed {
    pub fn new() -> Self {
        let path = env::temp_dir().join(format!("nickel-{}-windows.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let socket = std::os::unix::net::UnixDatagram::bind(&path).ok();
        if let Some(socket) = &socket {
            let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
        }
        Self {
            socket,
            path,
            outputs: RefCell::new(HashMap::new()),
            available_outputs: RefCell::new(Vec::new()),
            primary_output: RefCell::new(None),
        }
    }

    pub fn snapshot(&self, launcher: &Launcher) -> FeedState<Vec<OpenWindow>> {
        let Some(socket) = self.socket.as_ref() else {
            return FeedState::Disconnected;
        };
        match session_request_on(socket, SessionRequest::Query(SessionQuery::Snapshot)) {
            Ok(ServerMessage::Snapshot(snapshot)) => {
                let mut outputs = self.outputs.borrow_mut();
                outputs.clear();
                *self.available_outputs.borrow_mut() = snapshot
                    .outputs
                    .iter()
                    .map(|output| output.name.clone())
                    .collect();
                *self.primary_output.borrow_mut() = snapshot
                    .outputs
                    .iter()
                    .find(|output| output.primary)
                    .map(|output| output.name.clone());
                for window in &snapshot.windows {
                    if let Some(output) = owning_output(window.geometry, &snapshot.outputs) {
                        outputs.insert(WindowId(window.id.0), output);
                    }
                }
                let multiple_outputs = snapshot.outputs.len() > 1;
                FeedState::Ready(
                    snapshot
                        .windows
                        .into_iter()
                        .map(|window| OpenWindow {
                            state: crate::model::WindowState {
                                minimized: window.minimized,
                                maximized: window.maximized,
                                fullscreen: window.fullscreen,
                                workspace: Some(window.workspace.0),
                                output: outputs.get(&WindowId(window.id.0)).cloned(),
                                capabilities: crate::model::WindowCapabilities {
                                    fullscreen: true,
                                    move_workspace: true,
                                    move_display: multiple_outputs,
                                    ..crate::model::WindowCapabilities::default()
                                },
                            },
                            id: WindowId(window.id.0),
                            application_id: resolve_application_id(
                                &window.application_id,
                                launcher,
                            ),
                            active: window.active,
                            title: window.title,
                        })
                        .collect(),
                )
            }
            Ok(_) => FeedState::Failed,
            Err(error) => session_error_feed_state(error),
        }
    }

    pub fn window_output(&self, window: WindowId) -> Option<String> {
        self.outputs.borrow().get(&window).cloned()
    }

    pub fn outputs(&self) -> Vec<String> {
        self.available_outputs.borrow().clone()
    }

    pub fn primary_output(&self) -> Option<String> {
        self.primary_output.borrow().clone()
    }

    pub fn workspaces(&self) -> FeedState<Vec<super::WorkspaceSummary>> {
        let Some(socket) = self.socket.as_ref() else {
            return FeedState::Disconnected;
        };
        match session_request_on(socket, SessionRequest::Query(SessionQuery::Workspaces)) {
            Ok(ServerMessage::Workspaces(state)) => FeedState::Ready(
                state
                    .ordered
                    .into_iter()
                    .map(|workspace| super::WorkspaceSummary {
                        id: workspace.id.0,
                        active: workspace.id == state.active,
                    })
                    .collect(),
            ),
            Ok(_) => FeedState::Failed,
            Err(error) => session_error_feed_state(error),
        }
    }

    pub fn preview(&self, window: WindowId) -> Option<WindowPreview> {
        let socket = self.socket.as_ref()?;
        let ServerMessage::Preview(preview) = session_request_on(
            socket,
            SessionRequest::Query(SessionQuery::Preview {
                window: SessionWindowId(window.0),
            }),
        )
        .ok()?
        else {
            return None;
        };
        let image = protocol_preview_image(preview)?;
        Some(WindowPreview { window, image })
    }

    pub fn supports_previews(&self) -> bool {
        true
    }

    pub fn icon(&self, _: WindowId) -> Option<image::RgbaImage> {
        None
    }
}

fn protocol_preview_image(
    preview: nickel_session_protocol::PreviewFrame,
) -> Option<image::RgbaImage> {
    preview.validate().ok()?;
    image::RgbaImage::from_raw(
        u32::from(preview.width),
        u32::from(preview.height),
        preview.rgba,
    )
}

fn owning_output(
    geometry: Option<SessionGeometry>,
    outputs: &[nickel_session_protocol::OutputSnapshot],
) -> Option<String> {
    let geometry = geometry?;
    outputs
        .iter()
        .filter(|output| output.enabled)
        .map(|output| {
            let left = geometry.x.max(output.geometry.x);
            let top = geometry.y.max(output.geometry.y);
            let right =
                (geometry.x + geometry.width).min(output.geometry.x + output.geometry.width);
            let bottom =
                (geometry.y + geometry.height).min(output.geometry.y + output.geometry.height);
            let area = i64::from((right - left).max(0)) * i64::from((bottom - top).max(0));
            (area, output.primary, output.name.as_str())
        })
        .filter(|(area, _, _)| *area > 0)
        .max_by(|left, right| {
            left.0
                .cmp(&right.0)
                .then_with(|| left.1.cmp(&right.1))
                .then_with(|| right.2.cmp(left.2))
        })
        .map(|(_, _, name)| name.to_owned())
}

fn session_error_feed_state<T>(error: SessionRequestError) -> FeedState<T> {
    match error {
        SessionRequestError::MissingControlSocket
        | SessionRequestError::MissingSessionToken
        | SessionRequestError::Send
        | SessionRequestError::Receive
        | SessionRequestError::ReceiveTimeout => FeedState::Disconnected,
        SessionRequestError::SocketBind
        | SessionRequestError::SocketConfiguration
        | SessionRequestError::Encoding
        | SessionRequestError::Decoding
        | SessionRequestError::Authorization { .. }
        | SessionRequestError::ServerRejection { .. }
        | SessionRequestError::UnexpectedResponse { .. } => FeedState::Failed,
    }
}

pub fn launcher_hotkey_receiver() -> super::GlobalShortcutFeed {
    use std::os::unix::net::UnixDatagram;

    let (sender, receiver) = mpsc::channel();
    let audio_sender = sender.clone();
    thread::Builder::new()
        .name("nickel-audio-events".into())
        .spawn(move || {
            let updates = linux_audio::subscribe();
            while let Ok(status) = updates.recv() {
                if audio_sender
                    .send(GlobalShortcut::AudioChanged {
                        available: status.available,
                        volume_percent: status.volume_percent,
                        muted: status.muted,
                        output_name: status
                            .devices
                            .iter()
                            .find(|device| device.is_default)
                            .map(|device| device.name.clone()),
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .expect("failed to start Nickel audio event listener");
    let Some(session) = env::var_os(SESSION_CONTROL_ENV) else {
        return super::GlobalShortcutFeed {
            receiver,
            ownership: nickel_input::global::ShortcutOwnership::Compositor,
            capability: nickel_input::global::ShortcutCapability::Unavailable(
                nickel_input::global::UnavailableReason::MissingRuntime,
            ),
        };
    };
    let path = env::temp_dir().join(format!(
        "nickel-{}-launcher-events.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let Ok(socket) = UnixDatagram::bind(&path) else {
        return super::GlobalShortcutFeed {
            receiver,
            ownership: nickel_input::global::ShortcutOwnership::Compositor,
            capability: nickel_input::global::ShortcutCapability::Unavailable(
                nickel_input::global::UnavailableReason::Backend(
                    "could not bind the session shortcut socket".into(),
                ),
            ),
        };
    };
    let envelope = ClientEnvelope {
        token: match env::var(SESSION_TOKEN_ENV) {
            Ok(token) => token,
            Err(_) => {
                return super::GlobalShortcutFeed {
                    receiver,
                    ownership: nickel_input::global::ShortcutOwnership::Compositor,
                    capability: nickel_input::global::ShortcutCapability::Unavailable(
                        nickel_input::global::UnavailableReason::PermissionDenied,
                    ),
                };
            }
        },
        request_id: SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
        request: SessionRequest::Subscribe,
    };
    if socket
        .send_to(&encode_session(&envelope).unwrap_or_default(), session)
        .is_err()
    {
        let _ = std::fs::remove_file(path);
        return super::GlobalShortcutFeed {
            receiver,
            ownership: nickel_input::global::ShortcutOwnership::Compositor,
            capability: nickel_input::global::ShortcutCapability::Unavailable(
                nickel_input::global::UnavailableReason::Backend(
                    "session shortcut subscription failed".into(),
                ),
            ),
        };
    }
    thread::Builder::new()
        .name("nickel-launcher-events".into())
        .spawn(move || {
            let mut event = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
            let mut state = SubscriptionState::default();
            while let Ok(length) = socket.recv(&mut event) {
                let Some(shortcut) = decode_session::<ServerEnvelope>(&event[..length])
                    .ok()
                    .map(|envelope| envelope.message)
                    .and_then(|message| subscription_shortcut(message, &mut state))
                else {
                    continue;
                };
                if sender.send(shortcut).is_err() {
                    break;
                }
            }
            let _ = std::fs::remove_file(path);
        })
        .expect("failed to start Nickel launcher event listener");
    super::GlobalShortcutFeed {
        receiver,
        ownership: nickel_input::global::ShortcutOwnership::Compositor,
        capability: nickel_input::global::ShortcutCapability::Available,
    }
}

pub fn semantic_target_receiver() -> mpsc::Receiver<super::ShellTestRequest> {
    use std::os::unix::net::UnixDatagram;

    let (sender, receiver) = mpsc::channel();
    let Some(path) = env::var_os(SHELL_TEST_CONTROL_ENV).map(PathBuf::from) else {
        return receiver;
    };
    let Ok(token) = env::var(SESSION_TOKEN_ENV) else {
        return receiver;
    };
    let _ = std::fs::remove_file(&path);
    let Ok(socket) = UnixDatagram::bind(&path) else {
        return receiver;
    };
    thread::Builder::new()
        .name("nickel-semantic-test-targets".into())
        .spawn(move || {
            let mut frame = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
            while let Ok((length, source)) = socket.recv_from(&mut frame) {
                let Some(reply_path) = source.as_pathname().map(Path::to_path_buf) else {
                    continue;
                };
                let request = decode_session::<ClientEnvelope>(&frame[..length]);
                let request = match request {
                    Ok(envelope) if envelope.token == token => match envelope.request {
                        SessionRequest::Query(SessionQuery::ShellSemanticTarget { target }) => {
                            super::ShellTestRequest::SemanticTarget {
                                request_id: envelope.request_id,
                                target,
                                reply_path,
                            }
                        }
                        SessionRequest::Query(SessionQuery::ShellRuntimeDiagnostics) => {
                            super::ShellTestRequest::RuntimeDiagnostics {
                                request_id: envelope.request_id,
                                reply_path,
                            }
                        }
                        _ => {
                            respond_semantic_target_error(
                                &reply_path,
                                envelope.request_id,
                                nickel_session_protocol::ErrorCode::InvalidRequest,
                                "expected a shell_semantic_target query",
                            );
                            continue;
                        }
                    },
                    Ok(envelope) => {
                        respond_semantic_target_error(
                            &reply_path,
                            envelope.request_id,
                            nickel_session_protocol::ErrorCode::Unauthorized,
                            "invalid session capability token",
                        );
                        continue;
                    }
                    Err(error) => {
                        respond_semantic_target_error(
                            &reply_path,
                            0,
                            nickel_session_protocol::ErrorCode::InvalidRequest,
                            &error.to_string(),
                        );
                        continue;
                    }
                };
                if sender.send(request).is_err() {
                    break;
                }
            }
            let _ = std::fs::remove_file(path);
        })
        .expect("failed to start semantic test target listener");
    receiver
}

pub fn respond_semantic_target(
    request_id: u64,
    reply_path: &Path,
    target: Option<nickel_session_protocol::ResolvedShellTarget>,
) {
    let message = target.map_or_else(
        || ServerMessage::Error {
            code: nickel_session_protocol::ErrorCode::InvalidRequest,
            message: "semantic target is not present in the live shell frame".into(),
        },
        ServerMessage::ShellSemanticTarget,
    );
    send_semantic_target_response(reply_path, request_id, message);
}

pub fn respond_semantic_action(request_id: u64, reply_path: &Path, performed: bool) {
    let message = if performed {
        ServerMessage::Ack
    } else {
        ServerMessage::Error {
            code: nickel_session_protocol::ErrorCode::InvalidRequest,
            message: "semantic action is not available in the live shell frame".into(),
        }
    };
    send_semantic_target_response(reply_path, request_id, message);
}

pub fn respond_runtime_diagnostics(
    request_id: u64,
    reply_path: &Path,
    diagnostics: nickel_session_protocol::ShellRuntimeDiagnostics,
) {
    send_semantic_target_response(
        reply_path,
        request_id,
        ServerMessage::ShellRuntimeDiagnostics(diagnostics),
    );
}

fn respond_semantic_target_error(
    path: &Path,
    request_id: u64,
    code: nickel_session_protocol::ErrorCode,
    message: &str,
) {
    send_semantic_target_response(
        path,
        request_id,
        ServerMessage::Error {
            code,
            message: message.to_owned(),
        },
    );
}

fn send_semantic_target_response(path: &Path, request_id: u64, message: ServerMessage) {
    use std::os::unix::net::UnixDatagram;

    let Ok(frame) = encode_session(&ServerEnvelope {
        request_id,
        message,
    }) else {
        return;
    };
    if let Ok(socket) = UnixDatagram::unbound() {
        let _ = socket.send_to(&frame, path);
    }
}

#[derive(Default)]
struct SubscriptionState {
    locked: Option<bool>,
    launcher_visible: Option<bool>,
}

fn subscription_shortcut(
    message: ServerMessage,
    state: &mut SubscriptionState,
) -> Option<GlobalShortcut> {
    match message {
        ServerMessage::Event(SessionEvent::ShellSettingsChanged) => {
            Some(GlobalShortcut::ReloadShellSettings)
        }
        ServerMessage::Event(SessionEvent::LauncherVisibility { visible })
        | ServerMessage::LauncherVisibility { visible } => {
            if state.launcher_visible.replace(visible) == Some(visible) {
                None
            } else if visible {
                Some(GlobalShortcut::ShowLauncher)
            } else {
                Some(GlobalShortcut::HideLauncher)
            }
        }
        ServerMessage::Event(SessionEvent::LockState { locked }) => (state.locked.replace(locked)
            != Some(locked))
        .then_some(GlobalShortcut::LockState { locked }),
        ServerMessage::Event(SessionEvent::GlobalShortcut { action }) => Some(match action {
            nickel_session_protocol::ShortcutAction::ShowRun => GlobalShortcut::ShowRun,
            nickel_session_protocol::ShortcutAction::ShowScreenshotTool => {
                GlobalShortcut::Screenshot(ScreenshotAction::InteractiveRegion)
            }
            nickel_session_protocol::ShortcutAction::CaptureActiveWindow => {
                GlobalShortcut::Screenshot(ScreenshotAction::ActiveWindow)
            }
            nickel_session_protocol::ShortcutAction::CaptureActiveWindowToFile => {
                GlobalShortcut::Screenshot(ScreenshotAction::ActiveWindowToFile)
            }
        }),
        ServerMessage::Event(SessionEvent::ConsumerControl { control }) => {
            Some(GlobalShortcut::ConsumerControl(control))
        }
        ServerMessage::Event(SessionEvent::Snapshot(snapshot)) => {
            let lock_changed = state.locked.replace(snapshot.locked) != Some(snapshot.locked);
            let launcher_changed = state.launcher_visible.replace(snapshot.launcher_visible)
                != Some(snapshot.launcher_visible);
            if lock_changed {
                Some(GlobalShortcut::LockState {
                    locked: snapshot.locked,
                })
            } else if launcher_changed && snapshot.launcher_visible {
                Some(GlobalShortcut::ShowLauncher)
            } else if launcher_changed {
                Some(GlobalShortcut::HideLauncher)
            } else {
                None
            }
        }
        _ => None,
    }
}

pub fn handle_focused_shortcut(_: nickel_core::hotkeys::KeyCode, _: nickel_core::hotkeys::KeyEdge) {
}

pub fn execute_run_command(command: &str) -> Result<(), super::LaunchError> {
    let mut process = std::process::Command::new("sh");
    process.arg("-c").arg(command);
    process
        .env_remove(SESSION_CONTROL_ENV)
        .env_remove(SESSION_TOKEN_ENV)
        .env_remove(SHELL_TEST_CONTROL_ENV);
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

pub fn launch_session_application(
    application: &Application,
) -> Result<Option<u32>, super::LaunchError> {
    application
        .launch_as_session_client()
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
        state: crate::model::WindowState::default(),
    })
}

fn resolve_application_id(native_app_id: &str, launcher: &Launcher) -> Option<ApplicationId> {
    let native_app_id = native_app_id.trim_end_matches(".desktop");
    if native_app_id.starts_with("io.nickel.codex.") {
        return Some(ApplicationId::new(native_app_id));
    }
    launcher
        .applications()
        .find(|application| application.matches_native_id(native_app_id))
        .map(|application| application.application_id().clone())
}

#[cfg(test)]
mod tests {
    use super::protocol_preview_image;
    use std::io;
    use std::time::Duration;

    use crate::{
        launcher::Launcher,
        model::Application,
        platform::{GlobalShortcut, ScreenshotAction, SessionRequestError, ShellCommand},
    };

    use super::{
        MAX_PROTOCOL_ERROR_MESSAGE_CHARS, SubscriptionState, bounded_notification_text,
        capture_active_window, capture_active_window_to_file, command_response,
        crop_output_geometry, intersection_area, notification_actions, notification_name_owned,
        owning_output, parse_window, pixmap_to_rgba, resolve_application_id, response_for_request,
        response_message, secure_storage_response, secure_storage_retry_response,
        session_receive_error, shell_command_payload, subscription_shortcut, tray_retry_delay,
    };

    #[test]
    fn window_output_uses_largest_intersection_and_primary_tie_break() {
        use nickel_session_protocol::{Geometry, OutputSnapshot, OutputTransform};

        let output = |name: &str, x: i32, primary: bool| OutputSnapshot {
            name: name.to_owned(),
            model: name.to_owned(),
            geometry: Geometry {
                x,
                y: 0,
                width: 100,
                height: 100,
            },
            work_area: Geometry {
                x,
                y: 0,
                width: 100,
                height: 100,
            },
            scale_120: 120,
            transform: OutputTransform::Normal,
            physical_width_mm: 1,
            physical_height_mm: 1,
            primary,
            enabled: true,
        };
        let outputs = vec![output("left", 0, false), output("right", 100, true)];
        let spanning = Geometry {
            x: 50,
            y: 0,
            width: 100,
            height: 100,
        };
        assert_eq!(
            owning_output(Some(spanning), &outputs).as_deref(),
            Some("right")
        );
        let mostly_left = Geometry {
            x: 10,
            y: 0,
            width: 100,
            height: 100,
        };
        assert_eq!(
            owning_output(Some(mostly_left), &outputs).as_deref(),
            Some("left")
        );
    }

    #[test]
    #[ignore = "requires an explicitly selected live Nickel Wayland session"]
    fn live_active_window_capture_reaches_image_clipboard() {
        capture_active_window().expect("active-window capture should succeed");
        let mut clipboard = arboard::Clipboard::new().expect("clipboard should be available");
        let image = clipboard
            .get_image()
            .expect("clipboard should contain image pixels");
        println!("clipboard_image={}x{}", image.width, image.height);
        assert!(image.width > 0);
        assert!(image.height > 0);
    }

    #[test]
    #[ignore = "requires a preceding live screenshot shortcut"]
    fn live_clipboard_contains_image_pixels_after_external_action() {
        let mut clipboard = arboard::Clipboard::new().expect("clipboard should be available");
        let image = clipboard
            .get_image()
            .expect("clipboard should contain image pixels");
        println!("clipboard_image={}x{}", image.width, image.height);
        assert!(image.width > 0);
        assert!(image.height > 0);
    }

    #[test]
    #[ignore = "requires an explicitly selected live Nickel Wayland session"]
    fn live_active_window_file_capture_reaches_clipboard_and_reopens() {
        capture_active_window_to_file().expect("active-window file capture should succeed");
        let mut clipboard = arboard::Clipboard::new().expect("clipboard should be available");
        let path = std::path::PathBuf::from(
            clipboard
                .get_text()
                .expect("clipboard should contain the temporary image path"),
        );
        let image = image::open(&path).expect("temporary capture should reopen");
        assert!(image.width() > 0);
        assert!(image.height() > 0);
        std::fs::remove_file(path).expect("temporary acceptance capture should be removable");
    }

    #[test]
    #[ignore = "requires a preceding live screenshot-to-file shortcut"]
    fn live_clipboard_path_from_external_action_reopens() {
        let mut clipboard = arboard::Clipboard::new().expect("clipboard should be available");
        let path = std::path::PathBuf::from(
            clipboard
                .get_text()
                .expect("clipboard should contain the temporary image path"),
        );
        let image = image::open(&path).expect("temporary capture should reopen");
        assert!(image.width() > 0);
        assert!(image.height() > 0);
        std::fs::remove_file(path).expect("temporary acceptance capture should be removable");
    }

    #[test]
    fn active_window_output_selection_uses_intersection_area() {
        use nickel_session_protocol::Geometry;

        let window = Geometry {
            x: 900,
            y: 100,
            width: 500,
            height: 400,
        };
        let left = Geometry {
            x: 0,
            y: 0,
            width: 1000,
            height: 800,
        };
        let right = Geometry {
            x: 1000,
            y: 0,
            width: 1000,
            height: 800,
        };
        assert_eq!(intersection_area(window, left), 40_000);
        assert_eq!(intersection_area(window, right), 160_000);
    }

    #[test]
    fn session_screenshot_events_map_to_typed_consumer_actions() {
        use nickel_session_protocol::{Event, ServerMessage, ShortcutAction};

        let cases = [
            (
                ShortcutAction::ShowScreenshotTool,
                ScreenshotAction::InteractiveRegion,
            ),
            (
                ShortcutAction::CaptureActiveWindow,
                ScreenshotAction::ActiveWindow,
            ),
            (
                ShortcutAction::CaptureActiveWindowToFile,
                ScreenshotAction::ActiveWindowToFile,
            ),
        ];
        for (wire, expected) in cases {
            let mut state = SubscriptionState::default();
            assert_eq!(
                subscription_shortcut(
                    ServerMessage::Event(Event::GlobalShortcut { action: wire }),
                    &mut state,
                ),
                Some(GlobalShortcut::Screenshot(expected))
            );
        }
    }

    #[test]
    fn active_window_crop_maps_logical_geometry_to_scaled_pixels() {
        use nickel_session_protocol::Geometry;

        let mut source = image::RgbaImage::new(400, 200);
        for (x, y, pixel) in source.enumerate_pixels_mut() {
            *pixel = image::Rgba([x as u8, y as u8, 0, 255]);
        }
        let crop = crop_output_geometry(
            source,
            Geometry {
                x: 100,
                y: 50,
                width: 200,
                height: 100,
            },
            Geometry {
                x: 150,
                y: 75,
                width: 50,
                height: 25,
            },
        )
        .unwrap();
        assert_eq!(crop.dimensions(), (100, 50));
        assert_eq!(crop.get_pixel(0, 0).0, [100, 50, 0, 255]);
    }

    #[test]
    fn existing_notification_owner_does_not_block_shell_startup() {
        assert!(!notification_name_owned(Err(zbus::Error::NameTaken)).unwrap());
        assert!(notification_name_owned(Ok(zbus::fdo::RequestNameReply::PrimaryOwner)).unwrap());
    }

    #[test]
    fn persistent_session_socket_discards_stale_responses() {
        let response = nickel_session_protocol::ServerEnvelope {
            request_id: 7,
            message: nickel_session_protocol::ServerMessage::Ack,
        };
        assert!(response_for_request(response.clone(), 8).is_none());
        assert!(matches!(
            response_for_request(response, 7),
            Some(nickel_session_protocol::ServerMessage::Ack)
        ));
    }

    #[test]
    fn session_response_preserves_authorization_and_server_rejections() {
        let authorization = response_message(nickel_session_protocol::ServerMessage::Error {
            code: nickel_session_protocol::ErrorCode::Unauthorized,
            message: "shell capability rejected".into(),
        });
        assert_eq!(
            authorization,
            Err(SessionRequestError::Authorization {
                message: "shell capability rejected".into(),
            })
        );

        let server_rejection = response_message(nickel_session_protocol::ServerMessage::Error {
            code: nickel_session_protocol::ErrorCode::ResourceLimit,
            message: "too many shell surfaces".into(),
        });
        assert_eq!(
            server_rejection,
            Err(SessionRequestError::ServerRejection {
                code: nickel_session_protocol::ErrorCode::ResourceLimit,
                message: "too many shell surfaces".into(),
            })
        );
    }

    #[test]
    fn session_response_error_details_are_bounded_and_control_free() {
        let error = response_message(nickel_session_protocol::ServerMessage::Error {
            code: nickel_session_protocol::ErrorCode::Internal,
            message: format!("bad\n{}", "x".repeat(300)),
        })
        .expect_err("server error should remain an error");
        let SessionRequestError::ServerRejection { message, .. } = error else {
            panic!("expected server rejection");
        };
        assert_eq!(
            message.chars().count(),
            MAX_PROTOCOL_ERROR_MESSAGE_CHARS + 1
        );
        assert!(!message.chars().any(char::is_control));
        assert!(message.ends_with('…'));
    }

    #[test]
    fn session_receive_distinguishes_timeout_from_transport_failure() {
        assert_eq!(
            session_receive_error(io::Error::from(io::ErrorKind::TimedOut)),
            SessionRequestError::ReceiveTimeout
        );
        assert_eq!(
            session_receive_error(io::Error::from(io::ErrorKind::ConnectionReset)),
            SessionRequestError::Receive
        );
    }

    #[test]
    fn session_command_acknowledgements_reject_unexpected_responses() {
        assert_eq!(
            command_response(nickel_session_protocol::ServerMessage::Ack),
            Ok(())
        );
        assert_eq!(
            command_response(nickel_session_protocol::ServerMessage::Workspaces(
                Default::default()
            )),
            Ok(())
        );
        assert_eq!(
            command_response(nickel_session_protocol::ServerMessage::Snapshot(
                Default::default()
            )),
            Err(SessionRequestError::UnexpectedResponse {
                expected: "shell command acknowledgement",
            })
        );
        assert_eq!(
            secure_storage_retry_response(nickel_session_protocol::ServerMessage::Snapshot(
                Default::default(),
            )),
            Err(SessionRequestError::UnexpectedResponse {
                expected: "secure-storage retry acknowledgement",
            })
        );
    }

    #[test]
    fn secure_storage_response_preserves_provider_state_reason_and_malformed_response() {
        use crate::platform::SecureStorageState;
        use nickel_session_protocol::{
            SecureStorageState as WireState, SecureStorageUnavailableReason as Reason,
            ServerMessage,
        };

        for (wire, expected) in [
            (WireState::Starting, SecureStorageState::Starting),
            (WireState::Locked, SecureStorageState::Locked),
            (
                WireState::PromptRequired,
                SecureStorageState::PromptRequired,
            ),
            (WireState::Ready, SecureStorageState::Ready),
        ] {
            assert_eq!(
                secure_storage_response(ServerMessage::SecureStorage {
                    state: wire,
                    reason: None,
                }),
                Ok(expected)
            );
        }
        assert_eq!(
            secure_storage_response(ServerMessage::SecureStorage {
                state: WireState::Unavailable,
                reason: Some(Reason::PromptTimedOut),
            }),
            Ok(SecureStorageState::UnavailableReason(
                Reason::PromptTimedOut
            ))
        );
        assert_eq!(
            secure_storage_response(ServerMessage::Ack),
            Err(SessionRequestError::UnexpectedResponse {
                expected: "secure-storage state",
            })
        );
    }

    #[test]
    fn notification_actions_require_complete_nonempty_pairs_and_are_bounded() {
        let actions = notification_actions(vec![
            "open".into(),
            "Open".into(),
            "".into(),
            "Ignored".into(),
            "reply".into(),
            "Reply".into(),
            "later".into(),
            "Later".into(),
            "overflow".into(),
            "Overflow".into(),
            "orphan".into(),
        ]);
        assert_eq!(
            actions
                .iter()
                .map(|action| action.key.as_str())
                .collect::<Vec<_>>(),
            ["open", "reply", "later"]
        );
    }

    #[test]
    fn notification_text_limits_preserve_unicode_boundaries() {
        assert_eq!(bounded_notification_text("éclair".into(), 2), "éc");
        assert_eq!(bounded_notification_text("short".into(), 8), "short");
    }

    #[test]
    fn logout_uses_the_session_control_protocol() {
        assert_eq!(
            shell_command_payload(ShellCommand::LogOut),
            nickel_session_protocol::Command::LogOut
        );
    }

    #[test]
    fn subscription_state_deduplicates_lock_snapshots() {
        let mut state = SubscriptionState::default();
        let snapshot = nickel_session_protocol::Snapshot {
            locked: true,
            ..Default::default()
        };
        assert_eq!(
            subscription_shortcut(
                nickel_session_protocol::ServerMessage::Event(
                    nickel_session_protocol::Event::Snapshot(snapshot.clone()),
                ),
                &mut state,
            ),
            Some(GlobalShortcut::LockState { locked: true })
        );
        assert_eq!(
            subscription_shortcut(
                nickel_session_protocol::ServerMessage::Event(
                    nickel_session_protocol::Event::Snapshot(snapshot),
                ),
                &mut state,
            ),
            None
        );
    }

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
    fn dynamic_codex_project_identity_needs_no_desktop_entry() {
        let launcher = Launcher::new(Vec::new());
        assert_eq!(
            resolve_application_id("io.nickel.codex.project.0123", &launcher)
                .map(|id| id.as_str().to_owned()),
            Some("io.nickel.codex.project.0123".into())
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

    #[test]
    fn preview_transport_metadata_is_validated_before_image_construction() {
        let valid = nickel_session_protocol::PreviewFrame {
            window: nickel_session_protocol::WindowId(7),
            width: 2,
            height: 1,
            rgba: vec![1, 2, 3, 255, 4, 5, 6, 255],
        };
        let image = protocol_preview_image(valid.clone()).expect("valid preview");
        assert_eq!(image.dimensions(), (2, 1));
        assert_eq!(image.as_raw(), &valid.rgba);

        for invalid in [
            nickel_session_protocol::PreviewFrame {
                width: 0,
                height: 1,
                rgba: Vec::new(),
                ..valid.clone()
            },
            nickel_session_protocol::PreviewFrame {
                width: nickel_session_protocol::MAX_PREVIEW_WIDTH + 1,
                ..valid.clone()
            },
            nickel_session_protocol::PreviewFrame {
                rgba: vec![0; 7],
                ..valid
            },
        ] {
            assert!(protocol_preview_image(invalid).is_none());
        }
    }
}
