//! JSX declaration of the Settings navigation destinations.

use std::collections::HashSet;

use nickel_i18n::Localizer;
use nickel_plugin_runtime::JsxRuntime;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsPage};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Destination {
    pub(super) page: SettingsPage,
    pub(super) label: String,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) section: String,
}

impl Destination {
    fn parse(value: &Value) -> Result<Self, String> {
        if value.get("kind").and_then(Value::as_str) != Some("settings-destination") {
            return Err("Settings navigation child must be a destination".into());
        }
        let id = text(value, "id")?;
        let page = match id.as_str() {
            "display" => SettingsPage::Display,
            "bar" => SettingsPage::Bar,
            "appearance" => SettingsPage::Appearance,
            "network" => SettingsPage::Network,
            "bluetooth" => SettingsPage::Bluetooth,
            "bluetooth-pair" => SettingsPage::BluetoothPair,
            "default-apps" => SettingsPage::DefaultApps,
            "optional-features" => SettingsPage::OptionalFeatures,
            "plugins" => SettingsPage::Plugins,
            "keyboard-shortcuts" => SettingsPage::KeyboardShortcuts,
            "about" => SettingsPage::About,
            _ => return Err(format!("Unknown Settings destination {id:?}")),
        };
        let children = value
            .get("children")
            .and_then(Value::as_array)
            .ok_or("Settings destination has no header")?;
        if children.len() != 1
            || children[0].get("kind").and_then(Value::as_str) != Some("settings-header")
        {
            return Err("Settings destination must have one header".into());
        }
        Ok(Self {
            page,
            label: text(value, "label")?,
            title: text(&children[0], "label")?,
            subtitle: text(&children[0], "value")?,
            section: text(value, "value")?,
        })
    }
}

fn text(value: &Value, key: &str) -> Result<String, String> {
    let text = value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Settings navigation is missing {key}"))?;
    if text.chars().count() > 256 {
        return Err(format!("Settings navigation {key} is too long"));
    }
    Ok(text.to_owned())
}

fn parse_tree(value: &Value) -> Result<Vec<Destination>, String> {
    if value.get("kind").and_then(Value::as_str) != Some("settings-navigation") {
        return Err("Settings navigation root is invalid".into());
    }
    let children = value
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Settings navigation has no destinations")?;
    if children.len() != 11 {
        return Err("Settings navigation must declare every destination".into());
    }
    let destinations = children
        .iter()
        .map(Destination::parse)
        .collect::<Result<Vec<_>, _>>()?;
    let unique = destinations
        .iter()
        .map(|destination| destination.page)
        .collect::<HashSet<_>>();
    if unique.len() != destinations.len() {
        return Err("Settings navigation has duplicate destinations".into());
    }
    Ok(destinations)
}

pub(super) struct NavigationPlugin {
    runtime: JsxRuntime,
    last_data: Option<String>,
    destinations: Vec<Destination>,
}

impl NavigationPlugin {
    pub(super) fn new() -> Result<Self, String> {
        Ok(Self {
            runtime: JsxRuntime::new(
                crate::settings_package::source(crate::settings_package::Script::Navigation)?,
                None,
            )?,
            last_data: None,
            destinations: Vec::new(),
        })
    }

    pub(super) fn render(&mut self, localizer: &Localizer) -> Result<Vec<Destination>, String> {
        let data =
            serde_json::to_string(&projection(localizer)).map_err(|error| error.to_string())?;
        if self.last_data.as_deref() != Some(&data) {
            self.runtime.set_data(&data)?;
            self.destinations = self.runtime.render("__nickelRender()", parse_tree)?;
            self.last_data = Some(data);
        }
        Ok(self.destinations.clone())
    }
}

