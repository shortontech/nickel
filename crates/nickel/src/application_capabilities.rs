//! Bounded application inventory and independent native search for authorized packages.
use crate::{launcher::Launcher, model::Application};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, HashSet};

pub(crate) const APPLICATION_LIMIT: usize = 256;
pub(crate) fn validate_query(query: &str) -> Result<(), String> {
    if query.chars().count() > 512 || query.contains('\0') {
        Err("application search query exceeds its bounds".into())
    } else {
        Ok(())
    }
}

pub(crate) fn icon_asset(id: &str) -> String {
    format!("application:{:x}", Sha256::digest(id.as_bytes()))
}

fn item(launcher: &Launcher, application: &Application) -> Value {
    let place = launcher.is_place_application(application.id());
    let path = application
        .launch_command()
        .and_then(|command| {
            if place {
                command.get(1)
            } else {
                command.first()
            }
        })
        .filter(|path| std::path::Path::new(path).is_absolute());
    let last_used = launcher
        .preferences()
        .recents()
        .iter()
        .find(|id| application.matches_native_id(id))
        .and_then(|id| launcher.preferences().last_used(id));
    json!({
        "id":application.id(), "name":application.name().chars().take(120).collect::<String>(),
        "icon":icon_asset(application.id()),
        "pinned":launcher.is_pinned(application.id()),
        "pinOrder":launcher.preferences().favorites().iter().position(|id| application.matches_native_id(id)),
        "recentOrder":launcher.preferences().recents().iter().position(|id| application.matches_native_id(id)),
        "kind":if launcher.is_place_application(application.id()) { "place" } else { "application" },
        "path":path.map(|path| path.chars().take(1024).collect::<String>()),
        "description":application.description(),
        "lastUsedUnixSeconds":last_used,
        "launchClass":format!("{:?}", application.launch_class()).to_lowercase(),
    })
}
fn valid(application: &&Application) -> bool {
    !application.id().is_empty() && application.id().len() <= 256
}

pub(crate) fn list(launcher: &Launcher) -> Value {
    let mut seen = HashSet::new();
    let applications = launcher
        .place_applications()
        .chain(launcher.favorite_applications())
        .chain(launcher.recent_applications())
        .chain(launcher.applications());
    Value::Array(
        applications
            .filter(valid)
            .filter(|application| seen.insert(application.id()))
            .take(APPLICATION_LIMIT)
            .map(|application| item(launcher, application))
            .collect(),
    )
}

/// Add running application identities admitted by the platform window registry.
/// Protected shell utilities never enter that registry. These entries describe
/// existing applications; launch remains restricted to the discovered catalog.
pub(crate) fn include_running(launcher: &Launcher, windows: &[crate::model::OpenWindow]) -> Value {
    let mut items = Vec::new();
    let mut seen = HashSet::new();
    for window in windows {
        let Some(id) = window.application_id.as_ref().map(|id| id.as_str()) else {
            continue;
        };
        if items.len() >= APPLICATION_LIMIT {
            break;
        }
        if !id.is_empty() && id.len() <= 256 {
            if let Some(application) = launcher
                .applications()
                .find(|application| application.matches_native_id(id))
            {
                if !seen.insert(application.id().to_owned()) {
                    continue;
                }
                items.push(item(launcher, application));
                continue;
            }
            if !seen.insert(id.to_owned()) {
                continue;
            }
            items.push(
                json!({"id":id, "name":window.title.chars().take(120).collect::<String>(),
                "icon":icon_asset(id), "pinned":launcher.is_pinned(id),
                "pinOrder":launcher.preferences().favorites().iter().position(|item| item == id),
                "recentOrder":launcher.preferences().recents().iter().position(|item| item == id),
                "kind":"application", "launchClass":"running", "canLaunch":false, "canPin":false}),
            );
        }
    }
    for item in list(launcher).as_array().expect("application list") {
        if items.len() >= APPLICATION_LIMIT {
            break;
        }
        if let Some(id) = item["id"].as_str()
            && seen.insert(id.to_owned())
        {
            items.push(item.clone());
        }
    }
    Value::Array(items)
}

