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
        Ok(self.context.render(data)?.view_with_input(
            theme,
            "Search file types and protocols",
            SettingsMessage::DefaultAppsJsxAction,
            SettingsMessage::DefaultAppsJsxInput,
        ))
    }

    fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        data: &Value,
    ) -> Result<SettingsMessage, String> {
        self.context.dispatch(index, &value, data, |effect| {
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
    if value.get("kind").and_then(Value::as_str) != Some("settings-stack")
        || children.len() != 2
        || children[0].get("kind").and_then(Value::as_str) != Some("settings-compact-list")
        || children[1].get("kind").and_then(Value::as_str) != Some("settings-card")
    {
        return Err("Default Apps page structure is invalid".into());
    }
    let rows = children[0]
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Default Apps rows are missing")?;
    let advanced = children[1]
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Default Apps filters are missing")?;
    if rows.is_empty()
        || rows.len() > 64
        || rows
            .iter()
            .any(|child| child.get("kind").and_then(Value::as_str) != Some("settings-row"))
        || advanced.len() != 2
        || advanced[0].get("kind").and_then(Value::as_str) != Some("settings-input")
        || advanced[1].get("kind").and_then(Value::as_str) != Some("settings-grid")
    {
        return Err("Default Apps controls are invalid".into());
    }
    Node::parse(value)
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum DefaultAppsRequest {
    ChooseDefault { index: usize, target: String },
    SearchTargets { value: String },
    SetFamily { index: usize },
}

fn validate_request(request: DefaultAppsRequest, data: &Value) -> Result<SettingsMessage, String> {
    match request {
        DefaultAppsRequest::ChooseDefault { index, target } => {
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
        DefaultAppsRequest::SearchTargets { value } => {
            if value.chars().count() > 256 {
                return Err("Association search is too long".into());
            }
            Ok(SettingsMessage::DefaultAppTargetChanged(value))
        }
        DefaultAppsRequest::SetFamily { index } => {
            if !data
                .get("families")
                .and_then(Value::as_array)
                .is_some_and(|families| {
                    families.iter().any(|family| {
                        family.get("index").and_then(Value::as_u64) == Some(index as u64)
                    })
                })
            {
                return Err(STALE_STATUS.into());
            }
            Ok(SettingsMessage::DefaultAppTargetFamily(family_by_index(
                index,
            )?))
        }
    }
}

fn family_by_index(index: usize) -> Result<Option<nickel_platform::AssociationFamily>, String> {
    use nickel_platform::AssociationFamily as Family;
    Ok(match index {
        0 => None,
        1 => Some(Family::Web),
        2 => Some(Family::Documents),
        3 => Some(Family::Images),
        4 => Some(Family::Audio),
        5 => Some(Family::Video),
        6 => Some(Family::Archives),
        7 => Some(Family::OtherFiles),
        8 => Some(Family::Protocols),
        _ => return Err("Association family is invalid".into()),
    })
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
        }).collect::<Vec<_>>(),
        "advancedTitle":"File types and links",
        "advancedStatus":app.default_app_target_status.as_deref().unwrap_or_default(),
        "query":app.default_app_target_query,
        "families":(0..=8).filter_map(|index| {
            let family = family_by_index(index).ok()?;
            let count = family.map_or(app.default_app_targets.len(), |family| {
                app.default_app_targets.iter().filter(|target| target.family() == family).count()
            });
            if index != 0 && count == 0 { return None; }
            let label = family.map_or_else(|| "All".to_owned(), |family| family.label().to_owned());
            Some(json!({
                "index":index,
                "label":format!("{label} ({count})"),
                "selected":app.default_app_target_family == family,
            }))
        }).collect::<Vec<_>>()
    })
}

impl SettingsApp {
    pub(super) fn handle_default_apps_jsx_action(&mut self, index: usize, value: Value) {
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
            .and_then(|page| page.dispatch(index, value, &data));
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
            page.dispatch(action, Value::Null, &data).unwrap(),
            SettingsMessage::ToggleDefaultAppSelect(0)
        );
        let old_target = app.default_apps[0].target.platform_key();
        app.default_apps[0].target = nickel_platform::AssociationTarget::scheme("fixture");
        let changed = projection(&app);
        assert_eq!(
            page.dispatch(action, Value::Null, &changed).unwrap_err(),
            STALE_STATUS
        );
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

    #[test]
    fn search_and_family_filter_dispatch_typed_requests() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::DefaultApps);
        app.default_app_targets
            .push(nickel_platform::AssociationTarget::mime("image/png"));
        let data = projection(&app);
        let mut page = DefaultAppsPage::new().unwrap();
        page.render(&data, app.ui_theme()).unwrap();
        let search = page.action_for_id("default-app-advanced-target").unwrap();
        assert_eq!(
            page.dispatch(search, Value::String("png".into()), &data)
                .unwrap(),
            SettingsMessage::DefaultAppTargetChanged("png".into())
        );
        let family = data["families"]
            .as_array()
            .unwrap()
            .iter()
            .find_map(|family| family["index"].as_u64().filter(|index| *index > 0))
            .unwrap() as usize;
        let filter = page
            .action_for_id(&format!("default-app-family-{family}"))
            .unwrap();
        assert_eq!(
            page.dispatch(filter, Value::Null, &data).unwrap(),
            SettingsMessage::DefaultAppTargetFamily(family_by_index(family).unwrap())
        );
        assert!(
            validate_request(
                DefaultAppsRequest::SetFamily { index: 8 },
                &json!({"families":[]}),
            )
            .is_err()
        );
        assert!(
            validate_request(
                DefaultAppsRequest::SearchTargets {
                    value: "x".repeat(257)
                },
                &data,
            )
            .is_err()
        );
    }
}
