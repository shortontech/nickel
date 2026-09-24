//! Windows connectivity observations and controls shared by Settings and Control Center.

use std::{collections::HashMap, future::IntoFuture};
use windows::{
    Devices::{
        Bluetooth::{BluetoothConnectionStatus, BluetoothDevice, BluetoothLEDevice},
        Enumeration::{DeviceInformation, DevicePairingResultStatus},
        Radios::{Radio, RadioAccessStatus, RadioKind, RadioState},
    },
    Win32::{
        Foundation::{HANDLE, NO_ERROR},
        NetworkManagement::WiFi::{
            WLAN_AVAILABLE_NETWORK_CONNECTED, WLAN_AVAILABLE_NETWORK_LIST,
            WLAN_CONNECTION_ATTRIBUTES, WLAN_CONNECTION_PARAMETERS, WLAN_INTERFACE_INFO_LIST,
            WLAN_PHY_RADIO_STATE, WLAN_PROFILE_INFO_LIST, WLAN_RADIO_STATE, WlanCloseHandle,
            WlanConnect, WlanEnumInterfaces, WlanFreeMemory, WlanGetAvailableNetworkList,
            WlanGetProfileList, WlanOpenHandle, WlanQueryInterface, dot11_BSS_type_any,
            dot11_radio_state_on, wlan_connection_mode_profile, wlan_interface_state_connected,
            wlan_intf_opcode_current_connection, wlan_intf_opcode_radio_state,
        },
    },
    core::{GUID, PCWSTR},
};

#[derive(Clone, Debug, Default)]
pub struct WifiSnapshot {
    pub available: bool,
    pub enabled: bool,
    pub connected: bool,
    pub name: String,
    pub signal_percent: u32,
    pub networks: Vec<WifiNetwork>,
}

#[derive(Clone, Debug)]
pub struct WifiNetwork {
    pub profile: String,
    pub interface: u128,
    pub signal_percent: u32,
    pub connected: bool,
}

impl WifiNetwork {
    pub fn id(&self) -> String {
        format!("{:032x}:{}", self.interface, self.profile)
    }
}

#[derive(Clone, Debug, Default)]
pub struct BluetoothSnapshot {
    pub available: bool,
    pub powered: bool,
    pub adapter_name: String,
    pub devices: Vec<BluetoothDeviceSnapshot>,
}

#[derive(Clone, Debug)]
pub struct BluetoothDeviceSnapshot {
    pub id: String,
    pub name: String,
    pub paired: bool,
    pub connected: bool,
}

fn radio(kind: RadioKind) -> Result<Option<Radio>, String> {
    let radios = futures_lite::future::block_on(
        Radio::GetRadiosAsync()
            .map_err(|error| error.to_string())?
            .into_future(),
    )
    .map_err(|error| error.to_string())?;
    Ok((0..radios.Size().map_err(|error| error.to_string())?)
        .filter_map(|index| radios.GetAt(index).ok())
        .find(|radio| radio.Kind().ok() == Some(kind)))
}

fn set_radio(kind: RadioKind, powered: bool) -> Result<(), String> {
    let radio = radio(kind)?.ok_or_else(|| "radio is unavailable".to_owned())?;
    let access = futures_lite::future::block_on(
        radio
            .SetStateAsync(if powered {
                RadioState::On
            } else {
                RadioState::Off
            })
            .map_err(|error| error.to_string())?
            .into_future(),
    )
    .map_err(|error| error.to_string())?;
    if access == RadioAccessStatus::Allowed {
        Ok(())
    } else {
        Err(format!("Windows denied radio access ({})", access.0))
    }
}

pub fn set_wifi_powered(powered: bool) -> Result<(), String> {
    set_radio(RadioKind::WiFi, powered)
}

pub fn set_bluetooth_powered(powered: bool) -> Result<(), String> {
    set_radio(RadioKind::Bluetooth, powered)
}

fn wide_text(buffer: &[u16]) -> String {
    let length = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..length])
}

