//! Selected-display actions supplied by the bundled Settings JSX package.
//! Topology, mode validation, drag geometry, and timed revert stay in Settings.

use nickel_plugin_presentation::{components::PluginImages, page::JsxPage};
use nickel_ui::{AnyView, SemanticTheme};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage, SettingsPage, settings_plugin::StyledSettingsPage};

const STALE_STATUS: &str = "Display selection changed; refresh the page";

pub(super) struct DisplayPage {
    page: StyledSettingsPage,
}

impl DisplayPage {
    #[cfg(test)]
    pub(super) fn new() -> Result<Self, String> {
        Self::new_with_page(JsxPage::new(
            crate::settings_package::source(crate::settings_package::Script::Display)?,
            crate::settings_package::manifest()?.clone(),
            None,
        )?)
    }

    pub(super) fn new_with_page(page: JsxPage) -> Result<Self, String> {
        Ok(Self {
            page: StyledSettingsPage::new(page),
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes()
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let (root, stylesheet) = self.page.render(
            data,
            theme,
            include_str!("../../../assets/plugins/settings/settings-display.css"),
        )?;
        Ok(root.view_as_scoped::<SettingsMessage>(
            &PluginImages::new(),
            stylesheet,
            Some("display"),
        ))
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
    ApplicationScalePolicy {
        connector: String,
        policy: String,
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
        DisplayRequest::ApplicationScalePolicy { connector, policy } => {
            let available = data["applicationPolicies"]
                .as_array()
                .is_some_and(|policies| {
                    policies
                        .iter()
                        .any(|entry| entry["id"].as_str() == Some(policy.as_str()))
                });
            if !available {
                return Err(STALE_STATUS.into());
            }
            let message = match policy.as_str() {
                "follow" => SettingsMessage::ApplicationScaleFollow,
                "unchanged" => SettingsMessage::ApplicationScaleUnchanged,
                "custom" => {
                    let step = data["customScaleStep"]
                        .as_u64()
                        .and_then(|step| u32::try_from(step).ok())
                        .filter(|step| *step <= 14)
                        .ok_or(STALE_STATUS)?;
                    SettingsMessage::SetApplicationScale(step)
                }
                _ => return Err(STALE_STATUS.into()),
            };
            (connector, message)
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
                "geometry": {"x": display.rect.x, "y": display.rect.y,
                    "width": display.rect.w, "height": display.rect.h},
                "scale_120": display.scale.units(),
                "current_mode": display.mode,
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
        "title": app.localizer.text("settings-display-title"),
        "subtitle": app.localizer.text("settings-display-subtitle"),
        "selectedName": selected.name,
        "selectedDetail": selected.detail,
        "status": app.status,
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
        "customScaleStep": custom_scale_units.saturating_sub(60).min(420) / 30,
        "applicationPolicies": [
            {"id":"follow", "label":"Follow Nickel", "selected":app.application_scale_policy == crate::ApplicationScalePolicy::FollowNickel},
            {"id":"unchanged", "label":"Leave unchanged", "selected":app.application_scale_policy == crate::ApplicationScalePolicy::Unchanged},
            {"id":"custom", "label":"Custom", "selected":matches!(app.application_scale_policy, crate::ApplicationScalePolicy::Custom(_))},
        ],
        "enabledLabel": "Display enabled",
        "identifyLabel": app.localizer.text("settings-display-identify"),
        "primaryLabel": app.localizer.text("settings-display-make-primary"),
        "applyLabel": app.localizer.text("settings-display-apply"),
        "keepLabel": "Keep",
        "revertLabel": "Revert",
    })
}

impl SettingsApp {
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
        let _ = page.render(&data, app.ui_theme()).unwrap();
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
    fn failed_display_jsx_shows_recovery_without_native_actions() {
        let app = SettingsApp::with_initial_page(SettingsPage::Display);
        *app.display_page.borrow_mut() = Some(Err("JSX failed".into()));
        let tree = app.build_ui_with_diagnostics(1280.0, 720.0);
        assert_eq!(
            tree.semantic_targets_for_message(&SettingsMessage::DisplayIdentify)
                .len(),
            0
        );
        assert!(
            !tree
                .semantic_targets_for_message(&SettingsMessage::Navigate(SettingsPage::Plugins))
                .is_empty()
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
        let unchanged = page
            .action_for_id("application-scale-policy-unchanged")
            .unwrap();
        assert!(matches!(
            page.dispatch(unchanged, Value::Null, &data),
            Ok(SettingsMessage::ApplicationScaleUnchanged)
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
