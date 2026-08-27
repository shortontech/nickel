use super::*;

pub(super) fn parse_output(line: &str) -> Option<OutputSnapshot> {
    let mut fields = line.split('\t');
    Some(OutputSnapshot {
        name: fields.next()?.to_owned(),
        model: fields.next()?.to_owned(),
        x: fields.next()?.parse().ok()?,
        y: fields.next()?.parse().ok()?,
        width: fields.next()?.parse().ok()?,
        height: fields.next()?.parse().ok()?,
        physical_width: fields.next()?.parse().ok()?,
        physical_height: fields.next()?.parse().ok()?,
        primary: fields.next()? == "1",
    })
}

#[cfg(target_os = "linux")]
type BluezProperties = HashMap<String, OwnedValue>;
#[cfg(target_os = "linux")]
type BluezInterfaces = HashMap<String, BluezProperties>;
#[cfg(target_os = "linux")]
type BluezObjects = HashMap<OwnedObjectPath, BluezInterfaces>;

#[cfg(target_os = "linux")]
const NETWORK_MANAGER: &str = "org.freedesktop.NetworkManager";
#[cfg(target_os = "linux")]
const NETWORK_MANAGER_PATH: &str = "/org/freedesktop/NetworkManager";

#[cfg(target_os = "linux")]
fn bluez_objects(connection: &Connection) -> zbus::Result<BluezObjects> {
    Proxy::new(
        connection,
        "org.bluez",
        "/",
        "org.freedesktop.DBus.ObjectManager",
    )?
    .call("GetManagedObjects", &())
}

#[cfg(target_os = "linux")]
fn bluez_property<T>(properties: &BluezProperties, name: &str) -> Option<T>
where
    T: TryFrom<OwnedValue>,
{
    properties
        .get(name)?
        .try_clone()
        .ok()
        .and_then(|value| T::try_from(value).ok())
}