pub fn wifi_snapshot() -> Result<WifiSnapshot, String> {
    let mut negotiated = 0;
    let mut handle = HANDLE::default();
    // SAFETY: Both output pointers refer to initialized writable stack storage.
    let result = unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) };
    if result != NO_ERROR.0 {
        return Err(format!("WlanOpenHandle failed ({result})"));
    }
    let snapshot = wifi_snapshot_with_handle(handle);
    // SAFETY: WlanOpenHandle succeeded and the handle is closed exactly once.
    unsafe { WlanCloseHandle(handle, None) };
    snapshot
}

fn wifi_snapshot_with_handle(handle: HANDLE) -> Result<WifiSnapshot, String> {
    let mut interfaces = std::ptr::null_mut::<WLAN_INTERFACE_INFO_LIST>();
    // SAFETY: The handle is open and interfaces points to writable pointer storage.
    let result = unsafe { WlanEnumInterfaces(handle, None, &mut interfaces) };
    if result != NO_ERROR.0 || interfaces.is_null() {
        return Err(format!("WlanEnumInterfaces failed ({result})"));
    }
    let snapshot = (|| {
        // SAFETY: WlanEnumInterfaces returned a non-null allocation containing its reported count.
        let entries = unsafe {
            std::slice::from_raw_parts(
                (*interfaces).InterfaceInfo.as_ptr(),
                (*interfaces).dwNumberOfItems as usize,
            )
        };
        let wifi_radio = radio(RadioKind::WiFi).ok().flatten();
        let mut snapshot = WifiSnapshot {
            available: !entries.is_empty(),
            enabled: wifi_radio.as_ref().and_then(|radio| radio.State().ok())
                == Some(RadioState::On),
            ..Default::default()
        };
        let mut networks = HashMap::<String, WifiNetwork>::new();
        for interface in entries {
            let mut radio_bytes = 0;
            let mut radio_data = std::ptr::null_mut();
            // SAFETY: The GUID comes from WLAN and the result pointers are writable.
            let radio_result = unsafe {
                WlanQueryInterface(
                    handle,
                    &raw const interface.InterfaceGuid,
                    wlan_intf_opcode_radio_state,
                    None,
                    &mut radio_bytes,
                    &mut radio_data,
                    None,
                )
            };
            if radio_result == NO_ERROR.0 && !radio_data.is_null() {
                let offset = std::mem::offset_of!(WLAN_RADIO_STATE, PhyRadioState);
                if radio_bytes as usize >= offset {
                    let capacity = (radio_bytes as usize - offset)
                        / std::mem::size_of::<WLAN_PHY_RADIO_STATE>();
                    // SAFETY: The result includes the count field and at least capacity PHY
                    // records; bound the reported count by both the allocation and WLAN's maximum.
                    let count = unsafe { radio_data.cast::<u32>().read_unaligned() } as usize;
                    let phys = unsafe {
                        std::slice::from_raw_parts(
                            radio_data
                                .cast::<u8>()
                                .add(offset)
                                .cast::<WLAN_PHY_RADIO_STATE>(),
                            count.min(capacity).min(64),
                        )
                    };
                    snapshot.enabled |= phys.iter().any(|phy| {
                        phy.dot11SoftwareRadioState == dot11_radio_state_on
                            && phy.dot11HardwareRadioState == dot11_radio_state_on
                    });
                }
                // SAFETY: WLAN owns this non-null query result; free it exactly once.
                unsafe { WlanFreeMemory(radio_data) };
            }
            let mut bytes = 0;
            let mut data = std::ptr::null_mut();
            // SAFETY: The interface GUID comes from WLAN and all result pointers are writable.
            let result = unsafe {
                WlanQueryInterface(
                    handle,
                    &raw const interface.InterfaceGuid,
                    wlan_intf_opcode_current_connection,
                    None,
                    &mut bytes,
                    &mut data,
                    None,
                )
            };
            if result == NO_ERROR.0 && !data.is_null() {
                if bytes >= std::mem::size_of::<WLAN_CONNECTION_ATTRIBUTES>() as u32 {
                    // SAFETY: WLAN supplied at least the complete connection-attributes structure.
                    let connection = unsafe { &*data.cast::<WLAN_CONNECTION_ATTRIBUTES>() };
                    if connection.isState == wlan_interface_state_connected {
                        let ssid = &connection.wlanAssociationAttributes.dot11Ssid;
                        let length = (ssid.uSSIDLength as usize).min(ssid.ucSSID.len());
                        snapshot.connected = true;
                        snapshot.enabled = true;
                        snapshot.name =
                            String::from_utf8_lossy(&ssid.ucSSID[..length]).into_owned();
                        snapshot.signal_percent =
                            connection.wlanAssociationAttributes.wlanSignalQuality;
                    }
                }
                // SAFETY: WLAN owns this non-null query result; free it exactly once.
                unsafe { WlanFreeMemory(data) };
            }
            let mut available = std::ptr::null_mut::<WLAN_AVAILABLE_NETWORK_LIST>();
            let mut signals = HashMap::<String, (u32, bool)>::new();
            // SAFETY: The GUID is valid for this open handle and the output pointer is writable.
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
                // SAFETY: WLAN returned a non-null list with the reported item count.
                let items = unsafe {
                    std::slice::from_raw_parts(
                        (*available).Network.as_ptr(),
                        (*available).dwNumberOfItems as usize,
                    )
                };
                for item in items {
                    let name = wide_text(&item.strProfileName);
                    if !name.is_empty() {
                        signals.insert(
                            name.to_ascii_lowercase(),
                            (
                                item.wlanSignalQuality,
                                item.dwFlags & WLAN_AVAILABLE_NETWORK_CONNECTED != 0,
                            ),
                        );
                    }
                }
                // SAFETY: WLAN allocated this list; free it exactly once after reading it.
                unsafe { WlanFreeMemory(available.cast()) };
            }
            let mut profiles = std::ptr::null_mut::<WLAN_PROFILE_INFO_LIST>();
            // SAFETY: The GUID and handle are valid and the output pointer is writable.
            if unsafe {
                WlanGetProfileList(
                    handle,
                    &raw const interface.InterfaceGuid,
                    None,
                    &mut profiles,
                )
            } == NO_ERROR.0
                && !profiles.is_null()
            {
                // SAFETY: WLAN returned a non-null list with the reported profile count.
                let items = unsafe {
                    std::slice::from_raw_parts(
                        (*profiles).ProfileInfo.as_ptr(),
                        (*profiles).dwNumberOfItems as usize,
                    )
                };
                for item in items {
                    let profile = wide_text(&item.strProfileName);
                    if profile.is_empty() {
                        continue;
                    }
                    let (signal_percent, connected) = signals
                        .get(&profile.to_ascii_lowercase())
                        .copied()
                        .unwrap_or((0, false));
                    let network = WifiNetwork {
                        profile,
                        interface: interface.InterfaceGuid.to_u128(),
                        signal_percent,
                        connected,
                    };
                    networks.insert(network.id(), network);
                }
                // SAFETY: WLAN allocated this list; free it exactly once after reading it.
                unsafe { WlanFreeMemory(profiles.cast()) };
            }
        }
        snapshot.networks = networks.into_values().collect();
        snapshot.networks.sort_by_key(|network| {
            (
                !network.connected,
                network.signal_percent == 0,
                std::cmp::Reverse(network.signal_percent),
                network.profile.to_ascii_lowercase(),
            )
        });
        Ok(snapshot)
    })();
    // SAFETY: WLAN allocated the interface list; free it exactly once after all reads.
    unsafe { WlanFreeMemory(interfaces.cast()) };
    snapshot
}

