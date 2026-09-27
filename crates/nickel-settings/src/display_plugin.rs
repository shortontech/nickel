//! Selected-display actions supplied by the bundled Settings JSX package.
//! Topology, mode choices, drag geometry, and timed revert stay in Settings.

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
    ) -> Result<[AnyView<SettingsMessage>; 3], String> {
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
        || children.len() != 3
        || children[0]["kind"] != "settings-row"
        || children[1]["kind"] != "settings-grid"
        || children[2]["kind"] != "settings-inline"
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
    Enabled { connector: String, enabled: bool },
    Identify { connector: String },
    Primary { connector: String },
    Apply { connector: String },
    Keep { connector: String },
    Revert { connector: String },
}

fn validate_request(request: DisplayRequest, data: &Value) -> Result<SettingsMessage, String> {
    let (connector, message) = match request {
        DisplayRequest::Enabled { connector, enabled } => {
            if data["enabled"].as_bool() == Some(enabled) {
                return Err(STALE_STATUS.into());
            }
            (connector, SettingsMessage::DisplayEnabled(enabled))
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
    json!({
        "connector": selected.connector,
        "enabled": selected.enabled,
        "pendingRevert": app.pending_display_revert.is_some(),
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
}
