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
static MPRIS_ACTIVITY: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
static MPRIS_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
static MPRIS_COMMANDS: OnceLock<mpsc::SyncSender<ConsumerControl>> = OnceLock::new();

const MPRIS_COMMAND_CAPACITY: usize = 16;
const MPRIS_ACTIVITY_CAPACITY: usize = 64;

#[derive(Clone, Debug, Eq, PartialEq)]
struct MprisCandidate {
    name: String,
    playing: bool,
    recent: u64,
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
    while let Ok(control) = commands.recv() {
        if let Err(error) = dispatch_mpris(control) {
            tracing::debug!(?control, %error, "MPRIS command was not delivered");
        }
    }
}

fn dispatch_mpris(control: ConsumerControl) -> Result<(), String> {
    let connection = Connection::session().map_err(|error| error.to_string())?;
    let dbus =
        zbus::blocking::fdo::DBusProxy::new(&connection).map_err(|error| error.to_string())?;
    let mut players = dbus
        .list_names()
        .map_err(|error| error.to_string())?
        .into_iter()
        .filter(|name| name.as_str().starts_with("org.mpris.MediaPlayer2."))
        .filter_map(|name| {
            let name = name.to_string();
            let proxy = Proxy::new(
                &connection,
                name.as_str(),
                "/org/mpris/MediaPlayer2",
                "org.mpris.MediaPlayer2.Player",
            )
            .ok()?;
            let can_control = proxy.get_property::<bool>("CanControl").unwrap_or(false);
            let status = proxy
                .get_property::<String>("PlaybackStatus")
                .unwrap_or_else(|_| "Stopped".into());
            let capable = can_control && mpris_capable(&proxy, control, &status);
            drop(proxy);
            let recent = MPRIS_ACTIVITY
                .get_or_init(Default::default)
                .lock()
                .ok()
                .and_then(|activity| activity.get(&name).copied())
                .unwrap_or(0);
            capable.then_some(MprisCandidate {
                name,
                playing: status == "Playing",
                recent,
            })
        })
        .collect::<Vec<_>>();
    let name = select_mpris_candidate(&mut players)
        .ok_or_else(|| "no capable MPRIS player is available".to_owned())?;
    let proxy = Proxy::new(
        &connection,
        name,
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
            let sequence = MPRIS_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if let Ok(mut activity) = MPRIS_ACTIVITY.get_or_init(Default::default).lock() {
                record_mpris_activity(&mut activity, name, sequence);
            }
        })
        .map_err(|error| error.to_string())
}

fn record_mpris_activity(activity: &mut HashMap<String, u64>, name: &str, sequence: u64) {
    activity.insert(name.to_owned(), sequence);
    if activity.len() > MPRIS_ACTIVITY_CAPACITY
        && let Some(oldest) = activity
            .iter()
            .min_by(|left, right| left.1.cmp(right.1).then_with(|| left.0.cmp(right.0)))
            .map(|(name, _)| name.clone())
    {
        activity.remove(&oldest);
    }
}

fn select_mpris_candidate(players: &mut [MprisCandidate]) -> Option<&str> {
    players.sort_by(|left, right| {
        right
            .playing
            .cmp(&left.playing)
            .then_with(|| right.recent.cmp(&left.recent))
            .then_with(|| left.name.cmp(&right.name))
    });
    players.first().map(|candidate| candidate.name.as_str())
}

fn mpris_capable(proxy: &Proxy<'_>, control: ConsumerControl, status: &str) -> bool {
    let property = match control {
        ConsumerControl::PlayPause => {
            return if status == "Playing" {
                proxy.get_property::<bool>("CanPause").unwrap_or(false)
            } else {
                proxy.get_property::<bool>("CanPlay").unwrap_or(false)
            };
        }
        ConsumerControl::Play => "CanPlay",
        ConsumerControl::Pause => "CanPause",
        ConsumerControl::Stop => "CanControl",
        ConsumerControl::Next => "CanGoNext",
        ConsumerControl::Previous => "CanGoPrevious",
        ConsumerControl::FastForward | ConsumerControl::Rewind => "CanSeek",
        ConsumerControl::VolumeUp | ConsumerControl::VolumeDown | ConsumerControl::VolumeMute => {
            return false;
        }
    };
    proxy.get_property::<bool>(property).unwrap_or(false)
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
    use super::{
        MPRIS_ACTIVITY_CAPACITY, MprisCandidate, record_mpris_activity, select_mpris_candidate,
    };

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
        let candidates = || {
            vec![
                MprisCandidate {
                    name: "org.mpris.MediaPlayer2.zeta".into(),
                    playing: false,
                    recent: 40,
                },
                MprisCandidate {
                    name: "org.mpris.MediaPlayer2.beta".into(),
                    playing: true,
                    recent: 3,
                },
                MprisCandidate {
                    name: "org.mpris.MediaPlayer2.alpha".into(),
                    playing: true,
                    recent: 3,
                },
            ]
        };

        let mut players = candidates();
        assert_eq!(
            select_mpris_candidate(&mut players),
            Some("org.mpris.MediaPlayer2.alpha")
        );

        let mut players = candidates();
        players[1].recent = 9;
        assert_eq!(
            select_mpris_candidate(&mut players),
            Some("org.mpris.MediaPlayer2.beta")
        );

        let mut players = candidates();
        players.iter_mut().for_each(|player| player.playing = false);
        assert_eq!(
            select_mpris_candidate(&mut players),
            Some("org.mpris.MediaPlayer2.zeta")
        );
    }

    #[test]
    fn mpris_selection_has_a_bounded_empty_result() {
        assert_eq!(select_mpris_candidate(&mut []), None);
    }

    #[test]
    fn mpris_activity_history_evicts_the_oldest_player() {
        let mut activity = std::collections::HashMap::new();
        for sequence in 0..=MPRIS_ACTIVITY_CAPACITY {
            record_mpris_activity(
                &mut activity,
                &format!("player-{sequence}"),
                sequence as u64,
            );
        }
        assert_eq!(activity.len(), MPRIS_ACTIVITY_CAPACITY);
        assert!(!activity.contains_key("player-0"));
        assert!(activity.contains_key(&format!("player-{MPRIS_ACTIVITY_CAPACITY}")));
    }
}
