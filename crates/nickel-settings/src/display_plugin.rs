//! Selected-display actions supplied by the bundled Settings JSX package.
//! Topology, mode validation, drag geometry, and timed revert stay in Settings.

use nickel_plugin_presentation::{
    components::{PanelNode, PluginImages},
    css::StyleSheet,
    page::JsxPage,
};
use nickel_ui::{AnyView, SemanticTheme};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage, SettingsPage};

const STALE_STATUS: &str = "Display selection changed; refresh the page";

pub(super) struct DisplayPage {
    page: JsxPage,
    stylesheet: StyleSheet,
    last_theme: Option<SemanticTheme>,
}

#[derive(Clone)]
pub(super) struct DisplayCardView {
    pub(super) index: usize,
    pub(super) name: String,
    pub(super) detail: String,
    pub(super) primary_label: String,
}

impl DisplayPage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            page: JsxPage::new(
                crate::settings_package::source(crate::settings_package::Script::Display)?,
                crate::settings_package::manifest()?.clone(),
                None,
            )?,
            stylesheet: StyleSheet::default(),
            last_theme: None,
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes() + self.stylesheet.estimated_retained_bytes() as usize
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<(Vec<DisplayCardView>, [AnyView<SettingsMessage>; 7]), String> {
        self.page.render(data)?;
        if self.last_theme != Some(theme) {
            self.stylesheet = crate::settings_plugin::stylesheet_template(
                include_str!("../../../assets/plugins/settings/settings-display.css"),
                theme,
            )?;
            self.last_theme = Some(theme);
        }
        let Some(root @ PanelNode::Div { .. }) = self.page.node() else {
            return Err("Display actions have an invalid root".into());
        };
        let PanelNode::Div {
            children: card_nodes,
            ..
        } = root
            .direct_child_with_class("display-cards")
            .ok_or("Display arrangement is unavailable")?
        else {
            return Err("Display arrangement is invalid".into());
        };
        let projected = data["cards"]
            .as_array()
            .ok_or("Display arrangement projection is invalid")?;
        if card_nodes.len() != projected.len() || card_nodes.len() > 16 {
            return Err("Display arrangement changed".into());
        }
        let mut cards = Vec::with_capacity(card_nodes.len());
        for node in card_nodes {
            let PanelNode::Div { children, .. } = node else {
                return Err("Display card is invalid".into());
            };
            let Some(PanelNode::Button { id, label, .. }) = children.last() else {
                return Err("Display card action is invalid".into());
            };
            let [
                PanelNode::Text { value: name, .. },
                PanelNode::Text { value: detail, .. },
                _,
            ] = children.as_slice()
            else {
                return Err("Display card labels are invalid".into());
            };
            let index = id
                .strip_prefix("display-card-")
                .and_then(|index| index.parse::<usize>().ok())
                .ok_or("Display card ID is invalid")?;
            let Some(card) = projected
                .iter()
                .find(|card| card["index"].as_u64() == Some(index as u64))
            else {
                return Err("Display card identity changed".into());
            };
            if cards
                .iter()
                .any(|card: &DisplayCardView| card.index == index)
            {
                return Err("Display card identity changed".into());
            }
            cards.push(DisplayCardView {
                index,
                name: name.clone(),
                detail: detail.clone(),
                primary_label: if card["primary"] == true {
                    label.clone()
                } else {
                    String::new()
                },
            });
        }
        let images = PluginImages::new();
        let control_nodes = [
            "display-enabled-row",
            "display-resolution",
            "display-refresh",
            "display-scale",
            "display-actions",
            "display-confirmation",
            "display-application-scale",
        ]
        .map(|class| {
            root.direct_child_with_class(class)
                .ok_or_else(|| format!("Display section {class:?} is unavailable"))
        })
        .into_iter()
        .collect::<Result<Vec<_>, _>>()?;
        let controls = std::array::from_fn(|index| {
            control_nodes[index].view_as_scoped::<SettingsMessage>(
                &images,
                &self.stylesheet,
                Some("display"),
            )
        });
        Ok((cards, controls))
    }

    fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        data: &Value,
    ) -> Result<SettingsMessage, String> {
        self.page.dispatch(index, &value, data, |effect| {
            let request: DisplayRequest =
                serde_json::from_value(effect).map_err(|error| error.to_string())?;
            validate_request(request, data)
        })
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.page
            .node()
            .and_then(|node| node.button_action(id).or_else(|| node.slider_action(id)))
    }

    pub(super) fn card_action(&self, index: usize) -> Option<usize> {
        self.page
            .node()
            .and_then(|node| node.button_action(&format!("display-card-{index}")))
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum DisplayRequest {
    SelectDisplay {
        index: usize,
        connector: String,
    },
    Enabled {
        connector: String,
        enabled: bool,
    },
    ToggleResolution {
        connector: String,
    },
    Resolution {
        connector: String,
        width: i32,
        height: i32,
    },
    ToggleRefresh {
        connector: String,
    },
    Refresh {
        connector: String,
        refresh: i32,
    },
    Scale {
        connector: String,
        fraction: f32,
    },
    ApplicationScaleValue {
        connector: String,
        fraction: f32,
    },
    Identify {
        connector: String,
    },
    Primary {
        connector: String,
    },
    Apply {
        connector: String,
    },
    Keep {
        connector: String,
    },
    Revert {
        connector: String,
    },
}

fn validate_request(request: DisplayRequest, data: &Value) -> Result<SettingsMessage, String> {
    let (connector, message) = match request {
        DisplayRequest::SelectDisplay { index, connector } => {
            if !data["cards"].as_array().is_some_and(|cards| {
                cards.iter().any(|card| {
                    card["index"].as_u64() == Some(index as u64)
                        && card["connector"].as_str() == Some(connector.as_str())
                })
            }) {
                return Err(STALE_STATUS.into());
            }
            return Ok(SettingsMessage::SelectDisplay(index));
        }
        DisplayRequest::Enabled { connector, enabled } => {
            if data["enabled"].as_bool() == Some(enabled) {
                return Err(STALE_STATUS.into());
            }
            (connector, SettingsMessage::DisplayEnabled(enabled))
        }
        DisplayRequest::ToggleResolution { connector } => {
            (connector, SettingsMessage::ToggleDisplayResolutionSelect)
        }
        DisplayRequest::Resolution {
            connector,
            width,
            height,
        } => {
            let supported = data["resolutions"].as_array().is_some_and(|options| {
                options.iter().any(|option| {
                    option["width"].as_i64() == Some(i64::from(width))
                        && option["height"].as_i64() == Some(i64::from(height))
                })
            });
            if !supported {
                return Err(STALE_STATUS.into());
            }
            (
                connector,
                SettingsMessage::SetDisplayResolution { width, height },
            )
        }
        DisplayRequest::ToggleRefresh { connector } => {
            (connector, SettingsMessage::ToggleDisplayRefreshSelect)
        }
        DisplayRequest::Refresh { connector, refresh } => {
            let supported = data["refreshRates"].as_array().is_some_and(|options| {
                options
                    .iter()
                    .any(|option| option["refresh"].as_i64() == Some(i64::from(refresh)))
            });
            if !supported {
                return Err(STALE_STATUS.into());
            }
            (connector, SettingsMessage::SetDisplayRefresh(refresh))
        }
        DisplayRequest::Scale {
            connector,
            fraction,
        } => {
            if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
                return Err(STALE_STATUS.into());
            }
            (connector, crate::display_scale_message(fraction))
        }
        DisplayRequest::ApplicationScaleValue {
            connector,
            fraction,
        } => {
            if !fraction.is_finite() || !(0.0..=1.0).contains(&fraction) {
                return Err(STALE_STATUS.into());
            }
            (connector, crate::application_scale_message(fraction))
        }
        DisplayRequest::Identify { connector } => (connector, SettingsMessage::DisplayIdentify),
        DisplayRequest::Primary { connector } => (connector, SettingsMessage::DisplayPrimary),
        DisplayRequest::Apply { connector } => (connector, SettingsMessage::DisplayApply),
        DisplayRequest::Keep { connector } => {
            if data["pendingRevert"] != true {
                return Err(STALE_STATUS.into());
            }
            (connector, SettingsMessage::DisplayKeep)
        }
        DisplayRequest::Revert { connector } => {
            if data["pendingRevert"] != true {
                return Err(STALE_STATUS.into());
            }
            (connector, SettingsMessage::DisplayRevert)
        }
    };
    if data["connector"].as_str() != Some(connector.as_str()) {
        return Err(STALE_STATUS.into());
    }
    Ok(message)
}

