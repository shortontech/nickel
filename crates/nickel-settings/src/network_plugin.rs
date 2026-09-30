//! Network page JSX through the shared native component renderer.

use nickel_plugin_presentation::{
    components::PluginImages,
    page::{JsxPage, STALE_DATA},
};
use nickel_ui::{AnyView, Column, SemanticTheme, VerticalScroll};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::settings_plugin::StyledSettingsPage;
use crate::{SettingsApp, SettingsMessage, SettingsPage};

const STALE_STATUS: &str = STALE_DATA;

pub(super) struct NetworkPage {
    page: StyledSettingsPage,
}

impl NetworkPage {
    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes()
    }

    #[cfg(test)]
    pub(super) fn new() -> Result<Self, String> {
        Self::new_with_page(JsxPage::new(
            crate::settings_package::source(crate::settings_package::Script::Network)?,
            crate::settings_package::manifest()?.clone(),
            None,
        )?)
    }

    pub(super) fn new_with_page(page: JsxPage) -> Result<Self, String> {
        Ok(Self {
            page: StyledSettingsPage::new(page),
        })
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let (node, stylesheet) = self.page.render(
            data,
            theme,
            include_str!("../../../assets/plugins/settings/settings-network.css"),
        )?;
        Ok(AnyView::new(
            Column::new().grow(1.0).child(
                VerticalScroll::new(SettingsMessage::NetworkScroll, 0.0)
                    .grow(1.0)
                    .theme(theme)
                    .child(node.view_as::<SettingsMessage>(&PluginImages::new(), stylesheet)),
            ),
        ))
    }

    fn dispatch(&mut self, index: usize, data: &Value) -> Result<SettingsMessage, String> {
        self.page.dispatch(index, &Value::Null, data, |effect| {
            let request: NetworkRequest =
                serde_json::from_value(effect).map_err(|error| error.to_string())?;
            validate_request(request, data)
        })
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.page.node()?.button_action(id)
    }
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
    fn shared_network_page_renders_with_connections_and_adapters() {
        fn save(host: &nickel_ui::UiHost<SettingsApp>, name: &str) {
            const WIDTH: u32 = 850;
            const HEIGHT: u32 = 580;
            let mut renderer = nickel_ui::SoftwareRenderer::new_pixel_buffer(WIDTH, HEIGHT, 1.0);
            host.render_software(&mut renderer);
            let image =
                image::ImageBuffer::<image::Rgba<u8>, Vec<u8>>::from_fn(WIDTH, HEIGHT, |x, y| {
                    let pixel = renderer.pixels()[(y * WIDTH + x) as usize];
                    image::Rgba([pixel.r, pixel.g, pixel.b, pixel.a])
                });
            let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/nickel-ui-snapshots")
                .join(name);
            std::fs::create_dir_all(output.parent().unwrap()).unwrap();
            image.save(output).unwrap();
        }

        fn fixture() -> SettingsApp {
            let mut app = SettingsApp::with_initial_page(SettingsPage::Network);
            app.network_available = true;
            app.wifi_enabled = true;
            app.wifi_status = "Wi-Fi is on".into();
            app.wifi_networks = vec![
                crate::WifiNetwork {
                    #[cfg(target_os = "linux")]
                    id: "connected".into(),
                    profile: "Nickel Home".into(),
                    signal: 91,
                    connected: true,
                    saved: true,
                    secure: true,
                    #[cfg(target_os = "windows")]
                    interface: 0,
                },
                crate::WifiNetwork {
                    #[cfg(target_os = "linux")]
                    id: "available".into(),
                    profile: "Studio Guest".into(),
                    signal: 68,
                    connected: false,
                    saved: false,
                    secure: false,
                    #[cfg(target_os = "windows")]
                    interface: 0,
                },
            ];
            app.network_adapters.push(crate::NetworkAdapter {
                name: "Ethernet".into(),
                description: "Wired connection".into(),
                connected: true,
                speed: 1_000_000_000,
            });
            app
        }
        let jsx = nickel_ui::UiHost::new(fixture(), 850, 580);
        save(&jsx, "settings-network-shared.png");
    }

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
