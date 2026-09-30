//! Default application rows declared by the bundled Settings plugin.
//! The host retains association discovery, virtual list geometry, and the picker.

use std::collections::BTreeMap;

use nickel_plugin_presentation::{
    components::{PanelNode, PluginImages},
    css::StyleSheet,
    page::{JsxPage, STALE_DATA},
};
use nickel_ui::{AnyView, Column, SemanticTheme, VirtualWindow};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage, SettingsPage};

const STALE_STATUS: &str = STALE_DATA;

pub(super) struct DefaultAppsPage {
    page: JsxPage,
    stylesheet: StyleSheet,
    last_theme: Option<SemanticTheme>,
}

pub(super) struct DefaultAppsRendered {
    pub curated: AnyView<SettingsMessage>,
    pub catalog_nodes: BTreeMap<String, PanelNode>,
    pub stylesheet: StyleSheet,
}

impl DefaultAppsPage {
    #[cfg(test)]
    pub(super) fn new() -> Result<Self, String> {
        Self::new_with_page(JsxPage::new(
            crate::settings_package::source(crate::settings_package::Script::DefaultApps)?,
            crate::settings_package::manifest()?.clone(),
            None,
        )?)
    }

    pub(super) fn new_with_page(page: JsxPage) -> Result<Self, String> {
        Ok(Self {
            page,
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
    ) -> Result<DefaultAppsRendered, String> {
        self.page.render(data)?;
        if self.last_theme != Some(theme) {
            self.stylesheet = crate::settings_plugin::stylesheet_template(
                include_str!("../../../assets/plugins/settings/settings-default-apps.css"),
                theme,
            )?;
            self.last_theme = Some(theme);
        }
        let Some(root @ PanelNode::Div { children, .. }) = self.page.node() else {
            return Err("Default Apps page structure is invalid".into());
        };
        root.direct_child_with_class("default-app-curated")
            .ok_or("Default Apps curated section is unavailable")?;
        root.direct_child_with_class("default-app-advanced")
            .ok_or("Default Apps advanced section is unavailable")?;
        let catalog_section = root
            .direct_child_with_class("default-app-catalog")
            .ok_or("Default Apps catalog is unavailable")?;
        let PanelNode::Div {
            children: catalog, ..
        } = catalog_section
        else {
            return Err("Default Apps catalog structure is invalid".into());
        };
        let projected = data["catalogRows"]
            .as_array()
            .ok_or("Default Apps catalog projection is invalid")?;
        if catalog.len() != projected.len() || catalog.len() > 32 {
            return Err("Default Apps catalog row count changed".into());
        }
        let catalog_nodes = projected
            .iter()
            .zip(catalog)
            .map(|(row, node)| {
                Ok((
                    row["key"]
                        .as_str()
                        .ok_or("Default Apps catalog target is invalid")?
                        .to_owned(),
                    node.clone(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        if catalog_nodes.len() != catalog.len() {
            return Err("Default Apps catalog has duplicate targets".into());
        }
        let images = PluginImages::new();
        let view = AnyView::new(
            Column::new().fill_width().gap(16.0).children(
                children
                    .iter()
                    .filter(|child| !std::ptr::eq(*child, catalog_section))
                    .map(|child| child.view_as::<SettingsMessage>(&images, &self.stylesheet)),
            ),
        );
        Ok(DefaultAppsRendered {
            curated: view,
            catalog_nodes,
            stylesheet: self.stylesheet.clone(),
        })
    }

    fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        data: &Value,
    ) -> Result<SettingsMessage, String> {
        self.page.dispatch(index, &value, data, |effect| {
            let request: DefaultAppsRequest =
                serde_json::from_value(effect).map_err(|error| error.to_string())?;
            validate_request(request, data)
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
enum DefaultAppsRequest {
    ChooseDefault { index: usize, target: String },
    SearchTargets { value: String },
    SetFamily { index: usize },
    BrowseTarget { index: usize, key: String },
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
        DefaultAppsRequest::BrowseTarget { index, key } => {
            let row = data
                .get("catalogRows")
                .and_then(Value::as_array)
                .and_then(|rows| {
                    rows.iter().find(|row| {
                        row["index"].as_u64() == Some(index as u64)
                            && row["key"].as_str() == Some(key.as_str())
                    })
                })
                .ok_or(STALE_STATUS)?;
            let value = row["targetValue"]
                .as_str()
                .ok_or("Association target is invalid")?;
            let target = match row["targetKind"].as_str() {
                Some("extension") => nickel_platform::AssociationTarget::extension(value),
                Some("mime") => nickel_platform::AssociationTarget::mime(value),
                Some("scheme") => nickel_platform::AssociationTarget::scheme(value),
                _ => return Err("Association target kind is invalid".into()),
            };
            if target.platform_key() != key {
                return Err(STALE_STATUS.into());
            }
            Ok(SettingsMessage::BrowseDefaultAppTarget(target))
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

pub(super) fn matching_targets(app: &SettingsApp) -> Vec<nickel_platform::AssociationTarget> {
    let query = app.default_app_target_query.trim().to_lowercase();
    app.default_app_targets
        .iter()
        .filter(|target| {
            (query.is_empty() || target.platform_key().to_lowercase().contains(&query))
                && app
                    .default_app_target_family
                    .is_none_or(|family| target.family() == family)
                && !app.default_apps.iter().any(|row| row.target == **target)
        })
        .cloned()
        .collect()
}

pub(super) fn projection(app: &SettingsApp) -> Value {
    projection_for_targets(app, &matching_targets(app))
}

pub(super) fn projection_for_targets(
    app: &SettingsApp,
    matching: &[nickel_platform::AssociationTarget],
) -> Value {
    let heights = vec![58.0; matching.len()];
    let window = VirtualWindow::from_heights(
        &heights,
        2.0,
        app.default_app_catalog_scroll_offset,
        300.0,
        116.0,
    );
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
        "searchPlaceholder":app.localizer.text("settings-default-apps-search-placeholder"),
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
        }).collect::<Vec<_>>(),
        "catalogRows":matching[window.range.clone()].iter().enumerate().map(|(offset,target)| {
            let (kind,value) = match target {
                nickel_platform::AssociationTarget::Extension(value) => ("extension",value),
                nickel_platform::AssociationTarget::Mime(value) => ("mime",value),
                nickel_platform::AssociationTarget::Scheme(value) => ("scheme",value),
            };
            json!({
                "index":window.range.start + offset,
                "key":target.platform_key(),
                "family":target.family().label(),
                "targetKind":kind,
                "targetValue":value,
            })
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
            Err(error) => {
                if error != STALE_STATUS {
                    *self.default_apps_page.borrow_mut() = Some(Err(error));
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

    #[test]
    fn catalog_renders_a_bounded_window_and_validates_target_kind() {
        let mut app = SettingsApp::with_initial_page(SettingsPage::DefaultApps);
        app.default_app_targets = (0..250)
            .map(|index| {
                nickel_platform::AssociationTarget::mime(format!(
                    "application/x-nickel-fixture-{index:03}"
                ))
            })
            .collect();
        let initial = projection(&app);
        assert!(initial["catalogRows"].as_array().unwrap().len() < 20);
        assert!(
            initial["catalogRows"]
                .as_array()
                .unwrap()
                .iter()
                .all(|row| { row["key"] != "application/x-nickel-fixture-249" })
        );

        app.default_app_catalog_scroll_offset = 20_000.0;
        let scrolled = projection(&app);
        assert!(
            scrolled["catalogRows"]
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["key"] == "application/x-nickel-fixture-249")
        );

        app.default_app_catalog_scroll_offset = 0.0;
        app.default_app_target_query = "x-nickel-fixture-249".into();
        let data = projection(&app);
        let mut page = DefaultAppsPage::new().unwrap();
        let rendered = page.render(&data, app.ui_theme()).unwrap();
        assert!(
            rendered
                .catalog_nodes
                .contains_key("application/x-nickel-fixture-249")
        );
        let action = page.action_for_id("default-app-target-0").unwrap();
        assert_eq!(
            page.dispatch(action, Value::Null, &data).unwrap(),
            SettingsMessage::BrowseDefaultAppTarget(nickel_platform::AssociationTarget::mime(
                "application/x-nickel-fixture-249"
            ))
        );
        assert!(
            validate_request(
                DefaultAppsRequest::BrowseTarget {
                    index: 0,
                    key: "different".into(),
                },
                &data,
            )
            .is_err()
        );
    }
}
