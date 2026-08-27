#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

use std::{
    cell::Cell,
    time::{Duration, Instant},
};

#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::collections::HashMap;

#[cfg(target_os = "linux")]
use zbus::{
    blocking::{Connection, Proxy},
    zvariant::{OwnedObjectPath, OwnedValue},
};

use nickel_core::{
    shell_settings::{ShellSettings, ThemePreference},
    theme::{ThemeMode, ThemePalette},
    wallpaper_settings::{WallpaperPosition, WallpaperSettings},
};
use nickel_i18n::Localizer;
use nickel_ui::{
    AnyView, ControllerAction, ControllerInput, Insets, LinearGradient, NavigationPane,
    PaintCommand, PaneNavigation, Point as UiPoint, Rect as UiRect, SdlCanvasPresenter,
    SemanticColors, SemanticTheme, TextAlign, UiStateStore, UiTree, ui,
};
use sdl3::{
    event::{Event, WindowEvent},
    keyboard::Keycode,
    mouse::{MouseButton, MouseWheelDirection},
};

const SIDEBAR_WIDTH: i32 = 190;
const DISPLAY_PLANE: Rect = Rect {
    x: 210,
    y: 96,
    w: 600,
    h: 340,
};

#[derive(Clone, Copy)]
struct Rect {
    x: i32,
    y: i32,
    w: i32,
    h: i32,
}

fn semantic_theme(palette: ThemePalette) -> SemanticTheme {
    SemanticTheme::new(SemanticColors {
        window: palette.background,
        sidebar: palette.panel,
        card: palette.surface,
        raised: palette.surface_hover,
        hover: palette.surface_hover,
        primary_text: palette.text,
        secondary_text: palette.muted,
        accent: palette.accent,
        accent_soft: palette.accent_soft,
        positive: palette.complement,
    })
}

struct OutputSnapshot {
    name: String,
    model: String,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    physical_width: i32,
    physical_height: i32,
    primary: bool,
}

