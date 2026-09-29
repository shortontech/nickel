//! JSX-owned Settings window and navigation layout.

use nickel_i18n::Localizer;
use nickel_plugin_presentation::{components::PluginImages, css::StyleSheet, page::JsxPage};
use nickel_ui::SettingsSearchEntry;
use nickel_ui::{AnyView, SemanticTheme};
use serde::Deserialize;
use serde_json::Value;

use crate::navigation::{Destination, SearchDefinition};
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
    last_navigation_data: Option<String>,
    destinations: Vec<Destination>,
    search: Vec<SearchDefinition>,
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
            last_navigation_data: None,
            destinations: Vec::new(),
            search: Vec::new(),
        })
    }

    pub(super) fn retained_bytes(&self) -> usize {
        self.page.retained_bytes()
            + self.stylesheet.estimated_retained_bytes() as usize
            + self
                .last_data
                .as_ref()
                .map_or(0, |data| data.to_string().len())
            + self
                .last_navigation_data
                .as_ref()
                .map_or(0, String::capacity)
            + self.destinations.capacity() * std::mem::size_of::<Destination>()
            + self
                .destinations
                .iter()
                .map(Destination::retained_bytes)
                .sum::<usize>()
            + self.search.capacity() * std::mem::size_of::<SearchDefinition>()
            + self
                .search
                .iter()
                .map(SearchDefinition::retained_bytes)
                .sum::<usize>()
    }

    pub(super) fn navigation(
        &mut self,
        localizer: &Localizer,
    ) -> Result<(Vec<Destination>, Vec<SettingsSearchEntry<SettingsMessage>>), String> {
        let data = crate::navigation::projection(localizer);
        let serialized = serde_json::to_string(&data).map_err(|error| error.to_string())?;
        if self.last_navigation_data.as_deref() != Some(&serialized) {
            (self.destinations, self.search) = self.page.evaluate_with_data(
                &data,
                "__nickelRender(SettingsNavigation)",
                crate::navigation::parse_tree,
            )?;
            self.last_navigation_data = Some(serialized);
        }
        Ok((
            self.destinations.clone(),
            self.search
                .clone()
                .into_iter()
                .map(SearchDefinition::entry)
                .collect(),
        ))
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
        let (destinations, entries) = self.navigation_document();
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
    fn navigation_metadata_reuses_the_shell_context_and_invalidates_stale_views() {
        let mut shell = SettingsShell::new().unwrap();
        let english = Localizer::for_locale(Some("en-US"));
        let german = Localizer::for_locale(Some("de-DE"));
        let (destinations, _) = shell.navigation(&english).unwrap();
        assert_eq!(destinations.len(), 11);
        let data = serde_json::json!({"width":1100,"height":800,"active":true});
        shell.page.render(&data).unwrap();
        assert!(shell.page.node().is_some());
        shell.navigation(&english).unwrap();
        assert!(shell.page.node().is_some());
        shell.navigation(&german).unwrap();
        assert!(shell.page.node().is_none());
    }

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
