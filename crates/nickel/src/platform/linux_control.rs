use std::{
    collections::HashMap,
    sync::{Arc, Mutex, OnceLock, RwLock, mpsc},
    thread,
    time::{Duration, Instant},
};

use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue},
};

use super::super::{
    BluetoothDeviceStatus, BluetoothStatus, CONNECTIVITY_DEVICE_LIMIT, CONNECTIVITY_TEXT_LIMIT,
    ConnectivityRefresh, NetworkStatus, SystemStatusUpdate, WifiNetworkStatus,
    bound_connectivity_refresh,
};
use nickel_session_protocol::ConsumerControl;

const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
const NETWORK_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
pub(super) const BLUEZ: &str = "org.bluez";

type Properties = HashMap<String, OwnedValue>;
type Interfaces = HashMap<String, Properties>;
type ManagedObjects = HashMap<OwnedObjectPath, Interfaces>;

#[derive(Clone, Debug)]
enum Command {
    SetWifiEnabled(bool),
    ActivateWifi(String),
    SetBluetoothPowered(bool),
    SetBluetoothDiscovery(bool),
    ToggleBluetoothDevice(String),
    RefreshConnectivity(mpsc::SyncSender<Result<ConnectivityRefresh, String>>),
}

struct ControlBackend {
    network: Arc<RwLock<NetworkStatus>>,
    bluetooth: Arc<RwLock<BluetoothStatus>>,
    commands: mpsc::Sender<Command>,
    subscribers: Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
}

static BACKEND: OnceLock<ControlBackend> = OnceLock::new();
static MPRIS_COMMANDS: OnceLock<mpsc::SyncSender<ConsumerControl>> = OnceLock::new();

const MPRIS_COMMAND_CAPACITY: usize = 16;
const MPRIS_PLAYER_CAPACITY: usize = 64;
const MPRIS_REFRESH_INTERVAL: Duration = Duration::from_millis(750);

#[derive(Clone, Debug, Eq, PartialEq)]
struct MprisPlayer {
    name: String,
    owner: String,
    status: String,
    can_play: bool,
    can_pause: bool,
    can_control: bool,
    can_next: bool,
    can_previous: bool,
    can_seek: bool,
    recent: u64,
}

impl MprisPlayer {
    fn playing(&self) -> bool {
        self.status == "Playing"
    }

    fn supports(&self, control: ConsumerControl) -> bool {
        self.can_control
            && match control {
                ConsumerControl::PlayPause => {
                    if self.playing() {
                        self.can_pause
                    } else {
                        self.can_play
                    }
                }
                ConsumerControl::Play => self.can_play,
                ConsumerControl::Pause => self.can_pause,
                ConsumerControl::Stop => true,
                ConsumerControl::Next => self.can_next,
                ConsumerControl::Previous => self.can_previous,
                ConsumerControl::FastForward | ConsumerControl::Rewind => self.can_seek,
                ConsumerControl::VolumeUp
                | ConsumerControl::VolumeDown
                | ConsumerControl::VolumeMute => false,
            }
    }
}

#[derive(Debug, Default)]
struct MprisTracker {
    players: HashMap<String, MprisPlayer>,
    sequence: u64,
    bus_generation: u64,
}

impl MprisTracker {
    fn replace_snapshot(&mut self, mut observed: Vec<MprisPlayer>) {
        observed.sort_by(|left, right| left.name.cmp(&right.name));
        observed.truncate(MPRIS_PLAYER_CAPACITY);
        let old = std::mem::take(&mut self.players);
        let mut sequence = self.sequence;
        let players = observed
            .into_iter()
            .map(|mut player| {
                if let Some(old) = old
                    .get(&player.name)
                    .filter(|old| old.owner == player.owner)
                {
                    player.recent = old.recent;
                    if old.status != player.status {
                        sequence = sequence.saturating_add(1);
                        player.recent = sequence;
                    }
                }
                (player.name.clone(), player)
            })
            .collect();
        self.sequence = sequence;
        self.players = players;
    }

    fn bus_restarted(&mut self) {
        self.players.clear();
        self.bus_generation = self.bus_generation.saturating_add(1);
    }

    fn select(&self, control: ConsumerControl) -> Option<&MprisPlayer> {
        self.players
            .values()
            .filter(|player| player.supports(control))
            .max_by(|left, right| {
                left.playing()
                    .cmp(&right.playing())
                    .then_with(|| left.recent.cmp(&right.recent))
                    .then_with(|| right.name.cmp(&left.name))
            })
    }

    fn dispatched(&mut self, name: &str, owner: &str) {
        self.sequence = self.sequence.saturating_add(1);
        if let Some(player) = self
            .players
            .get_mut(name)
            .filter(|player| player.owner == owner)
        {
            player.recent = self.sequence;
        }
    }
}

pub fn handle_consumer_control(control: ConsumerControl) -> bool {
    match control {
        ConsumerControl::VolumeUp | ConsumerControl::VolumeDown | ConsumerControl::VolumeMute => {
            tracing::warn!(?control, "PipeWire audio control is not initialized");
            false
        }
        _ => {
            let commands = MPRIS_COMMANDS.get_or_init(|| {
                let (sender, receiver) = mpsc::sync_channel(MPRIS_COMMAND_CAPACITY);
                let _ = thread::Builder::new()
                    .name("nickel-mpris-control".into())
                    .spawn(move || mpris_worker(receiver));
                sender
            });
            if let Err(error) = commands.try_send(control) {
                tracing::debug!(?control, %error, "MPRIS command queue is unavailable or full");
                false
            } else {
                true
            }
        }
    }
}