pub fn connect_wifi(interface: u128, profile: &str) -> Result<(), String> {
    let mut negotiated = 0;
    let mut handle = HANDLE::default();
    // SAFETY: Both output pointers refer to initialized writable stack storage.
    let result = unsafe { WlanOpenHandle(2, None, &mut negotiated, &mut handle) };
    if result != NO_ERROR.0 {
        return Err(format!("WlanOpenHandle failed ({result})"));
    }
    let profile_wide: Vec<u16> = profile.encode_utf16().chain([0]).collect();
    let parameters = WLAN_CONNECTION_PARAMETERS {
        wlanConnectionMode: wlan_connection_mode_profile,
        strProfile: PCWSTR(profile_wide.as_ptr()),
        dot11BssType: dot11_BSS_type_any,
        ..Default::default()
    };
    // SAFETY: The handle is open and the GUID, parameters, and UTF-16 profile live through the call.
    let result = unsafe { WlanConnect(handle, &GUID::from_u128(interface), &parameters, None) };
    // SAFETY: WlanOpenHandle succeeded and this closes the handle exactly once.
    unsafe { WlanCloseHandle(handle, None) };
    if result == NO_ERROR.0 {
        Ok(())
    } else {
        Err(format!("WlanConnect failed ({result})"))
    }
}

