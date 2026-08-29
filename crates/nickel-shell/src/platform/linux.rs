use nickel_session_protocol::{
    ClientEnvelope, Command as SessionCommand, Event as SessionEvent, Geometry as SessionGeometry,
    Query as SessionQuery, Request as SessionRequest, SecureStorageState as SessionSecureStorage,
    ServerEnvelope, ServerMessage, ShellRole as SessionShellRole,
    WindowAction as SessionWindowAction, WindowId as SessionWindowId, decode as decode_session,
    encode as encode_session,
};
use std::{
    collections::HashMap,
    env,
    hash::{DefaultHasher, Hash, Hasher},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread,
    time::Duration,
};

use crate::{
    desktop::Wallpaper,
    icons,
    launcher::Launcher,
    model::{Application, ApplicationId, OpenWindow, TrayItem, WindowId, WindowPreview},
    notification::{
        ClosedNotification, DesktopNotification, MAX_NOTIFICATION_ACTIONS, NotificationAction,
        NotificationRequest, NotificationStore,
    },
    platform::{GlobalShortcut, NotificationSource, ShellCommand, TraySource, WindowAction},
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
    let applications = APPLICATIONS.get_or_init(desktop_entries::load_applications);
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
    desktop_entries::load_applications()
}

fn session_request_on(
    socket: &std::os::unix::net::UnixDatagram,
    request: SessionRequest,
) -> Option<ServerMessage> {
    let server = env::var_os(SESSION_CONTROL_ENV)?;
    let request_id = SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed);
    let envelope = ClientEnvelope {
        token: env::var(SESSION_TOKEN_ENV).ok()?,
        request_id,
        request,
    };
    socket
        .send_to(&encode_session(&envelope).ok()?, server)
        .ok()?;
    let mut response = vec![0_u8; nickel_session_protocol::MAX_FRAME_BYTES];
    loop {
        let length = socket.recv(&mut response).ok()?;
        let response = decode_session::<ServerEnvelope>(&response[..length]).ok()?;
        if let Some(message) = response_for_request(response, request_id) {
            return Some(message);
        }
    }
}

fn response_for_request(response: ServerEnvelope, request_id: u64) -> Option<ServerMessage> {
    (response.request_id == request_id).then_some(response.message)
}