fn mpris_worker(commands: mpsc::Receiver<ConsumerControl>) {
    let mut connection = None;
    let mut tracker = MprisTracker::default();
    loop {
        match commands.recv_timeout(MPRIS_REFRESH_INTERVAL) {
            Ok(control) => match refresh_mpris(&mut connection, &mut tracker) {
                Ok(()) => {
                    if let Err(error) = dispatch_mpris(&connection, &mut tracker, control) {
                        tracing::debug!(?control, %error, "MPRIS command was not delivered");
                    }
                }
                Err(error) => {
                    tracing::debug!(?control, %error, "MPRIS bus failed before dispatch");
                    connection = None;
                    tracker.bus_restarted();
                }
            },
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if let Err(error) = refresh_mpris(&mut connection, &mut tracker) {
                    tracing::debug!(%error, "MPRIS tracker will reconnect after a bus failure");
                    connection = None;
                    tracker.bus_restarted();
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
}

fn refresh_mpris(
    connection: &mut Option<Connection>,
    tracker: &mut MprisTracker,
) -> Result<(), String> {
    if connection.is_none() {
        *connection = Some(Connection::session().map_err(|error| error.to_string())?);
    }
    let connection = connection.as_ref().expect("connection was initialized");
    let dbus =
        zbus::blocking::fdo::DBusProxy::new(connection).map_err(|error| error.to_string())?;
    let players = dbus
        .list_names()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|name| name.as_str().starts_with("org.mpris.MediaPlayer2."))
        .filter_map(|name| {
            let owner = dbus.get_name_owner(name.clone().into()).ok()?.to_string();
            let name = name.as_str().to_owned();
            let proxy = Proxy::new(
                connection,
                name.as_str(),
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2.Player",
            )
            .ok()?;
            let can_control = proxy.get_property::<bool>("CanControl").unwrap_or(false);
            let status = proxy
                .get_property::<String>("PlaybackStatus")
                .unwrap_or_else(|_| "Stopped".into());
            Some(MprisPlayer {
                name: name.clone(),
                owner,
                status,
                can_play: proxy.get_property::<bool>("CanPlay").unwrap_or(false),
                can_pause: proxy.get_property::<bool>("CanPause").unwrap_or(false),
                can_control,
                can_next: proxy.get_property::<bool>("CanGoNext").unwrap_or(false),
                can_previous: proxy.get_property::<bool>("CanGoPrevious").unwrap_or(false),
                can_seek: proxy.get_property::<bool>("CanSeek").unwrap_or(false),
                recent: 0,
            })
        })
        .collect::<Vec<_>>();
    tracker.replace_snapshot(players);
    Ok(())
}

fn dispatch_mpris(
    connection: &Option<Connection>,
    tracker: &mut MprisTracker,
    control: ConsumerControl,
) -> Result<(), String> {
    let connection = connection.as_ref().ok_or("MPRIS bus is unavailable")?;
    let selected = tracker
        .select(control)
        .ok_or_else(|| "no capable MPRIS player is available".to_owned())?;
    let name = selected.name.clone();
    let owner = selected.owner.clone();
    let proxy = Proxy::new(
        connection,
        name.as_str(),
        "/org/mpris/MediaPlayer2",
        "org.mpris.MediaPlayer2.Player",
    )
    .map_err(|error| error.to_string())?;
    let result = match control {
        ConsumerControl::PlayPause => proxy.call_method("PlayPause", &()),
        ConsumerControl::Play => proxy.call_method("Play", &()),
        ConsumerControl::Pause => proxy.call_method("Pause", &()),
        ConsumerControl::Stop => proxy.call_method("Stop", &()),
        ConsumerControl::Next => proxy.call_method("Next", &()),
        ConsumerControl::Previous => proxy.call_method("Previous", &()),
        ConsumerControl::FastForward => proxy.call_method("Seek", &(10_000_000_i64,)),
        ConsumerControl::Rewind => proxy.call_method("Seek", &(-10_000_000_i64,)),
        ConsumerControl::VolumeUp | ConsumerControl::VolumeDown | ConsumerControl::VolumeMute => {
            return Err("not an MPRIS command".into());
        }
    };
    result
        .map(|_| {
            tracker.dispatched(&name, &owner);
        })
        .map_err(|error| error.to_string())
}

pub fn network_status() -> NetworkStatus {
    let backend = backend();
    backend
        .network
        .read()
        .map(|snapshot| snapshot.clone())
        .unwrap_or_default()
}

pub fn bluetooth_status() -> BluetoothStatus {
    let backend = backend();
    backend
        .bluetooth
        .read()
        .map(|snapshot| snapshot.clone())
        .unwrap_or_default()
}

pub fn subscribe_into(receiver: &mut crate::platform::status_mailbox::StatusReceiver) {
    let backend = backend();
    let sender = receiver.sender();
    if let Ok(mut subscribers) = backend.subscribers.lock() {
        subscribers.push(sender.clone());
    }
    if let Ok(status) = backend.network.read() {
        let _ = sender.send(Arc::new(SystemStatusUpdate::Network(status.clone())));
    }
    if let Ok(status) = backend.bluetooth.read() {
        let _ = sender.send(Arc::new(SystemStatusUpdate::Bluetooth(status.clone())));
    }
    let subscribers = Arc::clone(&backend.subscribers);
    receiver.on_drop(move || {
        subscribers
            .lock()
            .unwrap()
            .retain(|entry| !entry.same_channel(&sender))
    });
}

pub fn set_wifi_enabled(enabled: bool) -> bool {
    backend()
        .commands
        .send(Command::SetWifiEnabled(enabled))
        .is_ok()
}

pub fn activate_wifi_network(id: &str) -> bool {
    backend()
        .commands
        .send(Command::ActivateWifi(id.to_owned()))
        .is_ok()
}

pub fn set_bluetooth_powered(powered: bool) -> bool {
    backend()
        .commands
        .send(Command::SetBluetoothPowered(powered))
        .is_ok()
}

pub fn set_bluetooth_discovery(discovering: bool) -> bool {
    backend()
        .commands
        .send(Command::SetBluetoothDiscovery(discovering))
        .is_ok()
}

pub fn toggle_bluetooth_device(id: &str) -> bool {
    backend()
        .commands
        .send(Command::ToggleBluetoothDevice(id.to_owned()))
        .is_ok()
}

pub fn refresh_connectivity() -> Result<ConnectivityRefresh, String> {
    let (reply, response) = mpsc::sync_channel(1);
    backend()
        .commands
        .send(Command::RefreshConnectivity(reply))
        .map_err(|_| "connectivity worker stopped".to_owned())?;
    response
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| "connectivity refresh timed out".to_owned())?
}

fn backend() -> &'static ControlBackend {
    BACKEND.get_or_init(|| {
        let network = Arc::new(RwLock::new(NetworkStatus::default()));
        let bluetooth = Arc::new(RwLock::new(BluetoothStatus::default()));
        let (commands, receiver) = mpsc::channel();
        let subscribers = Arc::new(Mutex::new(Vec::new()));
        let worker_network = network.clone();
        let worker_bluetooth = bluetooth.clone();
        let _ = thread::Builder::new()
            .name("nickel-linux-control".into())
            .spawn({
                let subscribers = subscribers.clone();
                move || worker(worker_network, worker_bluetooth, subscribers, receiver)
            });
        ControlBackend {
            network,
            bluetooth,
            commands,
            subscribers,
        }
    })
}