fn parse_output(line: &str) -> Option<OutputSnapshot> {
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
fn read_bluetooth_snapshot() -> Result<BluetoothSnapshot, String> {
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
fn set_bluetooth_adapter_property(name: &str, value: bool) -> Result<(), String> {
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
fn toggle_bluetooth_device(device: &BluetoothDevice) -> Result<(), String> {
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
fn read_linux_network() -> Result<(bool, Vec<WifiNetwork>, Vec<NetworkAdapter>), String> {
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
fn set_linux_wifi_enabled(enabled: bool) -> Result<(), String> {
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
fn activate_linux_wifi(network: &WifiNetwork) -> Result<(), String> {
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
fn read_bluetooth_snapshot() -> Result<BluetoothSnapshot, String> {
    Ok(BluetoothSnapshot::default())
}

#[cfg(not(target_os = "linux"))]
fn set_bluetooth_adapter_property(_name: &str, _value: bool) -> Result<(), String> {
    Err("Bluetooth settings are unavailable on this platform".into())
}

#[cfg(not(target_os = "linux"))]
fn toggle_bluetooth_device(_device: &BluetoothDevice) -> Result<(), String> {
    Err("Bluetooth settings are unavailable on this platform".into())
}

#[cfg(target_os = "windows")]
fn windows_display_names() -> HashMap<String, String> {
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
fn wide_text(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

#[cfg(target_os = "linux")]
fn session_request(command: &str) -> std::io::Result<String> {
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
fn session_request(_command: &str) -> std::io::Result<String> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "live display settings are currently Linux-only",
    ))
}

impl Rect {
    fn contains(self, x: i32, y: i32) -> bool {
        x >= self.x && y >= self.y && x < self.x + self.w && y < self.y + self.h
    }
}

struct DisplayCard {
    connector: String,
    name: String,
    detail: String,
    logical_width: i32,
    logical_height: i32,
    rect: Rect,
    primary: bool,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct BluetoothDevice {
    id: String,
    name: String,
    paired: bool,
    connected: bool,
    battery_percent: Option<u8>,
}

#[derive(Clone, Debug, Default, Eq, PartialEq)]
struct BluetoothSnapshot {
    available: bool,
    powered: bool,
    discovering: bool,
    adapter_name: String,
    devices: Vec<BluetoothDevice>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SettingsPage {
    Display,
    Bar,
    Appearance,
    Network,
    Bluetooth,
}

impl SettingsPage {
    fn previous(self) -> Self {
        match self {
            Self::Display => Self::Display,
            Self::Bar => Self::Display,
            Self::Appearance => Self::Bar,
            Self::Network => Self::Appearance,
            Self::Bluetooth => Self::Network,
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Display => Self::Bar,
            Self::Bar => Self::Appearance,
            Self::Appearance => Self::Network,
            Self::Network => Self::Bluetooth,
            Self::Bluetooth => Self::Bluetooth,
        }
    }
}

struct NetworkAdapter {
    name: String,
    description: String,
    connected: bool,
    speed: u64,
}

struct WifiNetwork {
    id: String,
    profile: String,
    signal: u32,
    connected: bool,
    saved: bool,
    secure: bool,
    #[cfg(target_os = "windows")]
    interface: u128,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SettingsMessage {
    Navigate(SettingsPage),
    BluetoothPower,
    BluetoothDiscovery,
    BluetoothDevice(usize),
    BluetoothScroll,
    WifiPower,
    WifiNetwork(usize),
    NetworkScroll,
    AppearanceLight,
    AppearanceDark,
    SetAppearanceHue(u16),
    SetAppearanceIntensity(u8),
    WallpaperChoose,
    WallpaperPosition(WallpaperPosition),
    BarPrimaryDisplay,
    BarAllDisplays,
    BarDisplayWindows,
    BarAllWindows,
    SetDesktopCount(u8),
    DisplayIdentify,
    DisplayPrimary,
    DisplayApply,
}

fn desktop_count_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::SetDesktopCount(1 + (fraction.clamp(0.0, 1.0) * 7.0).round() as u8)
}

fn appearance_hue_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::SetAppearanceHue((fraction.clamp(0.0, 1.0) * 359.0).round() as u16)
}

fn appearance_intensity_message(fraction: f32) -> SettingsMessage {
    SettingsMessage::SetAppearanceIntensity((fraction.clamp(0.0, 1.0) * 100.0).round() as u8)
}

struct SettingsApp {
    localizer: Localizer,
    redraw_requested: Cell<bool>,
    running: bool,
    displays: Vec<DisplayCard>,
    selected: usize,
    cursor: (i32, i32),
    drag_offset: Option<(i32, i32)>,
    applied: bool,
    pixels_per_logical: f64,
    status: String,
    page: SettingsPage,
    shell_settings: ShellSettings,
    wallpaper_settings: WallpaperSettings,
    appearance_save_deadline: Option<Instant>,
    resize_deadline: Option<Instant>,
    frame_interval: Duration,
    network_adapters: Vec<NetworkAdapter>,
    wifi_networks: Vec<WifiNetwork>,
    network_available: bool,
    wifi_enabled: bool,
    wifi_status: String,
    pending_wifi_profile: Option<String>,
    next_wifi_refresh: Option<Instant>,
    wifi_refreshes_left: u8,
    bluetooth: BluetoothSnapshot,
    next_bluetooth_refresh: Instant,
    next_network_refresh: Instant,
    ui: UiTree<SettingsMessage>,
    ui_state: UiStateStore,
    controller: ControllerInput,
    navigation: PaneNavigation,
    controller_page: SettingsPage,
}

impl Default for SettingsApp {
    fn default() -> Self {
        let localizer = Localizer::system();
        let status = localizer.text("settings-status-changes-not-applied");
        Self {
            localizer,
            redraw_requested: Cell::new(true),
            running: true,
            displays: vec![
                DisplayCard {
                    connector: "DVI-I-1".into(),
                    name: "ASUS MB16ACV".into(),
                    detail: "DISPLAYLINK  1920 X 1080".into(),
                    logical_width: 1920,
                    logical_height: 1080,
                    rect: Rect {
                        x: 225,
                        y: 186,
                        w: 270,
                        h: 160,
                    },
                    primary: false,
                },
                DisplayCard {
                    connector: "DP-3".into(),
                    name: "DP-3".into(),
                    detail: "NVIDIA  1920 X 1080".into(),
                    logical_width: 1920,
                    logical_height: 1080,
                    rect: Rect {
                        x: 495,
                        y: 176,
                        w: 300,
                        h: 180,
                    },
                    primary: true,
                },
            ],
            selected: 1,
            cursor: (0, 0),
            drag_offset: None,
            applied: false,
            pixels_per_logical: 0.14,
            status,
            page: SettingsPage::Display,
            shell_settings: ShellSettings::load_default(),
            wallpaper_settings: WallpaperSettings::load_default(),
            appearance_save_deadline: None,
            resize_deadline: None,
            frame_interval: Duration::from_millis(16),
            network_adapters: Vec::new(),
            wifi_networks: Vec::new(),
            network_available: false,
            wifi_enabled: false,
            wifi_status: String::new(),
            pending_wifi_profile: None,
            next_wifi_refresh: None,
            wifi_refreshes_left: 0,
            bluetooth: BluetoothSnapshot::default(),
            next_bluetooth_refresh: Instant::now(),
            next_network_refresh: Instant::now(),
            ui: UiTree::default(),
            ui_state: UiStateStore::default(),
            controller: ControllerInput::new(),
            navigation: PaneNavigation::default(),
            controller_page: SettingsPage::Display,
        }
    }
}

impl SettingsApp {
    fn transient_scroll(&self, message: &SettingsMessage) -> f32 {
        self.ui
            .id_for_message(message)
            .and_then(|id| self.ui_state.state(id))
            .map(|state| state.scroll_offset)
            .unwrap_or(0.0)
    }

    fn captured_message(&self) -> Option<&SettingsMessage> {
        self.ui_state
            .captured()
            .and_then(|id| self.ui.message_for_id(id))
    }

    fn hovered_message(&self) -> Option<&SettingsMessage> {
        self.ui_state
            .hovered()
            .and_then(|id| self.ui.message_for_id(id))
    }

    fn request_redraw(&self) {
        self.redraw_requested.set(true);
    }

    fn palette(&self) -> ThemePalette {
        ThemePalette::from_appearance(
            self.shell_settings
                .resolve_appearance(nickel_platform::appearance()),
        )
    }

    fn ui_theme(&self) -> SemanticTheme {
        semantic_theme(self.palette())
    }

    fn navigation_item(
        &self,
        message: SettingsMessage,
        message_key: &'static str,
        glyph: &'static str,
        selected: bool,
        palette: ThemePalette,
    ) -> impl nickel_ui::Component<SettingsMessage> {
        let label = self.localizer.text(message_key);
        let underline_width = (label.chars().count() as f32 * 8.0).clamp(24.0, 112.0);
        ui! {
            <Container width={(SIDEBAR_WIDTH - 24) as f32} height={36.0}
                padding={Insets { top: 4.0, right: 8.0, bottom: 2.0, left: 8.0 }}
                on_press={message}>
                <Row gap={10.0}>
                    <Text width={22.0} scale={1.6}
                        color={if selected { palette.accent } else { palette.muted }}>{glyph}</Text>
                    <Column gap={2.0}>
                        <Text height={20.0} scale={2.0} bold={selected} color={palette.text}>{label}</Text>
                        <Container width={underline_width} height={2.0}
                            background={if selected { palette.accent } else { palette.panel }} />
                    </Column>
                </Row>
            </Container>
        }
    }

    fn handle_controller_action(&mut self, action: ControllerAction) {
        if self.navigation.handle(action) {
            self.controller_page = self.page;
            self.request_redraw();
            return;
        }
        if self.navigation.pane() == NavigationPane::Sidebar {
            self.controller_page = match action {
                ControllerAction::Up => self.controller_page.previous(),
                ControllerAction::Down => self.controller_page.next(),
                ControllerAction::Confirm => {
                    self.page = self.controller_page;
                    self.controller_page
                }
                ControllerAction::Cancel => {
                    self.running = false;
                    self.controller_page
                }
                _ => self.controller_page,
            };
            self.request_redraw();
        }
    }

    fn build_ui(&self, width: f32, height: f32) -> UiTree<SettingsMessage> {
        let theme = self.ui_theme();
        let palette = self.palette();
        let (title, subtitle) = match self.page {
            SettingsPage::Display => (
                self.localizer.text("settings-display-title"),
                self.localizer.text("settings-display-subtitle"),
            ),
            SettingsPage::Bar => (
                self.localizer.text("settings-bar-title"),
                self.localizer.text("settings-bar-subtitle"),
            ),
            SettingsPage::Appearance => (
                self.localizer.text("settings-appearance-title"),
                self.localizer.text("settings-appearance-subtitle"),
            ),
            SettingsPage::Network => (
                self.localizer.text("settings-network-title"),
                self.localizer.text("settings-network-subtitle"),
            ),
            SettingsPage::Bluetooth => (
                self.localizer.text("settings-bluetooth-title"),
                self.localizer.text("settings-bluetooth-subtitle"),
            ),
        };
        let header = ui! {
            <Row height={72.0}>
                <Container width={SIDEBAR_WIDTH as f32} height={72.0} background={palette.panel} />
                <Container grow={1.0} height={72.0} background={palette.panel} padding={Insets {
                    top: 11.0, right: 40.0, bottom: 8.0, left: 20.0,
                }}>
                    <Column gap={4.0}>
                        <Text scale={3.0} color={palette.text}>{title}</Text>
                        <Text scale={1.0} color={palette.muted}>{subtitle}</Text>
                    </Column>
                </Container>
            </Row>
        };
        let selected_page = if self.navigation.pane() == NavigationPane::Sidebar {
            self.controller_page
        } else {
            self.page
        };
        let display_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Display),
            "settings-nav-display",
            "▣",
            selected_page == SettingsPage::Display,
            palette,
        );
        let bar_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Bar),
            "settings-nav-bar",
            "▤",
            selected_page == SettingsPage::Bar,
            palette,
        );
        let appearance_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Appearance),
            "settings-nav-appearance",
            "◐",
            selected_page == SettingsPage::Appearance,
            palette,
        );
        let network_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Network),
            "settings-nav-network",
            "⌁",
            selected_page == SettingsPage::Network,
            palette,
        );
        let bluetooth_button = self.navigation_item(
            SettingsMessage::Navigate(SettingsPage::Bluetooth),
            "settings-nav-bluetooth",
            "ᛒ",
            selected_page == SettingsPage::Bluetooth,
            palette,
        );
        let sidebar = ui! {
            <Sidebar width={SIDEBAR_WIDTH as f32}
                background={LinearGradient::vertical(theme.colors.sidebar, theme.colors.window)}
                padding={Insets { top: 20.0, right: 12.0, bottom: 12.0, left: 12.0 }} gap={4.0}>
                {display_button}{bar_button}{appearance_button}{network_button}{bluetooth_button}
                {if self.controller.connected() {
                    ui! {
                        <Container grow={1.0} padding={Insets {
                            top: 12.0, right: 0.0, bottom: 0.0, left: 8.0,
                        }}>
                            <ShoulderHints color={palette.text} muted={palette.muted} />
                        </Container>
                    }
                } else {
                    ui! { <></> }
                }}
            </Sidebar>
        };

        let content = match self.page {
            SettingsPage::Display => AnyView::new(self.display_components()),
            SettingsPage::Bar => AnyView::new(self.bar_components()),
            SettingsPage::Appearance => AnyView::new(self.appearance_components()),
            SettingsPage::Network => AnyView::new(self.network_components()),
            SettingsPage::Bluetooth => AnyView::new(self.bluetooth_components()),
        };
        let root = ui! {
            <Column height={height} background={theme.colors.window}>
                {header}
                <Row grow={1.0}>
                    {sidebar}
                    <Container grow={1.0}>{content}</Container>
                </Row>
            </Column>
        };
        UiTree::layout(root, UiRect::new(0.0, 0.0, width, height))
    }

    fn display_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let selected = &self.displays[self.selected];
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 24.0, right: 40.0, bottom: 20.0, left: 20.0,
            }} gap={12.0}>
                <Container height={340.0} background={palette.surface} border={(palette.muted, 2.0)} />
                <Row height={42.0} gap={15.0}>
                    <Text color={palette.text} width={183.0}>{&selected.name}</Text>
                    <Button on_press={SettingsMessage::DisplayIdentify} width={135.0} color={palette.text}
                        background={palette.surface_hover} border={(palette.muted, 1.0)}>
                        {self.localizer.text("settings-display-identify")}
                    </Button>
                    <Button on_press={SettingsMessage::DisplayPrimary} width={145.0} color={palette.text}
                        background={palette.surface_hover} border={(palette.muted, 1.0)}>
                        {self.localizer.text("settings-display-make-primary")}
                    </Button>
                    <Button on_press={SettingsMessage::DisplayApply} width={105.0} color={palette.text}
                        background={palette.accent}>{self.localizer.text("settings-display-apply")}</Button>
                </Row>
                <Text color={if self.applied { palette.complement } else { palette.muted }} height={18.0}>
                    {&self.status}
                </Text>
            </Column>
        }
    }

    fn network_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let wifi_cards = self
            .wifi_networks
            .iter()
            .enumerate()
            .map(|(index, network)| {
                let detail = if network.connected {
                    self.localizer.number(
                        "settings-network-connected-signal",
                        "signal",
                        i64::from(network.signal),
                    )
                } else if !network.saved {
                    self.localizer.number(
                        if network.secure {
                            "settings-network-secured-signal"
                        } else {
                            "settings-network-open-signal"
                        },
                        "signal",
                        i64::from(network.signal),
                    )
                } else {
                    self.localizer.number(
                        "settings-network-connect-action",
                        "signal",
                        i64::from(network.signal),
                    )
                };
                ui! {
                    <Container height={44.0}
                        background={if self.hovered_message() == Some(&SettingsMessage::WifiNetwork(index)) {
                            palette.surface_hover
                        } else { palette.surface }}
                        border={(if network.connected { palette.accent } else { palette.muted },
                            if network.connected { 2.0 } else { 1.0 })}
                        padding={Insets { top: 12.0, right: 14.0, bottom: 8.0, left: 14.0 }}
                        on_press={SettingsMessage::WifiNetwork(index)}>
                        <Row>
                            <Text color={palette.text} width={316.0}>{&network.profile}</Text>
                            <Text scale={1.0} color={if network.connected { palette.complement } else { palette.muted }}>
                                {detail}
                            </Text>
                        </Row>
                    </Container>
                }
            });
        let adapter_cards = self.network_adapters.iter().map(|adapter| {
            let status = if adapter.connected {
                if adapter.speed > 0 {
                    self.localizer.number(
                        "settings-network-connected-speed",
                        "speed",
                        (adapter.speed / 1_000_000) as i64,
                    )
                } else {
                    self.localizer.text("settings-network-connected")
                }
            } else {
                self.localizer.text("settings-network-disconnected")
            };
            ui! {
                <Container height={72.0} background={palette.surface} border={(palette.muted, 1.0)}
                    padding={Insets { top: 11.0, right: 14.0, bottom: 8.0, left: 14.0 }}>
                    <Column gap={7.0}>
                        <Text color={palette.text}>{&adapter.name}</Text>
                        <Row>
                            <Text color={if adapter.connected { palette.complement } else { palette.muted }}>{status}</Text>
                            <Text scale={1.0} color={palette.muted}>{&adapter.description}</Text>
                        </Row>
                    </Column>
                </Container>
            }
        });
        let wifi_list = if self.wifi_networks.is_empty() {
            ui! { <Column><Text scale={1.0} color={palette.muted}>{&self.wifi_status}</Text></Column> }
        } else {
            ui! { <Column gap={8.0} children={wifi_cards} /> }
        };
        let adapter_list = if self.network_adapters.is_empty() {
            ui! {
                <Column><Text scale={1.0} color={palette.muted}>
                    {self.localizer.text("settings-network-no-adapters")}
                </Text></Column>
            }
        } else {
            ui! { <Column gap={12.0} children={adapter_cards} /> }
        };
        let content = ui! {
            <Column gap={12.0}>
                <Container height={58.0} background={palette.surface}
                    border={(if self.wifi_enabled { palette.accent } else { palette.muted }, 2.0)}
                    on_press={SettingsMessage::WifiPower}
                    padding={Insets { top: 10.0, right: 14.0, bottom: 10.0, left: 14.0 }}>
                    <Row>
                        <Text width={500.0} color={palette.text}>{self.localizer.text("settings-network-wifi")}</Text>
                        <Text bold={true} color={if self.wifi_enabled { palette.accent } else { palette.muted }}>
                            {if self.wifi_enabled {
                                self.localizer.text("settings-network-wifi-on")
                            } else if !self.network_available {
                                self.localizer.text("settings-network-wifi-unavailable")
                            } else {
                                self.localizer.text("settings-network-wifi-off")
                            }}
                        </Text>
                    </Row>
                </Container>
                <Row height={26.0}>
                    <Text color={palette.text} width={308.0}>{self.localizer.text("settings-network-visible-wifi")}</Text>
                    <Text scale={1.0} color={palette.muted}>{&self.wifi_status}</Text>
                </Row>
                {wifi_list}
                <Text color={palette.text} height={18.0}>{self.localizer.text("settings-network-adapters")}</Text>
                {adapter_list}
            </Column>
        };

        ui! {
            <Column grow={1.0} padding={Insets {
                top: 20.0, right: 40.0, bottom: 20.0, left: 20.0,
            }}>
                <VerticalScroll id={"network-list"} on_scroll={SettingsMessage::NetworkScroll}
                    offset={self.transient_scroll(&SettingsMessage::NetworkScroll)}>{content}</VerticalScroll>
            </Column>
        }
    }

    fn bluetooth_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let device_cards = self
            .bluetooth
            .devices
            .iter()
            .enumerate()
            .map(|(index, device)| {
                let status = if device.connected {
                    self.localizer.text("settings-bluetooth-connected")
                } else if device.paired {
                    self.localizer.text("settings-bluetooth-paired")
                } else {
                    self.localizer.text("settings-bluetooth-available")
                };
                let detail = device
                    .battery_percent
                    .map(|percent| format!("{percent}%"))
                    .unwrap_or_default();
                ui! {
                    <Container height={68.0}
                        background={if self.hovered_message() == Some(&SettingsMessage::BluetoothDevice(index)) {
                            palette.surface_hover
                        } else { palette.surface }}
                        border={(if device.connected { palette.accent } else { palette.muted },
                            if device.connected { 2.0 } else { 1.0 })}
                        on_press={SettingsMessage::BluetoothDevice(index)}
                        padding={Insets { top: 12.0, right: 14.0, bottom: 10.0, left: 14.0 }}>
                        <Row>
                            <Column grow={1.0} gap={7.0}>
                                <Text color={palette.text}>{&device.name}</Text>
                                <Text scale={1.0} color={if device.connected { palette.complement } else { palette.muted }}>
                                    {status}
                                </Text>
                            </Column>
                            <Text color={palette.muted}>{detail}</Text>
                        </Row>
                    </Container>
                }
            });

        let adapter_status = if !self.bluetooth.available {
            self.localizer
                .text("settings-bluetooth-service-unavailable")
        } else if self.bluetooth.powered {
            self.localizer.text("settings-bluetooth-on")
        } else {
            self.localizer.text("settings-bluetooth-off")
        };
        let discoverability = if self.bluetooth.discovering {
            self.localizer.text("settings-bluetooth-discovery-stop")
        } else {
            self.localizer.text("settings-bluetooth-discovery-start")
        };
        let device_list = if self.bluetooth.devices.is_empty() {
            ui! { <Column><Text color={palette.muted}>{if self.bluetooth.available {
                self.localizer.text("settings-bluetooth-no-devices")
            } else {
                self.localizer
                    .text("settings-bluetooth-service-unavailable")
            }}</Text></Column> }
        } else {
            ui! { <Column gap={10.0} children={device_cards} /> }
        };
        let content = ui! {
            <Column gap={12.0}>
                <Container height={58.0} background={palette.surface} border={(palette.accent, 2.0)}
                    on_press={SettingsMessage::BluetoothPower}
                    padding={Insets { top: 10.0, right: 14.0, bottom: 10.0, left: 14.0 }}>
                    <Row>
                        <Column grow={1.0} gap={5.0}>
                            <Text color={palette.text}>{self.localizer.text("settings-bluetooth-enabled")}</Text>
                            <Text scale={1.0} color={palette.muted}>{if self.bluetooth.adapter_name.is_empty() {
                                self.localizer.text("settings-bluetooth-adapter-unnamed")
                            } else {
                                self.bluetooth.adapter_name.clone()
                            }}</Text>
                        </Column>
                        <Text bold={true} color={if self.bluetooth.powered { palette.accent } else { palette.muted }}>
                            {adapter_status}
                        </Text>
                    </Row>
                </Container>
                <Row height={36.0}>
                    <Text width={390.0} color={palette.text}>{self.localizer.text("settings-bluetooth-devices")}</Text>
                    <Button on_press={SettingsMessage::BluetoothDiscovery} width={150.0} color={palette.text}
                        background={palette.surface_hover} border={(palette.muted, 1.0)}>{discoverability}</Button>
                </Row>
                {device_list}
            </Column>
        };

        ui! {
            <Column grow={1.0} padding={Insets {
                top: 20.0, right: 40.0, bottom: 20.0, left: 20.0,
            }}>
                <VerticalScroll id={"bluetooth-list"} on_scroll={SettingsMessage::BluetoothScroll}
                    offset={self.transient_scroll(&SettingsMessage::BluetoothScroll)}>{content}</VerticalScroll>
            </Column>
        }
    }

    fn bar_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let palette = self.palette();
        let display_count = self.displays.len().max(1);
        let desktop_choices = (0..self.shell_settings.desktop_count).map(|index| {
            ui! {
                <Container width={64.0} height={46.0} background={palette.surface}
                    border={(if index == 0 { palette.accent } else { palette.muted }, 2.0)}
                    padding={Insets { top: 9.0, right: 4.0, bottom: 4.0, left: 4.0 }}>
                    <Text align={TextAlign::Center} scale={1.0}
                        color={if index == 0 { palette.text } else { palette.muted }}>
                        {format!("{}", index + 1)}
                    </Text>
                </Container>
            }
        });
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 24.0, right: 40.0, bottom: 20.0, left: 20.0,
            }} gap={14.0}>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-bar-show-on")}</Text>
                <Row height={38.0} gap={28.0}>
                    <RadioButton on_press={SettingsMessage::BarPrimaryDisplay}
                        label={self.localizer.text("settings-bar-primary-display")}
                        selected={!self.shell_settings.bar_on_all_displays}
                        colors_pair={(if !self.shell_settings.bar_on_all_displays { palette.accent } else { palette.muted }, palette.text)}
                        width={210.0} />
                    <RadioButton on_press={SettingsMessage::BarAllDisplays}
                        label={self.localizer.number("settings-bar-all-displays", "count", display_count as i64)}
                        selected={self.shell_settings.bar_on_all_displays}
                        colors_pair={(if self.shell_settings.bar_on_all_displays { palette.accent } else { palette.muted }, palette.text)}
                        width={210.0} />
                </Row>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-bar-window-scope")}</Text>
                <Row height={38.0} gap={28.0}>
                    <RadioButton on_press={SettingsMessage::BarDisplayWindows}
                        label={self.localizer.text("settings-bar-this-display")}
                        selected={!self.shell_settings.all_windows_on_every_bar}
                        colors_pair={(if !self.shell_settings.all_windows_on_every_bar { palette.accent } else { palette.muted }, palette.text)}
                        width={210.0} />
                    <RadioButton on_press={SettingsMessage::BarAllWindows}
                        label={self.localizer.text("settings-bar-all-windows")}
                        selected={self.shell_settings.all_windows_on_every_bar}
                        colors_pair={(if self.shell_settings.all_windows_on_every_bar { palette.accent } else { palette.muted }, palette.text)}
                        width={210.0} />
                </Row>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-bar-desktops")}</Text>
                <Text scale={1.0} color={palette.muted} height={18.0}>
                    {self.localizer.number("settings-bar-desktop-count", "count", i64::from(self.shell_settings.desktop_count))}
                </Text>
                <Slider id={"bar-desktop-count"}
                    value={f32::from(self.shell_settings.desktop_count.saturating_sub(1)) / 7.0}
                    on_change={desktop_count_message} width={520.0} />
                <Row height={46.0} gap={8.0} children={desktop_choices} />
            </Column>
        }
    }

    fn appearance_components(&self) -> impl nickel_ui::Component<SettingsMessage> {
        let system = nickel_platform::appearance();
        let appearance = self.shell_settings.resolve_appearance(system);
        let palette = ThemePalette::from_appearance(appearance);
        let hue = self.shell_settings.displayed_hue(system);
        let intensity = self.shell_settings.displayed_intensity(system);
        let swatches = [
            ("settings-swatch-background", palette.background),
            ("settings-swatch-panel", palette.panel),
            ("settings-swatch-surface", palette.surface),
            ("settings-swatch-hover", palette.surface_hover),
            ("settings-swatch-accent", palette.accent),
            ("settings-swatch-complement", palette.complement),
        ]
        .into_iter()
        .map(|(label, color)| {
            ui! {
                <Container width={88.0} height={76.0} background={color}
                    border={(palette.muted, 1.0)}
                    padding={Insets { top: 46.0, right: 4.0, bottom: 4.0, left: 4.0 }}>
                    <Text align={TextAlign::Center} scale={0.72} color={palette.text}>
                        {self.localizer.text(label)}
                    </Text>
                </Container>
            }
        });
        let wallpaper_name = self
            .wallpaper_settings
            .image
            .as_deref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.localizer.text("settings-wallpaper-none"));
        let positions = [
            ("settings-wallpaper-fill", WallpaperPosition::Fill),
            ("settings-wallpaper-fit", WallpaperPosition::Fit),
            ("settings-wallpaper-stretch", WallpaperPosition::Stretch),
            ("settings-wallpaper-center", WallpaperPosition::Center),
            ("settings-wallpaper-tile", WallpaperPosition::Tile),
            ("settings-wallpaper-span", WallpaperPosition::Span),
        ]
        .into_iter()
        .map(|(label, position)| ui! {
            <RadioButton on_press={SettingsMessage::WallpaperPosition(position)}
                label={self.localizer.text(label)} selected={self.wallpaper_settings.position == position}
                colors_pair={(palette.accent, palette.text)} width={82.0} />
        });
        ui! {
            <Column grow={1.0} padding={Insets {
                top: 24.0, right: 40.0, bottom: 20.0, left: 20.0,
            }} gap={14.0}>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-appearance-mode")}</Text>
                <Row height={38.0} gap={28.0}>
                    <RadioButton on_press={SettingsMessage::AppearanceLight}
                        label={self.localizer.text("settings-appearance-light")}
                        selected={appearance.mode == ThemeMode::Light}
                        colors_pair={(if appearance.mode == ThemeMode::Light { palette.accent } else { palette.muted }, palette.text)}
                        width={180.0} />
                    <RadioButton on_press={SettingsMessage::AppearanceDark}
                        label={self.localizer.text("settings-appearance-dark")}
                        selected={appearance.mode == ThemeMode::Dark}
                        colors_pair={(if appearance.mode == ThemeMode::Dark { palette.accent } else { palette.muted }, palette.text)}
                        width={180.0} />
                </Row>
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-wallpaper-image")}</Text>
                <Row height={38.0} gap={12.0}>
                    <Button on_press={SettingsMessage::WallpaperChoose} width={150.0}
                        color={palette.text} background={palette.surface_hover}>
                        {self.localizer.text("settings-wallpaper-choose")}
                    </Button>
                    <Text color={palette.muted} width={370.0}>{wallpaper_name}</Text>
                </Row>
                <Row height={34.0} gap={10.0} children={positions} />
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-appearance-starting-hue")}</Text>
                <Text scale={1.0} color={palette.muted} height={18.0}>
                    {self.localizer.number("settings-appearance-hue-value", "degrees", i64::from(hue))}
                </Text>
                <Slider id={"appearance-hue"} value={f32::from(hue) / 359.0}
                    on_change={appearance_hue_message}
                    colors_triplet={(palette.surface, palette.accent, palette.text)} width={520.0} />
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-appearance-color-intensity")}</Text>
                <Text scale={1.0} color={palette.muted} height={18.0}>
                    {self.localizer.number("settings-appearance-intensity-value", "percent", i64::from(intensity))}
                </Text>
                <Slider id={"appearance-intensity"} value={f32::from(intensity) / 100.0}
                    on_change={appearance_intensity_message}
                    colors_triplet={(palette.surface, palette.accent, palette.text)} width={520.0} />
                <Text color={palette.text} height={20.0}>{self.localizer.text("settings-appearance-color-palette")}</Text>
                <Row height={76.0} gap={8.0} children={swatches} />
            </Column>
        }
    }

    fn pointer_pressed(&mut self) {
        let (x, y) = self.cursor;
        let point = UiPoint {
            x: x as f32,
            y: y as f32,
        };
        if let Some(SettingsMessage::SetDesktopCount(count)) = self.ui.message_at_owned(point) {
            let id = self
                .ui
                .id_for_message(&SettingsMessage::SetDesktopCount(count))
                .cloned();
            self.ui_state.set_capture(id);
            self.set_desktop_count(count);
            self.request_redraw();
            return;
        }
        if let Some(SettingsMessage::SetAppearanceHue(hue)) = self.ui.message_at_owned(point) {
            let id = self
                .ui
                .id_for_message(&SettingsMessage::SetAppearanceHue(hue))
                .cloned();
            self.ui_state.set_capture(id);
            self.set_appearance_hue(hue);
            self.request_redraw();
            return;
        }
        if let Some(SettingsMessage::SetAppearanceIntensity(intensity)) =
            self.ui.message_at_owned(point)
        {
            let id = self
                .ui
                .id_for_message(&SettingsMessage::SetAppearanceIntensity(intensity))
                .cloned();
            self.ui_state.set_capture(id);
            self.set_appearance_intensity(intensity);
            self.request_redraw();
            return;
        }
        let message = self.ui.message_at(point).cloned();
        if let Some(message) = message {
            match message {
                SettingsMessage::Navigate(page) => {
                    self.page = page;
                    match page {
                        SettingsPage::Network => self.load_linux_network(),
                        SettingsPage::Bluetooth => self.load_bluetooth(),
                        _ => {}
                    }
                }
                SettingsMessage::BluetoothPower => {
                    let _ = set_bluetooth_adapter_property("Powered", !self.bluetooth.powered);
                    self.next_bluetooth_refresh = Instant::now();
                }
                SettingsMessage::BluetoothDiscovery => {
                    let _ =
                        set_bluetooth_adapter_property("Discovering", !self.bluetooth.discovering);
                    self.next_bluetooth_refresh = Instant::now();
                }
                SettingsMessage::WifiPower => {
                    #[cfg(target_os = "linux")]
                    if let Err(error) = set_linux_wifi_enabled(!self.wifi_enabled) {
                        self.wifi_status = self.localizer.value(
                            "settings-network-connection-failed",
                            "error",
                            &error,
                        );
                    }
                    self.next_network_refresh = Instant::now();
                }
                SettingsMessage::BluetoothDevice(index) => {
                    if let Some(device) = self.bluetooth.devices.get(index) {
                        let _ = toggle_bluetooth_device(device);
                        self.next_bluetooth_refresh = Instant::now();
                    }
                }
                SettingsMessage::AppearanceLight => {
                    self.shell_settings.theme = ThemePreference::Light;
                    let _ = self.shell_settings.save_default();
                }
                SettingsMessage::AppearanceDark => {
                    self.shell_settings.theme = ThemePreference::Dark;
                    let _ = self.shell_settings.save_default();
                }
                SettingsMessage::WallpaperChoose => {
                    if let Some(path) = choose_wallpaper() {
                        self.wallpaper_settings.image = Some(path);
                        let _ = self.wallpaper_settings.save_default();
                    }
                }
                SettingsMessage::WallpaperPosition(position) => {
                    self.wallpaper_settings.position = position;
                    let _ = self.wallpaper_settings.save_default();
                }
                SettingsMessage::BarPrimaryDisplay => {
                    self.shell_settings.bar_on_all_displays = false;
                    let _ = self.shell_settings.save_default();
                }
                SettingsMessage::BarAllDisplays => {
                    self.shell_settings.bar_on_all_displays = true;
                    let _ = self.shell_settings.save_default();
                }
                SettingsMessage::BarDisplayWindows => {
                    self.shell_settings.all_windows_on_every_bar = false;
                    let _ = self.shell_settings.save_default();
                }
                SettingsMessage::BarAllWindows => {
                    self.shell_settings.all_windows_on_every_bar = true;
                    let _ = self.shell_settings.save_default();
                }
                SettingsMessage::DisplayIdentify => match session_request("identify-outputs") {
                    Ok(response) if response == "ok" => {
                        self.status = self.localizer.text("settings-status-identifying")
                    }
                    _ => self.status = self.localizer.text("settings-status-identify-failed"),
                },
                SettingsMessage::DisplayPrimary => {
                    for (index, display) in self.displays.iter_mut().enumerate() {
                        display.primary = index == self.selected;
                    }
                    self.applied = false;
                    self.status = self.localizer.text("settings-status-changes-not-applied");
                }
                SettingsMessage::DisplayApply => self.apply_layout(),
                SettingsMessage::WifiNetwork(index) => self.connect_windows_wifi(index),
                SettingsMessage::SetDesktopCount(_)
                | SettingsMessage::SetAppearanceHue(_)
                | SettingsMessage::SetAppearanceIntensity(_)
                | SettingsMessage::BluetoothScroll
                | SettingsMessage::NetworkScroll => {}
            }
            self.request_redraw();
            return;
        }
        if self.page != SettingsPage::Display {
            self.request_redraw();
            return;
        } else if let Some(index) = self
            .displays
            .iter()
            .rposition(|display| display.rect.contains(x, y))
        {
            self.selected = index;
            let rect = self.displays[index].rect;
            self.drag_offset = Some((x - rect.x, y - rect.y));
            self.applied = false;
            self.status = self.localizer.text("settings-status-changes-not-applied");
        }
        self.request_redraw();
    }

    fn pointer_moved(&mut self, x: f32, y: f32) {
        self.cursor = (x.round() as i32, y.round() as i32);
        if matches!(
            self.captured_message(),
            Some(SettingsMessage::SetDesktopCount(_))
        ) {
            if let Some(fraction) = self.ui.horizontal_fraction_for_matching(x, |message| {
                matches!(message, SettingsMessage::SetDesktopCount(_))
            }) {
                self.set_desktop_count_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        if matches!(
            self.captured_message(),
            Some(SettingsMessage::SetAppearanceHue(_))
        ) {
            if let Some(fraction) = self.ui.horizontal_fraction_for_matching(x, |message| {
                matches!(message, SettingsMessage::SetAppearanceHue(_))
            }) {
                self.set_appearance_hue_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        if matches!(
            self.captured_message(),
            Some(SettingsMessage::SetAppearanceIntensity(_))
        ) {
            if let Some(fraction) = self.ui.horizontal_fraction_for_matching(x, |message| {
                matches!(message, SettingsMessage::SetAppearanceIntensity(_))
            }) {
                self.set_appearance_intensity_from_fraction(fraction);
                self.request_redraw();
            }
            return;
        }
        let hovered = self.ui.id_at(UiPoint { x, y }).cloned();
        if self.ui_state.set_hovered(hovered) != nickel_ui::Invalidation::None {
            self.request_redraw();
        }
        if matches!(self.page, SettingsPage::Bluetooth | SettingsPage::Network) {
            return;
        }
        if self.page != SettingsPage::Display {
            return;
        }
        if let Some((offset_x, offset_y)) = self.drag_offset {
            let mut rect = self.displays[self.selected].rect;
            rect.x = self.cursor.0 - offset_x;
            rect.y = self.cursor.1 - offset_y;
            rect = constrain_center(rect, DISPLAY_PLANE);
            rect = self
                .displays
                .iter()
                .enumerate()
                .filter(|(index, _)| *index != self.selected)
                .fold(rect, |moving, (_, other)| snap_rect(moving, other.rect, 42));
            self.displays[self.selected].rect = rect;
            self.applied = false;
            self.status = self.localizer.text("settings-status-changes-not-applied");
            self.request_redraw();
        }
    }

    fn finish_drag(&mut self) {
        self.ui_state.set_pressed(None);
        self.ui_state.set_capture(None);
        if self.page != SettingsPage::Display {
            return;
        }
        if self.drag_offset.take().is_none() {
            return;
        }
        let selected = self.displays[self.selected].rect;
        let snapped = self
            .displays
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != self.selected)
            .map(|(_, other)| attach_rect_centered(selected, other.rect))
            .min_by_key(|rect| {
                let dx = rect.x - selected.x;
                let dy = rect.y - selected.y;
                dx * dx + dy * dy
            })
            .unwrap_or(selected);
        self.displays[self.selected].rect = snapped;
        self.request_redraw();
    }

    fn scroll_settings(&mut self, wheel_y: f32) {
        let delta = -wheel_y * 42.0;
        match self.page {
            SettingsPage::Network => {
                if let Some(id) = self
                    .ui
                    .id_for_message(&SettingsMessage::NetworkScroll)
                    .cloned()
                {
                    let maximum = self
                        .ui
                        .scroll_extent(&SettingsMessage::NetworkScroll)
                        .map(|extent| (extent.content.height - extent.viewport.height).max(0.0))
                        .unwrap_or(0.0);
                    if self.ui_state.scroll_by(id, delta, maximum) != nickel_ui::Invalidation::None
                    {
                        self.request_redraw();
                    }
                }
            }
            SettingsPage::Bluetooth => {
                if let Some(id) = self
                    .ui
                    .id_for_message(&SettingsMessage::BluetoothScroll)
                    .cloned()
                {
                    let maximum = self
                        .ui
                        .scroll_extent(&SettingsMessage::BluetoothScroll)
                        .map(|extent| (extent.content.height - extent.viewport.height).max(0.0))
                        .unwrap_or(0.0);
                    if self.ui_state.scroll_by(id, delta, maximum) != nickel_ui::Invalidation::None
                    {
                        self.request_redraw();
                    }
                }
            }
            _ => {}
        }
    }

    fn set_desktop_count_from_fraction(&mut self, fraction: f32) {
        let SettingsMessage::SetDesktopCount(count) = desktop_count_message(fraction) else {
            unreachable!()
        };
        self.set_desktop_count(count);
    }

    fn set_desktop_count(&mut self, count: u8) {
        if count == self.shell_settings.desktop_count {
            return;
        }
        self.shell_settings.desktop_count = count;
        self.shell_settings.active_desktop = self
            .shell_settings
            .active_desktop
            .min(count.saturating_sub(1));
        let _ = self.shell_settings.save_default();
    }

    fn set_appearance_hue_from_fraction(&mut self, fraction: f32) {
        let SettingsMessage::SetAppearanceHue(hue) = appearance_hue_message(fraction) else {
            unreachable!()
        };
        self.set_appearance_hue(hue);
    }

    fn set_appearance_hue(&mut self, hue: u16) {
        if self.shell_settings.accent_hue == Some(hue) {
            return;
        }
        self.shell_settings.accent_hue = Some(hue);
        self.appearance_save_deadline = Some(Instant::now() + self.frame_interval);
    }

    fn set_appearance_intensity_from_fraction(&mut self, fraction: f32) {
        let SettingsMessage::SetAppearanceIntensity(intensity) =
            appearance_intensity_message(fraction)
        else {
            unreachable!()
        };
        self.set_appearance_intensity(intensity);
    }

    fn set_appearance_intensity(&mut self, intensity: u8) {
        if self.shell_settings.accent_intensity == Some(intensity) {
            return;
        }
        self.shell_settings.accent_intensity = Some(intensity);
        self.appearance_save_deadline = Some(Instant::now() + self.frame_interval);
    }

    fn load_bluetooth(&mut self) {
        self.bluetooth = match read_bluetooth_snapshot() {
            Ok(snapshot) => snapshot,
            Err(error) => {
                tracing::warn!(%error, "failed to read Bluetooth settings");
                BluetoothSnapshot::default()
            }
        };
        tracing::debug!(
            available = self.bluetooth.available,
            powered = self.bluetooth.powered,
            discovering = self.bluetooth.discovering,
            devices = self.bluetooth.devices.len(),
            "Bluetooth settings refreshed"
        );
        self.next_bluetooth_refresh = Instant::now() + Duration::from_secs(2);
        self.request_redraw();
    }

    #[cfg(target_os = "linux")]
    fn load_linux_network(&mut self) {
        match read_linux_network() {
            Ok((enabled, networks, adapters)) => {
                self.network_available = true;
                self.wifi_enabled = enabled;
                self.wifi_status = if !enabled {
                    self.localizer.text("settings-network-wifi-disabled")
                } else if networks.is_empty() {
                    self.localizer.text("settings-network-no-visible-networks")
                } else {
                    self.localizer.number(
                        "settings-network-visible-count",
                        "count",
                        networks.len() as i64,
                    )
                };
                self.wifi_networks = networks;
                self.network_adapters = adapters;
                tracing::debug!(
                    enabled,
                    networks = self.wifi_networks.len(),
                    adapters = self.network_adapters.len(),
                    "NetworkManager settings refreshed"
                );
            }
            Err(error) => {
                tracing::warn!(%error, "failed to read NetworkManager settings");
                self.network_available = false;
                self.wifi_enabled = false;
                self.wifi_networks.clear();
                self.network_adapters.clear();
                self.wifi_status = self.localizer.text("settings-network-service-unavailable");
            }
        }
        self.next_network_refresh = Instant::now() + Duration::from_secs(2);
        self.request_redraw();
    }

    #[cfg(target_os = "windows")]
    fn load_linux_network(&mut self) {
        self.load_windows_network();
        self.load_windows_wifi();
        self.next_network_refresh = Instant::now() + Duration::from_secs(2);
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    fn load_linux_network(&mut self) {
        self.next_network_refresh = Instant::now() + Duration::from_secs(2);
    }

    fn load_outputs(&mut self) {
        let Ok(payload) = session_request("list-outputs") else {
            self.status = self.localizer.text("settings-status-using-mock-displays");
            return;
        };
        let outputs: Vec<_> = payload.lines().filter_map(parse_output).collect();
        if outputs.is_empty() {
            self.status = self.localizer.text("settings-status-using-mock-displays");
            return;
        }
        let minimum_x = outputs.iter().map(|output| output.x).min().unwrap_or(0);
        let minimum_y = outputs.iter().map(|output| output.y).min().unwrap_or(0);
        let maximum_x = outputs
            .iter()
            .map(|output| output.x + output.width)
            .max()
            .unwrap_or(1);
        let maximum_y = outputs
            .iter()
            .map(|output| output.y + output.height)
            .max()
            .unwrap_or(1);
        let maximum_physical_width = outputs
            .iter()
            .map(|output| output.physical_width)
            .max()
            .unwrap_or(1)
            .max(1);
        let maximum_physical_height = outputs
            .iter()
            .map(|output| output.physical_height)
            .max()
            .unwrap_or(1)
            .max(1);
        let physical_scale = (280.0 / f64::from(maximum_physical_width))
            .min(160.0 / f64::from(maximum_physical_height));
        // Leave enough empty plane around the arrangement to drag one display
        // completely around another before release snapping chooses an edge.
        self.pixels_per_logical = (470.0 / f64::from((maximum_x - minimum_x).max(1)))
            .min(190.0 / f64::from((maximum_y - minimum_y).max(1)))
            .max(0.04);
        self.displays = outputs
            .into_iter()
            .map(|output| DisplayCard {
                connector: output.name.clone(),
                detail: format!("{}  {} X {}", output.name, output.width, output.height),
                name: output.model,
                logical_width: output.width,
                logical_height: output.height,
                rect: Rect {
                    x: 95
                        + (f64::from(output.x - minimum_x) * self.pixels_per_logical).round()
                            as i32,
                    y: 155
                        + (f64::from(output.y - minimum_y) * self.pixels_per_logical).round()
                            as i32,
                    w: (f64::from(output.physical_width.max(1)) * physical_scale).round() as i32,
                    h: (f64::from(output.physical_height.max(1)) * physical_scale).round() as i32,
                },
                primary: output.primary,
            })
            .collect();
        for index in 1..self.displays.len() {
            let previous = self.displays[index - 1].rect;
            self.displays[index].rect = attach_rect_centered(self.displays[index].rect, previous);
        }
        self.selected = self
            .displays
            .iter()
            .position(|display| display.primary)
            .unwrap_or(0);
        self.status.clear();
    }

    #[cfg(target_os = "windows")]
    fn load_windows_outputs(&mut self, video: &sdl3::VideoSubsystem) {
        let monitors = video.displays().unwrap_or_default();
        if monitors.is_empty() {
            self.status = self.localizer.text("settings-status-no-displays");
            return;
        }
        let primary = video
            .get_primary_display()
            .ok()
            .map(|display| display.to_ll());
        let minimum_x = monitors
            .iter()
            .filter_map(|monitor| monitor.get_bounds().ok().map(|bounds| bounds.x()))
            .min()
            .unwrap_or(0);
        let minimum_y = monitors
            .iter()
            .filter_map(|monitor| monitor.get_bounds().ok().map(|bounds| bounds.y()))
            .min()
            .unwrap_or(0);
        let maximum_x = monitors
            .iter()
            .filter_map(|monitor| {
                monitor
                    .get_bounds()
                    .ok()
                    .map(|bounds| bounds.x() + bounds.width() as i32)
            })
            .max()
            .unwrap_or(1);
        let maximum_y = monitors
            .iter()
            .filter_map(|monitor| {
                monitor
                    .get_bounds()
                    .ok()
                    .map(|bounds| bounds.y() + bounds.height() as i32)
            })
            .max()
            .unwrap_or(1);
        let desktop_width = (maximum_x - minimum_x).max(1);
        let desktop_height = (maximum_y - minimum_y).max(1);
        self.pixels_per_logical = (380.0 / f64::from(desktop_width))
            .min(210.0 / f64::from(desktop_height))
            .max(0.04);
        let rendered_width = (f64::from(desktop_width) * self.pixels_per_logical).round() as i32;
        let rendered_height = (f64::from(desktop_height) * self.pixels_per_logical).round() as i32;
        let origin_x = DISPLAY_PLANE.x + (DISPLAY_PLANE.w - rendered_width) / 2;
        let origin_y = DISPLAY_PLANE.y + (DISPLAY_PLANE.h - rendered_height) / 2;
        let friendly_names = windows_display_names();
        self.displays = monitors
            .into_iter()
            .enumerate()
            .map(|(index, monitor)| {
                let bounds = monitor
                    .get_bounds()
                    .unwrap_or_else(|_| sdl3::rect::Rect::new(0, 0, 1, 1));
                let raw_name = monitor
                    .get_name()
                    .unwrap_or_else(|_| format!("Display {}", index + 1));
                let connector = raw_name
                    .strip_prefix(r"\\.\")
                    .unwrap_or(&raw_name)
                    .to_owned();
                let name = friendly_names
                    .get(&raw_name)
                    .cloned()
                    .unwrap_or_else(|| connector.clone());
                DisplayCard {
                    connector: connector.clone(),
                    name: name.clone(),
                    detail: format!("{}  {} X {}", connector, bounds.width(), bounds.height()),
                    logical_width: bounds.width() as i32,
                    logical_height: bounds.height() as i32,
                    rect: Rect {
                        x: origin_x
                            + (f64::from(bounds.x() - minimum_x) * self.pixels_per_logical).round()
                                as i32,
                        y: origin_y
                            + (f64::from(bounds.y() - minimum_y) * self.pixels_per_logical).round()
                                as i32,
                        w: (f64::from(bounds.width()) * self.pixels_per_logical).round() as i32,
                        h: (f64::from(bounds.height()) * self.pixels_per_logical).round() as i32,
                    },
                    primary: primary == Some(monitor.to_ll()),
                }
            })
            .collect();
        self.selected = self
            .displays
            .iter()
            .position(|display| display.primary)
            .unwrap_or(0);
        self.applied = true;
        self.status.clear();
    }

    #[cfg(target_os = "windows")]
    fn load_windows_network(&mut self) {
        use std::mem::size_of;
        use windows::Win32::{
            Foundation::{ERROR_BUFFER_OVERFLOW, NO_ERROR},
            NetworkManagement::{
                IpHelper::{
                    GAA_FLAG_INCLUDE_PREFIX, GetAdaptersAddresses, IP_ADAPTER_ADDRESSES_LH,
                },
                Ndis::IfOperStatusUp,
            },
            Networking::WinSock::AF_UNSPEC,
        };

        let mut byte_count = 0;
        let first = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_PREFIX,
                None,
                None,
                &mut byte_count,
            )
        };
        if first != ERROR_BUFFER_OVERFLOW.0 || byte_count == 0 {
            return;
        }
        let words = (byte_count as usize).div_ceil(size_of::<usize>());
        let mut storage = vec![0_usize; words];
        let first_adapter = storage.as_mut_ptr().cast::<IP_ADAPTER_ADDRESSES_LH>();
        let result = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC.0 as u32,
                GAA_FLAG_INCLUDE_PREFIX,
                None,
                Some(first_adapter),
                &mut byte_count,
            )
        };
        if result != NO_ERROR.0 {
            return;
        }

        let mut adapters = Vec::new();
        let mut current = first_adapter;
        while let Some(adapter) = unsafe { current.as_ref() } {
            let name = unsafe { adapter.FriendlyName.to_string() }.unwrap_or_default();
            let description = unsafe { adapter.Description.to_string() }.unwrap_or_default();
            if !name.is_empty() && adapter.IfType != 24 {
                adapters.push(NetworkAdapter {
                    name,
                    description,
                    connected: adapter.OperStatus == IfOperStatusUp,
                    speed: adapter.ReceiveLinkSpeed.max(adapter.TransmitLinkSpeed),
                });
            }
            current = adapter.Next;
        }
        adapters.sort_by_key(|adapter| (!adapter.connected, adapter.name.to_ascii_lowercase()));
        self.network_adapters = adapters;
    }

    #[cfg(target_os = "windows")]
    fn load_windows_wifi(&mut self) {
        use std::{collections::HashMap, slice};
        use windows::Win32::{
            Foundation::{HANDLE, NO_ERROR},
            NetworkManagement::WiFi::{
                WLAN_AVAILABLE_NETWORK_CONNECTED, WLAN_AVAILABLE_NETWORK_LIST,
                WLAN_INTERFACE_INFO_LIST, WLAN_PROFILE_INFO_LIST, WlanCloseHandle,
                WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
                WlanGetProfileList, WlanOpenHandle,
            },
        };

        let mut negotiated = 0;
        let mut handle = HANDLE::default();
        if unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) } != NO_ERROR.0 {
            self.wifi_status = self.localizer.text("settings-network-service-unavailable");
            return;
        }
        let mut interface_list = std::ptr::null_mut::<WLAN_INTERFACE_INFO_LIST>();
        if unsafe { WlanEnumInterfaces(handle, None, &mut interface_list) } != NO_ERROR.0
            || interface_list.is_null()
        {
            unsafe {
                WlanCloseHandle(handle, None);
            }
            self.wifi_status = self
                .localizer
                .text("settings-network-interface-unavailable");
            return;
        }

        let interfaces = unsafe {
            slice::from_raw_parts(
                (*interface_list).InterfaceInfo.as_ptr(),
                (*interface_list).dwNumberOfItems as usize,
            )
        };
        let mut networks_by_profile = HashMap::<String, WifiNetwork>::new();
        for interface in interfaces {
            let interface_id = interface.InterfaceGuid.to_u128();
            let mut available_profiles = HashMap::<String, (u32, bool)>::new();
            let mut available = std::ptr::null_mut::<WLAN_AVAILABLE_NETWORK_LIST>();
            if unsafe {
                WlanGetAvailableNetworkList(
                    handle,
                    &raw const interface.InterfaceGuid,
                    0,
                    None,
                    &mut available,
                )
            } == NO_ERROR.0
                && !available.is_null()
            {
                let entries = unsafe {
                    slice::from_raw_parts(
                        (*available).Network.as_ptr(),
                        (*available).dwNumberOfItems as usize,
                    )
                };
                for network in entries {
                    let profile = wide_text(&network.strProfileName);
                    if !profile.is_empty() {
                        available_profiles.insert(
                            profile.to_ascii_lowercase(),
                            (
                                network.wlanSignalQuality,
                                network.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                            ),
                        );
                    }
                }
                unsafe { WlanFreeMemory(available.cast()) };
            }

            let mut profile_list = std::ptr::null_mut::<WLAN_PROFILE_INFO_LIST>();
            if unsafe {
                WlanGetProfileList(
                    handle,
                    &raw const interface.InterfaceGuid,
                    None,
                    &mut profile_list,
                )
            } != NO_ERROR.0
                || profile_list.is_null()
            {
                continue;
            }
            let profiles = unsafe {
                slice::from_raw_parts(
                    (*profile_list).ProfileInfo.as_ptr(),
                    (*profile_list).dwNumberOfItems as usize,
                )
            };
            for saved in profiles {
                let profile = wide_text(&saved.strProfileName);
                if profile.is_empty() {
                    continue;
                }
                let key = profile.to_ascii_lowercase();
                let (signal, connected) =
                    available_profiles.get(&key).copied().unwrap_or((0, false));
                networks_by_profile.entry(key).or_insert(WifiNetwork {
                    id: profile.clone(),
                    profile,
                    signal,
                    connected,
                    saved: true,
                    secure: true,
                    interface: interface_id,
                });
            }
            unsafe { WlanFreeMemory(profile_list.cast()) };
        }
        unsafe {
            WlanFreeMemory(interface_list.cast());
            WlanCloseHandle(handle, None);
        }
        let mut networks: Vec<_> = networks_by_profile.into_values().collect();
        networks.sort_by_key(|network| {
            (
                !network.connected,
                network.signal == 0,
                std::cmp::Reverse(network.signal),
                network.profile.to_ascii_lowercase(),
            )
        });
        self.wifi_status = if networks.is_empty() {
            self.localizer.text("settings-network-no-saved-profiles")
        } else {
            self.localizer.number(
                "settings-network-saved-profile-count",
                "count",
                networks.len() as i64,
            )
        };
        self.network_available = true;
        self.wifi_enabled = true;
        self.wifi_networks = networks;
    }

    #[cfg(not(target_os = "windows"))]
    fn load_windows_wifi(&mut self) {}

    #[cfg(target_os = "windows")]
    fn connect_windows_wifi(&mut self, index: usize) {
        use windows::{
            Win32::{
                Foundation::{HANDLE, NO_ERROR},
                NetworkManagement::WiFi::{
                    WLAN_CONNECTION_PARAMETERS, WlanCloseHandle, WlanConnect, WlanOpenHandle,
                    dot11_BSS_type_any, wlan_connection_mode_profile,
                },
            },
            core::{GUID, PCWSTR},
        };

        let Some(network) = self.wifi_networks.get(index) else {
            return;
        };
        let profile = network.profile.clone();
        let interface = GUID::from_u128(network.interface);
        let profile_wide: Vec<u16> = profile.encode_utf16().chain([0]).collect();
        let mut negotiated = 0;
        let mut handle = HANDLE::default();
        if unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) } != NO_ERROR.0 {
            self.wifi_status = self.localizer.text("settings-network-service-unavailable");
            return;
        }
        let parameters = WLAN_CONNECTION_PARAMETERS {
            wlanConnectionMode: wlan_connection_mode_profile,
            strProfile: PCWSTR(profile_wide.as_ptr()),
            dot11BssType: dot11_BSS_type_any,
            ..Default::default()
        };
        let result =
            unsafe { WlanConnect(handle, &raw const interface, &raw const parameters, None) };
        unsafe {
            WlanCloseHandle(handle, None);
        }
        self.wifi_status = if result == NO_ERROR.0 {
            self.pending_wifi_profile = Some(profile.clone());
            self.next_wifi_refresh = Some(Instant::now() + Duration::from_millis(400));
            self.wifi_refreshes_left = 15;
            self.localizer
                .value("settings-network-connecting", "profile", &profile)
        } else {
            self.localizer.value(
                "settings-network-connection-failed",
                "error",
                &result.to_string(),
            )
        };
    }

    #[cfg(target_os = "linux")]
    fn connect_windows_wifi(&mut self, index: usize) {
        let Some(network) = self.wifi_networks.get(index) else {
            return;
        };
        if !network.connected && !network.saved {
            self.wifi_status = self.localizer.text("settings-network-profile-required");
            return;
        }
        let profile = network.profile.clone();
        self.wifi_status = match activate_linux_wifi(network) {
            Ok(()) if network.connected => {
                self.localizer
                    .value("settings-network-disconnecting", "profile", &profile)
            }
            Ok(()) => self
                .localizer
                .value("settings-network-connecting", "profile", &profile),
            Err(error) => {
                self.localizer
                    .value("settings-network-connection-failed", "error", &error)
            }
        };
        self.next_network_refresh = Instant::now() + Duration::from_millis(400);
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    fn connect_windows_wifi(&mut self, _index: usize) {}

    fn apply_layout(&mut self) {
        let primary = self
            .displays
            .iter()
            .find(|display| display.primary)
            .map(|display| display.connector.as_str())
            .unwrap_or(self.displays[self.selected].connector.as_str());
        let placements = logical_placements(&self.displays);

        #[cfg(target_os = "windows")]
        {
            match apply_windows_layout(&self.displays, &placements, primary) {
                Ok(()) => {
                    self.applied = true;
                    self.status = self.localizer.text("settings-status-layout-applied");
                }
                Err(error) => {
                    self.applied = false;
                    self.status = self.localizer.value(
                        "settings-status-apply-failed",
                        "error",
                        &error.to_string(),
                    );
                }
            }
            return;
        }

        #[cfg(not(target_os = "windows"))]
        let mut command = format!("apply-outputs\nprimary\t{primary}\n");
        #[cfg(not(target_os = "windows"))]
        for (display, (x, y)) in self.displays.iter().zip(placements) {
            command.push_str(&format!("{}\t{x}\t{y}\n", display.connector));
        }
        #[cfg(not(target_os = "windows"))]
        match session_request(&command) {
            Ok(response) if response == "ok" => {
                self.applied = true;
                self.status = self.localizer.text("settings-status-layout-applied");
            }
            Ok(response) => {
                self.applied = false;
                self.status = self.localizer.value(
                    "settings-status-apply-failed",
                    "error",
                    response.strip_prefix("error\t").unwrap_or(&response),
                );
            }
            Err(_) => {
                self.applied = false;
                self.status = self.localizer.text("settings-status-session-unavailable");
            }
        }
    }

    fn render(&mut self, presenter: &mut SdlCanvasPresenter) -> Result<(), String> {
        let appearance = self
            .shell_settings
            .resolve_appearance(nickel_platform::appearance());
        let palette = ThemePalette::from_appearance(appearance);
        let (logical_width, logical_height) = presenter.window().size();
        let pixel_width = presenter.window().size_in_pixels().0;
        let scale = pixel_width as f32 / logical_width.max(1) as f32;
        let ui = self.build_ui(logical_width as f32, logical_height as f32);
        ui.reconcile_state(&mut self.ui_state);
        self.ui = ui;
        let mut commands = self.ui.commands().to_vec();
        if self.page == SettingsPage::Display {
            for (index, display) in self.displays.iter().enumerate() {
                let rect = UiRect::new(
                    display.rect.x as f32,
                    display.rect.y as f32,
                    display.rect.w as f32,
                    display.rect.h as f32,
                );
                commands.push(PaintCommand::Fill {
                    rect,
                    color: if index == self.selected {
                        palette.accent_soft
                    } else {
                        palette.surface
                    },
                });
                commands.push(PaintCommand::Stroke {
                    rect,
                    color: if display.primary {
                        palette.accent
                    } else {
                        palette.muted
                    },
                    width: if display.primary { 4.0 } else { 2.0 },
                });
                commands.push(PaintCommand::Text {
                    bounds: UiRect::new(
                        (display.rect.x + 18) as f32,
                        (display.rect.y + 20) as f32,
                        (display.rect.w - 36) as f32,
                        32.0,
                    ),
                    text: display.name.clone(),
                    scale: 3.0,
                    color: palette.text,
                    align: TextAlign::Start,
                    bold: false,
                });
                commands.push(PaintCommand::Text {
                    bounds: UiRect::new(
                        (display.rect.x + 18) as f32,
                        (display.rect.y + 58) as f32,
                        (display.rect.w - 36) as f32,
                        24.0,
                    ),
                    text: display.detail.clone(),
                    scale: 2.0,
                    color: palette.muted,
                    align: TextAlign::Start,
                    bold: false,
                });
                if display.primary {
                    commands.push(PaintCommand::Text {
                        bounds: UiRect::new(
                            (display.rect.x + 18) as f32,
                            (display.rect.y + display.rect.h - 30) as f32,
                            (display.rect.w - 36) as f32,
                            24.0,
                        ),
                        text: "PRIMARY".into(),
                        scale: 2.0,
                        color: palette.accent,
                        align: TextAlign::Start,
                        bold: true,
                    });
                }
            }
        }
        presenter.present_accelerated(&commands, scale)?;
        Ok(())
    }
}

