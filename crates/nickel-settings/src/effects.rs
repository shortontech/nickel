use super::*;

impl SettingsApp {
    pub(crate) fn load_bluetooth(&mut self) {
        match read_bluetooth_snapshot() {
            Ok(snapshot) => self.bluetooth = snapshot,
            Err(error) => {
                tracing::warn!(%error, "failed to read Bluetooth settings");
                self.bluetooth_status = Some(error);
            }
        }
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
        let Ok(ServerMessage::Outputs(protocol_outputs)) =
            session_request(SessionRequest::Query(SessionQuery::Outputs))
        else {
            self.status = self.localizer.text("settings-status-using-mock-displays");
            return;
        };
        let outputs: Vec<_> = protocol_outputs
            .into_iter()
            .map(|output| OutputSnapshot {
                name: output.name,
                model: output.model,
                x: output.geometry.x,
                y: output.geometry.y,
                width: output.geometry.width,
                height: output.geometry.height,
                physical_width: output.physical_width_mm,
                physical_height: output.physical_height_mm,
                primary: output.primary,
                enabled: output.enabled,
                scale_120: output.scale_120,
            })
            .collect();
        if outputs.is_empty() {
            self.status = self.localizer.text("settings-status-using-mock-displays");
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
        // Leave enough empty plane around the arrangement to drag one display
        // completely around another before release snapping chooses an edge.
        self.pixels_per_logical = (470.0 / f64::from((maximum_x - minimum_x).max(1)))
            .min(190.0 / f64::from((maximum_y - minimum_y).max(1)))
            .max(0.04);
        self.displays = outputs
            .into_iter()
            .map(|output| {
                let physical_size_known =
                    output.physical_width >= 50 && output.physical_height >= 50;
                let (card_width, card_height) = if physical_size_known {
                    (
                        compressed_physical_extent(
                            output.physical_width,
                            maximum_physical_width,
                            280,
                        ),
                        compressed_physical_extent(
                            output.physical_height,
                            maximum_physical_height,
                            160,
                        ),
                    )
                } else {
                    (
                        (f64::from(output.width) * self.pixels_per_logical).round() as i32,
                        (f64::from(output.height) * self.pixels_per_logical).round() as i32,
                    )
                };
                DisplayCard {
                    connector: output.name.clone(),
                    detail: format!("{}  {} × {}", output.name, output.width, output.height),
                    name: output.model,
                    logical_width: output.width,
                    logical_height: output.height,
                    rect: Rect {
                        x: self.display_plane.x
                            + (f64::from(output.x - minimum_x) * self.pixels_per_logical).round()
                                as i32,
                        y: self.display_plane.y
                            + (f64::from(output.y - minimum_y) * self.pixels_per_logical).round()
                                as i32,
                        w: card_width.max(120),
                        h: card_height.max(80),
                    },
                    primary: output.primary,
                    enabled: output.enabled,
                    scale_120: output.scale_120,
                }
            })
            .collect();
        for index in 1..self.displays.len() {
            let previous = self.displays[index - 1].rect;
            self.displays[index].rect = attach_rect_centered(self.displays[index].rect, previous);
        }
        center_display_rects(&mut self.displays, self.display_plane);
        self.selected = self
            .displays
            .iter()
            .position(|display| display.primary)
            .unwrap_or(0);
        if self.pending_display_revert.is_none() {
            self.confirmed_displays = self.displays.clone();
        }
        self.applied = true;
        self.status.clear();
    }

    #[cfg(target_os = "windows")]
    pub(crate) fn load_windows_outputs(&mut self, window: &winit::window::Window) {
        let monitors = window.available_monitors().collect::<Vec<_>>();
        if monitors.is_empty() {
            self.status = self.localizer.text("settings-status-no-displays");
            return;
        }
        let primary = window.primary_monitor();
        let minimum_x = monitors
            .iter()
            .map(|monitor| monitor.position().x)
            .min()
            .unwrap_or(0);
        let minimum_y = monitors
            .iter()
            .map(|monitor| monitor.position().y)
            .min()
            .unwrap_or(0);
        let maximum_x = monitors
            .iter()
            .map(|monitor| monitor.position().x + monitor.size().width as i32)
            .max()
            .unwrap_or(1);
        let maximum_y = monitors
            .iter()
            .map(|monitor| monitor.position().y + monitor.size().height as i32)
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
                let position = monitor.position();
                let size = monitor.size();
                let raw_name = monitor
                    .name()
                    .unwrap_or_else(|| format!("Display {}", index + 1));
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
                    detail: format!("{}  {} X {}", connector, size.width, size.height),
                    logical_width: size.width as i32,
                    logical_height: size.height as i32,
                    rect: Rect {
                        x: origin_x
                            + (f64::from(position.x - minimum_x) * self.pixels_per_logical).round()
                                as i32,
                        y: origin_y
                            + (f64::from(position.y - minimum_y) * self.pixels_per_logical).round()
                                as i32,
                        w: (f64::from(size.width) * self.pixels_per_logical).round() as i32,
                        h: (f64::from(size.height) * self.pixels_per_logical).round() as i32,
                    },
                    primary: primary.as_ref() == Some(&monitor),
                    enabled: true,
                    scale_120: (monitor.scale_factor() * 120.0).round() as u32,
                }
            })
            .collect();
        self.selected = self
            .displays
            .iter()
            .position(|display| display.primary)
            .unwrap_or(0);
        if self.pending_display_revert.is_none() {
            self.confirmed_displays = self.displays.clone();
        }
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
                    self.display_apply_succeeded();
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
        }

        #[cfg(not(target_os = "windows"))]
        let layout = SessionOutputLayout {
            primary: primary.to_owned(),
            placements: self
                .displays
                .iter()
                .zip(placements)
                .map(|(display, (x, y))| SessionOutputPlacement {
                    name: display.connector.clone(),
                    x,
                    y,
                    enabled: display.enabled,
                    scale_120: display.scale_120,
                })
                .collect(),
        };
        #[cfg(not(target_os = "windows"))]
        match session_request(SessionRequest::Command(SessionCommand::ApplyOutputs {
            layout,
        })) {
            Ok(ServerMessage::Ack) => {
                self.display_apply_succeeded();
            }
            Ok(ServerMessage::Error { message, .. }) => {
                self.applied = false;
                self.status =
                    self.localizer
                        .value("settings-status-apply-failed", "error", &message);
            }
            Ok(_) | Err(_) => {
                self.applied = false;
                self.status = self.localizer.text("settings-status-session-unavailable");
            }
        }
    }

    pub(crate) fn display_apply_succeeded(&mut self) {
        self.applied = true;
        if self.reverting_display {
            self.reverting_display = false;
            self.confirmed_displays = self.displays.clone();
            self.pending_display_revert = None;
            self.status = "Previous display settings restored.".into();
        } else {
            self.pending_display_revert = Some((
                Instant::now() + Duration::from_secs(15),
                self.confirmed_displays.clone(),
            ));
            self.status = "Keep these display settings? They will revert in 15 seconds.".into();
        }
    }

    pub(crate) fn revert_display_layout(&mut self) {
        let Some((_, previous)) = self.pending_display_revert.take() else {
            return;
        };
        self.displays = previous;
        self.selected = self.selected.min(self.displays.len().saturating_sub(1));
        self.reverting_display = true;
        self.apply_layout();
    }
}

/// Preserve the perceived ordering of physical display sizes without making a
/// portable monitor unusably tiny beside a desktop panel. A fifth-root curve
/// deliberately compresses the real-world ratio while keeping equal extents
/// equal and the largest extent at the available visual size.
fn compressed_physical_extent(value: i32, maximum: i32, rendered_maximum: i32) -> i32 {
    let ratio = f64::from(value.max(1)) / f64::from(maximum.max(1));
    (ratio.powf(0.2) * f64::from(rendered_maximum)).round() as i32
}

#[cfg(test)]
mod display_size_tests {
    use super::compressed_physical_extent;

    #[test]
    fn physical_size_ratios_are_compressed_without_becoming_equal() {
        let desktop = compressed_physical_extent(600, 600, 280);
        let portable = compressed_physical_extent(310, 600, 280);

        assert_eq!(desktop, 280);
        assert!((240..desktop).contains(&portable));
    }
}