fn worker(
    network: Arc<RwLock<NetworkStatus>>,
    bluetooth: Arc<RwLock<BluetoothStatus>>,
    subscribers: Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
    commands: mpsc::Receiver<Command>,
) {
    let system = Connection::system().ok();
    let mut next_refresh = Instant::now();
    loop {
        let timeout = next_refresh.saturating_duration_since(Instant::now());
        match commands.recv_timeout(timeout) {
            Ok(command) => {
                let refresh_reply = match command {
                    Command::RefreshConnectivity(reply) => Some(reply),
                    command => {
                        if let Some(connection) = system.as_ref()
                            && let Err(error) = apply_command(connection, command)
                        {
                            tracing::warn!(%error, "Linux Control Center command failed");
                        }
                        None
                    }
                };
                next_refresh = Instant::now();
                if refresh_reply.is_some() {
                    refresh_connectivity_snapshot(
                        system.as_ref(),
                        &network,
                        &bluetooth,
                        &subscribers,
                        refresh_reply,
                    );
                    next_refresh = Instant::now() + Duration::from_secs(2);
                    continue;
                }
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if Instant::now() < next_refresh {
            continue;
        }
        refresh_connectivity_snapshot(system.as_ref(), &network, &bluetooth, &subscribers, None);
        next_refresh = Instant::now() + Duration::from_secs(2);
    }
}

fn refresh_connectivity_snapshot(
    system: Option<&Connection>,
    network: &Arc<RwLock<NetworkStatus>>,
    bluetooth: &Arc<RwLock<BluetoothStatus>>,
    subscribers: &Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
    reply: Option<mpsc::SyncSender<Result<ConnectivityRefresh, String>>>,
) {
    let (network_snapshot, network_partial) = system
        .and_then(|connection| read_network_status(connection).ok())
        .unwrap_or_default();
    let (bluetooth_snapshot, bluetooth_partial) = system
        .and_then(|connection| read_bluetooth_status(connection).ok())
        .unwrap_or_default();
    let mut refresh = bound_connectivity_refresh(network_snapshot, bluetooth_snapshot);
    refresh.partial |= network_partial || bluetooth_partial;
    let network_snapshot = refresh.network;
    let bluetooth_snapshot = refresh.bluetooth;
    if reply.is_none() {
        if let Ok(mut current) = network.write()
            && *current != network_snapshot
        {
            current.clone_from(&network_snapshot);
            publish(
                subscribers,
                SystemStatusUpdate::Network(network_snapshot.clone()),
            );
        }
        if let Ok(mut current) = bluetooth.write()
            && *current != bluetooth_snapshot
        {
            current.clone_from(&bluetooth_snapshot);
            publish(
                subscribers,
                SystemStatusUpdate::Bluetooth(bluetooth_snapshot.clone()),
            );
        }
    }
    if let Some(reply) = reply {
        let result = system
            .is_some()
            .then_some(ConnectivityRefresh {
                network: network_snapshot,
                bluetooth: bluetooth_snapshot,
                partial: refresh.partial,
            })
            .ok_or_else(|| "system bus is unavailable".to_owned());
        let _ = reply.send(result);
    }
}

fn publish(
    subscribers: &Arc<Mutex<Vec<crate::platform::status_mailbox::StatusSender>>>,
    update: SystemStatusUpdate,
) {
    if let Ok(mut subscribers) = subscribers.lock() {
        let update = Arc::new(update);
        subscribers.retain(|subscriber| subscriber.send(update.clone()).is_ok());
    }
}

fn read_network_status(connection: &Connection) -> zbus::Result<(NetworkStatus, bool)> {
    let manager = Proxy::new(
        connection,
        NETWORK_MANAGER,
        NETWORK_MANAGER_PATH,
        NETWORK_MANAGER,
    )?;
    let enabled = manager
        .get_property::<bool>("WirelessEnabled")
        .unwrap_or(false);
    let devices = manager
        .call::<_, _, Vec<OwnedObjectPath>>("GetDevices", &())
        .unwrap_or_default();
    let mut partial = devices.len() > CONNECTIVITY_DEVICE_LIMIT;
    let saved = nickel_platform::network_manager_saved_wifi_connections_bounded(
        connection,
        CONNECTIVITY_DEVICE_LIMIT,
        || true,
    );
    partial |= saved.len() == CONNECTIVITY_DEVICE_LIMIT;
    let mut networks = Vec::new();

    for device_path in devices.into_iter().take(CONNECTIVITY_DEVICE_LIMIT) {
        let device = Proxy::new(
            connection,
            NETWORK_MANAGER,
            device_path.as_str(),
            "org.freedesktop.NetworkManager.Device",
        )?;
        if device.get_property::<u32>("DeviceType").unwrap_or(0) != 2 {
            continue;
        }
        let wireless = Proxy::new(
            connection,
            NETWORK_MANAGER,
            device_path.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
        )?;
        let active = wireless
            .get_property::<OwnedObjectPath>("ActiveAccessPoint")
            .ok();
        let access_points = wireless
            .get_property::<Vec<OwnedObjectPath>>("AccessPoints")
            .unwrap_or_default();
        partial |= access_points.len() > CONNECTIVITY_DEVICE_LIMIT;
        for access_point_path in access_points.into_iter().take(CONNECTIVITY_DEVICE_LIMIT) {
            if networks.len() == CONNECTIVITY_DEVICE_LIMIT {
                partial = true;
                break;
            }
            let access_point = Proxy::new(
                connection,
                NETWORK_MANAGER,
                access_point_path.as_str(),
                "org.freedesktop.NetworkManager.AccessPoint",
            )?;
            let ssid = access_point
                .get_property::<Vec<u8>>("Ssid")
                .unwrap_or_default();
            let raw_name = String::from_utf8_lossy(&ssid);
            partial |= raw_name.chars().count() > CONNECTIVITY_TEXT_LIMIT;
            let name = raw_name
                .trim()
                .chars()
                .take(CONNECTIVITY_TEXT_LIMIT)
                .collect::<String>();
            if name.is_empty() {
                continue;
            }
            let connected = active.as_ref() == Some(&access_point_path);
            networks.push(WifiNetworkStatus {
                id: format!("{}\t{}", device_path.as_str(), access_point_path.as_str()),
                name: name.clone(),
                signal_percent: u32::from(access_point.get_property::<u8>("Strength").unwrap_or(0)),
                connected,
                saved: saved.contains_key(&ssid),
            });
        }
    }
    networks.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.signal_percent.cmp(&left.signal_percent))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    networks.dedup_by(|left, right| left.name == right.name);
    let active = networks.iter().find(|network| network.connected);

    Ok((
        NetworkStatus {
            available: true,
            enabled,
            connected: active.is_some(),
            name: active
                .map(|network| network.name.clone())
                .unwrap_or_default(),
            signal_percent: active.map(|network| network.signal_percent).unwrap_or(0),
            networks,
        },
        partial,
    ))
}

fn read_bluetooth_status(connection: &Connection) -> zbus::Result<(BluetoothStatus, bool)> {
    let objects = managed_bluez_objects(connection)?;
    let mut partial = objects.len() > CONNECTIVITY_DEVICE_LIMIT;
    let adapter = objects
        .iter()
        .find(|(_, interfaces)| interfaces.contains_key("org.bluez.Adapter1"));
    let Some((_, adapter_interfaces)) = adapter else {
        return Ok((BluetoothStatus::default(), partial));
    };
    let properties = &adapter_interfaces["org.bluez.Adapter1"];
    let powered = property::<bool>(properties, "Powered").unwrap_or(false);
    let discovering = property::<bool>(properties, "Discovering").unwrap_or(false);
    let mut devices = objects
        .iter()
        .take(CONNECTIVITY_DEVICE_LIMIT)
        .filter_map(|(path, interfaces)| {
            let properties = interfaces.get("org.bluez.Device1")?;
            let raw_name = property::<String>(properties, "Alias")
                .or_else(|| property::<String>(properties, "Name"))
                .unwrap_or_else(|| "Unknown device".into());
            partial |= raw_name.chars().count() > CONNECTIVITY_TEXT_LIMIT;
            let name = raw_name.chars().take(CONNECTIVITY_TEXT_LIMIT).collect();
            Some(BluetoothDeviceStatus {
                id: path.as_str().to_owned(),
                name,
                paired: property::<bool>(properties, "Paired").unwrap_or(false),
                connected: property::<bool>(properties, "Connected").unwrap_or(false),
            })
        })
        .collect::<Vec<_>>();
    devices.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.paired.cmp(&left.paired))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok((
        BluetoothStatus {
            available: true,
            powered,
            discovering,
            devices,
        },
        partial,
    ))
}

