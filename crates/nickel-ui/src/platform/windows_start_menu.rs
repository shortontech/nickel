use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

use crate::model::Application;

const START_MENU_RELATIVE: &str = "Microsoft/Windows/Start Menu/Programs";

pub fn load_applications() -> Vec<Application> {
    let mut roots = [env::var_os("APPDATA"), env::var_os("PROGRAMDATA")]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .map(|root| root.join(START_MENU_RELATIVE))
        .collect::<Vec<_>>();
    if let Some(home) = env::var_os("HOME").or_else(|| env::var_os("USERPROFILE")) {
        roots.push(PathBuf::from(home).join("Desktop"));
    }
    load_from_roots(&roots)
}

fn load_from_roots(roots: &[PathBuf]) -> Vec<Application> {
    let mut shortcuts = Vec::new();
    for root in roots {
        let mut root_shortcuts = Vec::new();
        collect_shortcuts(root, &mut root_shortcuts);
        root_shortcuts.sort_by_key(|path| path.to_string_lossy().to_ascii_lowercase());
        shortcuts.extend(root_shortcuts);
    }

    let mut names = HashSet::new();
    let mut applications = Vec::new();
    for shortcut in shortcuts {
        let Some(name) = shortcut.file_stem().and_then(|name| name.to_str()) else {
            continue;
        };
        let name = name.trim();
        if name.is_empty() || !names.insert(name.to_ascii_lowercase()) {
            continue;
        }
        let path = shortcut.to_string_lossy().into_owned();
        applications.push(Application::new(
            format!("windows-shortcut:{}", path.to_ascii_lowercase()),
            name.to_owned(),
            Some(path.clone()),
            None,
            Some(vec![
                "cmd.exe".into(),
                "/d".into(),
                "/c".into(),
                "start".into(),
                "".into(),
                path,
            ]),
        ));
    }
    applications.sort_by(|left, right| {
        left.name()
            .to_ascii_lowercase()
            .cmp(&right.name().to_ascii_lowercase())
            .then_with(|| left.id().cmp(right.id()))
    });
    applications
}

fn collect_shortcuts(directory: &Path, output: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if file_type.is_dir() {
            collect_shortcuts(&path, output);
        } else if file_type.is_file()
            && path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| {
                    matches!(
                        extension.to_ascii_lowercase().as_str(),
                        "lnk" | "url" | "appref-ms"
                    )
                })
        {
            output.push(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::load_from_roots;

    #[test]
    fn recursively_indexes_and_sorts_start_menu_shortcuts() {
        let directory = tempdir().expect("temporary start menu");
        let nested = directory.path().join("Utilities");
        fs::create_dir(&nested).expect("nested group");
        fs::write(directory.path().join("Browser.lnk"), []).expect("shortcut");
        fs::write(nested.join("Calculator.lnk"), []).expect("nested shortcut");
        fs::write(nested.join("Readme.txt"), []).expect("non-shortcut");

        let applications = load_from_roots(&[directory.path().to_owned()]);
        assert_eq!(
            applications
                .iter()
                .map(|application| application.name())
                .collect::<Vec<_>>(),
            ["Browser", "Calculator"]
        );
        assert_eq!(applications[0].launch_command().unwrap()[3], "start");
    }

    #[test]
    fn duplicate_machine_shortcut_is_shadowed_by_user_shortcut() {
        let user = tempdir().expect("user start menu");
        let machine = tempdir().expect("machine start menu");
        fs::write(user.path().join("Editor.lnk"), []).expect("user shortcut");
        fs::write(machine.path().join("Editor.lnk"), []).expect("machine shortcut");

        let applications = load_from_roots(&[user.path().to_owned(), machine.path().to_owned()]);
        assert_eq!(applications.len(), 1);
        assert!(
            applications[0]
                .id()
                .contains(&user.path().to_string_lossy().to_ascii_lowercase())
        );
    }

    #[test]
    fn desktop_duplicate_is_deduplicated_by_display_name() {
        let start_menu = tempdir().expect("start menu");
        let desktop = tempdir().expect("desktop");
        fs::write(start_menu.path().join("Fortnite.lnk"), []).expect("menu shortcut");
        fs::write(desktop.path().join("FORTNITE.url"), []).expect("desktop shortcut");

        let applications =
            load_from_roots(&[start_menu.path().to_owned(), desktop.path().to_owned()]);
        assert_eq!(applications.len(), 1);
        assert_eq!(applications[0].name(), "Fortnite");
    }
}
