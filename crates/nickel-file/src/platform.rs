use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocationGroup {
    pub(crate) id: &'static str,
    pub(crate) title: &'static str,
    pub(crate) entries: Vec<(String, PathBuf)>,
}

#[derive(Default)]
pub(crate) struct LocationSources {
    pub(crate) nickel_home: Vec<(String, PathBuf)>,
    pub(crate) user_pins: Vec<(String, PathBuf)>,
    pub(crate) user_locations: Vec<(String, PathBuf)>,
    pub(crate) computer_and_volumes: Vec<(String, PathBuf)>,
    pub(crate) network: Vec<(String, PathBuf)>,
}

pub(crate) fn location_groups() -> Vec<LocationGroup> {
    let mut locations = places().into_iter();
    location_groups_from(LocationSources {
        nickel_home: locations.next().into_iter().collect(),
        user_locations: locations.collect(),
        ..LocationSources::default()
    })
}

pub(crate) fn location_groups_from(sources: LocationSources) -> Vec<LocationGroup> {
    let mut seen = HashSet::new();
    [
        LocationGroup {
            id: "nickel-home",
            title: "Nickel Home",
            entries: sources.nickel_home,
        },
        LocationGroup {
            id: "user-pins",
            title: "Pins",
            entries: sources.user_pins,
        },
        LocationGroup {
            id: "user-locations",
            title: "Cloud & locations",
            entries: sources.user_locations,
        },
        LocationGroup {
            id: "computer-volumes",
            title: "Computer & volumes",
            entries: sources.computer_and_volumes,
        },
        LocationGroup {
            id: "network",
            title: "Network",
            entries: sources.network,
        },
    ]
    .into_iter()
    .map(|mut group| {
        group.entries.retain(|(_, path)| seen.insert(path.clone()));
        group
    })
    .filter(|group| !group.entries.is_empty())
    .collect()
}

#[cfg(target_os = "linux")]
pub(crate) fn places() -> Vec<(String, PathBuf)> {
    let home = home_directory();
    let mut places = vec![("Home".to_owned(), home.clone())];
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"));
    let configured = std::fs::read_to_string(config_home.join("user-dirs.dirs")).ok();
    places.extend(user_directories(&home, configured.as_deref()));
    places
}

