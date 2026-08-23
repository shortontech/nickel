use std::{
    collections::HashSet,
    env, fs,
    path::{Path, PathBuf},
};

use crate::model::Application;
use windows::Win32::{
    System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize},
    UI::Shell::{
        BHID_EnumItems, FOLDERID_AppsFolder, IEnumShellItems, IShellItem, KF_FLAG_DEFAULT,
        SHGetKnownFolderItem, SIGDN_DESKTOPABSOLUTEPARSING, SIGDN_NORMALDISPLAY,
    },
};

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
    let mut applications = load_from_roots(&roots);
    let mut names = applications
        .iter()
        .map(|application| application.name().to_ascii_lowercase())
        .collect::<HashSet<_>>();
    for application in load_packaged_applications() {
        if names.insert(application.name().to_ascii_lowercase()) {
            applications.push(application);
        }
    }
    sort_applications(&mut applications);
    applications
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
            Some(vec![path]),
        ));
    }
    sort_applications(&mut applications);
    applications
}

fn sort_applications(applications: &mut [Application]) {
    applications.sort_by(|left, right| {
        left.name()
            .to_ascii_lowercase()
            .cmp(&right.name().to_ascii_lowercase())
            .then_with(|| left.id().cmp(right.id()))
    });
}

fn load_packaged_applications() -> Vec<Application> {
    // SAFETY: COM is initialized for this thread while the shell items are enumerated. A
    // successful call, including S_FALSE, is balanced with CoUninitialize.
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) }.is_ok();
    let applications = unsafe { enumerate_apps_folder() }.unwrap_or_else(|error| {
        tracing::warn!(%error, "could not enumerate the Windows AppsFolder");
        Vec::new()
    });
    if initialized {
        // SAFETY: This balances the successful CoInitializeEx call above.
        unsafe { CoUninitialize() };
    }
    applications
}

unsafe fn enumerate_apps_folder() -> windows::core::Result<Vec<Application>> {
    // SAFETY: The known-folder identifier and bind-handler identifier are static Windows values.
    let folder: IShellItem =
        unsafe { SHGetKnownFolderItem(&FOLDERID_AppsFolder, KF_FLAG_DEFAULT, None)? };
    let items: IEnumShellItems = unsafe { folder.BindToHandler(None, &BHID_EnumItems)? };
    let mut applications = Vec::new();
    loop {
        let mut fetched = 0;
        let mut item = [None];
        if unsafe { items.Next(&mut item, Some(&mut fetched)) }.is_err() || fetched == 0 {
            break;
        }
        let Some(item) = item[0].take() else {
            continue;
        };
        let name = unsafe { shell_item_name(&item, SIGDN_NORMALDISPLAY)? };
        let target = unsafe { shell_item_name(&item, SIGDN_DESKTOPABSOLUTEPARSING)? };
        if name.trim().is_empty() || target.trim().is_empty() {
            continue;
        }
        applications.push(Application::new(
            format!("windows-app:{}", target.to_ascii_lowercase()),
            name,
            Some(target.clone()),
            None,
            Some(vec![target]),
        ));
    }
    Ok(applications)
}

unsafe fn shell_item_name(
    item: &IShellItem,
    format: windows::Win32::UI::Shell::SIGDN,
) -> windows::core::Result<String> {
    let value = unsafe { item.GetDisplayName(format)? };
    let text = unsafe { value.to_string() }.unwrap_or_default();
    // SAFETY: IShellItem::GetDisplayName allocates this string with the COM task allocator.
    unsafe { CoTaskMemFree(Some(value.as_ptr().cast())) };
    Ok(text)
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
        assert!(applications[0].launch_command().unwrap()[0].ends_with("Browser.lnk"));
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