pub fn connect_wifi_id(id: &str) -> Result<(), String> {
    let (interface, profile) = id.split_once(':').ok_or("invalid Wi-Fi network ID")?;
    let interface =
        u128::from_str_radix(interface, 16).map_err(|_| "invalid Wi-Fi interface ID")?;
    connect_wifi(interface, profile)
}

pub fn bluetooth_snapshot() -> Result<BluetoothSnapshot, String> {
    let Some(radio) = radio(RadioKind::Bluetooth)? else {
        return Ok(BluetoothSnapshot::default());
    };
    let mut devices = HashMap::<String, BluetoothDeviceSnapshot>::new();
    for selector in [
        BluetoothDevice::GetDeviceSelector().map_err(|error| error.to_string())?,
        BluetoothLEDevice::GetDeviceSelector().map_err(|error| error.to_string())?,
    ] {
        let found = futures_lite::future::block_on(
            DeviceInformation::FindAllAsyncAqsFilter(&selector)
                .map_err(|error| error.to_string())?
                .into_future(),
        )
        .map_err(|error| error.to_string())?;
        for index in 0..found.Size().map_err(|error| error.to_string())? {
            let info = found.GetAt(index).map_err(|error| error.to_string())?;
            let id = info.Id().map_err(|error| error.to_string())?.to_string();
            let paired = info
                .Pairing()
                .and_then(|pairing| pairing.IsPaired())
                .unwrap_or(false);
            let connected = futures_lite::future::block_on(
                BluetoothDevice::FromIdAsync(&id.clone().into())
                    .map_err(|error| error.to_string())?
                    .into_future(),
            )
            .ok()
            .and_then(|device| device.ConnectionStatus().ok())
                == Some(BluetoothConnectionStatus::Connected)
                || futures_lite::future::block_on(
                    BluetoothLEDevice::FromIdAsync(&id.clone().into())
                        .map_err(|error| error.to_string())?
                        .into_future(),
                )
                .ok()
                .and_then(|device| device.ConnectionStatus().ok())
                    == Some(BluetoothConnectionStatus::Connected);
            devices
                .entry(id.clone())
                .or_insert_with(|| BluetoothDeviceSnapshot {
                    id,
                    name: info
                        .Name()
                        .map(|name| name.to_string())
                        .unwrap_or_else(|_| "Unknown device".into()),
                    paired,
                    connected,
                });
        }
    }
    let mut devices: Vec<_> = devices.into_values().collect();
    devices.sort_by(|left, right| {
        right
            .connected
            .cmp(&left.connected)
            .then_with(|| right.paired.cmp(&left.paired))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(BluetoothSnapshot {
        available: true,
        powered: radio.State().ok() == Some(RadioState::On),
        adapter_name: radio
            .Name()
            .map(|name| name.to_string())
            .unwrap_or_default(),
        devices,
    })
}

pub fn pair_bluetooth_device(id: &str, connected: bool) -> Result<(), String> {
    if connected {
        return Err("Windows cannot safely disconnect this Bluetooth profile".into());
    }
    let info = futures_lite::future::block_on(
        DeviceInformation::CreateFromIdAsync(&id.into())
            .map_err(|error| error.to_string())?
            .into_future(),
    )
    .map_err(|error| error.to_string())?;
    let pairing = info.Pairing().map_err(|error| error.to_string())?;
    let result = futures_lite::future::block_on(
        pairing
            .PairAsync()
            .map_err(|error| error.to_string())?
            .into_future(),
    )
    .map_err(|error| error.to_string())?;
    let status = result.Status().map_err(|error| error.to_string())?;
    if matches!(
        status,
        DevicePairingResultStatus::Paired | DevicePairingResultStatus::AlreadyPaired
    ) {
        Ok(())
    } else {
        Err(format!("Windows Bluetooth pairing failed ({})", status.0))
    }
}