pub(super) fn managed_bluez_objects(connection: &Connection) -> zbus::Result<ManagedObjects> {
    Proxy::new(connection, BLUEZ, "/", "org.freedesktop.DBus.ObjectManager")?
        .call("GetManagedObjects", &())
}

fn property<T>(properties: &Properties, name: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    properties
        .get(name)?
        .try_clone()
        .ok()
        .and_then(|value| T::try_from(value).ok())
}

enum PreparedControl {
    Property {
        destination: &'static str,
        path: String,
        interface: &'static str,
        name: &'static str,
        value: bool,
    },
    Method {
        destination: &'static str,
        path: String,
        interface: &'static str,
        method: &'static str,
    },
    Activate {
        connection: OwnedObjectPath,
        device: OwnedObjectPath,
        access_point: OwnedObjectPath,
    },
}

impl PreparedControl {
    fn execute(&self, connection: &Connection) -> Result<(), String> {
        match self {
            Self::Property {
                destination,
                path,
                interface,
                name,
                value,
            } => Proxy::new(connection, *destination, path.as_str(), *interface)
                .and_then(|proxy| Ok(proxy.set_property(name, *value)?))
                .map_err(|error| error.to_string()),
            Self::Method {
                destination,
                path,
                interface,
                method,
            } => Proxy::new(connection, *destination, path.as_str(), *interface)
                .and_then(|proxy| proxy.call_method(*method, &()))
                .map(|_| ())
                .map_err(|error| error.to_string()),
            Self::Activate {
                connection: profile,
                device,
                access_point,
            } => Proxy::new(
                connection,
                NETWORK_MANAGER,
                NETWORK_MANAGER_PATH,
                NETWORK_MANAGER,
            )
            .and_then(|proxy| {
                proxy.call_method("ActivateConnection", &(profile, device, access_point))
            })
            .map(|_| ())
            .map_err(|error| error.to_string()),
        }
    }

    fn confirmed(&self, connection: &Connection, service_owner: &str) -> bool {
        match self {
            Self::Property {
                destination: _,
                path,
                interface,
                name,
                value,
            } => {
                observed_property::<bool>(connection, service_owner, path, interface, name)
                    == Some(*value)
            }
            Self::Method {
                destination: _,
                path,
                interface,
                method,
            } => {
                let (name, desired) = match *method {
                    "StartDiscovery" => ("Discovering", true),
                    "StopDiscovery" => ("Discovering", false),
                    "Connect" => ("Connected", true),
                    "Disconnect" => ("Connected", false),
                    _ => return false,
                };
                observed_property::<bool>(connection, service_owner, path, interface, name)
                    == Some(desired)
            }
            Self::Activate {
                device,
                access_point,
                ..
            } => {
                observed_property::<OwnedObjectPath>(
                    connection,
                    service_owner,
                    device.as_str(),
                    "org.freedesktop.NetworkManager.Device.Wireless",
                    "ActiveAccessPoint",
                )
                .as_ref()
                    == Some(access_point)
            }
        }
    }