#[derive(Default)]
pub(crate) struct ApplicationSearch {
    queries: BTreeMap<String, String>,
}
impl ApplicationSearch {
    pub(crate) fn set_query(&mut self, package: &str, query: String) -> Result<(), String> {
        validate_query(&query)?;
        if !self.queries.contains_key(package) && self.queries.len() >= 128 {
            return Err("application search package limit reached".into());
        }
        self.queries.insert(package.into(), query);
        Ok(())
    }
    pub(crate) fn retire(&mut self, package: &str) {
        self.queries.remove(package);
    }
    pub(crate) fn snapshot(&self, launcher: &Launcher, package: &str) -> Value {
        let query = self
            .queries
            .get(package)
            .map(String::as_str)
            .unwrap_or_default();
        let matches = if query.is_empty() {
            Vec::new()
        } else {
            launcher.search(query, usize::MAX)
        };
        let matches = matches.into_iter().filter(valid).collect::<Vec<_>>();
        let settings = destination_matches(
            query,
            &[
                (
                    "appearance",
                    "Appearance",
                    "Theme, colors, wallpaper, and transparency",
                ),
                (
                    "display",
                    "Displays",
                    "Resolution, scaling, and monitor arrangement",
                ),
                (
                    "network",
                    "Wi-Fi and network",
                    "Internet and wireless connections",
                ),
                (
                    "bluetooth",
                    "Bluetooth",
                    "Connect and manage Bluetooth devices",
                ),
                (
                    "default-apps",
                    "Default applications",
                    "Choose your default browser, terminal, and file manager",
                ),
                (
                    "keyboard-shortcuts",
                    "Keyboard shortcuts",
                    "View shell keyboard shortcuts",
                ),
                (
                    "nickel-bar",
                    "Desktop and taskbar",
                    "Configure Nickel Bar and desktop behavior",
                ),
                ("plugins", "Plugins", "Manage shell plugins"),
                (
                    "optional-features",
                    "Optional features",
                    "Codex integration and on-screen keyboard",
                ),
                (
                    "about",
                    "About Nickel",
                    "System and application information",
                ),
            ],
            "setting",
        );
        let actions = destination_matches(
            query,
            &[
                ("run", "Run a command", "Open the command dialog"),
                ("settings", "Open Settings", "Configure Nickel"),
            ],
            "action",
        );
        json!({"available":true,"query":query,"settingsResults":settings,"actionResults":actions,"total":matches.len(),"truncated":matches.len()>APPLICATION_LIMIT,
            "catalogTruncated":launcher.applications().count()>APPLICATION_LIMIT,
            "nativeProjectsAvailable":launcher.codex_available(),
            "results":matches.into_iter().take(APPLICATION_LIMIT).map(|application|item(launcher, application)).collect::<Vec<_>>()})
    }
}

