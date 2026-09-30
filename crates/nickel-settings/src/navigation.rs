//! JSX declaration of the Settings navigation destinations.

use std::collections::HashSet;

use nickel_i18n::Localizer;
use serde_json::{Value, json};

use crate::{SettingsApp, SettingsMessage, SettingsPage};
use nickel_ui::SettingsSearchEntry;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct Destination {
    pub(super) page: SettingsPage,
    pub(super) label: String,
    pub(super) title: String,
    pub(super) subtitle: String,
    pub(super) section: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct SearchDefinition {
    target: String,
    page_label: String,
    section: String,
    control: String,
}

impl SearchDefinition {
    pub(super) fn retained_bytes(&self) -> usize {
        self.target.capacity()
            + self.page_label.capacity()
            + self.section.capacity()
            + self.control.capacity()
    }

    fn parse(value: &Value) -> Result<Self, String> {
        if value["kind"] != "settings-search-entry" {
            return Err("Settings search child must be an entry".into());
        }
        let target = text(value, "id")?;
        if search_message(&target).is_none() {
            return Err(format!("Unknown Settings search target {target:?}"));
        }
        Ok(Self {
            target,
            page_label: text(value, "state")?,
            section: text(value, "value")?,
            control: text(value, "label")?,
        })
    }

    pub(super) fn entry(self) -> SettingsSearchEntry<SettingsMessage> {
        let message = search_message(&self.target).expect("validated search target");
        SettingsSearchEntry::new(
            self.page_label,
            self.section,
            self.control,
            self.target,
            message,
        )
    }
}

fn search_message(target: &str) -> Option<SettingsMessage> {
    let page = match target {
        "appearance-mode-system"
        | "appearance-hue"
        | "appearance-intensity"
        | "appearance-transparency"
        | "appearance-animations" => SettingsPage::Appearance,
        "optional-feature-codex-enabled" | "on-screen-keyboard-mode" => {
            SettingsPage::OptionalFeatures
        }
        "plugins-page" => return Some(SettingsMessage::Navigate(SettingsPage::Plugins)),
        _ => return None,
    };
    Some(SettingsMessage::NavigateTarget(page, target.into()))
}

impl Destination {
    pub(super) fn retained_bytes(&self) -> usize {
        self.label.capacity()
            + self.title.capacity()
            + self.subtitle.capacity()
            + self.section.capacity()
    }

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

pub(super) fn parse_tree(
    value: &Value,
) -> Result<(Vec<Destination>, Vec<SearchDefinition>), String> {
    if value.get("kind").and_then(Value::as_str) != Some("settings-navigation") {
        return Err("Settings navigation root is invalid".into());
    }
    let children = value
        .get("children")
        .and_then(Value::as_array)
        .ok_or("Settings navigation has no destinations")?;
    if children.len() != 12 || children[11]["kind"] != "settings-search-index" {
        return Err("Settings navigation must declare destinations and search".into());
    }
    let destinations = children[..11]
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
    let search = children[11]["children"]
        .as_array()
        .ok_or("Settings search index has no entries")?;
    if search.len() > 32 {
        return Err("Settings search index is too large".into());
    }
    let definitions = search
        .iter()
        .map(SearchDefinition::parse)
        .collect::<Result<Vec<_>, _>>()?;
    if definitions
        .iter()
        .map(|entry| entry.target.as_str())
        .collect::<HashSet<_>>()
        .len()
        != definitions.len()
    {
        return Err("Settings search has duplicate targets".into());
    }
    Ok((destinations, definitions))
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
        "search": {
            "interface":localizer.text("settings-interface-settings"),
            "mode":localizer.text("settings-appearance-mode"),
            "automatic":localizer.text("settings-appearance-automatic"),
            "startingHue":localizer.text("settings-appearance-starting-hue"),
            "colorIntensity":localizer.text("settings-appearance-color-intensity"),
            "reduceTransparency":localizer.text("settings-reduce-transparency"),
            "animations":localizer.text("settings-animations"),
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
    pub(super) fn navigation_document(
        &self,
    ) -> (Vec<Destination>, Vec<SettingsSearchEntry<SettingsMessage>>) {
        if !self.settings_jsx_enabled {
            return (recovery(&self.localizer), recovery_search(&self.localizer));
        }
        self.settings_shell
            .borrow_mut()
            .get_or_insert_with(|| {
                self.shared_settings_page(crate::settings_package::Script::Shell)
                    .and_then(crate::settings_shell::SettingsShell::new_with_page)
            })
            .as_mut()
            .map_err(|error| error.clone())
            .and_then(|shell| shell.navigation(&self.localizer))
            .unwrap_or_else(|_| (recovery(&self.localizer), recovery_search(&self.localizer)))
    }

    pub(super) fn navigation_destinations(&self) -> Vec<Destination> {
        self.navigation_document().0
    }

    pub(super) fn navigation_search_entries(&self) -> Vec<SettingsSearchEntry<SettingsMessage>> {
        self.navigation_document().1
    }
}

fn recovery_search(localizer: &Localizer) -> Vec<SettingsSearchEntry<SettingsMessage>> {
    let appearance = localizer.text("settings-nav-appearance");
    let section = localizer.text("settings-interface-settings");
    let mut entries = [
        (
            "appearance-mode-system",
            localizer.text("settings-appearance-mode"),
            localizer.text("settings-appearance-automatic"),
        ),
        (
            "appearance-hue",
            section.clone(),
            localizer.text("settings-appearance-starting-hue"),
        ),
        (
            "appearance-intensity",
            section.clone(),
            localizer.text("settings-appearance-color-intensity"),
        ),
        (
            "appearance-transparency",
            section.clone(),
            localizer.text("settings-reduce-transparency"),
        ),
        (
            "appearance-animations",
            section,
            localizer.text("settings-animations"),
        ),
    ]
    .into_iter()
    .map(|(target, section, control)| {
        SettingsSearchEntry::new(
            appearance.clone(),
            section,
            control,
            target,
            search_message(target).expect("recovery search target"),
        )
    })
    .collect::<Vec<_>>();
    entries.push(SettingsSearchEntry::new(
        "Optional Features",
        "Codex",
        "Use Codex projects and conversations in Nickel",
        "optional-feature-codex-enabled",
        search_message("optional-feature-codex-enabled").unwrap(),
    ));
    entries.push(SettingsSearchEntry::new(
        "Optional Features",
        "On-screen keyboard",
        "Screen keyboard · touch keyboard · virtual keyboard",
        "on-screen-keyboard-mode",
        search_message("on-screen-keyboard-mode").unwrap(),
    ));
    entries.push(SettingsSearchEntry::new(
        "Plugins",
        "Plugin memory and permissions",
        "Enable or disable shell plugins and review their access",
        "plugins-page",
        search_message("plugins-page").unwrap(),
    ));
    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn jsx_navigation_declares_every_page_once() {
        let mut shell = crate::settings_shell::SettingsShell::new().unwrap();
        let (destinations, search) = shell.navigation(&Localizer::system()).unwrap();
        assert_eq!(destinations.len(), 11);
        assert_eq!(destinations[0].page, SettingsPage::Display);
        assert_eq!(destinations[8].page, SettingsPage::Plugins);
        assert_eq!(destinations.last().unwrap().page, SettingsPage::About);
        assert_eq!(destinations, recovery(&Localizer::system()));
        assert_eq!(search, recovery_search(&Localizer::system()));
    }

    #[test]
    fn malformed_navigation_cannot_invent_a_page() {
        assert!(Destination::parse(&json!({"kind":"settings-destination","id":"superuser","label":"Root","value":"","children":[{"kind":"settings-header","label":"Root","value":""}]})).is_err());
        assert!(
            SearchDefinition::parse(&json!({
                "kind": "settings-search-entry",
                "id": "superuser-password",
                "state": "Security",
                "value": "Account",
                "label": "Password"
            }))
            .is_err()
        );
    }
}
