//! Ordinary Settings pages rendered by the shared JavaScript component runtime.

use nickel_i18n::Localizer;
use nickel_plugin_runtime::JsxRuntime;
use nickel_ui::{AnyView, SemanticTheme, SettingsCard, SettingsRow};
use serde_json::{Value, json};

use crate::SettingsMessage;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Page {
    title: String,
    description: String,
    rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Row {
    label: String,
    value: String,
}

impl Page {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-card") {
            return Err("Settings page root must be a settings-card".into());
        }
        let title = required_text(value, "label")?;
        let description = required_text(value, "value")?;
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Settings card must contain rows")?;
        if children.len() > 32 {
            return Err("Settings card has too many rows".into());
        }
        let rows = children
            .iter()
            .map(|child| {
                if child.get("kind").and_then(Value::as_str) != Some("settings-row") {
                    return Err("Settings card child must be a settings-row".into());
                }
                Ok(Row {
                    label: required_text(child, "label")?,
                    value: required_text(child, "value")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self {
            title,
            description,
            rows,
        })
    }

    fn view(&self, theme: SemanticTheme) -> AnyView<SettingsMessage> {
        let rows = self
            .rows
            .iter()
            .map(|row| SettingsRow::new(theme, &row.label, &row.value))
            .collect::<Vec<_>>();
        AnyView::new(SettingsCard::titled(theme, &self.title, &self.description).children(rows))
    }
}

fn required_text(value: &Value, key: &str) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Settings component is missing {key}"))?;
    if text.chars().count() > 256 {
        return Err(format!("Settings component {key} is too long"));
    }
    Ok(text.to_owned())
}

pub(super) struct OrdinaryPages {
    runtime: JsxRuntime,
    last_data: Option<String>,
    page: Option<Page>,
}

impl OrdinaryPages {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(
                include_str!("../../../assets/plugin-runtime/settings-pages.js"),
                None,
            )?,
            last_data: None,
            page: None,
        })
    }

    fn render(
        &mut self,
        data: Value,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        let data = serde_json::to_string(&data).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&data) {
            self.runtime.set_data(&data)?;
            let page = self.runtime.render("__nickelRender()", Page::parse)?;
            self.page = Some(page);
            self.last_data = Some(data);
        }
        Ok(self
            .page
            .as_ref()
            .ok_or("Settings page is unavailable")?
            .view(theme))
    }

    pub(super) fn render_keyboard(
        &mut self,
        localizer: &Localizer,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        self.render(
            json!({
                "title": localizer.text("settings-keyboard-card-title"),
                "description": localizer.text("settings-keyboard-card-description"),
                "rows": [
                    {"id":"launcher", "label":localizer.text("settings-keyboard-open-launcher"), "value":"Super"},
                    {"id":"search", "label":localizer.text("settings-keyboard-search"), "value":localizer.text("settings-keyboard-search-value")},
                    {"id":"navigate", "label":localizer.text("settings-keyboard-navigate"), "value":"Arrow keys · Tab · Shift+Tab"},
                    {"id":"activate", "label":localizer.text("settings-keyboard-activate"), "value":"Enter"},
                    {"id":"back", "label":localizer.text("settings-keyboard-back"), "value":"Escape"},
                    {"id":"workspaces", "label":localizer.text("settings-keyboard-workspaces"), "value": if cfg!(target_os = "windows") {
                        localizer.text("settings-keyboard-workspaces-unavailable")
                    } else {
                        localizer.text("settings-keyboard-workspaces-value")
                    }}
                ]
            }),
            theme,
        )
    }

    pub(super) fn render_about(
        &mut self,
        localizer: &Localizer,
        theme: SemanticTheme,
    ) -> Result<AnyView<SettingsMessage>, String> {
        self.render(
            json!({
                "title": localizer.text("settings-about-card-title"),
                "description": localizer.text("settings-about-card-description"),
                "rows": [
                    {"id":"version", "label":localizer.text("settings-about-version"), "value":env!("CARGO_PKG_VERSION")},
                    {"id":"platform", "label":localizer.text("settings-about-platform"), "value":format!("{} · {}", std::env::consts::OS, std::env::consts::ARCH)}
                ]
            }),
            theme,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundled_settings_pages_render_as_native_cards() {
        let mut pages = OrdinaryPages::new().unwrap();
        let theme = crate::semantic_theme(nickel_core::theme::ThemePalette::from_appearance(
            nickel_core::theme::Appearance::default(),
        ));
        let localizer = Localizer::system();
        let keyboard = pages.render_keyboard(&localizer, theme).unwrap();
        let about = pages.render_about(&localizer, theme).unwrap();
        let _ = (keyboard, about);
        assert_eq!(pages.page.as_ref().unwrap().rows.len(), 2);
    }

    #[test]
    fn invalid_settings_component_is_rejected() {
        assert!(Page::parse(&json!({"kind":"settings-card","label":"A","value":"B","children":[{"kind":"button","label":"Apply"}]})).is_err());
    }
}