pub(super) fn projection(app: &SettingsApp) -> Value {
    let selected = &app.displays[app.selected];
    let mut display_order = (0..app.displays.len()).collect::<Vec<_>>();
    display_order.sort_by_key(|index| (*index == app.selected) as u8);
    let cards = display_order
        .into_iter()
        .map(|index| {
            let display = &app.displays[index];
            json!({
                "index": index,
                "connector": display.connector,
                "name": display.name,
                "detail": display.detail,
                "enabled": display.enabled,
                "primary": display.primary,
            })
        })
        .collect::<Vec<_>>();
    let mut resolutions = selected
        .modes
        .iter()
        .map(|mode| (mode.width, mode.height))
        .collect::<Vec<_>>();
    resolutions.sort_unstable_by(|left, right| right.cmp(left));
    resolutions.dedup();
    let mut refresh_rates = selected
        .modes
        .iter()
        .filter(|mode| mode.width == selected.mode.width && mode.height == selected.mode.height)
        .map(|mode| mode.refresh_millihz)
        .collect::<Vec<_>>();
    refresh_rates.sort_unstable_by(|left, right| right.cmp(left));
    refresh_rates.dedup();
    let custom_scale_units = match app.application_scale_policy {
        crate::ApplicationScalePolicy::FollowNickel | crate::ApplicationScalePolicy::Unchanged => {
            120
        }
        crate::ApplicationScalePolicy::Custom(scale) => scale.units(),
    };
    json!({
        "connector": selected.connector,
        "cards": cards,
        "disabledLabel": "DISABLED",
        "cardPrimaryLabel": "PRIMARY",
        "enabled": selected.enabled,
        "pendingRevert": app.pending_display_revert.is_some(),
        "resolutionLabel": "Resolution",
        "resolutionValue": format!("{} × {}", selected.mode.width, selected.mode.height),
        "resolutionOpen": app.display_resolution_select_expanded,
        "resolutions": resolutions.into_iter().map(|(width, height)| json!({
            "width": width, "height": height, "label": format!("{width} × {height}")
        })).collect::<Vec<_>>(),
        "refreshLabel": "Refresh rate",
        "refreshValue": format!("{:.2} Hz", f64::from(selected.mode.refresh_millihz) / 1000.0),
        "refreshOpen": app.display_refresh_select_expanded,
        "refreshRates": refresh_rates.into_iter().map(|refresh| json!({
            "refresh": refresh, "label": format!("{:.2} Hz", f64::from(refresh) / 1000.0)
        })).collect::<Vec<_>>(),
        "scaleLabel": "Scale",
        "scaleValue": format!("{}%", selected.scale.units() * 100 / 120),
        "scalePercent": (selected.scale.units().saturating_sub(60) as f32 / 420.0).clamp(0.0, 1.0),
        "customScaleLabel": "Custom application scale",
        "customScaleValue": format!("{}%", custom_scale_units * 100 / 120),
        "customScalePercent": (custom_scale_units.saturating_sub(60) as f32 / 420.0).clamp(0.0, 1.0),
        "enabledLabel": "Display enabled",
        "identifyLabel": app.localizer.text("settings-display-identify"),
        "primaryLabel": app.localizer.text("settings-display-make-primary"),
        "applyLabel": app.localizer.text("settings-display-apply"),
        "keepLabel": "Keep",
        "revertLabel": "Revert",
    })
}

