//! Public connectivity clients backed by the platform inventory, independent of presentation.
use std::hash::{Hash, Hasher};

use nickel_core::plugins::PluginCapability;
use serde_json::{Value, json};

use crate::platform::{self, BluetoothStatus, NetworkStatus};

fn revision(value: &Value) -> String {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    value.to_string().hash(&mut hash);
    format!("{:016x}", hash.finish())
}

pub(crate) fn wifi_snapshot(status: &NetworkStatus) -> Value {
    let mut value = json!({
        "available": status.available,
        "reason": (!status.available).then_some("Wi-Fi service is unavailable"),
        "enabled": status.enabled,
        "connected": status.connected,
        "operations": {"setEnabled": status.available, "connect": status.available},
        "networks": status.networks.iter().take(256).filter(|item| !item.id.is_empty() && item.id.len() <= 512).map(|item| json!({
            "id": item.id, "name": item.name.chars().take(512).collect::<String>(),
            "signalPercent": item.signal_percent.min(100), "connected": item.connected,
            "saved": item.saved, "canConnect": status.available && status.enabled && item.saved && !item.connected,
        })).collect::<Vec<_>>()
    });
    value["revision"] = revision(&value).into();
    value
}

pub(crate) fn bluetooth_snapshot(status: &BluetoothStatus) -> Value {
    let linux = cfg!(target_os = "linux");
    let windows = cfg!(target_os = "windows");
    let mut value = json!({
        "available": status.available,
        "reason": (!status.available).then_some("Bluetooth service is unavailable"),
        "powered": status.powered, "discovering": status.discovering,
        "operations": {"setPowered": status.available, "setDiscovery": status.available && linux,
            "connect": status.available && linux, "disconnect": status.available && linux,
            "pair": status.available && windows},
        "devices": status.devices.iter().take(256).filter(|item| !item.id.is_empty() && item.id.len() <= 512).map(|item| json!({
            "id": item.id, "name": item.name.chars().take(512).collect::<String>(),
            "paired": item.paired, "connected": item.connected,
        })).collect::<Vec<_>>()
    });
    value["revision"] = revision(&value).into();
    value
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConnectivityEffect {
    operation: String,
    revision: String,
    id: Option<String>,
    value: Option<bool>,
}

impl ConnectivityEffect {
    pub(crate) fn parse(effect: &Value) -> Result<Self, String> {
        let operation = effect["type"]
            .as_str()
            .ok_or("invalid connectivity operation")?;
        if !matches!(
            operation,
            "wifi.setEnabled"
                | "wifi.connect"
                | "bluetooth.setPowered"
                | "bluetooth.setDiscovery"
                | "bluetooth.connect"
                | "bluetooth.disconnect"
                | "bluetooth.pair"
        ) {
            return Err("unsupported connectivity operation".into());
        }
        let revision = effect["revision"]
            .as_str()
            .filter(|value| value.len() == 16)
            .ok_or("connectivity snapshot is unavailable")?;
        let needs_id = matches!(
            operation,
            "wifi.connect" | "bluetooth.connect" | "bluetooth.disconnect" | "bluetooth.pair"
        );
        let id = if needs_id {
            Some(
                effect["id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 512)
                    .ok_or("invalid connectivity identity")?
                    .to_owned(),
            )
        } else {
            None
        };
        let value = if needs_id {
            None
        } else {
            Some(
                effect["value"]
                    .as_bool()
                    .ok_or("invalid connectivity value")?,
            )
        };
        Ok(Self {
            operation: operation.into(),
            revision: revision.into(),
            id,
            value,
        })
    }

    pub(crate) fn capability(&self) -> PluginCapability {
        if self.operation.starts_with("wifi.") {
            PluginCapability::NetworkControl
        } else {
            PluginCapability::BluetoothControl
        }
    }

    pub(crate) fn validate(&self, snapshot: &Value) -> Result<(), String> {
        if snapshot["revision"].as_str() != Some(self.revision.as_str()) {
            return Err("connectivity snapshot is stale".into());
        }
        let operation = self.operation.split_once('.').unwrap().1;
        if snapshot["operations"][operation] != true {
            return Err("connectivity operation is unavailable".into());
        }
        if let Some(id) = &self.id {
            let wifi = self.operation.starts_with("wifi.");
            let item = snapshot[if wifi { "networks" } else { "devices" }]
                .as_array()
                .and_then(|items| items.iter().find(|item| item["id"].as_str() == Some(id)))
                .ok_or("connectivity identity is stale")?;
            if wifi {
                if item["canConnect"] != true {
                    return Err("Wi-Fi profile cannot be connected".into());
                }
            } else {
                if snapshot["powered"] != true {
                    return Err("Bluetooth is powered off".into());
                }
                if operation == "pair" && item["paired"] == true {
                    return Err("Bluetooth device is already paired".into());
                }
                if matches!(operation, "connect" | "disconnect")
                    && (item["paired"] != true
                        || (item["connected"] == true) != (operation == "disconnect"))
                {
                    return Err("Bluetooth connection state changed".into());
                }
            }
        }
        if operation == "setDiscovery" && snapshot["powered"] != true {
            return Err("Bluetooth is powered off".into());
        }
        Ok(())
    }

    pub(crate) fn resource(&self) -> &'static str {
        if self.operation.starts_with("wifi.") {
            "wifi"
        } else {
            "bluetooth"
        }
    }

    pub(crate) fn execute(&self) -> Result<bool, String> {
        let snapshot = if self.resource() == "wifi" {
            wifi_snapshot(&platform::network_status())
        } else {
            bluetooth_snapshot(&platform::bluetooth_status())
        };
        self.validate(&snapshot)?;
        let id = self.id.as_deref().unwrap_or_default();
        Ok(match self.operation.as_str() {
            "wifi.setEnabled" => platform::set_wifi_enabled(self.value.unwrap()),
            "wifi.connect" => platform::activate_wifi_network(id),
            "bluetooth.setPowered" => platform::set_bluetooth_powered(self.value.unwrap()),
            "bluetooth.setDiscovery" => platform::set_bluetooth_discovery(self.value.unwrap()),
            "bluetooth.connect" => platform::set_bluetooth_connected(id, true),
            "bluetooth.disconnect" => platform::set_bluetooth_connected(id, false),
            "bluetooth.pair" => platform::toggle_bluetooth_device(id),
            _ => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::WifiNetworkStatus;

    #[test]
    fn wifi_uses_profile_identity_and_rejects_stale_unsaved_and_unavailable_effects() {
        let mut status = NetworkStatus {
            available: true,
            enabled: true,
            networks: vec![WifiNetworkStatus {
                id: "profile-stable".into(),
                name: "same ssid".into(),
                saved: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let snapshot = wifi_snapshot(&status);
        let effect = ConnectivityEffect::parse(&json!({"type": "wifi.connect", "id": "profile-stable", "revision": snapshot["revision"]})).unwrap();
        assert_eq!(effect.capability(), PluginCapability::NetworkControl);
        assert!(effect.validate(&snapshot).is_ok());
        status.networks[0].saved = false;
        assert!(
            effect
                .validate(&wifi_snapshot(&status))
                .unwrap_err()
                .contains("stale")
        );
        let mut unsaved = snapshot.clone();
        unsaved["networks"][0]["canConnect"] = false.into();
        assert!(effect.validate(&unsaved).is_err());
        let mut unavailable = snapshot;
        unavailable["operations"]["connect"] = false.into();
        assert!(effect.validate(&unavailable).is_err());
    }

    #[test]
    fn bluetooth_reports_platform_operation_support_and_preserves_device_identity() {
        let status = BluetoothStatus {
            available: true,
            powered: true,
            devices: vec![crate::platform::BluetoothDeviceStatus {
                id: "/stable/device".into(),
                name: "Headset".into(),
                paired: true,
                connected: false,
            }],
            ..Default::default()
        };
        let snapshot = bluetooth_snapshot(&status);
        assert_eq!(snapshot["devices"][0]["id"], "/stable/device");
        assert_eq!(snapshot["operations"]["connect"], cfg!(target_os = "linux"));
        assert_eq!(snapshot["operations"]["pair"], cfg!(target_os = "windows"));
        let effect = ConnectivityEffect::parse(&json!({"type": "bluetooth.connect", "id": "/stable/device", "revision": snapshot["revision"]})).unwrap();
        assert_eq!(
            effect.validate(&snapshot).is_ok(),
            cfg!(target_os = "linux")
        );
        let mut changed = snapshot;
        changed["devices"][0]["connected"] = true.into();
        assert!(effect.validate(&changed).is_err());
    }
}