#[cfg(target_os = "linux")]
pub(super) fn read_bluetooth_snapshot() -> Result<BluetoothSnapshot, String> {
    let connection = Connection::system().map_err(|error| error.to_string())?;
    let objects = bluez_objects(&connection).map_err(|error| error.to_string())?;
    let Some((_, adapter_interfaces)) = objects
        .iter()
        .find(|(_, interfaces)| interfaces.contains_key("org.bluez.Adapter1"))
    else {
        return Ok(BluetoothSnapshot::default());
    };
    let adapter = &adapter_interfaces["org.bluez.Adapter1"];
    let mut devices = objects
        .iter()
        .filter_map(|(path, interfaces)| {
            let properties = interfaces.get("org.bluez.Device1")?;
            let battery_percent = interfaces
                .get("org.bluez.Battery1")
                .and_then(|battery| bluez_property::<u8>(battery, "Percentage"));
            Some(BluetoothDevice {
                id: path.as_str().to_owned(),
                name: bluez_property::<String>(properties, "Alias")
                    .or_else(|| bluez_property::<String>(properties, "Name"))
                    .unwrap_or_else(|| "Unknown device".into()),
                paired: bluez_property::<bool>(properties, "Paired").unwrap_or(false),
                connected: bluez_property::<bool>(properties, "Connected").unwrap_or(false),
                battery_percent,
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
    Ok(BluetoothSnapshot {
        available: true,
        powered: bluez_property::<bool>(adapter, "Powered").unwrap_or(false),
        discovering: bluez_property::<bool>(adapter, "Discovering").unwrap_or(false),
        adapter_name: bluez_property::<String>(adapter, "Alias").unwrap_or_default(),
        devices,
    })
}

#[cfg(target_os = "linux")]
pub(super) fn set_bluetooth_adapter_property(name: &str, value: bool) -> Result<(), String> {
    let connection = Connection::system().map_err(|error| error.to_string())?;
    let objects = bluez_objects(&connection).map_err(|error| error.to_string())?;
    let path = objects
        .iter()
        .find(|(_, interfaces)| interfaces.contains_key("org.bluez.Adapter1"))
        .map(|(path, _)| path)
        .ok_or_else(|| "no Bluetooth adapter is available".to_owned())?;
    let proxy = Proxy::new(
        &connection,
        "org.bluez",
        path.as_str(),
        "org.bluez.Adapter1",
    )
    .map_err(|error| error.to_string())?;
    if name == "Discovering" {
        proxy
            .call_method(
                if value {
                    "StartDiscovery"
                } else {
                    "StopDiscovery"
                },
                &(),
            )
            .map(|_| ())
            .map_err(|error| error.to_string())
    } else {
        proxy
            .set_property(name, value)
            .map_err(|error| error.to_string())
    }
}

#[cfg(target_os = "linux")]
pub(super) fn toggle_bluetooth_device(device: &BluetoothDevice) -> Result<(), String> {
    let connection = Connection::system().map_err(|error| error.to_string())?;
    Proxy::new(
        &connection,
        "org.bluez",
        device.id.as_str(),
        "org.bluez.Device1",
    )
    .map_err(|error| error.to_string())?
    .call_method(
        if device.connected {
            "Disconnect"
        } else {
            "Connect"
        },
        &(),
    )
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn saved_wifi_connections(connection: &Connection) -> HashMap<Vec<u8>, OwnedObjectPath> {
    let Ok(settings) = Proxy::new(
        connection,
        NETWORK_MANAGER,
        "/org/freedesktop/NetworkManager/Settings",
        "org.freedesktop.NetworkManager.Settings",
    ) else {
        return HashMap::new();
    };
    settings
        .call::<_, _, Vec<OwnedObjectPath>>("ListConnections", &())
        .unwrap_or_default()
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
                .call::<_, _, HashMap<String, BluezProperties>>("GetSettings", &())
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

#[cfg(target_os = "linux")]
pub(super) fn read_linux_network() -> Result<(bool, Vec<WifiNetwork>, Vec<NetworkAdapter>), String>
{
    let connection = Connection::system().map_err(|error| error.to_string())?;
    let manager = Proxy::new(
        &connection,
        NETWORK_MANAGER,
        NETWORK_MANAGER_PATH,
        NETWORK_MANAGER,
    )
    .map_err(|error| error.to_string())?;
    let enabled = manager
        .get_property::<bool>("WirelessEnabled")
        .unwrap_or(false);
    let devices = manager
        .call::<_, _, Vec<OwnedObjectPath>>("GetDevices", &())
        .map_err(|error| error.to_string())?;
    let saved = saved_wifi_connections(&connection);
    let mut networks = Vec::new();
    let mut adapters = Vec::new();

    for device_path in devices {
        let device = Proxy::new(
            &connection,
            NETWORK_MANAGER,
            device_path.as_str(),
            "org.freedesktop.NetworkManager.Device",
        )
        .map_err(|error| error.to_string())?;
        let device_type = device.get_property::<u32>("DeviceType").unwrap_or(0);
        let name = device
            .get_property::<String>("Interface")
            .unwrap_or_else(|_| device_path.as_str().to_owned());
        let connected = device.get_property::<u32>("State").unwrap_or(0) == 100;
        let (description, speed) = match device_type {
            1 => {
                let speed = Proxy::new(
                    &connection,
                    NETWORK_MANAGER,
                    device_path.as_str(),
                    "org.freedesktop.NetworkManager.Device.Wired",
                )
                .ok()
                .and_then(|proxy| proxy.get_property::<u32>("Speed").ok())
                .map(u64::from)
                .unwrap_or(0)
                    * 1_000_000;
                ("Ethernet".to_owned(), speed)
            }
            2 => {
                let speed = Proxy::new(
                    &connection,
                    NETWORK_MANAGER,
                    device_path.as_str(),
                    "org.freedesktop.NetworkManager.Device.Wireless",
                )
                .ok()
                .and_then(|proxy| proxy.get_property::<u32>("Bitrate").ok())
                .map(u64::from)
                .unwrap_or(0)
                    * 1_000;
                ("Wi-Fi".to_owned(), speed)
            }
            _ => continue,
        };
        adapters.push(NetworkAdapter {
            name,
            description,
            connected,
            speed,
        });
        if device_type != 2 {
            continue;
        }

        let wireless = Proxy::new(
            &connection,
            NETWORK_MANAGER,
            device_path.as_str(),
            "org.freedesktop.NetworkManager.Device.Wireless",
        )
        .map_err(|error| error.to_string())?;
        let active = wireless
            .get_property::<OwnedObjectPath>("ActiveAccessPoint")
            .ok();
        let access_points = wireless
            .call::<_, _, Vec<OwnedObjectPath>>("GetAllAccessPoints", &())
            .or_else(|_| wireless.get_property::<Vec<OwnedObjectPath>>("AccessPoints"))
            .unwrap_or_default();
        for access_point_path in access_points {
            let access_point = Proxy::new(
                &connection,
                NETWORK_MANAGER,
                access_point_path.as_str(),
                "org.freedesktop.NetworkManager.AccessPoint",
            )
            .map_err(|error| error.to_string())?;
            let ssid = access_point
                .get_property::<Vec<u8>>("Ssid")
                .unwrap_or_default();
            let profile = String::from_utf8_lossy(&ssid).trim().to_owned();
            if profile.is_empty() {
                continue;
            }
            let flags = access_point.get_property::<u32>("Flags").unwrap_or(0);
            let wpa_flags = access_point.get_property::<u32>("WpaFlags").unwrap_or(0);
            let rsn_flags = access_point.get_property::<u32>("RsnFlags").unwrap_or(0);
            networks.push(WifiNetwork {
                id: format!("{}\t{}", device_path.as_str(), access_point_path.as_str()),
                profile,
                signal: u32::from(access_point.get_property::<u8>("Strength").unwrap_or(0)),
                connected: active.as_ref() == Some(&access_point_path),
                saved: saved.contains_key(&ssid),
                secure: flags & 1 != 0 || wpa_flags != 0 || rsn_flags != 0,
            });
        }
    }
    networks.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.signal.cmp(&left.signal))
            .then_with(|| {
                left.profile
                    .to_lowercase()
                    .cmp(&right.profile.to_lowercase())
            })
    });
    networks.dedup_by(|left, right| left.profile == right.profile);
    adapters.sort_by_key(|adapter| (!adapter.connected, adapter.name.to_lowercase()));
    Ok((enabled, networks, adapters))
}

#[cfg(target_os = "linux")]
pub(super) fn set_linux_wifi_enabled(enabled: bool) -> Result<(), String> {
    let connection = Connection::system().map_err(|error| error.to_string())?;
    Proxy::new(
        &connection,
        NETWORK_MANAGER,
        NETWORK_MANAGER_PATH,
        NETWORK_MANAGER,
    )
    .map_err(|error| error.to_string())?
    .set_property("WirelessEnabled", enabled)
    .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
pub(super) fn activate_linux_wifi(network: &WifiNetwork) -> Result<(), String> {
    let (device_path, access_point_path) = network
        .id
        .split_once('\t')
        .ok_or_else(|| "invalid Wi-Fi network identity".to_owned())?;
    let connection = Connection::system().map_err(|error| error.to_string())?;
    if network.connected {
        return Proxy::new(
            &connection,
            NETWORK_MANAGER,
            device_path,
            "org.freedesktop.NetworkManager.Device",
        )
        .map_err(|error| error.to_string())?
        .call_method("Disconnect", &())
        .map(|_| ())
        .map_err(|error| error.to_string());
    }
    let access_point = Proxy::new(
        &connection,
        NETWORK_MANAGER,
        access_point_path,
        "org.freedesktop.NetworkManager.AccessPoint",
    )
    .map_err(|error| error.to_string())?;
    let ssid = access_point
        .get_property::<Vec<u8>>("Ssid")
        .map_err(|error| error.to_string())?;
    let saved = saved_wifi_connections(&connection);
    let connection_path = saved
        .get(&ssid)
        .ok_or_else(|| "network has no saved connection profile".to_owned())?;
    let specific =
        OwnedObjectPath::try_from(access_point_path).map_err(|error| error.to_string())?;
    let device = OwnedObjectPath::try_from(device_path).map_err(|error| error.to_string())?;
    Proxy::new(
        &connection,
        NETWORK_MANAGER,
        NETWORK_MANAGER_PATH,
        NETWORK_MANAGER,
    )
    .map_err(|error| error.to_string())?
    .call_method("ActivateConnection", &(connection_path, device, specific))
    .map(|_| ())
    .map_err(|error| error.to_string())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn read_bluetooth_snapshot() -> Result<BluetoothSnapshot, String> {
    Ok(BluetoothSnapshot::default())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn set_bluetooth_adapter_property(_name: &str, _value: bool) -> Result<(), String> {
    Err("Bluetooth settings are unavailable on this platform".into())
}

#[cfg(not(target_os = "linux"))]
pub(super) fn toggle_bluetooth_device(_device: &BluetoothDevice) -> Result<(), String> {
    Err("Bluetooth settings are unavailable on this platform".into())
}

#[cfg(target_os = "windows")]
pub(super) fn windows_display_names() -> HashMap<String, String> {
    use std::mem::size_of;
    use windows::Win32::{
        Devices::Display::{
            DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME, DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
            DISPLAYCONFIG_DEVICE_INFO_HEADER, DISPLAYCONFIG_MODE_INFO, DISPLAYCONFIG_PATH_INFO,
            DISPLAYCONFIG_SOURCE_DEVICE_NAME, DISPLAYCONFIG_TARGET_DEVICE_NAME,
            DisplayConfigGetDeviceInfo, GetDisplayConfigBufferSizes, QDC_ONLY_ACTIVE_PATHS,
            QueryDisplayConfig,
        },
        Foundation::ERROR_SUCCESS,
    };

    let mut path_count = 0;
    let mut mode_count = 0;
    // SAFETY: The count pointers are valid writable storage.
    if unsafe {
        GetDisplayConfigBufferSizes(QDC_ONLY_ACTIVE_PATHS, &mut path_count, &mut mode_count)
    } != ERROR_SUCCESS
    {
        return HashMap::new();
    }
    let mut paths = vec![DISPLAYCONFIG_PATH_INFO::default(); path_count as usize];
    let mut modes = vec![DISPLAYCONFIG_MODE_INFO::default(); mode_count as usize];
    // SAFETY: Both vectors have the capacities reported immediately above and the counts remain
    // writable so Windows can report how many entries it populated.
    if unsafe {
        QueryDisplayConfig(
            QDC_ONLY_ACTIVE_PATHS,
            &mut path_count,
            paths.as_mut_ptr(),
            &mut mode_count,
            modes.as_mut_ptr(),
            None,
        )
    } != ERROR_SUCCESS
    {
        return HashMap::new();
    }
    paths.truncate(path_count as usize);
    let mut names = HashMap::new();
    for path in paths {
        let mut source = DISPLAYCONFIG_SOURCE_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_SOURCE_NAME,
                size: size_of::<DISPLAYCONFIG_SOURCE_DEVICE_NAME>() as u32,
                adapterId: path.sourceInfo.adapterId,
                id: path.sourceInfo.id,
            },
            ..Default::default()
        };
        let mut target = DISPLAYCONFIG_TARGET_DEVICE_NAME {
            header: DISPLAYCONFIG_DEVICE_INFO_HEADER {
                r#type: DISPLAYCONFIG_DEVICE_INFO_GET_TARGET_NAME,
                size: size_of::<DISPLAYCONFIG_TARGET_DEVICE_NAME>() as u32,
                adapterId: path.targetInfo.adapterId,
                id: path.targetInfo.id,
            },
            ..Default::default()
        };
        // SAFETY: Each request packet has the documented type, size, adapter, and source/target ID.
        if unsafe { DisplayConfigGetDeviceInfo(&raw mut source.header) } != 0
            || unsafe { DisplayConfigGetDeviceInfo(&raw mut target.header) } != 0
        {
            continue;
        }
        let source_name = wide_text(&source.viewGdiDeviceName);
        let friendly_name = wide_text(&target.monitorFriendlyDeviceName);
        if !source_name.is_empty() && !friendly_name.is_empty() {
            names.insert(source_name, friendly_name);
        }
    }
    names
}

#[cfg(target_os = "windows")]
pub(super) fn wide_text(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

#[cfg(target_os = "linux")]
pub(super) fn session_request(command: &str) -> std::io::Result<String> {
    use std::{os::unix::net::UnixDatagram, path::PathBuf, time::Duration};

    let server = std::env::var_os("NICKEL_SESSION_CONTROL")
        .map(PathBuf::from)
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotFound, "Nickel session socket")
        })?;
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    let client = runtime.join(format!("nickel-settings-{}.sock", std::process::id()));
    let _ = std::fs::remove_file(&client);
    let socket = UnixDatagram::bind(&client)?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    let result = (|| {
        socket.send_to(command.as_bytes(), server)?;
        let mut response = [0_u8; 4096];
        let length = socket.recv(&mut response)?;
        Ok(String::from_utf8_lossy(&response[..length]).into_owned())
    })();
    let _ = std::fs::remove_file(client);
    result
}

#[cfg(not(target_os = "linux"))]
pub(super) fn session_request(_command: &str) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "live display settings are currently Linux-only",
    ))
}
