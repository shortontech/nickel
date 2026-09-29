//! JSX candidate controls inside the host-owned default application popover.

use std::collections::BTreeMap;

use nickel_plugin_presentation::{
    components::{PanelNode, PluginImages},
    css::StyleSheet,
    page::{JsxPage, STALE_DATA},
};
use nickel_ui::{AnyView, CollectionState, SemanticTheme, VirtualWindow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage};

const STALE_STATUS: &str = STALE_DATA;

pub(super) struct DefaultAppPickerPage {
    page: JsxPage,
    stylesheet: StyleSheet,
    last_theme: Option<SemanticTheme>,
    data: Option<Value>,
    data_bytes: usize,
}

pub(super) struct DefaultAppPickerRendered {
    pub header: AnyView<SettingsMessage>,
    pub candidates: BTreeMap<String, PanelNode>,
    pub stylesheet: StyleSheet,
}

impl DefaultAppPickerPage {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            page: JsxPage::new(
                crate::settings_package::source(crate::settings_package::Script::DefaultAppPicker)?,
                crate::settings_package::manifest()?.clone(),
                None,
            )?,
            stylesheet: StyleSheet::default(),
            last_theme: None,
            data: None,
            data_bytes: 0,
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes()
            + self.stylesheet.estimated_retained_bytes() as usize
            + self.data_bytes
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
    ) -> Result<DefaultAppPickerRendered, String> {
        self.page.render(data)?;
        if self.last_theme != Some(theme) {
            self.stylesheet = crate::settings_plugin::stylesheet_template(
                include_str!("../../../assets/plugins/settings/settings-default-app-picker.css"),
                theme,
            )?;
            self.last_theme = Some(theme);
        }
        let Some(PanelNode::Div { children, .. }) = self.page.node() else {
            return Err("Default application picker structure is invalid".into());
        };
        if children.len() != 2 {
            return Err("Default application picker needs header and candidate sections".into());
        }
        let PanelNode::Div {
            children: candidates,
            ..
        } = &children[1]
        else {
            return Err("Default application candidate list is invalid".into());
        };
        let projected = data["handlers"]
            .as_array()
            .ok_or("Default application candidates are invalid")?;
        if candidates.len() != projected.len() || candidates.len() > 32 {
            return Err("Default application candidate count changed".into());
        }
        let nodes = projected
            .iter()
            .zip(candidates)
            .map(|(handler, node)| {
                Ok((
                    handler["id"]
                        .as_str()
                        .ok_or("Default application candidate ID is invalid")?
                        .to_owned(),
                    node.clone(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        if nodes.len() != candidates.len() {
            return Err("Default application candidate IDs are duplicated".into());
        }
        self.data = Some(data.clone());
        self.data_bytes = data.to_string().len();
        Ok(DefaultAppPickerRendered {
            header: children[0].view_as_scoped::<SettingsMessage>(
                &PluginImages::new(),
                &self.stylesheet,
                Some("default-app-picker"),
            ),
            candidates: nodes,
            stylesheet: self.stylesheet.clone(),
        })
    }

    pub(super) fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        app: &SettingsApp,
    ) -> Result<SettingsMessage, String> {
        let data = self
            .data
            .as_ref()
            .ok_or("Default application picker is unavailable")?;
        self.page.dispatch(index, &value, data, |effect| {
            let request: PickerRequest =
                serde_json::from_value(effect).map_err(|error| error.to_string())?;
            validate_request(request, data, app)
        })
    }

    #[cfg(test)]
    pub(super) fn action_for_id(&self, id: &str) -> Option<usize> {
        self.page.node().and_then(|node| {
            node.button_action(id)
                .or_else(|| node.text_field_action(id))
        })
    }
}

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
enum PickerRequest {
    SearchHandlers {
        value: String,
    },
    ChooseHandler {
        row: usize,
        target: String,
        handler: String,
    },
}

fn validate_request(
    request: PickerRequest,
    data: &Value,
    app: &SettingsApp,
) -> Result<SettingsMessage, String> {
    let row_index = data["row"]
        .as_u64()
        .and_then(|index| usize::try_from(index).ok())
        .ok_or(STALE_STATUS)?;
    if app.default_app_picker_row.get() != Some(row_index) {
        return Err(STALE_STATUS.into());
    }
    match request {
        PickerRequest::SearchHandlers { value } => {
            if value.chars().count() > 256 {
                return Err("Application search is too long".into());
            }
            Ok(SettingsMessage::DefaultAppHandlerSearchChanged(value))
        }
        PickerRequest::ChooseHandler {
            row,
            target,
            handler,
        } => {
            if row != row_index
                || data["target"].as_str() != Some(target.as_str())
                || app
                    .default_apps
                    .get(row)
                    .map(|entry| entry.target.platform_key())
                    != Some(target)
            {
                return Err(STALE_STATUS.into());
            }
            let candidate = data["handlers"]
                .as_array()
                .and_then(|handlers| handlers.iter().find(|entry| entry["id"] == handler))
                .ok_or(STALE_STATUS)?;
            if candidate["editable"].as_bool() != Some(true) {
                return Err("Application choice is unavailable".into());
            }
            let snapshot = app.default_apps[row]
                .snapshot
                .as_ref()
                .ok_or(STALE_STATUS)?;
            if !matches!(
                snapshot.capability,
                nickel_platform::AssociationCapability::DirectUserChange
                    | nickel_platform::AssociationCapability::NativeConsent
            ) || !snapshot.handlers.iter().any(|entry| entry.id == handler)
                || snapshot
                    .effective
                    .as_ref()
                    .is_some_and(|entry| entry.id == handler)
            {
                return Err(STALE_STATUS.into());
            }
            Ok(SettingsMessage::SetDefaultApp {
                row,
                handler_id: handler,
            })
        }
    }
}

pub(super) fn projection(
    app: &SettingsApp,
    row: usize,
    state: &CollectionState<nickel_platform::ApplicationHandler>,
    can_change: bool,
    status: &str,
) -> Value {
    let target = &app.default_apps[row];
    let current = target
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.effective.as_ref());
    let handlers = match state {
        CollectionState::Ready(handlers) => handlers.as_slice(),
        _ => &[],
    };
    let window = VirtualWindow::from_heights(
        &vec![58.0; handlers.len()],
        2.0,
        app.default_app_handler_scroll_offset,
        320.0,
        116.0,
    );
    json!({
        "row":row,
        "target":target.target.platform_key(),
        "status":status,
        "query":app.default_app_handler_query,
        "searchPlaceholder":app.localizer.text("settings-default-app-picker-search-placeholder"),
        "currentLabel":"Current",
        "chooseLabel":"Choose",
        "handlers":handlers[window.range].iter().map(|handler| {
            let is_current = current.is_some_and(|current| current.id == handler.id);
            json!({
                "id":handler.id,
                "name":handler.name,
                "detail":if is_current {format!("Current • {}", handler.id)} else {handler.id.clone()},
                "current":is_current,
                "editable":can_change && !is_current,
            })
        }).collect::<Vec<_>>()
    })
}

impl SettingsApp {
    pub(super) fn handle_default_app_picker_jsx_action(&mut self, index: usize, value: Value) {
        if !self.settings_jsx_enabled || self.page != crate::SettingsPage::DefaultApps {
            return;
        }
        let result = self
            .default_app_picker_page
            .borrow_mut()
            .as_mut()
            .and_then(|page| page.as_mut().ok())
            .ok_or_else(|| "Default application picker is unavailable".to_owned())
            .and_then(|page| page.dispatch(index, value, self));
        match result {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => {
                if error != STALE_STATUS {
                    *self.default_app_picker_page.borrow_mut() = Some(Err(error));
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
    fn picker_search_and_choice_use_current_candidate_and_capability() {
        let mut app = SettingsApp::with_initial_page(crate::SettingsPage::DefaultApps);
        app.default_app_picker_row.set(Some(0));
        let target = app.default_apps[0].target.clone();
        let handler = nickel_platform::ApplicationHandler {
            id: "fixture.desktop".into(),
            name: "Fixture Browser".into(),
            icon: None,
            source: "fixture".into(),
        };
        app.default_apps[0].snapshot = Some(nickel_platform::AssociationSnapshot {
            target,
            effective: None,
            handlers: vec![handler.clone()],
            capability: nickel_platform::AssociationCapability::DirectUserChange,
            scope: nickel_platform::AssociationScope::User,
            detail: "User association".into(),
        });
        let state = CollectionState::Ready(vec![handler]);
        let data = projection(&app, 0, &state, true, "Choose an application below.");
        let mut page = DefaultAppPickerPage::new().unwrap();
        let rendered = page.render(&data, app.ui_theme()).unwrap();
        assert!(rendered.candidates.contains_key("fixture.desktop"));
        let search = page.action_for_id("default-app-handler-search-0").unwrap();
        assert_eq!(
            page.dispatch(search, Value::String("fixture".into()), &app)
                .unwrap(),
            SettingsMessage::DefaultAppHandlerSearchChanged("fixture".into())
        );
        let choose = page
            .action_for_id("default-app-handler-fixture.desktop")
            .unwrap();
        assert_eq!(
            page.dispatch(choose, Value::Null, &app).unwrap(),
            SettingsMessage::SetDefaultApp {
                row: 0,
                handler_id: "fixture.desktop".into(),
            }
        );
        app.default_apps[0].snapshot.as_mut().unwrap().capability =
            nickel_platform::AssociationCapability::ReadOnly;
        assert_eq!(
            page.dispatch(choose, Value::Null, &app).unwrap_err(),
            STALE_STATUS
        );
    }
}
