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

use super::super::{BluetoothDeviceStatus, BluetoothStatus, NetworkStatus, WifiNetworkStatus};
use nickel_session_protocol::ConsumerControl;

const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
const NETWORK_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";
const BLUEZ: &str = "org.bluez";

type Properties = HashMap<String, OwnedValue>;
type Interfaces = HashMap<String, Properties>;
type ManagedObjects = HashMap<OwnedObjectPath, Interfaces>;

#[derive(Debug)]
enum Command {
    SetWifiEnabled(bool),
    ActivateWifi(String),
    SetBluetoothPowered(bool),
    SetBluetoothDiscovery(bool),
    ToggleBluetoothDevice(String),
}

struct ControlBackend {
    network: Arc<RwLock<NetworkStatus>>,
    bluetooth: Arc<RwLock<BluetoothStatus>>,
    commands: mpsc::Sender<Command>,
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

pub fn handle_consumer_control(control: ConsumerControl) {
    match control {
        ConsumerControl::VolumeUp | ConsumerControl::VolumeDown | ConsumerControl::VolumeMute => {
            tracing::warn!(?control, "PipeWire audio control is not initialized");
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

fn backend() -> &'static ControlBackend {
    BACKEND.get_or_init(|| {
        let network = Arc::new(RwLock::new(NetworkStatus::default()));
        let bluetooth = Arc::new(RwLock::new(BluetoothStatus::default()));
        let (commands, receiver) = mpsc::channel();
        let worker_network = network.clone();
        let worker_bluetooth = bluetooth.clone();
        let _ = thread::Builder::new()
            .name("nickel-linux-control".into())
            .spawn(move || worker(worker_network, worker_bluetooth, receiver));
        ControlBackend {
            network,
            bluetooth,
            commands,
        }
    })
}

fn worker(
    network: Arc<RwLock<NetworkStatus>>,
    bluetooth: Arc<RwLock<BluetoothStatus>>,
    commands: mpsc::Receiver<Command>,
) {
    let system = Connection::system().ok();
    let mut next_refresh = Instant::now();
    loop {
        let timeout = next_refresh.saturating_duration_since(Instant::now());
        match commands.recv_timeout(timeout.min(Duration::from_millis(250))) {
            Ok(command) => {
                if let Some(connection) = system.as_ref()
                    && let Err(error) = apply_command(connection, command)
                {
                    tracing::warn!(%error, "Linux Control Center command failed");
                }
                next_refresh = Instant::now();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }

        if Instant::now() < next_refresh {
            continue;
        }
        let network_snapshot = system
            .as_ref()
            .and_then(|connection| read_network_status(connection).ok())
            .unwrap_or_default();
        let bluetooth_snapshot = system
            .as_ref()
            .and_then(|connection| read_bluetooth_status(connection).ok())
            .unwrap_or_default();
        if let Ok(mut current) = network.write() {
            *current = network_snapshot;
        }
        if let Ok(mut current) = bluetooth.write() {
            *current = bluetooth_snapshot;
        }
        next_refresh = Instant::now() + Duration::from_secs(2);
    }
}

fn read_network_status(connection: &Connection) -> zbus::Result<NetworkStatus> {
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
    let saved = saved_wifi_connections(connection);
    let mut networks = Vec::new();

    for device_path in devices {
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
        for access_point_path in access_points {
            let access_point = Proxy::new(
                connection,
                NETWORK_MANAGER,
                access_point_path.as_str(),
                "org.freedesktop.NetworkManager.AccessPoint",
            )?;
            let ssid = access_point
                .get_property::<Vec<u8>>("Ssid")
                .unwrap_or_default();
            let name = String::from_utf8_lossy(&ssid).trim().to_owned();
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

    Ok(NetworkStatus {
        available: true,
        enabled,
        connected: active.is_some(),
        name: active
            .map(|network| network.name.clone())
            .unwrap_or_default(),
        signal_percent: active.map(|network| network.signal_percent).unwrap_or(0),
        networks,
    })
}

fn saved_wifi_connections(connection: &Connection) -> HashMap<Vec<u8>, OwnedObjectPath> {
    let Ok(settings) = Proxy::new(
        connection,
        NETWORK_MANAGER,
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    ) else {
        return HashMap::new();
    };
    let paths = settings
        .call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &())
        .unwrap_or_default();
    paths
        .into_iter()
        .filter_map(|path| {
            let proxy = Proxy::new(
                connection,
                NETWORK_MANAGER,
                path.as_str(),
                "org.freedesktop.NetworkManager.Settings.Connection",
            )
            .ok()?;
            let values = proxy
                .call::<_, _, HashMap<String, Properties>>("GetSettings", &())
                .ok()?;
            let ssid = values
                .get("802-11-wireless")?
                .get("ssid")?
                .try_clone()
                .ok()
                .and_then(|value| Vec::<u8>::try_from(value).ok())?;
            Some((ssid, path.clone()))
        })
        .collect()
}

fn read_bluetooth_status(connection: &Connection) -> zbus::Result<BluetoothStatus> {
    let objects = managed_bluez_objects(connection)?;
    let adapter = objects
        .iter()
        .find(|(_, interfaces)| interfaces.contains_key("org.bluez.Adapter1"));
    let Some((_, adapter_interfaces)) = adapter else {
        return Ok(BluetoothStatus::default());
    };
    let properties = &adapter_interfaces["org.bluez.Adapter1"];
    let powered = property::<bool>(properties, "Powered").unwrap_or(false);
    let discovering = property::<bool>(properties, "Discovering").unwrap_or(false);
    let mut devices = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let properties = interfaces.get("org.bluez.Device1")?;
            let name = property::<String>(properties, "Alias")
                .or_else(|| property::<String>(properties, "Name"))
                .unwrap_or_else(|| "Unknown device".into());
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
    Ok(BluetoothStatus {
        available: true,
        powered,
        discovering,
        devices,
    })
}

fn managed_bluez_objects(connection: &Connection) -> zbus::Result<ManagedObjects> {
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

fn apply_command(connection: &Connection, command: Command) -> Result<(), String> {
    match command {
        Command::SetWifiEnabled(enabled) => {
            let proxy = Proxy::new(
                connection,
                NETWORK_MANAGER,
                NETWORK_MANAGER_PATH,
                NETWORK_MANAGER,
            )
            .map_err(|error| error.to_string())?;
            proxy
                .set_property("WirelessEnabled", enabled)
                .map_err(|error| error.to_string())
        }
        Command::ActivateWifi(id) => activate_wifi(connection, &id),
        Command::SetBluetoothPowered(powered) => {
            let path = bluetooth_adapter_path(connection)?;
            let proxy = Proxy::new(connection, BLUEZ, path.as_str(), "org.bluez.Adapter1")
                .map_err(|error| error.to_string())?;
            proxy
                .set_property("Powered", powered)
                .map_err(|error| error.to_string())
        }
        Command::SetBluetoothDiscovery(discovering) => {
            let path = bluetooth_adapter_path(connection)?;
            let proxy = Proxy::new(connection, BLUEZ, path.as_str(), "org.bluez.Adapter1")
                .map_err(|error| error.to_string())?;
            let method = if discovering {
                "StartDiscovery"
            } else {
                "StopDiscovery"
            };
            proxy
                .call_method(method, &())
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
        Command::ToggleBluetoothDevice(path) => {
            let objects = managed_bluez_objects(connection).map_err(|error| error.to_string())?;
            let connected = objects
                .iter()
                .find(|(object_path, _)| object_path.as_str() == path)
                .and_then(|(_, interfaces)| interfaces.get("org.bluez.Device1"))
                .and_then(|properties| property::<bool>(properties, "Connected"))
                .unwrap_or(false);
            let proxy = Proxy::new(connection, BLUEZ, path, "org.bluez.Device1")
                .map_err(|error| error.to_string())?;
            proxy
                .call_method(if connected { "Disconnect" } else { "Connect" }, &())
                .map(|_| ())
                .map_err(|error| error.to_string())
        }
    }
}

fn activate_wifi(connection: &Connection, id: &str) -> Result<(), String> {
    let (device_path, access_point_path) = id
        .split_once('\t')
        .ok_or_else(|| "invalid Wi-Fi network identity".to_owned())?;
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
    let saved = saved_wifi_connections(connection);
    let connection_path = saved
        .get(&ssid)
        .ok_or_else(|| "network has no saved connection profile".to_owned())?;
    let manager = Proxy::new(
        connection,
        NETWORK_MANAGER,
        NETWORK_MANAGER_PATH,
        NETWORK_MANAGER,
    )
    .map_err(|error| error.to_string())?;
    let specific =
        OwnedObjectPath::try_from(access_point_path).map_err(|error| error.to_string())?;
    let device = OwnedObjectPath::try_from(device_path).map_err(|error| error.to_string())?;
    manager
        .call_method("ActivateConnection", &(connection_path, device, specific))
        .map(|_| ())
        .map_err(|error| error.to_string())
}

fn bluetooth_adapter_path(connection: &Connection) -> Result<OwnedObjectPath, String> {
    managed_bluez_objects(connection)
        .map_err(|error| error.to_string())?
        .into_iter()
        .find(|(_, interfaces)| interfaces.contains_key("org.bluez.Adapter1"))
        .map(|(path, _)| path)
        .ok_or_else(|| "no Bluetooth adapter is available".to_owned())
}

#[cfg(test)]
mod tests {
    use super::{MPRIS_PLAYER_CAPACITY, MprisPlayer, MprisTracker};
    use nickel_session_protocol::ConsumerControl;

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