impl SettingsApp {
    fn finish_resize_if_due(&mut self) {
        let Some(deadline) = self.resize_deadline else {
            return;
        };
        if Instant::now() >= deadline {
            self.resize_deadline = None;
            self.request_redraw();
        }
    }

    fn handle_event(&mut self, event: Event) {
        match event {
            Event::Quit { .. }
            | Event::KeyDown {
                keycode: Some(Keycode::Escape),
                ..
            }
            | Event::Window {
                win_event: WindowEvent::CloseRequested,
                ..
            } => self.running = false,
            Event::Window {
                win_event: WindowEvent::Exposed,
                ..
            } => self.request_redraw(),
            Event::Window {
                win_event: WindowEvent::MouseLeave,
                ..
            } => {
                if self.ui_state.set_hovered(None) != nickel_ui::Invalidation::None {
                    self.request_redraw();
                }
            }
            Event::Window {
                win_event: WindowEvent::Resized(_, _) | WindowEvent::PixelSizeChanged(_, _),
                ..
            } => {
                self.resize_deadline = Some(Instant::now() + Duration::from_millis(24));
            }
            Event::MouseMotion { x, y, .. } => self.pointer_moved(x, y),
            Event::MouseButtonDown {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                self.pointer_moved(x, y);
                self.pointer_pressed();
            }
            Event::MouseButtonUp {
                mouse_btn: MouseButton::Left,
                x,
                y,
                ..
            } => {
                self.pointer_moved(x, y);
                self.finish_drag();
            }
            Event::MouseWheel {
                y,
                direction,
                mouse_x,
                mouse_y,
                ..
            } => {
                self.pointer_moved(mouse_x, mouse_y);
                let wheel_y = if direction == MouseWheelDirection::Flipped {
                    -y
                } else {
                    y
                };
                self.scroll_settings(wheel_y);
            }
            _ => {}
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self
            .appearance_save_deadline
            .is_some_and(|deadline| now >= deadline)
        {
            let _ = self.shell_settings.save_default();
            self.appearance_save_deadline = None;
        }
        for action in self.controller.poll(now) {
            self.handle_controller_action(action);
        }
        if self.page == SettingsPage::Bluetooth && now >= self.next_bluetooth_refresh {
            self.load_bluetooth();
        }
        if self.page == SettingsPage::Network && now >= self.next_network_refresh {
            self.load_linux_network();
        }
        let Some(refresh_at) = self.next_wifi_refresh else {
            return;
        };
        if now < refresh_at {
            return;
        }

        self.load_windows_wifi();
        let connected = self.pending_wifi_profile.as_ref().is_some_and(|profile| {
            self.wifi_networks
                .iter()
                .any(|network| network.connected && network.profile.eq_ignore_ascii_case(profile))
        });
        if connected {
            let profile = self.pending_wifi_profile.take().unwrap_or_default();
            self.wifi_status =
                self.localizer
                    .value("settings-network-connected-to", "profile", &profile);
            self.next_wifi_refresh = None;
            self.wifi_refreshes_left = 0;
        } else if self.wifi_refreshes_left > 1 {
            self.wifi_refreshes_left -= 1;
            self.next_wifi_refresh = Some(Instant::now() + Duration::from_millis(400));
            if let Some(profile) = &self.pending_wifi_profile {
                self.wifi_status =
                    self.localizer
                        .value("settings-network-connecting", "profile", profile);
            }
        } else {
            let profile = self.pending_wifi_profile.take().unwrap_or_default();
            self.wifi_status =
                self.localizer
                    .value("settings-network-connection-timeout", "profile", &profile);
            self.next_wifi_refresh = None;
            self.wifi_refreshes_left = 0;
        }
        self.request_redraw();
    }
}

fn snap_rect(mut moving: Rect, fixed: Rect, threshold: i32) -> Rect {
    let horizontal_candidates = [fixed.x - moving.w, fixed.x + fixed.w];
    if let Some(x) = horizontal_candidates
        .into_iter()
        .min_by_key(|candidate| (moving.x - candidate).abs())
        .filter(|candidate| (moving.x - candidate).abs() <= threshold)
    {
        moving.x = x;
        let vertical_candidates = [fixed.y, fixed.y + fixed.h - moving.h];
        if let Some(y) = vertical_candidates
            .into_iter()
            .min_by_key(|candidate| (moving.y - candidate).abs())
            .filter(|candidate| (moving.y - candidate).abs() <= threshold)
        {
            moving.y = y;
        }
        return moving;
    }

    let vertical_candidates = [fixed.y - moving.h, fixed.y + fixed.h];
    if let Some(y) = vertical_candidates
        .into_iter()
        .min_by_key(|candidate| (moving.y - candidate).abs())
        .filter(|candidate| (moving.y - candidate).abs() <= threshold)
    {
        moving.y = y;
        let horizontal_alignment = [fixed.x, fixed.x + fixed.w - moving.w];
        if let Some(x) = horizontal_alignment
            .into_iter()
            .min_by_key(|candidate| (moving.x - candidate).abs())
            .filter(|candidate| (moving.x - candidate).abs() <= threshold)
        {
            moving.x = x;
        }
    }
    moving
}

#[cfg(target_os = "linux")]
fn choose_wallpaper() -> Option<std::path::PathBuf> {
    let output = std::process::Command::new("kdialog")
        .args([
            "--getopenfilename",
            "",
            "image/png image/jpeg image/webp image/bmp",
        ])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let path = String::from_utf8(output.stdout).ok()?;
    let path = path.trim();
    (!path.is_empty()).then(|| path.into())
}

#[cfg(not(target_os = "linux"))]
fn choose_wallpaper() -> Option<std::path::PathBuf> {
    None
}

fn attach_rect_centered(moving: Rect, fixed: Rect) -> Rect {
    let candidates = [
        Rect {
            x: fixed.x - moving.w,
            y: fixed.y + (fixed.h - moving.h) / 2,
            ..moving
        },
        Rect {
            x: fixed.x + fixed.w,
            y: fixed.y + (fixed.h - moving.h) / 2,
            ..moving
        },
        Rect {
            x: fixed.x + (fixed.w - moving.w) / 2,
            y: fixed.y - moving.h,
            ..moving
        },
        Rect {
            x: fixed.x + (fixed.w - moving.w) / 2,
            y: fixed.y + fixed.h,
            ..moving
        },
    ];
    candidates
        .into_iter()
        .min_by_key(|candidate| {
            let dx = candidate.x - moving.x;
            let dy = candidate.y - moving.y;
            dx * dx + dy * dy
        })
        .unwrap_or(moving)
}

fn logical_placements(displays: &[DisplayCard]) -> Vec<(i32, i32)> {
    if displays.is_empty() {
        return Vec::new();
    }
    let mut placements = vec![(0, 0); displays.len()];
    for index in 1..displays.len() {
        let moving = &displays[index];
        let fixed = &displays[index - 1];
        let (fixed_x, fixed_y) = placements[index - 1];
        let edge_distances = [
            (moving.rect.x + moving.rect.w - fixed.rect.x).abs(),
            (moving.rect.x - fixed.rect.x - fixed.rect.w).abs(),
            (moving.rect.y + moving.rect.h - fixed.rect.y).abs(),
            (moving.rect.y - fixed.rect.y - fixed.rect.h).abs(),
        ];
        let edge = edge_distances
            .iter()
            .enumerate()
            .min_by_key(|(_, distance)| *distance)
            .map(|(edge, _)| edge)
            .unwrap_or(1);
        placements[index] = match edge {
            0 => (
                fixed_x - moving.logical_width,
                fixed_y + (fixed.logical_height - moving.logical_height) / 2,
            ),
            1 => (
                fixed_x + fixed.logical_width,
                fixed_y + (fixed.logical_height - moving.logical_height) / 2,
            ),
            2 => (
                fixed_x + (fixed.logical_width - moving.logical_width) / 2,
                fixed_y - moving.logical_height,
            ),
            _ => (
                fixed_x + (fixed.logical_width - moving.logical_width) / 2,
                fixed_y + fixed.logical_height,
            ),
        };
    }
    let minimum_x = placements.iter().map(|(x, _)| *x).min().unwrap_or(0);
    let minimum_y = placements.iter().map(|(_, y)| *y).min().unwrap_or(0);
    for (x, y) in &mut placements {
        *x -= minimum_x;
        *y -= minimum_y;
    }
    placements
}

#[cfg(target_os = "windows")]
fn apply_windows_layout(
    displays: &[DisplayCard],
    placements: &[(i32, i32)],
    primary: &str,
) -> Result<(), i32> {
    use std::mem::size_of;
    use windows::{
        Win32::Graphics::Gdi::{
            CDS_NORESET, CDS_SET_PRIMARY, CDS_TYPE, CDS_UPDATEREGISTRY, ChangeDisplaySettingsExW,
            DEVMODEW, DISP_CHANGE_SUCCESSFUL, DM_POSITION, ENUM_CURRENT_SETTINGS,
            EnumDisplaySettingsW,
        },
        core::PCWSTR,
    };

    let (primary_x, primary_y) = displays
        .iter()
        .zip(placements)
        .find(|(display, _)| display.connector == primary)
        .map(|(_, placement)| *placement)
        .unwrap_or((0, 0));

    for (display, &(x, y)) in displays.iter().zip(placements) {
        let device_name = format!(r"\\.\{}", display.connector);
        let device_wide: Vec<u16> = device_name.encode_utf16().chain([0]).collect();
        let device = PCWSTR(device_wide.as_ptr());
        let mut mode = DEVMODEW {
            dmSize: size_of::<DEVMODEW>() as u16,
            ..Default::default()
        };
        // SAFETY: `device` and `mode` remain valid for the duration of each Win32 call.
        if !unsafe { EnumDisplaySettingsW(device, ENUM_CURRENT_SETTINGS, &raw mut mode) }.as_bool()
        {
            return Err(-1);
        }
        mode.dmFields |= DM_POSITION;
        mode.Anonymous1.Anonymous2.dmPosition.x = x - primary_x;
        mode.Anonymous1.Anonymous2.dmPosition.y = y - primary_y;
        let mut flags = CDS_UPDATEREGISTRY | CDS_NORESET;
        if display.connector == primary {
            flags |= CDS_SET_PRIMARY;
        }
        // SAFETY: The device name and initialized DEVMODEW are valid for this synchronous call.
        let result =
            unsafe { ChangeDisplaySettingsExW(device, Some(&raw const mode), None, flags, None) };
        if result != DISP_CHANGE_SUCCESSFUL {
            return Err(result.0);
        }
    }

    // Commit all staged changes together to avoid transient intermediate layouts.
    // SAFETY: A null device and mode apply the previously staged changes.
    let result =
        unsafe { ChangeDisplaySettingsExW(PCWSTR::null(), None, None, CDS_TYPE::default(), None) };
    if result == DISP_CHANGE_SUCCESSFUL {
        Ok(())
    } else {
        Err(result.0)
    }
}

fn constrain_center(mut monitor: Rect, plane: Rect) -> Rect {
    monitor.x = monitor.x.clamp(plane.x, plane.x + plane.w - monitor.w);
    monitor.y = monitor.y.clamp(plane.y, plane.y + plane.h - monitor.h);
    monitor
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let started = Instant::now();
    let _log_path = nickel_logging::init("nickel-settings").ok();
    sdl3::hint::set("SDL_APP_ID", "nickel-settings");
    let sdl = sdl3::init()?;
    let video = sdl.video()?;
    let mut window = video
        .window("Nickel Settings", 850, 580)
        .position_centered()
        .resizable()
        .build()
        .map_err(|error| error.to_string())?;
    window.set_minimum_size(850, 580)?;
    video.text_input().start(&window);
    let mut events = sdl.event_pump()?;
    let frame_interval = window
        .get_display()
        .ok()
        .and_then(|display| display.get_mode().ok())
        .map(|mode| mode.refresh_rate)
        .filter(|refresh_rate| refresh_rate.is_finite() && *refresh_rate > 1.0)
        .map(|refresh_rate| Duration::from_secs_f64(1.0 / f64::from(refresh_rate)))
        .unwrap_or_else(|| Duration::from_millis(16));
    let mut app = SettingsApp {
        frame_interval,
        ..SettingsApp::default()
    };
    let mut presenter = SdlCanvasPresenter::new(window)?;
    app.render(&mut presenter)?;
    tracing::info!(
        target: "nickel",
        elapsed_ms = started.elapsed().as_secs_f64() * 1_000.0,
        "Nickel Settings first frame presented"
    );
    app.load_outputs();
    app.load_bluetooth();
    app.load_linux_network();
    #[cfg(target_os = "windows")]
    {
        app.load_windows_outputs(&video);
        app.load_windows_network();
        app.load_windows_wifi();
    }
    let mut next_frame = Instant::now();

    while app.running {
        if let Some(event) = events.wait_event_timeout(app.frame_interval) {
            app.handle_event(event);
            for event in events.poll_iter() {
                app.handle_event(event);
            }
        }
        app.tick();
        app.finish_resize_if_due();
        let now = Instant::now();
        if app.redraw_requested.get() && now >= next_frame {
            app.redraw_requested.set(false);
            if let Err(error) = app.render(&mut presenter) {
                tracing::error!(%error, "failed to render Nickel Settings");
                app.running = false;
            }
            next_frame = Instant::now() + app.frame_interval;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        NetworkAdapter, Rect, SettingsApp, SettingsMessage, SettingsPage, attach_rect_centered,
        constrain_center, snap_rect,
    };

    #[test]
    fn display_remains_completely_inside_plane() {
        let plane = Rect {
            x: 40,
            y: 120,
            w: 770,
            h: 340,
        };
        let monitor = Rect {
            x: -500,
            y: -500,
            w: 300,
            h: 180,
        };

        let constrained = constrain_center(monitor, plane);
        assert_eq!(constrained.x, plane.x);
        assert_eq!(constrained.y, plane.y);
    }

    #[test]
    fn nearby_monitor_edges_snap_together_and_align() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 300,
            h: 180,
        };
        let moving = Rect {
            x: 105,
            y: 190,
            w: 270,
            h: 160,
        };

        let snapped = snap_rect(moving, fixed, 42);
        assert_eq!(snapped.x + snapped.w, fixed.x);
        assert_eq!(snapped.y, fixed.y);
    }

