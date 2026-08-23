use std::{
    collections::HashMap,
    sync::{Arc, OnceLock, RwLock, mpsc},
    thread,
    time::{Duration, Instant},
};

use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue},
};

use super::super::{BluetoothDeviceStatus, BluetoothStatus, NetworkStatus, WifiNetworkStatus};

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
    #[test]
    fn wifi_network_identity_contains_device_and_access_point() {
        let id = "/device/wlan0\t/access-point/7";
        assert_eq!(
            id.split_once('\t'),
            Some(("/device/wlan0", "/access-point/7"))
        );
    }
}
