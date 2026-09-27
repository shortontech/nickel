//! Bluetooth JSX page. Adapter and device operations remain in the Settings host.

use nickel_ui::{AnyView, Column, Insets, SemanticTheme, VerticalScroll};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    BluetoothOperation, SettingsApp, SettingsMessage, SettingsPage,
    settings_components::{Node, SettingsJsxContext},
};

const STALE_STATUS: &str = "Bluetooth status changed; refresh the page";

pub(super) struct BluetoothPage {
    context: SettingsJsxContext,
}

impl BluetoothPage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            context: SettingsJsxContext::new(
                crate::settings_package::source(crate::settings_package::Script::Bluetooth)?,
                parse_tree,
                STALE_STATUS,
                "Bluetooth action must request one operation",
            )?,
        })
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
        pairing: bool,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let node = self.context.render(data)?;
        Ok(AnyView::new(
            Column::new()
                .grow(1.0)
                .padding(if pairing {
                    Insets::all(0.0)
                } else {
                    Insets {
                        top: 20.0,
                        right: 40.0,
                        bottom: 20.0,
                        left: 20.0,
                    }
                })
                .child(
                    VerticalScroll::new(SettingsMessage::BluetoothScroll, 0.0)
                        .grow(1.0)
                        .theme(theme)
                        .child(node.view(theme, "", SettingsMessage::BluetoothJsxAction)),
                ),
        ))
    }

    fn dispatch(&mut self, index: usize, data: &Value) -> Result<SettingsMessage, String> {
        self.context.dispatch(index, &Value::Null, data, |effect| {
            let request: BluetoothRequest =
                serde_json::from_value(effect.clone()).map_err(|error| error.to_string())?;
            validate_request(request, data)
        })
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.context.action_for_id(id)
    }
}