    fn message(&self, service_owner: &str) -> zbus::Result<zbus::Message> {
        use zbus::message::Flags;
        match self {
            Self::Property {
                destination: _,
                path,
                interface,
                name,
                value,
            } => zbus::Message::method_call(path.as_str(), "Set")?
                .destination(service_owner)?
                .interface("org.freedesktop.DBus.Properties")?
                .with_flags(Flags::NoReplyExpected)?
                .build(&(*interface, *name, zbus::zvariant::Value::new(*value))),
            Self::Method {
                destination: _,
                path,
                interface,
                method,
            } => zbus::Message::method_call(path.as_str(), *method)?
                .destination(service_owner)?
                .interface(*interface)?
                .with_flags(Flags::NoReplyExpected)?
                .build(&()),
            Self::Activate {
                connection,
                device,
                access_point,
            } => zbus::Message::method_call(NETWORK_MANAGER_PATH, "ActivateConnection")?
                .destination(service_owner)?
                .interface(NETWORK_MANAGER)?
                .with_flags(Flags::NoReplyExpected)?
                .build(&(connection, device, access_point)),
        }
    }
}

// Explicit Properties.Get bypasses proxy caches. Confirmation represents a fresh
// service observation, not a cached value left over from an earlier command.
fn observed_property<T: TryFrom<OwnedValue>>(
    connection: &Connection,
    destination: &str,
    path: &str,
    interface: &str,
    name: &str,
) -> Option<T> {
    Proxy::new(
        connection,
        destination,
        path,
        "org.freedesktop.DBus.Properties",
    )
    .ok()?
    .call::<_, _, OwnedValue>("Get", &(interface, name))
    .ok()
    .and_then(|value| T::try_from(value).ok())
}

fn apply_command(connection: &Connection, command: Command) -> Result<(), String> {
    prepare_command(connection, command)?.execute(connection)
}

fn prepare_command(connection: &Connection, command: Command) -> Result<PreparedControl, String> {
    prepare_command_with_guard(connection, command, None)
}

fn prepare_command_with_guard(
    connection: &Connection,
    command: Command,
    permit: Option<&nickel_remote_control::DesktopPermit>,
) -> Result<PreparedControl, String> {
    Ok(match command {
        Command::SetWifiEnabled(value) => PreparedControl::Property {
            destination: NETWORK_MANAGER,
            path: NETWORK_MANAGER_PATH.into(),
            interface: NETWORK_MANAGER,
            name: "WirelessEnabled",
            value,
        },
        Command::ActivateWifi(id) => {
            let (device_path, access_point_path) = id
                .split_once('\t')
                .ok_or("invalid Wi-Fi network identity")?;
            let access_point = Proxy::new(
                connection,
                NETWORK_MANAGER,
                access_point_path,
                "org.freedesktop.NetworkManager.AccessPoint",
            )
            .map_err(|error| error.to_string())?;
            let ssid = access_point
                .get_property::<Vec<u8>>("Ssid")
                .map_err(|error| error.to_string())?;
            let saved = if let Some(permit) = permit {
                nickel_platform::network_manager_saved_wifi_connections_bounded(
                    connection,
                    64,
                    || permit.check_live().is_ok(),
                )
            } else {
                nickel_platform::network_manager_saved_wifi_connections(connection)
            };
            let profile = saved
                .get(&ssid)
                .ok_or("network has no saved connection profile")?;
            PreparedControl::Activate {
                connection: profile.clone(),
                device: OwnedObjectPath::try_from(device_path)
                    .map_err(|error| error.to_string())?,
                access_point: OwnedObjectPath::try_from(access_point_path)
                    .map_err(|error| error.to_string())?,
            }
        }
        Command::SetBluetoothPowered(value) => PreparedControl::Property {
            destination: BLUEZ,
            path: bluetooth_adapter_path(connection)?.to_string(),
            interface: "org.bluez.Adapter1",
            name: "Powered",
            value,
        },
        Command::SetBluetoothDiscovery(discovering) => PreparedControl::Method {
            destination: BLUEZ,
            path: bluetooth_adapter_path(connection)?.to_string(),
            interface: "org.bluez.Adapter1",
            method: if discovering {
                "StartDiscovery"
            } else {
                "StopDiscovery"
            },
        },
        Command::ToggleBluetoothDevice(path) => {
            let objects = managed_bluez_objects(connection).map_err(|error| error.to_string())?;
            let connected = objects
                .iter()
                .find(|(object_path, _)| object_path.as_str() == path)
                .and_then(|(_, interfaces)| interfaces.get("org.bluez.Device1"))
                .and_then(|properties| property::<bool>(properties, "Connected"))
                .unwrap_or(false);
            PreparedControl::Method {
                destination: BLUEZ,
                path,
                interface: "org.bluez.Device1",
                method: if connected { "Disconnect" } else { "Connect" },
            }
        }
        Command::RefreshConnectivity(_) => {
            return Err("refresh command cannot enter the mutation path".into());
        }
    })
}

pub(super) fn service_owner(connection: &Connection, name: &str) -> Option<String> {
    Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .ok()?
    .call("GetNameOwner", &(name,))
    .ok()
}

fn guarded_wifi_target_present(connection: &Connection, id: &str) -> bool {
    let Some((device, access_point)) = id.split_once('\t') else {
        return false;
    };
    let Ok(manager) = Proxy::new(
        connection,
        NETWORK_MANAGER,
        NETWORK_MANAGER_PATH,
        NETWORK_MANAGER,
    ) else {
        return false;
    };
    let Ok(devices) = manager.call::<_, _, Vec<OwnedObjectPath>>("GetDevices", &()) else {
        return false;
    };
    if devices.len() > 64 || !devices.iter().any(|path| path.as_str() == device) {
        return false;
    }
    let Ok(wireless) = Proxy::new(
        connection,
        NETWORK_MANAGER,
        device,
        "org.freedesktop.NetworkManager.Device.Wireless",
    ) else {
        return false;
    };
    wireless
        .get_property::<Vec<OwnedObjectPath>>("AccessPoints")
        .is_ok_and(|points| {
            points.len() <= 256 && points.iter().any(|path| path.as_str() == access_point)
        })
}