pub(super) fn projection(localizer: &Localizer) -> Value {
    json!({
        "labels": {
            "display": localizer.text("settings-nav-display"),
            "bar": localizer.text("settings-nav-bar"),
            "appearance": localizer.text("settings-nav-appearance"),
            "network": localizer.text("settings-nav-network"),
            "bluetooth": localizer.text("settings-nav-bluetooth"),
            "bluetooth-pair": localizer.text("settings-bluetooth-pair-title"),
            "default-apps": localizer.text("settings-nav-default-apps"),
            "optional-features": "Optional Features",
            "plugins": "Plugins",
            "keyboard-shortcuts": localizer.text("settings-nav-keyboard"),
            "about": localizer.text("settings-nav-about"),
        },
        "headers": {
            "display": {"title":localizer.text("settings-display-title"),"subtitle":localizer.text("settings-display-subtitle")},
            "bar": {"title":localizer.text("settings-bar-title"),"subtitle":localizer.text("settings-bar-subtitle")},
            "appearance": {"title":localizer.text("settings-appearance-title"),"subtitle":localizer.text("settings-appearance-subtitle")},
            "network": {"title":localizer.text("settings-network-title"),"subtitle":localizer.text("settings-network-subtitle")},
            "bluetooth": {"title":localizer.text("settings-bluetooth-title"),"subtitle":localizer.text("settings-bluetooth-subtitle")},
            "bluetooth-pair": {"title":localizer.text("settings-bluetooth-pair-title"),"subtitle":localizer.text("settings-bluetooth-pair-subtitle")},
            "default-apps": {"title":localizer.text("settings-default-apps-title"),"subtitle":localizer.text("settings-default-apps-subtitle")},
            "optional-features": {"title":"Optional Features","subtitle":"Enable integrations and inspect their availability"},
            "plugins": {"title":"Plugins","subtitle":"Review access, memory, and installed shell components"},
            "keyboard-shortcuts": {"title":localizer.text("settings-keyboard-title"),"subtitle":localizer.text("settings-keyboard-subtitle")},
            "about": {"title":localizer.text("settings-about-title"),"subtitle":localizer.text("settings-about-subtitle")},
        },
        "sections": {
            "system":localizer.text("settings-nav-section-system"),
            "personalization":localizer.text("settings-nav-section-personalization"),
            "connectivity":localizer.text("settings-nav-section-connectivity"),
            "support":localizer.text("settings-nav-section-support"),
        },
    })
}

pub(super) fn recovery(localizer: &Localizer) -> Vec<Destination> {
    let data = projection(localizer);
    let labels = &data["labels"];
    let headers = &data["headers"];
    let sections = &data["sections"];
    [
        (SettingsPage::Display, "display", "system"),
        (SettingsPage::Bar, "bar", "personalization"),
        (SettingsPage::Appearance, "appearance", ""),
        (SettingsPage::Network, "network", "connectivity"),
        (SettingsPage::Bluetooth, "bluetooth", ""),
        (SettingsPage::BluetoothPair, "bluetooth-pair", ""),
        (SettingsPage::DefaultApps, "default-apps", ""),
        (SettingsPage::OptionalFeatures, "optional-features", ""),
        (SettingsPage::Plugins, "plugins", ""),
        (
            SettingsPage::KeyboardShortcuts,
            "keyboard-shortcuts",
            "support",
        ),
        (SettingsPage::About, "about", ""),
    ]
    .into_iter()
    .map(|(page, id, section)| Destination {
        page,
        label: labels[id].as_str().unwrap_or(id).into(),
        title: headers[id]["title"].as_str().unwrap_or(id).into(),
        subtitle: headers[id]["subtitle"].as_str().unwrap_or("").into(),
        section: sections[section].as_str().unwrap_or("").into(),
    })
    .collect()
}

impl SettingsApp {
    pub(super) fn navigation_destinations(&self) -> Vec<Destination> {
        if !self.settings_jsx_enabled {
            return recovery(&self.localizer);
        }
        let result = self
            .navigation_plugin
            .borrow_mut()
            .get_or_insert_with(NavigationPlugin::new)
            .as_mut()
            .map_err(|error| error.clone())
            .and_then(|plugin| plugin.render(&self.localizer));
        result.unwrap_or_else(|_| recovery(&self.localizer))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsx_navigation_declares_every_page_once() {
        let mut plugin = NavigationPlugin::new().unwrap();
        let destinations = plugin.render(&Localizer::system()).unwrap();
        assert_eq!(destinations.len(), 11);
        assert_eq!(destinations[0].page, SettingsPage::Display);
        assert_eq!(destinations[8].page, SettingsPage::Plugins);
        assert_eq!(destinations.last().unwrap().page, SettingsPage::About);
        assert_eq!(destinations, recovery(&Localizer::system()));
    }

    #[test]
    fn malformed_navigation_cannot_invent_a_page() {
        assert!(Destination::parse(&json!({"kind":"settings-destination","id":"superuser","label":"Root","value":"","children":[{"kind":"settings-header","label":"Root","value":""}]})).is_err());
    }
}
