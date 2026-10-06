//! Ordinary package page behavior through the shared module graph and capability ABI.
use super::{JsxModuleGraph, JsxRuntime, ModuleSource};
use serde_json::{Value, json};

fn runtime(page: &str, writable: bool) -> JsxRuntime {
    let graph = JsxModuleGraph::new("main.js", [
        ModuleSource { path:"main.js", source:"import {Preferences, IdlePreferences, PreferredApplications, FileArtwork} from './Preferences.js';\nexport function App(){const pages={shell:Preferences,idle:IdlePreferences,apps:PreferredApplications,icons:FileArtwork}; return h(Window,{id:'settings',width:1100,height:800},h(pages[nickel.data.testPage],{}));}" },
        ModuleSource { path:"Preferences.js", source:include_str!("../../../assets/plugins/nickel-default/src/Preferences.js") },
        ModuleSource { path:"styles/preferences.css", source:include_str!("../../../assets/plugins/nickel-default/src/styles/preferences.css") },
    ]).unwrap();
    let data = json!({"testPage":page,"preferences":{"available":true,"writable":writable,"revision":"0123456789abcdef",
        "configured":{"barOnAllDisplays":true,"allWindowsOnEveryBar":false,"desktopCount":4,"preferredTerminal":null,"preferredFileManager":"files.desktop","fileIconProvider":"system","fileIconTheme":"temporarily-missing","idleDimSeconds":300,"idleLockSeconds":900,"idleSuspendSeconds":null},
        "applications":[{"id":"terminal.desktop"},{"id":"files.desktop"}],"iconThemes":["Papirus"],"unavailableSelections":{"preferredTerminal":true,"preferredFileManager":false,"fileIconTheme":true}},
        "applications":[{"id":"terminal.desktop","name":"Terminal"},{"id":"files.desktop","name":"Files"}]}).to_string();
    JsxRuntime::new_modules(&graph, Some(&data)).unwrap()
}
fn node<'a>(value: &'a Value, id: &str) -> Option<&'a Value> {
    if value["id"] == id {
        return Some(value);
    }
    value["children"]
        .as_array()?
        .iter()
        .find_map(|child| node(child, id))
}
fn render(runtime: &mut JsxRuntime) -> Value {
    runtime
        .render("__nickelRender()", |value| Ok(value.clone()))
        .unwrap()
}
fn click(runtime: &mut JsxRuntime, id: &str) -> Value {
    let tree = render(runtime);
    let action = node(&tree, id).unwrap()["action"].as_u64().unwrap();
    runtime
        .render(&format!("__nickelDispatch({action})"), |value| {
            Ok(value.clone())
        })
        .unwrap();
    runtime.take_effects().unwrap().pop().unwrap()
}

#[test]
fn preferences_package_pages_register_and_emit_atomic_desktop_and_idle_patches() {
    let mut runtime = runtime("shell", true);
    let mut registry = nickel_core::settings_registry::SettingsRegistry::default();
    runtime
        .publish_settings(&mut registry, "nickel-default")
        .unwrap();
    let mut definitions = registry
        .settings_pages_snapshot()
        .pages
        .into_iter()
        .map(|page| page.registration.id)
        .collect::<Vec<_>>();
    definitions.sort();
    assert_eq!(
        definitions,
        [
            "file-artwork",
            "idle-preferences",
            "preferred-applications",
            "shell-preferences"
        ]
    );
    let effect = click(&mut runtime, "preferences-desktop-count-8");
    assert_eq!(
        effect["transaction"]["changedFields"],
        json!(["desktopCount"])
    );
    assert_eq!(effect["transaction"]["requested"]["desktopCount"], 8);
    assert_eq!(
        effect["transaction"]["requested"]["preferredTerminal"],
        Value::Null
    );
    let mut idle = self::runtime("idle", true);
    let effect = click(&mut idle, "preferences-idleLockSeconds-1800");
    assert_eq!(
        effect["transaction"]["changedFields"],
        json!(["idleLockSeconds"])
    );
    assert_eq!(effect["transaction"]["requested"]["idleLockSeconds"], 1800);
    let effect = click(&mut idle, "preferences-idleSuspendSeconds-null");
    assert_eq!(
        effect["transaction"]["requested"]["idleSuspendSeconds"],
        Value::Null
    );
}

#[test]
fn preferences_package_pages_preserve_unavailable_choices_until_explicit_selection() {
    let mut apps = runtime("apps", true);
    let tree = render(&mut apps);
    assert_eq!(
        node(&tree, "preferences-preferredTerminal-system").unwrap()["state"],
        "unselected"
    );
    assert!(
        tree.to_string()
            .contains("configured application is unavailable")
    );
    assert!(tree.to_string().contains("Current: Files"));
    assert!(apps.take_effects().unwrap().is_empty());
    let effect = click(&mut apps, "preferences-preferredTerminal-system");
    assert_eq!(
        effect["transaction"]["changedFields"],
        json!(["preferredTerminal"])
    );
    assert_eq!(
        effect["transaction"]["requested"]["preferredTerminal"],
        Value::Null
    );
    let effect = click(&mut apps, "preferences-preferredTerminal-choice-0");
    assert_eq!(
        effect["transaction"]["requested"]["preferredTerminal"],
        "terminal.desktop"
    );
    let mut icons = runtime("icons", true);
    let tree = render(&mut icons);
    assert!(
        tree.to_string()
            .contains("Configured theme temporarily-missing is unavailable")
    );
    assert!(icons.take_effects().unwrap().is_empty());
    let effect = click(&mut icons, "preferences-file-theme-choice-0");
    assert_eq!(
        effect["transaction"]["requested"]["fileIconTheme"],
        "Papirus"
    );
    assert_eq!(
        effect["transaction"]["requested"]["fileIconProvider"],
        "system"
    );
    assert_eq!(
        effect["transaction"]["changedFields"],
        json!(["fileIconProvider", "fileIconTheme"])
    );
}