pub(super) fn execute_guarded(
    action: crate::control_view::ControlAction,
    permit: nickel_remote_control::DesktopPermit,
    origin: super::linux_guarded_control::GuardedControlOrigin,
) -> super::linux_guarded_control::GuardedControlOutcome {
    use super::linux_guarded_control::GuardedControlOutcome as Outcome;
    use crate::control_view::ControlAction;
    if let ControlAction::SetBluetoothDiscovery(start) = action {
        return super::linux_discovery::execute(start, permit, origin);
    }
    if origin.check().is_err() {
        return Outcome::Cancelled;
    }
    let evidence = nickel_remote_control::leases::ResourceEvidence {
        surface: None,
        window: None,
        verified_application: None,
        output: None,
        authorized_surface_ancestors: &[],
        protected: false,
    };
    if permit.with_resource(&evidence, || Ok(())).is_err() {
        return Outcome::Cancelled;
    }
    let Ok(address) = zbus::Address::system() else {
        return Outcome::Unavailable;
    };
    let Ok((connection, sender)) = nickel_platform::bounded_dbus::connect_guarded_blocking(
        address,
        nickel_platform::bounded_dbus::Limits::ACCESSIBILITY,
        Duration::from_millis(300),
    ) else {
        return Outcome::Unavailable;
    };
    let service = match &action {
        ControlAction::SetWifiEnabled(_) | ControlAction::ActivateWifi { .. } => NETWORK_MANAGER,
        _ => BLUEZ,
    };
    let Some(owner) = service_owner(&connection, service) else {
        return Outcome::Unavailable;
    };
    // Resolve caller-visible identities through the same production inventory.
    // A path supplied by an MCP caller is never treated as a general DBus proxy.
    let command = match action {
        ControlAction::SetWifiEnabled(value) => Command::SetWifiEnabled(value),
        ControlAction::ActivateWifi { id } => {
            if !guarded_wifi_target_present(&connection, &id) {
                return Outcome::Unavailable;
            }
            Command::ActivateWifi(id)
        }
        ControlAction::SetBluetoothPowered(value) => Command::SetBluetoothPowered(value),
        ControlAction::SetBluetoothDiscovery(value) => Command::SetBluetoothDiscovery(value),
        ControlAction::ToggleBluetoothDevice { id } => {
            if !managed_bluez_objects(&connection).is_ok_and(|objects| {
                objects.len() <= 128
                    && objects.iter().any(|(path, interfaces)| {
                        path.as_str() == id && interfaces.contains_key("org.bluez.Device1")
                    })
            }) {
                return Outcome::Unavailable;
            }
            Command::ToggleBluetoothDevice(id)
        }
        _ => return Outcome::Unavailable,
    };
    let Ok(prepared) = prepare_command_with_guard(&connection, command.clone(), Some(&permit))
    else {
        return Outcome::Unavailable;
    };
    if service_owner(&connection, service).as_deref() != Some(owner.as_str()) {
        return Outcome::Unavailable;
    }
    let Ok(message) = prepared.message(&owner) else {
        return Outcome::Unavailable;
    };
    if message.data().len() > 8192 {
        return Outcome::Unavailable;
    }

    if origin.expects_device() {
        let domain = if service == NETWORK_MANAGER {
            nickel_remote_control::device_settings::Domain::Wifi
        } else {
            nickel_remote_control::device_settings::Domain::Bluetooth
        };
        let target = match &prepared {
            PreparedControl::Property { path, .. } | PreparedControl::Method { path, .. } => {
                path.as_str()
            }
            _ => return Outcome::Unavailable,
        };
        if origin.validate_bus_target(&owner, target).is_err() {
            return Outcome::Unavailable;
        }
        let observed = observe_device_on(&connection, domain);
        if observed
            .as_ref()
            .map_or(true, |value| origin.validate_observation(value).is_err())
        {
            return Outcome::Unavailable;
        }
    }
    // The dedicated connection has no exported objects and no other writer.
    // Recheck authority at every native write. Incomplete messages retire the
    // dedicated socket, so no later task can send the abandoned remainder.
    let mut attempted = false;
    let mut not_accepted = false;
    let result = origin.with_boundary(&permit, &evidence, |boundary| {
        permit.check_commit_boundary(boundary)?;
        origin.check()?;
        attempted = true;
        sender
            .send_guarded(&message, || {
                permit.check_commit_boundary(boundary)?;
                origin.check()
            })
            .map_err(|error| {
                not_accepted =
                    error == nickel_platform::bounded_dbus::GuardedSendError::NotAccepted;
                error.to_string()
            })
    });
    if result.is_err() {
        return if not_accepted {
            Outcome::Unavailable
        } else if attempted {
            Outcome::Uncertain
        } else {
            Outcome::Cancelled
        };
    }
    // NoReplyExpected avoids holding authority over a service reply. Confirm
    // the exact prepared object/property; failed queries never count as false.
    if permit.check_live().is_err() || origin.check().is_err() {
        return Outcome::Uncertain;
    }
    let confirmed = prepared.confirmed(&connection, &owner);
    if permit.check_live().is_err() || origin.check().is_err() {
        Outcome::Uncertain
    } else if confirmed {
        Outcome::Confirmed
    } else {
        Outcome::Requested
    }
}

fn bluetooth_adapter_path(connection: &Connection) -> Result<OwnedObjectPath, String> {
    managed_bluez_objects(connection)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|(_, interfaces)| interfaces.contains_key("org.bluez.Adapter1"))
        .map(|(path, _)| path)
        .ok_or_else(|| "no Bluetooth adapter is available".to_owned())
}

