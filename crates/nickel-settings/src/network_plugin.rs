//! Network page JSX adapter. The platform operations remain in Settings.

use nickel_ui::{AnyView, Column, Insets, SemanticTheme, VerticalScroll};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    SettingsApp, SettingsMessage, SettingsPage,
    settings_components::{Node, SettingsJsxContext},
};

const STALE_STATUS: &str = "Network status changed; refresh the page";

pub(super) struct NetworkPage {
    context: SettingsJsxContext,
}

impl NetworkPage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            context: SettingsJsxContext::new(
                crate::settings_package::source(crate::settings_package::Script::Network)?,
                parse_tree,
                STALE_STATUS,
                "Network action must request one operation",
            )?,
        })
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let node = self.context.render(data)?;
        Ok(AnyView::new(
            Column::new()
                .grow(1.0)
                .padding(Insets {
                    top: 20.0,
                    right: 40.0,
                    bottom: 20.0,
                    left: 20.0,
                })
                .child(
                    VerticalScroll::new(SettingsMessage::NetworkScroll, 0.0)
                        .grow(1.0)
                        .theme(theme)
                        .child(node.view(theme, "", SettingsMessage::NetworkJsxAction)),
                ),
        ))
    }

    fn dispatch(&mut self, index: usize, data: &Value) -> Result<SettingsMessage, String> {
        self.context.dispatch(index, &Value::Null, data, |effect| {
            let request: NetworkRequest =
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
    let Some(children) = value.get("children").and_then(Value::as_array) else {
        return Err("Network page has no components".into());
    };
    if value.get("kind").and_then(Value::as_str) != Some("settings-stack")
        || children.len() != 3
        || children[0].get("kind").and_then(Value::as_str) != Some("settings-row")
        || children[1..]
            .iter()
            .any(|child| child.get("kind").and_then(Value::as_str) != Some("settings-card"))
    {
        return Err("Network page structure is invalid".into());
    }
    let node = Node::parse(value)?;
    if node.contains_input() {
        return Err("Network page cannot request text input".into());
    }
    Ok(node)
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum NetworkRequest {
    WifiPower { enabled: bool },
    WifiNetwork { index: usize, profile: String },
}

fn validate_request(request: NetworkRequest, data: &Value) -> Result<SettingsMessage, String> {
    match request {
        NetworkRequest::WifiPower { enabled } => {
            if data.get("powerEditable").and_then(Value::as_bool) != Some(true)
                || data.get("wifiEnabled").and_then(Value::as_bool) == Some(enabled)
            {
                return Err("Wi-Fi power is unavailable".into());
            }
            Ok(SettingsMessage::SetWifiPower(enabled))
        }
        NetworkRequest::WifiNetwork { index, profile } => {
            if data.get("networkPending").and_then(Value::as_bool) != Some(false)
                || data
                    .get("networks")
                    .and_then(Value::as_array)
                    .and_then(|networks| networks.get(index))
                    .and_then(|network| network.get("profile"))
                    .and_then(Value::as_str)
                    != Some(profile.as_str())
            {
                return Err("Wi-Fi network changed".into());
            }
            Ok(SettingsMessage::WifiNetwork(index))
        }
    }
}

pub(super) fn projection(app: &SettingsApp) -> Value {
    let power_available =
        app.network_available && cfg!(any(target_os = "linux", target_os = "windows"));
    let power_editable = power_available && app.wifi_power_rx.is_none();
    let switch_state = match (app.wifi_enabled, power_editable) {
        (true, true) => "on",
        (false, true) => "off",
        (true, false) => "disabled-on",
        (false, false) => "disabled-off",
    };
    let networks = app.wifi_networks.iter().enumerate().map(|(index, network)| {
        let detail = if network.connected {
            app.localizer.number("settings-network-connected-signal", "signal", i64::from(network.signal))
        } else if !network.saved {
            app.localizer.number(if network.secure { "settings-network-secured-signal" } else { "settings-network-open-signal" }, "signal", i64::from(network.signal))
        } else {
            app.localizer.number("settings-network-connect-action", "signal", i64::from(network.signal))
        };
        json!({
            "index":index, "profile":network.profile, "detail":detail,
            "connected":network.connected,
            "actionLabel":if network.connected { app.localizer.text("settings-network-connected") } else { app.localizer.text("settings-network-connect") },
        })
    }).collect::<Vec<_>>();
    let adapters = app
        .network_adapters
        .iter()
        .map(|adapter| {
            let status = if adapter.connected {
                if adapter.speed > 0 {
                    app.localizer.number(
                        "settings-network-connected-speed",
                        "speed",
                        (adapter.speed / 1_000_000) as i64,
                    )
                } else {
                    app.localizer.text("settings-network-connected")
                }
            } else {
                app.localizer.text("settings-network-disconnected")
            };
            json!({"name":adapter.name,"status":format!("{status} · {}",adapter.description)})
        })
        .collect::<Vec<_>>();
    json!({
        "wifiLabel":app.localizer.text("settings-network-wifi"),
        "wifiStatus":app.wifi_status,
        "wifiEnabled":app.wifi_enabled,
        "powerEditable":power_editable,
        "switchState":switch_state,
        "visibleWifi":app.localizer.text("settings-network-visible-wifi"),
        "networkPending":app.pending_wifi_profile.is_some(),
        "networks":networks,
        "adaptersLabel":app.localizer.text("settings-network-adapters"),
        "noAdapters":app.localizer.text("settings-network-no-adapters"),
        "adapters":adapters,
    })
}

impl SettingsApp {
    pub(super) fn handle_network_jsx_action(&mut self, index: usize) {
        if self.page != SettingsPage::Network {
            return;
        }
        let data = projection(self);
        let result = self
            .network_page
            .borrow_mut()
            .as_mut()
            .ok_or_else(|| "Network page is not loaded".to_owned())
            .and_then(|page| page.as_mut().map_err(|error| error.clone()))
            .and_then(|page| page.dispatch(index, &data));
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => {
                if error != STALE_STATUS {
                    *self.network_page.borrow_mut() = Some(Err(error));
                }
                self.request_redraw();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wifi_power_request_uses_the_confirmed_state_and_current_availability() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Network);
        app.network_available = true;
        app.wifi_enabled = false;
        let data = projection(&app);
        let mut page = NetworkPage::new().unwrap();
        page.render(&data, app.ui_theme()).unwrap();
        let action = page.action_for_id("network-wifi-power").unwrap();
        assert_eq!(
            page.dispatch(action, &data).unwrap(),
            SettingsMessage::SetWifiPower(true)
        );
        app.network_available = false;
        let changed = projection(&app);
        assert_eq!(page.dispatch(action, &changed).unwrap_err(), STALE_STATUS);
        assert!(validate_request(NetworkRequest::WifiPower { enabled: true }, &changed).is_err());
    }

    #[test]
    fn stale_network_identity_cannot_connect() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Network);
        app.wifi_networks.push(crate::WifiNetwork {
            #[cfg(target_os = "linux")]
            id: "example".into(),
            profile: "Example".into(),
            signal: 75,
            secure: true,
            saved: true,
            connected: false,
            #[cfg(target_os = "windows")]
            interface: 0,
        });
        let data = projection(&app);
        let mut page = NetworkPage::new().unwrap();
        page.render(&data, app.ui_theme()).unwrap();
        let action = page.action_for_id("wifi-network-0").unwrap();
        let message = page.dispatch(action, &data).unwrap();
        assert_eq!(message, SettingsMessage::WifiNetwork(0));
        app.wifi_networks[0].profile = "Different".into();
        assert_eq!(
            page.dispatch(action, &projection(&app)).unwrap_err(),
            STALE_STATUS
        );
        assert!(
            validate_request(
                NetworkRequest::WifiNetwork {
                    index: 0,
                    profile: "Example".into()
                },
                &projection(&app)
            )
            .is_err()
        );
    }
}