impl SettingsApp {
    pub(super) fn handle_display_jsx_action(&mut self, index: usize) {
        self.handle_display_jsx_event(index, Value::Null);
    }

    pub(super) fn handle_display_jsx_event(&mut self, index: usize, value: Value) {
        if self.page != SettingsPage::Display || !self.settings_jsx_enabled {
            return;
        }
        let data = projection(self);
        let message = self
            .display_page
            .get_mut()
            .as_mut()
            .and_then(|page| page.as_mut().ok())
            .ok_or_else(|| STALE_STATUS.to_owned())
            .and_then(|page| page.dispatch(index, value, &data));
        match message {
            Ok(SettingsMessage::SelectDisplay(index)) => self.select_display_native(index),
            Ok(message) => self.handle_settings_message(message),
            Err(error) => {
                if error != STALE_STATUS {
                    *self.display_page.borrow_mut() = Some(Err(error));
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
    fn jsx_display_actions_require_the_selected_connector_and_pending_confirmation() {
        let app = SettingsApp::with_initial_page(SettingsPage::Display);
        let data = projection(&app);
        let mut page = DisplayPage::new().unwrap();
        let (cards, _) = page.render(&data, app.ui_theme()).unwrap();
        assert_eq!(cards.len(), app.displays.len());
        assert!(cards.iter().any(|card| card.name == app.displays[0].name));
        let select = page.action_for_id("display-card-0").unwrap();
        assert!(matches!(
            page.dispatch(select, Value::Null, &data),
            Ok(SettingsMessage::SelectDisplay(0))
        ));
        let enabled = page.action_for_id("display-enabled").unwrap();
        assert!(matches!(
            page.dispatch(enabled, Value::Null, &data),
            Ok(SettingsMessage::DisplayEnabled(false))
        ));
        let mut stale = data.clone();
        stale["connector"] = json!("another-output");
        assert!(page.dispatch(enabled, Value::Null, &stale).is_err());
        assert!(
            validate_request(
                DisplayRequest::Keep {
                    connector: app.displays[app.selected].connector.clone()
                },
                &data
            )
            .is_err()
        );
    }

    #[test]
    fn failed_display_jsx_restores_native_actions() {
        let app = SettingsApp::with_initial_page(SettingsPage::Display);
        *app.display_page.borrow_mut() = Some(Err("JSX failed".into()));
        let tree = app.build_ui_with_diagnostics(1280.0, 720.0);
        assert_eq!(
            tree.semantic_targets_for_message(&SettingsMessage::DisplayIdentify)
                .len(),
            1
        );
    }

    #[test]
    fn jsx_mode_and_scale_policy_requests_are_typed_and_current() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::Display);
        app.selected = 0;
        app.displays[0]
            .modes
            .push(nickel_session_protocol::OutputMode {
                width: 1920,
                height: 1080,
                refresh_millihz: 120_000,
            });
        app.display_resolution_select_expanded = true;
        let data = projection(&app);
        let mut page = DisplayPage::new().unwrap();
        let _ = page.render(&data, app.ui_theme()).unwrap();
        let resolution = page.action_for_id("display-resolution-1920x1080").unwrap();
        assert!(matches!(
            page.dispatch(resolution, Value::Null, &data),
            Ok(SettingsMessage::SetDisplayResolution {
                width: 1920,
                height: 1080
            })
        ));
        let display_scale = page.action_for_id("display-scale").unwrap();
        assert!(matches!(
            page.dispatch(display_scale, json!(1.0), &data),
            Ok(SettingsMessage::SetDisplayScale(14))
        ));
        let application_scale = page.action_for_id("application-custom-scale").unwrap();
        assert!(matches!(
            page.dispatch(application_scale, json!(0.5), &data),
            Ok(SettingsMessage::SetApplicationScale(7))
        ));
        assert!(
            validate_request(
                DisplayRequest::Scale {
                    connector: app.displays[0].connector.clone(),
                    fraction: 1.25,
                },
                &data
            )
            .is_err()
        );
        assert!(
            validate_request(
                DisplayRequest::Refresh {
                    connector: app.displays[0].connector.clone(),
                    refresh: 120_000
                },
                &data
            )
            .is_err()
        );
        app.selected = 1;
        assert!(
            page.dispatch(display_scale, json!(0.5), &projection(&app))
                .is_err()
        );
    }
}