    #[test]
    fn released_monitor_touches_and_centers_on_nearest_edge() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 220,
            h: 125,
        };
        let moving = Rect {
            x: 150,
            y: 205,
            w: 220,
            h: 125,
        };

        let attached = attach_rect_centered(moving, fixed);

        assert_eq!(attached.x + attached.w, fixed.x);
        assert_eq!(attached.y + attached.h / 2, fixed.y + fixed.h / 2);
    }

    #[test]
    fn distant_monitors_keep_freeform_position() {
        let fixed = Rect {
            x: 400,
            y: 180,
            w: 300,
            h: 180,
        };
        let moving = Rect {
            x: 40,
            y: 430,
            w: 200,
            h: 120,
        };

        let snapped = snap_rect(moving, fixed, 42);
        assert_eq!((snapped.x, snapped.y), (moving.x, moving.y));
    }

    #[test]
    fn settings_lists_derive_scroll_extents_from_intrinsic_children() {
        let mut app = SettingsApp {
            page: SettingsPage::Network,
            network_adapters: (0..12)
                .map(|index| NetworkAdapter {
                    name: format!("Adapter {index}"),
                    description: "Synthetic adapter".into(),
                    connected: false,
                    speed: 0,
                })
                .collect(),
            ..SettingsApp::default()
        };
        let narrow = app.build_ui(560.0, 360.0);
        assert!(
            narrow
                .scroll_extent(&SettingsMessage::NetworkScroll)
                .is_some_and(|extent| extent.can_scroll())
        );

        app.page = SettingsPage::Bluetooth;
        let compact = app.build_ui(560.0, 360.0);
        assert!(compact.commands().iter().all(|command| match command {
            nickel_ui::PaintCommand::Fill { rect, .. }
            | nickel_ui::PaintCommand::OverlayFill { rect, .. }
            | nickel_ui::PaintCommand::Stroke { rect, .. }
            | nickel_ui::PaintCommand::OverlayStroke { rect, .. }
            | nickel_ui::PaintCommand::TopRoundedFill { rect, .. }
            | nickel_ui::PaintCommand::Gradient { rect, .. } => {
                rect.size.width >= 0.0 && rect.size.height >= 0.0
            }
            _ => true,
        }));

        let source = include_str!("main.rs");
        assert!(!source.contains(&["network_", "content_height"].concat()));
        assert!(!source.contains(&["bluetooth_", "content_height"].concat()));
        assert!(!source.contains(&["device_", "height"].concat()));
    }
}