#[cfg(target_os = "linux")]
fn user_directories(home: &Path, configured: Option<&str>) -> Vec<(String, PathBuf)> {
    let defaults = [
        ("Desktop", "XDG_DESKTOP_DIR", "Desktop"),
        ("Documents", "XDG_DOCUMENTS_DIR", "Documents"),
        ("Downloads", "XDG_DOWNLOAD_DIR", "Downloads"),
        ("Pictures", "XDG_PICTURES_DIR", "Pictures"),
        ("Music", "XDG_MUSIC_DIR", "Music"),
        ("Videos", "XDG_VIDEOS_DIR", "Videos"),
    ];
    defaults
        .into_iter()
        .filter_map(|(label, key, fallback)| {
            let configured_path = configured
                .and_then(|contents| {
                    contents.lines().find_map(|line| {
                        line.split_once('=')
                            .filter(|(candidate, _)| candidate.trim() == key)
                    })
                })
                .and_then(|(_, value)| xdg_user_path(value.trim(), home));
            let path = configured_path.unwrap_or_else(|| home.join(fallback));
            (path != home && path.is_dir()).then(|| (label.to_owned(), path))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn xdg_user_path(value: &str, home: &Path) -> Option<PathBuf> {
    let value = value.strip_prefix('"')?.strip_suffix('"')?;
    let suffix = value
        .strip_prefix("$HOME")
        .or_else(|| value.strip_prefix("${HOME}"))?;
    if suffix.contains('$') {
        return None;
    }
    let suffix = suffix.strip_prefix('/').unwrap_or(suffix);
    Some(home.join(suffix.replace("\\\"", "\"").replace("\\\\", "\\")))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) fn places() -> Vec<(String, PathBuf)> {
    let home = home_directory();
    let mut places = vec![("Home".to_owned(), home.clone())];
    for (label, folder) in [
        ("Desktop", "Desktop"),
        ("Documents", "Documents"),
        ("Downloads", "Downloads"),
        ("Pictures", "Pictures"),
        ("Music", "Music"),
        ("Videos", "Videos"),
    ] {
        let path = home.join(folder);
        if path.is_dir() {
            places.push((label.to_owned(), path));
        }
    }
    places
}

#[cfg(target_os = "windows")]
pub(crate) fn places() -> Vec<(String, PathBuf)> {
    use windows::Win32::UI::Shell::{
        FOLDERID_Desktop, FOLDERID_Documents, FOLDERID_Downloads, FOLDERID_Music,
        FOLDERID_Pictures, FOLDERID_Profile, FOLDERID_Videos,
    };

    [
        ("Home", &FOLDERID_Profile),
        ("Desktop", &FOLDERID_Desktop),
        ("Documents", &FOLDERID_Documents),
        ("Downloads", &FOLDERID_Downloads),
        ("Pictures", &FOLDERID_Pictures),
        ("Music", &FOLDERID_Music),
        ("Videos", &FOLDERID_Videos),
    ]
    .into_iter()
    .filter_map(|(label, id)| known_folder_path(id).map(|path| (label.to_owned(), path)))
    .collect()
}

#[cfg(target_os = "windows")]
pub(crate) fn known_folder_path(id: &windows::core::GUID) -> Option<PathBuf> {
    use windows::Win32::{
        System::Com::CoTaskMemFree,
        UI::Shell::{KF_FLAG_DEFAULT, SHGetKnownFolderPath},
    };

    // SAFETY: SHGetKnownFolderPath allocates a terminated string for the supplied known-folder
    // identifier. We copy it into an owned PathBuf and release the allocation with CoTaskMemFree.
    unsafe {
        let value = SHGetKnownFolderPath(id, KF_FLAG_DEFAULT, None).ok()?;
        let path = value.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(value.as_ptr().cast()));
        path.filter(|path| path.is_dir())
    }
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn home_directory() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "windows")]
pub(crate) fn home_directory() -> PathBuf {
    use windows::Win32::UI::Shell::FOLDERID_Profile;

    known_folder_path(&FOLDERID_Profile)
        .or_else(|| std::env::var_os("USERPROFILE").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

#[cfg(target_os = "windows")]
pub(crate) fn open_path(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::PCWSTR,
    };

    let path = path
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let verb = "open\0".encode_utf16().collect::<Vec<_>>();
    // SAFETY: both strings are terminated and live for the synchronous ShellExecuteW call.
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(path.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    if result.0 as isize > 32 {
        Ok(())
    } else {
        Err(format!("Windows shell error {}", result.0 as isize))
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn open_path(path: &Path) -> Result<(), String> {
    std::process::Command::new("xdg-open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "macos")]
pub(crate) fn open_path(path: &Path) -> Result<(), String> {
    std::process::Command::new("open")
        .arg(path)
        .spawn()
        .map(|_| ())
        .map_err(|error| error.to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "linux", target_os = "macos")))]
pub(crate) fn open_path(_path: &Path) -> Result<(), String> {
    Err("opening files is unsupported on this platform".into())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::user_directories;

    #[test]
    fn xdg_user_directories_honor_localized_disabled_and_default_paths() {
        let home = tempfile::tempdir().unwrap();
        for directory in ["Arbeitsfläche", "Documents", "Downloads"] {
            std::fs::create_dir(home.path().join(directory)).unwrap();
        }
        let configured = concat!(
            "XDG_DESKTOP_DIR=\"$HOME/Arbeitsfläche\"\n",
            "XDG_DOCUMENTS_DIR=\"$HOME\"\n",
            "XDG_DOWNLOAD_DIR=\"${HOME}/Downloads\"\n",
            "XDG_PICTURES_DIR=\"$OTHER/Pictures\"\n",
        );

        let directories = user_directories(home.path(), Some(configured));
        assert_eq!(
            directories,
            [
                ("Desktop".to_owned(), home.path().join("Arbeitsfläche")),
                ("Downloads".to_owned(), home.path().join("Downloads")),
            ]
        );
    }
}
