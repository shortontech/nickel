//! Selected-display actions supplied by the bundled Settings JSX package.
//! Topology, mode validation, drag geometry, and timed revert stay in Settings.

use nickel_ui::{AnyView, SemanticTheme};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    SettingsApp, SettingsMessage, SettingsPage,
    settings_components::{Node, SettingsJsxContext},
};

const STALE_STATUS: &str = "Display selection changed; refresh the page";

pub(super) struct DisplayPage {
    context: SettingsJsxContext,
}

impl DisplayPage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            context: SettingsJsxContext::new(
                crate::settings_package::source(crate::settings_package::Script::Display)?,
                parse_tree,
                STALE_STATUS,
                "Display action must request one operation",
            )?,
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.context.retained_bytes()
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<[AnyView<SettingsMessage>; 6], String> {
        let Node::Stack(children) = self.context.render(data)? else {
            return Err("Display actions have an invalid root".into());
        };
        Ok(std::array::from_fn(|index| {
            children[index].view(theme, "", SettingsMessage::DisplayJsxAction)
        }))
    }

    fn dispatch(&mut self, index: usize, data: &Value) -> Result<SettingsMessage, String> {
        self.context.dispatch(index, &Value::Null, data, |effect| {
            let request: DisplayRequest =
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
    let children = value
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Display actions have no children")?;
    if value["kind"] != "settings-stack"
        || children.len() != 6
        || children[0]["kind"] != "settings-row"
        || children[1]["kind"] != "settings-select"
        || children[2]["kind"] != "settings-select"
        || children[3]["kind"] != "settings-grid"
        || children[4]["kind"] != "settings-inline"
        || children[5]["kind"] != "settings-radio-group"
    {
        return Err("Display actions have an invalid structure".into());
    }
    let node = Node::parse(value)?;
    if node.contains_input() {
        return Err("Display actions cannot request text input".into());
    }
    Ok(node)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum DisplayRequest {
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
    ApplicationScale {
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
        DisplayRequest::ApplicationScale { connector, policy } => {
            let message = match policy.as_str() {
                "follow" => SettingsMessage::ApplicationScaleFollow,
                "unchanged" => SettingsMessage::ApplicationScaleUnchanged,
                "custom" => SettingsMessage::SetApplicationScale(
                    data["customScaleStep"]
                        .as_u64()
                        .and_then(|step| u32::try_from(step).ok())
                        .filter(|step| *step <= 14)
                        .ok_or(STALE_STATUS)?,
                ),
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
    let (application_scale_policy, custom_scale_units) = match app.application_scale_policy {
        crate::ApplicationScalePolicy::FollowNickel => ("follow", 120),
        crate::ApplicationScalePolicy::Unchanged => ("unchanged", 120),
        crate::ApplicationScalePolicy::Custom(scale) => ("custom", scale.units()),
    };
    json!({
        "connector": selected.connector,
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
        "applicationScalePolicy": application_scale_policy,
        "customScaleStep": custom_scale_units.saturating_sub(60).min(420) / 30,
        "applicationScaleFollowLabel": "Follow Nickel",
        "applicationScaleUnchangedLabel": "Leave unchanged",
        "applicationScaleCustomLabel": "Custom",
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
            .and_then(|page| page.dispatch(index, &data));
        match message {
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
        let enabled = page.action_for_id("display-enabled").unwrap();
        assert!(matches!(
            page.dispatch(enabled, &data),
            Ok(SettingsMessage::DisplayEnabled(false))
        ));
        let mut stale = data.clone();
        stale["connector"] = json!("another-output");
        assert!(page.dispatch(enabled, &stale).is_err());
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
        let data = projection(&app);
        let mut page = DisplayPage::new().unwrap();
        let _ = page.render(&data, app.ui_theme()).unwrap();
        let resolution = page.action_for_id("display-resolution-1920x1080").unwrap();
        assert!(matches!(
            page.dispatch(resolution, &data),
            Ok(SettingsMessage::SetDisplayResolution {
                width: 1920,
                height: 1080
            })
        ));
        let custom = page.action_for_id("application-scale-custom").unwrap();
        assert!(matches!(
            page.dispatch(custom, &data),
            Ok(SettingsMessage::SetApplicationScale(2))
        ));
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
        assert!(page.dispatch(custom, &projection(&app)).is_err());
    }
}