pub(super) fn observe_device_on(
    connection: &Connection,
    domain: nickel_remote_control::device_settings::Domain,
) -> Result<super::linux_device_settings::GuardedDeviceObservation, String> {
    use super::linux_device_settings::{GuardedDeviceObservation, NativeIdentity};
    use nickel_remote_control::device_settings::{Domain, Values};
    let service = match domain {
        Domain::Wifi => NETWORK_MANAGER,
        Domain::Bluetooth => BLUEZ,
        _ => return Err("unsupported connectivity domain".into()),
    };
    let owner = service_owner(connection, service).ok_or("device service unavailable")?;
    let path = if domain == Domain::Wifi {
        NETWORK_MANAGER_PATH.to_owned()
    } else {
        let objects =
            managed_bluez_objects(connection).map_err(|_| "Bluetooth inventory unavailable")?;
        if objects.len() > 128 {
            return Err("Bluetooth inventory exceeded bound".into());
        }
        let adapters = objects
            .iter()
            .filter(|(_, interfaces)| interfaces.contains_key("org.bluez.Adapter1"))
            .collect::<Vec<_>>();
        if adapters.len() != 1 {
            return Err("Bluetooth adapter selection is ambiguous or unavailable".into());
        }
        adapters[0].0.to_string()
    };
    let values = if domain == Domain::Wifi {
        Values::Wifi {
            powered: observed_property(
                connection,
                &owner,
                &path,
                NETWORK_MANAGER,
                "WirelessEnabled",
            )
            .ok_or("Wi-Fi power unobserved")?,
        }
    } else {
        Values::Bluetooth {
            powered: observed_property(connection, &owner, &path, "org.bluez.Adapter1", "Powered")
                .ok_or("Bluetooth power unobserved")?,
            discovering: observed_property(
                connection,
                &owner,
                &path,
                "org.bluez.Adapter1",
                "Discovering",
            )
            .ok_or("Bluetooth discovery unobserved")?,
        }
    };
    if service_owner(connection, service).as_ref() != Some(&owner) {
        return Err("device service changed".into());
    }
    Ok(GuardedDeviceObservation {
        values,
        identity: NativeIdentity::Bus { owner, path },
    })
}
pub(super) fn observe_guarded_device(
    domain: nickel_remote_control::device_settings::Domain,
    permit: &nickel_remote_control::DesktopPermit,
) -> Result<super::linux_device_settings::GuardedDeviceObservation, String> {
    permit.check_live()?;
    let address = zbus::Address::system().map_err(|_| "device bus unavailable")?;
    let (connection, _) = nickel_platform::bounded_dbus::connect_guarded_blocking(
        address,
        nickel_platform::bounded_dbus::Limits::ACCESSIBILITY,
        Duration::from_millis(300),
    )
    .map_err(|_| "device bus unavailable")?;
    let observation = observe_device_on(&connection, domain)?;
    permit.check_live()?;
    Ok(observation)
}

#[cfg(test)]
mod tests {
    use super::{MPRIS_PLAYER_CAPACITY, MprisPlayer, MprisTracker, refresh_connectivity_snapshot};
    use nickel_session_protocol::ConsumerControl;

    #[derive(Default)]
    struct PrivateWifi {
        enabled: bool,
    }
    #[zbus::interface(name = "org.freedesktop.NetworkManager")]
    impl PrivateWifi {
        #[zbus(property)]
        fn wireless_enabled(&self) -> bool {
            self.enabled
        }
        #[zbus(property)]
        fn set_wireless_enabled(&mut self, value: bool) {
            self.enabled = value;
        }
    }

