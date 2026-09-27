//! Default application rows declared by the bundled Settings plugin.
//! The host retains association discovery and the native handler picker.

use nickel_ui::{AnyView, SemanticTheme};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    SettingsApp, SettingsMessage, SettingsPage,
    settings_components::{Node, SettingsJsxContext},
};

const STALE_STATUS: &str = "Default applications changed; refresh the page";

pub(super) struct DefaultAppsPage {
    context: SettingsJsxContext,
}

impl DefaultAppsPage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            context: SettingsJsxContext::new(
                crate::settings_package::source(crate::settings_package::Script::DefaultApps)?,
                parse_tree,
                STALE_STATUS,
                "Default application action must request one operation",
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
    ) -> Result<AnyView<SettingsMessage>, String> {
        Ok(self
            .context
            .render(data)?
            .view(theme, "", SettingsMessage::DefaultAppsJsxAction))
    }

    fn dispatch(&mut self, index: usize, data: &Value) -> Result<SettingsMessage, String> {
        self.context.dispatch(index, &Value::Null, data, |effect| {
            let request: DefaultAppsRequest =
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
        .ok_or("Default Apps page has no children")?;
    if value.get("kind").and_then(Value::as_str) != Some("settings-compact-list")
        || children.is_empty()
        || children.len() > 64
        || children
            .iter()
            .any(|child| child.get("kind").and_then(Value::as_str) != Some("settings-row"))
    {
        return Err("Default Apps page structure is invalid".into());
    }
    let node = Node::parse(value)?;
    if node.contains_input() {
        return Err("Default Apps rows cannot request text input".into());
    }
    Ok(node)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum DefaultAppsRequest {
    ChooseDefault { index: usize, target: String },
}

fn validate_request(request: DefaultAppsRequest, data: &Value) -> Result<SettingsMessage, String> {
    let DefaultAppsRequest::ChooseDefault { index, target } = request;
    if data
        .pointer(&format!("/rows/{index}/target"))
        .and_then(Value::as_str)
        != Some(target.as_str())
        || data
            .pointer(&format!("/rows/{index}/index"))
            .and_then(Value::as_u64)
            != Some(index as u64)
    {
        return Err(STALE_STATUS.into());
    }
    Ok(SettingsMessage::ToggleDefaultAppSelect(index))
}

pub(super) fn projection(app: &SettingsApp) -> Value {
    json!({
        "rows":app.default_apps.iter().enumerate().map(|(index,row)| {
            let current = row.snapshot.as_ref()
                .and_then(|snapshot| snapshot.effective.as_ref())
                .map(|handler| handler.name.clone())
                .unwrap_or_else(|| "No default".into());
            json!({
                "index":index,
                "target":row.target.platform_key(),
                "label":row.label,
                "current":current,
                "status":row.status.as_deref().unwrap_or_default(),
            })
        }).collect::<Vec<_>>()
    })
}

impl SettingsApp {
    pub(super) fn handle_default_apps_jsx_action(&mut self, index: usize) {
        if self.page != SettingsPage::DefaultApps || !self.settings_jsx_enabled {
            return;
        }
        let data = projection(self);
        let result = self
            .default_apps_page
            .borrow_mut()
            .as_mut()
            .and_then(|page| page.as_mut().ok())
            .ok_or_else(|| "Default Apps plugin is unavailable".to_owned())
            .and_then(|page| page.dispatch(index, &data));
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => self.plugin_notice = Some(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chooser_request_uses_current_target_identity() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::DefaultApps);
        let data = projection(&app);
        let mut page = DefaultAppsPage::new().unwrap();
        page.render(&data, app.ui_theme()).unwrap();
        let action = page.action_for_id("default-app-0").unwrap();
        assert_eq!(
            page.dispatch(action, &data).unwrap(),
            SettingsMessage::ToggleDefaultAppSelect(0)
        );
        let old_target = app.default_apps[0].target.platform_key();
        app.default_apps[0].target = nickel_platform::AssociationTarget::scheme("fixture");
        let changed = projection(&app);
        assert_eq!(page.dispatch(action, &changed).unwrap_err(), STALE_STATUS);
        assert!(
            validate_request(
                DefaultAppsRequest::ChooseDefault {
                    index: 0,
                    target: old_target,
                },
                &changed,
            )
            .is_err()
        );
    }
}