/// Small native destination catalogs share the application's fuzzy matcher.
fn destination_matches(query: &str, entries: &[(&str, &str, &str)], kind: &str) -> Vec<Value> {
    use nucleo_matcher::{
        Config, Matcher,
        pattern::{AtomKind, CaseMatching, Normalization, Pattern},
    };
    if query.is_empty() {
        return Vec::new();
    }
    let pattern = Pattern::new(
        query,
        CaseMatching::Ignore,
        Normalization::Smart,
        AtomKind::Fuzzy,
    );
    let mut matcher = Matcher::new(Config::DEFAULT);
    let text = entries
        .iter()
        .map(|(_, name, description)| format!("{name} {description}"))
        .collect::<Vec<_>>();
    pattern
        .match_list(text.iter().map(String::as_str), &mut matcher)
        .into_iter()
        .filter_map(|(matched, _)| text.iter().position(|value| value == matched))
        .map(|index| {
            let (destination, name, description) = entries[index];
            json!({"id":format!("{kind}:{destination}"),"kind":kind,"destination":destination,
                "name":name,"description":description,"canPin":false})
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn launcher_details_expose_real_metadata_and_destinations_without_fabricated_times() {
        let path = if cfg!(target_os = "windows") {
            r"C:\Windows\System32\wt.exe"
        } else {
            "/usr/bin/terminal"
        };
        let application = Application::new(
            "terminal".into(),
            "Terminal".into(),
            None,
            None,
            Some(vec![path.into()]),
        )
        .with_description(Some("Run commands and scripts"));
        let mut launcher = Launcher::new(vec![application]);
        let before = list(&launcher);
        assert_eq!(before[0]["description"], "Run commands and scripts");
        assert_eq!(before[0]["path"], path);
        assert!(before[0]["lastUsedUnixSeconds"].is_null());
        launcher.record_launch("terminal");
        assert!(list(&launcher)[0]["lastUsedUnixSeconds"].as_u64().is_some());
        let mut search = ApplicationSearch::default();
        search.set_query("shell", "terminal".into()).unwrap();
        let results = search.snapshot(&launcher, "shell");
        assert_eq!(results["results"][0]["id"], "terminal");
        assert!(
            results["settingsResults"]
                .as_array()
                .unwrap()
                .iter()
                .any(|item| item["destination"] == "default-apps" && item["kind"] == "setting")
        );
        search.set_query("shell", "command".into()).unwrap();
        assert_eq!(
            search.snapshot(&launcher, "shell")["actionResults"][0]["destination"],
            "run"
        );
        search.set_query("shell", String::new()).unwrap();
        assert_eq!(
            search.snapshot(&launcher, "shell")["settingsResults"],
            json!([])
        );
    }
    fn app(id: &str, name: &str) -> Application {
        Application::new(id.into(), name.into(), None, None, None)
    }
    #[test]
    fn full_catalog_keeps_running_native_identity_and_prefers_discovered_metadata() {
        let native = "io.nickel.codex.project.boundary";
        let mut applications = (0..APPLICATION_LIMIT)
            .map(|index| app(&format!("app-{index}"), "App"))
            .collect::<Vec<_>>();
        let window = crate::model::OpenWindow {
            id: crate::model::WindowId(71),
            application_id: Some(crate::model::ApplicationId::new(native)),
            active: true,
            title: "Running project".into(),
            state: crate::model::WindowState::default(),
        };
        let running = include_running(
            &Launcher::new(applications.clone()),
            std::slice::from_ref(&window),
        );
        assert_eq!(running.as_array().unwrap().len(), APPLICATION_LIMIT);
        assert_eq!(running[0]["id"], native);
        assert_eq!(running[0]["canPin"], false);
        applications.push(app(native, "Discovered project"));
        let discovered = include_running(&Launcher::new(applications), &[window.clone(), window]);
        assert_eq!(discovered.as_array().unwrap().len(), APPLICATION_LIMIT);
        assert_eq!(discovered[0]["name"], "Discovered project");
        assert!(discovered[0].get("canLaunch").is_none());
        assert_eq!(
            discovered
                .as_array()
                .unwrap()
                .iter()
                .filter(|item| item["id"] == native)
                .count(),
            1
        );
    }

    #[test]
    fn running_native_alias_reuses_pinned_catalog_identity() {
        let application = app("windows-shortcut:chrome.lnk", "Google Chrome")
            .with_identity_alias("windows-exe:c:\\program files\\google\\chrome\\chrome.exe");
        let mut launcher = Launcher::new(vec![application]);
        launcher.set_pins(vec![("windows-shortcut:chrome.lnk".into(), 0)]);
        let window = crate::model::OpenWindow {
            id: crate::model::WindowId(72),
            application_id: Some(crate::model::ApplicationId::new(
                "windows-exe:C:\\Program Files\\Google\\Chrome\\chrome.exe",
            )),
            active: true,
            title: "Chrome".into(),
            state: crate::model::WindowState::default(),
        };

        let applications = include_running(&launcher, &[window]);

        assert_eq!(applications.as_array().unwrap().len(), 1);
        assert_eq!(applications[0]["id"], "windows-shortcut:chrome.lnk");
        assert_eq!(applications[0]["pinned"], true);
    }
    #[test]
    fn package_search_uses_native_ranking_without_changing_launcher_query_and_refreshes_pins() {
        let mut launcher = Launcher::new(vec![
            app("editor", "Text Editor"),
            app("terminal", "Terminal"),
            app("native", "Nickel File Manager"),
        ]);
        launcher.set_query("edtr");
        let expected = launcher.result_at(0).unwrap().id().to_owned();
        launcher.set_query("Terminal");
        let mut search = ApplicationSearch::default();
        search.set_query("one", "edtr".into()).unwrap();
        search.set_query("two", "file".into()).unwrap();
        assert_eq!(
            search.snapshot(&launcher, "one")["results"][0]["id"],
            expected
        );
        assert_eq!(
            search.snapshot(&launcher, "two")["results"][0]["id"],
            "native"
        );
        assert_eq!(launcher.query(), "Terminal");
        launcher.toggle_pin("editor");
        assert_eq!(
            search.snapshot(&launcher, "one")["results"][0]["pinned"],
            true
        );
        search.retire("one");
        assert_eq!(search.snapshot(&launcher, "one")["query"], "");
        assert!(search.set_query("one", "x".repeat(513)).is_err());
        assert!(validate_query("\0").is_err());
    }
    #[test]
    fn inventory_and_search_are_bounded_and_preserve_place_identity() {
        let mut launcher = Launcher::new(
            (0..300)
                .map(|index| app(&format!("app-{index}"), &format!("Editor {index}")))
                .collect(),
        );
        launcher.set_places(vec![app("place-home", "Home")]);
        let mut search = ApplicationSearch::default();
        search.set_query("one", "editor".into()).unwrap();
        let snapshot = search.snapshot(&launcher, "one");
        assert_eq!(
            snapshot["results"].as_array().unwrap().len(),
            APPLICATION_LIMIT
        );
        assert_eq!(snapshot["truncated"], true);
        assert_eq!(list(&launcher).as_array().unwrap().len(), APPLICATION_LIMIT);
        assert!(icon_asset(&"x".repeat(256)).len() <= 128);
        search.set_query("one", "home".into()).unwrap();
        assert_eq!(
            search.snapshot(&launcher, "one")["results"][0]["kind"],
            "place"
        );
    }
}