    #[test]
    #[ignore = "requires explicitly owned private D-Bus daemon"]
    fn private_wifi_fixture_service_for_mcp() {
        assert_eq!(
            std::env::var("NICKEL_TEST_WIFI_FIXTURE_SERVICE").unwrap(),
            "1"
        );
        let address = std::env::var("NICKEL_TEST_GUARDED_DBUS_ADDRESS").unwrap();
        assert!(address.starts_with("unix:path=/tmp/nickel-guarded-standing/"));
        let _service = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .name(super::NETWORK_MANAGER)
            .unwrap()
            .serve_at(super::NETWORK_MANAGER_PATH, PrivateWifi::default())
            .unwrap()
            .build()
            .unwrap();
        std::fs::write("/tmp/nickel-guarded-standing/wifi-ready", "ready").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(120));
    }

    #[test]
    fn requested_connectivity_preparation_never_publishes_before_owner_commit() {
        let network = std::sync::Arc::new(std::sync::RwLock::new(super::NetworkStatus {
            available: true,
            ..Default::default()
        }));
        let bluetooth = std::sync::Arc::new(std::sync::RwLock::new(super::BluetoothStatus {
            available: true,
            ..Default::default()
        }));
        let (_sender, receiver) = crate::platform::status_mailbox::channel();
        let subscribers = std::sync::Arc::new(std::sync::Mutex::new(vec![receiver.sender()]));
        let (reply, response) = std::sync::mpsc::sync_channel(1);
        refresh_connectivity_snapshot(None, &network, &bluetooth, &subscribers, Some(reply));
        assert!(response.recv().unwrap().is_err());
        assert!(network.read().unwrap().available);
        assert!(bluetooth.read().unwrap().available);
        assert!(receiver.drain().is_empty());
    }

    #[test]
    #[ignore = "requires the native Linux system bus"]
    fn native_connectivity_refresh_returns_a_bounded_production_snapshot() {
        let refresh = super::refresh_connectivity().expect("native connectivity worker refresh");
        assert!(refresh.network.networks.len() <= super::CONNECTIVITY_DEVICE_LIMIT);
        assert!(refresh.bluetooth.devices.len() <= super::CONNECTIVITY_DEVICE_LIMIT);
        assert!(refresh.network.networks.iter().all(|entry| {
            entry.id.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT
                && entry.name.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT
        }));
        assert!(refresh.bluetooth.devices.iter().all(|entry| {
            entry.id.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT
                && entry.name.chars().count() <= super::CONNECTIVITY_TEXT_LIMIT
        }));
    }

    #[test]
    #[ignore = "requires explicitly owned private DBus daemon"]
    fn private_dbus_guarded_message_flush_updates_only_owned_service() {
        use super::*;
        let address = std::env::var("NICKEL_TEST_GUARDED_DBUS_ADDRESS")
            .expect("private bus address required");
        assert!(address.starts_with("unix:path=/tmp/nickel-guarded-"));
        let _service = zbus::blocking::connection::Builder::address(address.as_str())
            .unwrap()
            .name(NETWORK_MANAGER)
            .unwrap()
            .serve_at(NETWORK_MANAGER_PATH, PrivateWifi::default())
            .unwrap()
            .build()
            .unwrap();
        let (connection, sender) = nickel_platform::bounded_dbus::connect_guarded_blocking(
            address.parse().unwrap(),
            nickel_platform::bounded_dbus::Limits::ACCESSIBILITY,
            Duration::from_millis(300),
        )
        .unwrap();
        let prepared = prepare_command(&connection, Command::SetWifiEnabled(true)).unwrap();
        let owner = service_owner(&connection, NETWORK_MANAGER).unwrap();
        assert!(owner.starts_with(':'));
        let message = prepared.message(&owner).unwrap();
        assert!(message.data().len() < 8192);
        sender.send_guarded(&message, || Ok(())).unwrap();
        assert!(prepared.confirmed(&connection, &owner));
        let proxy = Proxy::new(
            &connection,
            NETWORK_MANAGER,
            NETWORK_MANAGER_PATH,
            NETWORK_MANAGER,
        )
        .unwrap();
        assert!(proxy.get_property::<bool>("WirelessEnabled").unwrap());
        prepare_command(&connection, Command::SetWifiEnabled(false))
            .unwrap()
            .execute(&connection)
            .unwrap();
        assert_eq!(
            observed_property::<bool>(
                &connection,
                NETWORK_MANAGER,
                NETWORK_MANAGER_PATH,
                NETWORK_MANAGER,
                "WirelessEnabled"
            ),
            Some(false)
        );
    }

    fn player(name: &str, owner: &str, status: &str) -> MprisPlayer {
        MprisPlayer {
            name: name.into(),
            owner: owner.into(),
            status: status.into(),
            can_play: true,
            can_pause: true,
            can_control: true,
            can_next: true,
            can_previous: true,
            can_seek: true,
            recent: 0,
        }
    }

    #[test]
    fn wifi_network_identity_contains_device_and_access_point() {
        let id = "/device/wlan0\t/access-point/7";
        assert_eq!(
            id.split_once('\t'),
            Some(("/device/wlan0", "/access-point/7"))
        );
    }

    #[test]
    fn mpris_selection_prefers_playing_then_recent_then_stable_name() {
        let mut tracker = MprisTracker::default();
        let mut zeta = player("org.mpris.MediaPlayer2.zeta", ":1.1", "Paused");
        zeta.recent = 40;
        let mut beta = player("org.mpris.MediaPlayer2.beta", ":1.2", "Playing");
        beta.recent = 3;
        let mut alpha = player("org.mpris.MediaPlayer2.alpha", ":1.3", "Playing");
        alpha.recent = 3;
        tracker.players = [zeta, beta, alpha]
            .into_iter()
            .map(|player| (player.name.clone(), player))
            .collect();
        assert_eq!(
            tracker
                .select(ConsumerControl::PlayPause)
                .map(|value| value.name.as_str()),
            Some("org.mpris.MediaPlayer2.alpha"),
        );

        tracker
            .players
            .get_mut("org.mpris.MediaPlayer2.beta")
            .unwrap()
            .recent = 9;
        assert_eq!(
            tracker
                .select(ConsumerControl::PlayPause)
                .map(|value| value.name.as_str()),
            Some("org.mpris.MediaPlayer2.beta"),
        );

        tracker
            .players
            .values_mut()
            .for_each(|player| player.status = "Paused".into());
        assert_eq!(
            tracker
                .select(ConsumerControl::PlayPause)
                .map(|value| value.name.as_str()),
            Some("org.mpris.MediaPlayer2.zeta"),
        );
    }

    #[test]
    fn mpris_capabilities_filter_every_command_without_losing_selection() {
        let mut tracker = MprisTracker::default();
        let mut incapable = player("incapable", ":1.1", "Playing");
        incapable.can_next = false;
        let capable = player("capable", ":1.2", "Paused");
        tracker.replace_snapshot(vec![incapable, capable]);
        assert_eq!(
            tracker
                .select(ConsumerControl::Next)
                .map(|value| value.name.as_str()),
            Some("capable")
        );
        tracker.players.get_mut("capable").unwrap().can_control = false;
        assert!(tracker.select(ConsumerControl::Next).is_none());
        assert!(tracker.select(ConsumerControl::VolumeUp).is_none());
    }

    #[test]
    fn mpris_snapshots_track_disappearance_owner_replacement_and_bus_restart() {
        let mut tracker = MprisTracker::default();
        tracker.replace_snapshot(vec![player("alpha", ":1.1", "Paused")]);
        tracker.dispatched("alpha", ":1.1");
        assert_eq!(tracker.players["alpha"].recent, 1);

        tracker.replace_snapshot(vec![player("alpha", ":1.1", "Playing")]);
        assert_eq!(
            tracker.players["alpha"].recent, 2,
            "external status changes count as recent player activity"
        );
        tracker.replace_snapshot(vec![player("alpha", ":1.9", "Paused")]);
        assert_eq!(
            tracker.players["alpha"].recent, 0,
            "owner replacement is new session"
        );
        tracker.replace_snapshot(Vec::new());
        assert!(tracker.players.is_empty(), "disappeared names are removed");

        tracker.replace_snapshot(vec![player("beta", ":2.1", "Playing")]);
        tracker.bus_restarted();
        assert!(tracker.players.is_empty());
        assert_eq!(tracker.bus_generation, 1);
    }

    #[test]
    fn mpris_snapshot_and_activity_state_are_strictly_bounded() {
        let mut tracker = MprisTracker::default();
        tracker.replace_snapshot(
            (0..MPRIS_PLAYER_CAPACITY + 20)
                .map(|index| {
                    player(
                        &format!("player-{index:03}"),
                        &format!(":1.{index}"),
                        "Paused",
                    )
                })
                .collect(),
        );
        assert_eq!(tracker.players.len(), MPRIS_PLAYER_CAPACITY);
    }
}