#[test]
fn preferred_terminal_dropdown_filters_games_keeps_icons_and_allows_other_providers() {
    let mut apps = runtime("apps", true);
    apps.render("nickel.data.preferences.applications.push({id:'org.kde.konsole.desktop'},{id:'steam-disgaea.desktop'},{id:'custom.desktop'},{id:'emacs-term.desktop'}); nickel.data.applications.push({id:'org.kde.konsole.desktop',name:'Konsole',icon:'application:konsole'},{id:'steam-disgaea.desktop',name:'Disgaea 5 Complete',icon:'application:disgaea'},{id:'custom.desktop',name:'Custom Console',icon:'application:custom'},{id:'emacs-term.desktop',name:'Emacs (Terminal)',icon:'application:emacs',launchClass:'terminal'}); __nickelRender()", |value| Ok(value.clone())).unwrap();
    let tree = render(&mut apps);
    let select = node(&tree, "preferences-preferredTerminal-select").unwrap();
    assert_eq!(select["kind"], "select");
    assert!(!select.to_string().contains("Disgaea"));
    assert!(!select.to_string().contains("Emacs"));
    assert!(!select.to_string().contains("Custom Console"));
    assert!(select.to_string().contains("application:konsole"));
    let action = node(&tree, "preferences-preferredTerminal-show-all").unwrap()["action"]
        .as_u64()
        .unwrap();
    apps.render(&format!("__nickelDispatch({action})"), |value| {
        Ok(value.clone())
    })
    .unwrap();
    let tree = render(&mut apps);
    assert!(
        node(&tree, "preferences-preferredTerminal-select")
            .unwrap()
            .to_string()
            .contains("Custom Console")
    );
    assert!(apps.take_effects().unwrap().is_empty());
    apps.render(
        "nickel.data.preferences.configured.preferredTerminal='custom.desktop'; __nickelRender()",
        |value| Ok(value.clone()),
    )
    .unwrap();
    let action = node(&tree, "preferences-preferredTerminal-show-all").unwrap()["action"]
        .as_u64()
        .unwrap();
    apps.render(&format!("__nickelDispatch({action})"), |value| {
        Ok(value.clone())
    })
    .unwrap();
    let tree = render(&mut apps);
    assert!(
        node(&tree, "preferences-preferredTerminal-select")
            .unwrap()
            .to_string()
            .contains("Custom Console")
    );
}

#[test]
fn preferences_package_pages_disable_writes_without_control_availability() {
    for (page, id) in [
        ("shell", "preferences-desktop-count-8"),
        ("idle", "preferences-idleLockSeconds-1800"),
        ("apps", "preferences-preferredTerminal-system"),
        ("icons", "preferences-file-icons-system"),
    ] {
        let mut runtime = runtime(page, false);
        let tree = render(&mut runtime);
        assert_eq!(node(&tree, id).unwrap()["disabled"], true);
        assert!(runtime.take_effects().unwrap().is_empty());
    }
}

#[test]
fn preferences_package_pages_validate_custom_idle_values_and_show_unavailability() {
    let mut runtime = runtime("idle", true);
    for draft in ["29", "30.5", "604801", "not a number", "75"] {
        let tree = render(&mut runtime);
        let action = node(&tree, "preferences-idleDimSeconds-custom").unwrap()["action"]
            .as_u64()
            .unwrap();
        let value = serde_json::to_string(draft).unwrap();
        let next = runtime
            .render(&format!("__nickelDispatch({action},{value})"), |value| {
                Ok(value.clone())
            })
            .unwrap();
        assert_eq!(
            node(&next, "preferences-idleDimSeconds-apply").unwrap()["disabled"],
            draft != "75"
        );
        assert!(runtime.take_effects().unwrap().is_empty());
    }
    let effect = click(&mut runtime, "preferences-idleDimSeconds-apply");
    assert_eq!(effect["transaction"]["requested"]["idleDimSeconds"], 75);
    assert_eq!(
        effect["transaction"]["changedFields"],
        json!(["idleDimSeconds"])
    );
    runtime.set_data(r#"{"testPage":"idle","preferences":{"available":false,"reason":"Preferences storage is unavailable"}}"#).unwrap();
    let tree = render(&mut runtime);
    assert!(
        tree.to_string()
            .contains("Preferences storage is unavailable")
    );
    assert!(node(&tree, "preferences-idleDimSeconds-apply").is_none());
    assert!(runtime.take_effects().unwrap().is_empty());
}