fn one_shot_session_request(request: SessionRequest) -> Option<ServerMessage> {
    use std::os::unix::net::UnixDatagram;

    let runtime = env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir);
    let path = runtime.join(format!(
        "nickel-{}-{}.sock",
        std::process::id(),
        SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_file(&path);
    let result = (|| {
        let socket = UnixDatagram::bind(&path).ok()?;
        socket
            .set_read_timeout(Some(Duration::from_millis(250)))
            .ok()?;
        session_request_on(&socket, request)
    })();
    let _ = std::fs::remove_file(path);
    result
}

pub fn register_session_shell() -> bool {
    matches!(
        one_shot_session_request(SessionRequest::RegisterShell {
            pid: std::process::id(),
        }),
        Some(ServerMessage::Snapshot(_))
    )
}

pub fn secure_storage_state() -> super::SecureStorageState {
    match one_shot_session_request(SessionRequest::Query(SessionQuery::SecureStorage)) {
        Some(ServerMessage::SecureStorage { state }) => match state {
            SessionSecureStorage::Starting => super::SecureStorageState::Starting,
            SessionSecureStorage::Locked => super::SecureStorageState::Locked,
            SessionSecureStorage::PromptRequired => super::SecureStorageState::PromptRequired,
            SessionSecureStorage::Ready => super::SecureStorageState::Ready,
            SessionSecureStorage::Unavailable => super::SecureStorageState::Unavailable,
        },
        _ => super::SecureStorageState::Unavailable,
    }
}

pub fn request_secure_storage_retry() -> bool {
    matches!(
        one_shot_session_request(SessionRequest::Command(SessionCommand::RetrySecureStorage)),
        Some(ServerMessage::Ack)
    )
}

pub fn send_shell_command(command: ShellCommand) -> bool {
    matches!(
        one_shot_session_request(SessionRequest::Command(shell_command_payload(command))),
        Some(ServerMessage::Ack | ServerMessage::Workspaces(_))
    )
}

fn shell_command_payload(command: ShellCommand) -> SessionCommand {
    match command {
        ShellCommand::Show => SessionCommand::SetLauncherVisible { visible: true },
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
        ShellCommand::FocusControlCenter => SessionCommand::FocusShellRole {
            role: SessionShellRole::ControlCenter,
        },
        ShellCommand::FocusPreview => SessionCommand::FocusShellRole {
            role: SessionShellRole::Preview,
        },
        ShellCommand::FocusContextMenu => SessionCommand::FocusShellRole {
            role: SessionShellRole::ContextMenu,
        },
        ShellCommand::RestoreApplicationFocus => SessionCommand::RestoreApplicationFocus,
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
    }
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
        let path = env::temp_dir().join(format!("nickel-{}-windows.sock", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let socket = std::os::unix::net::UnixDatagram::bind(&path).ok();
        if let Some(socket) = &socket {
            let _ = socket.set_read_timeout(Some(Duration::from_millis(100)));
        }
        Self { socket, path }
    }

    pub fn snapshot(&self, launcher: &Launcher) -> Option<Vec<OpenWindow>> {
        let socket = self.socket.as_ref()?;
        let ServerMessage::Windows(windows) =
            session_request_on(socket, SessionRequest::Query(SessionQuery::Windows))?
        else {
            return None;
        };
        Some(
            windows
                .into_iter()
                .map(|window| OpenWindow {
                    id: WindowId(window.id.0),
                    application_id: resolve_application_id(&window.application_id, launcher),
                    active: window.active,
                    title: window.title,
                })
                .collect(),
        )
    }

    pub fn workspaces(&self) -> Option<Vec<super::WorkspaceSummary>> {
        let socket = self.socket.as_ref()?;
        let ServerMessage::Workspaces(state) =
            session_request_on(socket, SessionRequest::Query(SessionQuery::Workspaces))?
        else {
            return None;
        };
        Some(
            state
                .ordered
                .into_iter()
                .map(|workspace| super::WorkspaceSummary {
                    id: workspace.id.0,
                    active: workspace.id == state.active,
                })
                .collect(),
        )
    }

    pub fn preview(&self, window: WindowId) -> Option<WindowPreview> {
        let socket = self.socket.as_ref()?;
        let ServerMessage::Preview(preview) = session_request_on(
            socket,
            SessionRequest::Query(SessionQuery::Preview {
                window: SessionWindowId(window.0),
            }),
        )?
        else {
            return None;
        };
        let image = image::RgbaImage::from_raw(
            u32::from(preview.width),
            u32::from(preview.height),
            preview.rgba,
        )?;
        Some(WindowPreview { window, image })
    }

    pub fn supports_previews(&self) -> bool {
        true
    }

    pub fn icon(&self, _: WindowId) -> Option<image::RgbaImage> {
        None
    }
}

pub fn launcher_hotkey_receiver() -> mpsc::Receiver<GlobalShortcut> {
    use std::os::unix::net::UnixDatagram;

    let (sender, receiver) = mpsc::channel();
    let Some(session) = env::var_os(SESSION_CONTROL_ENV) else {
        return receiver;
    };
    let path = env::temp_dir().join(format!(
        "nickel-{}-launcher-events.sock",
        std::process::id()
    ));
    let _ = std::fs::remove_file(&path);
    let Ok(socket) = UnixDatagram::bind(&path) else {
        return receiver;
    };
    let envelope = ClientEnvelope {
        token: match env::var(SESSION_TOKEN_ENV) {
            Ok(token) => token,
            Err(_) => return receiver,
        },
        request_id: SESSION_REQUEST_ID.fetch_add(1, Ordering::Relaxed),
        request: SessionRequest::Subscribe,
    };
    if socket
        .send_to(&encode_session(&envelope).unwrap_or_default(), session)
        .is_err()
    {
        let _ = std::fs::remove_file(path);
        return receiver;
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
    receiver
}

pub fn semantic_target_receiver() -> mpsc::Receiver<super::SemanticTargetRequest> {
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
                let (request_id, target) = match request {
                    Ok(envelope) if envelope.token == token => match envelope.request {
                        SessionRequest::Query(SessionQuery::ShellSemanticTarget { target }) => {
                            (envelope.request_id, target)
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
                if sender
                    .send(super::SemanticTargetRequest {
                        request_id,
                        target,
                        reply_path,
                    })
                    .is_err()
                {
                    break;
                }
            }
            let _ = std::fs::remove_file(path);
        })
        .expect("failed to start semantic test target listener");
    receiver
}

pub fn respond_semantic_target(
    request: super::SemanticTargetRequest,
    target: Option<nickel_session_protocol::ResolvedShellTarget>,
) {
    let message = target.map_or_else(
        || ServerMessage::Error {
            code: nickel_session_protocol::ErrorCode::InvalidRequest,
            message: "semantic target is not present in the live shell frame".into(),
        },
        ServerMessage::ShellSemanticTarget,
    );
    send_semantic_target_response(&request.reply_path, request.request_id, message);
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

pub fn handle_focused_shortcut(_: nickel_core::hotkeys::Hotkey, _: nickel_core::hotkeys::KeyEdge) {}

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
    if native_app_id.starts_with("io.nickel.codex.") {
        return Some(ApplicationId::new(native_app_id));
    }
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

    use crate::{
        launcher::Launcher,
        model::Application,
        platform::{GlobalShortcut, ShellCommand},
    };

    use super::{
        SubscriptionState, bounded_notification_text, notification_actions,
        notification_name_owned, parse_window, pixmap_to_rgba, resolve_application_id,
        response_for_request, shell_command_payload, subscription_shortcut, tray_retry_delay,
    };

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
}
