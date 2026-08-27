use super::*;

impl SettingsApp {
    pub(crate) fn load_bluetooth(&mut self) {
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
    pub(crate) fn load_linux_network(&mut self) {
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
    pub(crate) fn load_linux_network(&mut self) {
        self.load_windows_network();
        self.load_windows_wifi();
        self.next_network_refresh = Instant::now() + Duration::from_secs(2);
    }

    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    pub(crate) fn load_linux_network(&mut self) {
        self.next_network_refresh = Instant::now() + Duration::from_secs(2);
    }

    pub(crate) fn load_outputs(&mut self) {
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
    pub(crate) fn load_windows_outputs(&mut self, video: &sdl3::VideoSubsystem) {
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
    pub(crate) fn load_windows_network(&mut self) {
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
    pub(crate) fn load_windows_wifi(&mut self) {
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
    pub(crate) fn load_windows_wifi(&mut self) {}

    #[cfg(target_os = "windows")]
    pub(crate) fn connect_windows_wifi(&mut self, index: usize) {
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
    pub(crate) fn connect_windows_wifi(&mut self, index: usize) {
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
    pub(crate) fn connect_windows_wifi(&mut self, _index: usize) {}

    pub(crate) fn apply_layout(&mut self) {
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
}