fn parse_tree(value: &Value) -> Result<Node, String> {
    if value.get("kind").and_then(Value::as_str) != Some("settings-stack") {
        return Err("Bluetooth page root is invalid".into());
    }
    let node = Node::parse(value)?;
    if node.contains_input() {
        return Err("Bluetooth page cannot request text input".into());
    }
    Ok(node)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum BluetoothRequest {
    Power { enabled: bool },
    Discovery,
    OpenPairing,
    Device { index: usize, id: String },
}

fn validate_request(request: BluetoothRequest, data: &Value) -> Result<SettingsMessage, String> {
    match request {
        BluetoothRequest::Power { enabled } => {
            if data.get("pairing").and_then(Value::as_bool) != Some(false)
                || data.get("powerEditable").and_then(Value::as_bool) != Some(true)
                || data.get("powered").and_then(Value::as_bool) == Some(enabled)
            {
                return Err("Bluetooth power is unavailable".into());
            }
            Ok(SettingsMessage::SetBluetoothPower(enabled))
        }
        BluetoothRequest::Discovery => {
            if data.get("pairing").and_then(Value::as_bool) != Some(true)
                || data.get("discoveryEditable").and_then(Value::as_bool) != Some(true)
            {
                return Err("Bluetooth discovery is unavailable".into());
            }
            Ok(SettingsMessage::BluetoothDiscovery)
        }
        BluetoothRequest::OpenPairing => {
            if data.get("pairing").and_then(Value::as_bool) != Some(false)
                || data.get("discoveryEditable").and_then(Value::as_bool) != Some(true)
            {
                return Err("Bluetooth pairing is unavailable".into());
            }
            Ok(SettingsMessage::OpenBluetoothPairing)
        }
        BluetoothRequest::Device { index, id } => {
            if data.get("deviceEditable").and_then(Value::as_bool) != Some(true)
                || data
                    .get("devices")
                    .and_then(Value::as_array)
                    .and_then(|devices| {
                        devices.iter().find(|device| {
                            device.get("index").and_then(Value::as_u64) == Some(index as u64)
                        })
                    })
                    .and_then(|device| device.get("id"))
                    .and_then(Value::as_str)
                    != Some(id.as_str())
            {
                return Err("Bluetooth device changed".into());
            }
            Ok(SettingsMessage::BluetoothDevice(index))
        }
    }
}

pub(super) fn projection(app: &SettingsApp) -> Value {
    let pairing = app.page == SettingsPage::BluetoothPair;
    let pending = app.bluetooth_operation_rx.is_some();
    let ready = app.bluetooth.available && app.bluetooth.powered && !pending;
    let status = if let Some(operation) = &app.bluetooth_operation {
        match operation {
            BluetoothOperation::SetPower(true) => {
                app.localizer.text("settings-bluetooth-powering-on")
            }
            BluetoothOperation::SetPower(false) => {
                app.localizer.text("settings-bluetooth-powering-off")
            }
            BluetoothOperation::SetDiscovery(true) => {
                app.localizer.text("settings-bluetooth-discovery-starting")
            }
            BluetoothOperation::SetDiscovery(false) => {
                app.localizer.text("settings-bluetooth-discovery-stopping")
            }
            BluetoothOperation::ToggleDevice(id) => {
                let name = app
                    .bluetooth
                    .devices
                    .iter()
                    .find(|device| device.id == *id)
                    .map(|device| device.name.as_str())
                    .unwrap_or("device");
                app.localizer
                    .value("settings-bluetooth-device-updating", "device", name)
            }
        }
    } else if let Some(error) = &app.bluetooth_status {
        error.clone()
    } else if !app.bluetooth.available {
        app.localizer.text("settings-bluetooth-service-unavailable")
    } else if app.bluetooth.powered {
        app.localizer.text("settings-bluetooth-on")
    } else {
        app.localizer.text("settings-bluetooth-off")
    };
    let devices = app
        .bluetooth
        .devices
        .iter()
        .enumerate()
        .filter(|(_, device)| {
            if pairing {
                !device.paired && !device.connected
            } else {
                device.paired || device.connected
            }
        })
        .map(|(index, device)| {
            let state = if device.connected {
                app.localizer.text("settings-bluetooth-connected")
            } else if device.paired {
                app.localizer.text("settings-bluetooth-paired")
            } else {
                app.localizer.text("settings-bluetooth-available")
            };
            let detail = if pairing {
                [
                    device.kind.as_deref().and_then(kind_icon),
                    device.signal_dbm.map(|signal| format!("{signal} dBm")),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" · ")
            } else {
                device
                    .battery_percent
                    .map(|percent| format!("{percent}%"))
                    .unwrap_or_default()
            };
            let action = if device.connected {
                "settings-bluetooth-disconnect"
            } else if !device.paired {
                "settings-bluetooth-pair"
            } else {
                "settings-bluetooth-connect"
            };
            json!({"index":index,"id":device.id,"name":device.name,
            "detail":if detail.is_empty() { state } else { format!("{state} · {detail}") },
            "actionLabel":app.localizer.text(action)})
        })
        .collect::<Vec<_>>();
    let discovery_label = if pairing && app.bluetooth.discovering {
        "settings-bluetooth-discovery-stop"
    } else if pairing {
        "settings-bluetooth-discovery-start"
    } else {
        "settings-bluetooth-pair-devices"
    };
    json!({
        "pairing":pairing, "powered":app.bluetooth.powered,
        "powerLabel":app.localizer.text("settings-bluetooth-enabled"),
        "adapterName":if app.bluetooth.adapter_name.is_empty() { app.localizer.text("settings-bluetooth-adapter-unnamed") } else { app.bluetooth.adapter_name.clone() },
        "powerEditable":app.bluetooth.available && !pending,
        "switchState":match (app.bluetooth.powered,app.bluetooth.available && !pending) { (true,true)=>"on",(false,true)=>"off",(true,false)=>"disabled-on",(false,false)=>"disabled-off" },
        "statusLabel":app.localizer.text("settings-bluetooth-status"), "status":status,
        "devicesLabel":app.localizer.text(if pairing { "settings-bluetooth-nearby-devices" } else { "settings-bluetooth-devices" }),
        "discoveryLabel":app.localizer.text(discovery_label),
        "discoveryEditable":ready,"deviceEditable":ready,
        "emptyLabel":app.localizer.text(if app.bluetooth.available { "settings-bluetooth-no-devices" } else { "settings-bluetooth-service-unavailable" }),
        "devices":devices,
    })
}

fn kind_icon(kind: &str) -> Option<String> {
    Some(
        match kind {
            "audio-card" | "audio-headphones" | "audio-headset" => "🎧",
            "input-keyboard" => "⌨",
            "input-mouse" => "🖱",
            "input-gaming" => "🎮",
            "phone" => "📱",
            "computer" => "💻",
            _ => return None,
        }
        .to_owned(),
    )
}

impl SettingsApp {
    pub(super) fn handle_bluetooth_jsx_action(&mut self, index: usize) {
        if !matches!(
            self.page,
            SettingsPage::Bluetooth | SettingsPage::BluetoothPair
        ) {
            return;
        }
        let data = projection(self);
        let result = self
            .bluetooth_page
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "Bluetooth page is not loaded".to_owned())
            .and_then(|page| page.as_mut().map_err(|error| error.clone()))
            .and_then(|page| page.dispatch(index, &data));
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => {
                if error != STALE_STATUS {
                    *self.bluetooth_page.borrow_mut() = Some(Err(error));
                }
                self.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::BluetoothDevice;

    #[test]
    fn bluetooth_jsx_actions_require_current_adapter_and_device_identity() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Bluetooth);
        app.bluetooth.available = true;
        app.bluetooth.powered = true;
        app.bluetooth.devices.push(BluetoothDevice {
            id: "device-1".into(),
            name: "Headphones".into(),
            paired: true,
            ..BluetoothDevice::default()
        });
        let data = projection(&app);
        let mut page = BluetoothPage::new().unwrap();
        page.render(&data, app.ui_theme(), false).unwrap();
        let power = page.action_for_id("bluetooth-power").unwrap();
        let device = page.action_for_id("bluetooth-device-0-action").unwrap();
        assert_eq!(
            page.dispatch(power, &data).unwrap(),
            SettingsMessage::SetBluetoothPower(false)
        );
        assert_eq!(
            page.dispatch(device, &data).unwrap(),
            SettingsMessage::BluetoothDevice(0)
        );

        app.bluetooth.devices[0].id = "device-2".into();
        let changed = projection(&app);
        assert_eq!(page.dispatch(device, &changed).unwrap_err(), STALE_STATUS);
        assert!(
            validate_request(
                BluetoothRequest::Device {
                    index: 0,
                    id: "device-1".into()
                },
                &changed
            )
            .is_err()
        );

        app.bluetooth.available = false;
        let unavailable = projection(&app);
        assert!(
            validate_request(BluetoothRequest::Power { enabled: false }, &unavailable).is_err()
        );
    }

    #[test]
    fn pairing_jsx_only_exposes_unpaired_device_and_discovery_actions() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::BluetoothPair);
        app.bluetooth.available = true;
        app.bluetooth.powered = true;
        app.bluetooth.devices.push(BluetoothDevice {
            id: "paired".into(),
            paired: true,
            ..BluetoothDevice::default()
        });
        app.bluetooth.devices.push(BluetoothDevice {
            id: "new".into(),
            name: "Keyboard".into(),
            ..BluetoothDevice::default()
        });
        let data = projection(&app);
        let mut page = BluetoothPage::new().unwrap();
        page.render(&data, app.ui_theme(), true).unwrap();
        assert!(page.action_for_id("bluetooth-power").is_none());
        assert!(page.action_for_id("bluetooth-device-0-action").is_none());
        let discovery = page.action_for_id("bluetooth-discovery-action").unwrap();
        assert_eq!(
            page.dispatch(discovery, &data).unwrap(),
            SettingsMessage::BluetoothDiscovery
        );
        assert!(validate_request(BluetoothRequest::Power { enabled: false }, &data).is_err());
    }
}
