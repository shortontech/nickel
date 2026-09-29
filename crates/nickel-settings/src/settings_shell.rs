//! JSX-owned Settings window and navigation layout.

use nickel_plugin_presentation::{components::PluginImages, css::StyleSheet, page::JsxPage};
use nickel_ui::{AnyView, SemanticTheme};
use serde::Deserialize;
use serde_json::Value;

use crate::{SettingsApp, SettingsMessage, SettingsPage};
use nickel_ui::search_settings;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub(super) enum ShellRequest {
    Navigate { page: String },
    NavigateTarget { target: String },
    Search { value: String },
    ShowNavigation,
}

pub(super) struct SettingsShell {
    page: JsxPage,
    stylesheet: StyleSheet,
    last_theme: Option<SemanticTheme>,
    last_data: Option<Value>,
}

impl SettingsShell {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            page: JsxPage::new(
                crate::settings_package::source(crate::settings_package::Script::Shell)?,
                crate::settings_package::manifest()?.clone(),
                Some("main".into()),
            )?,
            stylesheet: StyleSheet::default(),
            last_theme: None,
            last_data: None,
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes()
            + self.stylesheet.estimated_retained_bytes() as usize
            + self
                .last_data
                .as_ref()
                .map_or(0, |data| data.to_string().len())
    }

    pub(super) fn render(
        &mut self,
        data: &Value,
        theme: SemanticTheme,
        content: AnyView<SettingsMessage>,
    ) -> Result<AnyView<SettingsMessage>, String> {
        self.page.render(data)?;
        if self.last_theme != Some(theme) {
            self.stylesheet = crate::settings_plugin::stylesheet_template(
                include_str!("../../../assets/plugins/settings/settings-shell.css"),
                theme,
            )?;
            self.last_theme = Some(theme);
        }
        self.last_data = Some(data.clone());
        let mut content = Some(content);
        Ok(self
            .page
            .node()
            .ok_or("Settings shell is unavailable")?
            .view_as_with_slots(
                &PluginImages::new(),
                &self.stylesheet,
                Some("settings-shell"),
                &mut |id| (id == "settings-content").then(|| content.take()).flatten(),
            ))
    }

    pub(super) fn dispatch(
        &mut self,
        index: usize,
        value: Value,
        validate: impl FnOnce(&ShellRequest) -> Result<SettingsMessage, String>,
    ) -> Result<SettingsMessage, String> {
        let data = self
            .last_data
            .as_ref()
            .ok_or("Settings shell has no data")?;
        self.page.dispatch(index, &value, data, |effect| {
            let request: ShellRequest =
                serde_json::from_value(effect).map_err(|error| error.to_string())?;
            validate(&request)
        })
    }
}

impl SettingsApp {
    pub(super) fn handle_shell_jsx_action(&mut self, index: usize, value: Value) {
        if !self.settings_jsx_enabled {
            return;
        }
        let destinations = self.navigation_destinations();
        let entries = self.navigation_search_entries();
        let query = self.sidebar_query.trim().to_lowercase();
        let results = search_settings(&query, &entries);
        let message = self
            .settings_shell
            .borrow_mut()
            .as_mut()
            .and_then(|shell| shell.as_mut().ok())
            .ok_or("Settings shell is unavailable".to_owned())
            .and_then(|shell| {
                shell.dispatch(index, value, |request| match request {
                    ShellRequest::Navigate { page } => destinations
                        .iter()
                        .find(|destination| {
                            destination.page != SettingsPage::BluetoothPair
                                && destination.page.to_string() == *page
                        })
                        .map(|destination| SettingsMessage::Navigate(destination.page))
                        .ok_or("Settings destination is unavailable".into()),
                    ShellRequest::NavigateTarget { target } => results
                        .iter()
                        .find(|result| result.available && result.target.as_str() == target)
                        .map(|result| result.message.clone())
                        .ok_or("Settings search target is unavailable".into()),
                    ShellRequest::Search { value }
                        if value.chars().count() <= 256 && !value.chars().any(char::is_control) =>
                    {
                        Ok(SettingsMessage::SidebarSearchChanged(value.clone()))
                    }
                    ShellRequest::Search { .. } => Err("Settings search is too long".into()),
                    ShellRequest::ShowNavigation => Ok(SettingsMessage::ShowNavigation),
                })
            });
        match message {
            Ok(message) => self.handle_settings_message(message),
            Err(error) => tracing::warn!(%error, "Settings shell ignored an invalid action"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_window_and_active_page_render_through_the_jsx_shell() {
        let app = SettingsApp::with_initial_page(SettingsPage::About);
        let frame = app.build_ui(1100.0, 800.0);
        let shell = app.settings_shell.borrow();
        let shell = shell.as_ref().unwrap().as_ref().unwrap();
        assert!(
            shell.last_theme.is_some(),
            "Settings used its native fallback"
        );
        assert!(
            frame
                .accessibility_nodes()
                .iter()
                .any(|node| node.id.as_str().ends_with("settings-content"))
        );
    }
}
