use std::{
    collections::HashSet,
    fmt,
    path::{Path, PathBuf},
};

#[cfg(target_os = "linux")]
use std::io;

pub(crate) fn publish_text_clipboard(text: &str) -> Result<(), String> {
    let mut clipboard = arboard::Clipboard::new().map_err(|error| error.to_string())?;
    clipboard
        .set_text(text.to_owned())
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
fn file_uri_list(paths: &[PathBuf]) -> Result<String, String> {
    paths
        .iter()
        .map(|path| {
            url::Url::from_file_path(path)
                .map(|url| format!("{url}\r\n"))
                .map_err(|()| format!("cannot represent {} as a file URI", path.display()))
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn paths_from_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|line| url::Url::parse(line).ok())
        .filter_map(|url| {
            (url.scheme() == "file")
                .then(|| url.to_file_path().ok())
                .flatten()
        })
        .collect()
}

#[cfg(target_os = "linux")]
pub(crate) fn publish_file_clipboard(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    use wl_clipboard_rs::copy::{MimeSource, MimeType, Options, Source};
    let mut payload = if cut {
        "cut\n".to_owned()
    } else {
        "copy\n".to_owned()
    };
    let uris = file_uri_list(paths)?;
    payload.push_str(&uris.replace("\r\n", "\n"));
    Options::new()
        .copy_multi(vec![
            MimeSource {
                source: Source::Bytes(payload.into_bytes().into()),
                mime_type: MimeType::Specific("x-special/gnome-copied-files".into()),
            },
            MimeSource {
                source: Source::Bytes(uris.into_bytes().into()),
                mime_type: MimeType::Specific("text/uri-list".into()),
            },
            MimeSource {
                source: Source::Bytes(if cut {
                    b"1".to_vec().into()
                } else {
                    b"0".to_vec().into()
                }),
                mime_type: MimeType::Specific("application/x-kde-cutselection".into()),
            },
        ])
        .map_err(|error| error.to_string())
}

#[cfg(target_os = "linux")]
pub(crate) fn read_file_clipboard() -> Result<(bool, Vec<PathBuf>), String> {
    use std::io::Read;
    use wl_clipboard_rs::paste::{ClipboardType, MimeType, Seat, get_contents, get_mime_types};
    let types = get_mime_types(ClipboardType::Regular, Seat::Unspecified)
        .map_err(|error| error.to_string())?;
    let kde_cut = if types.contains("application/x-kde-cutselection") {
        get_contents(
            ClipboardType::Regular,
            Seat::Unspecified,
            MimeType::Specific("application/x-kde-cutselection"),
        )
        .ok()
        .and_then(|(mut pipe, _)| {
            let mut value = String::new();
            pipe.read_to_string(&mut value).ok()?;
            Some(value.trim() == "1")
        })
        .unwrap_or(false)
    } else {
        false
    };
    let requested = if types.contains("x-special/gnome-copied-files") {
        "x-special/gnome-copied-files"
    } else {
        "text/uri-list"
    };
    let (mut pipe, mime) = get_contents(
        ClipboardType::Regular,
        Seat::Unspecified,
        MimeType::Specific(requested),
    )
    .map_err(|error| error.to_string())?;
    let mut text = String::new();
    pipe.read_to_string(&mut text)
        .map_err(|error| error.to_string())?;
    let mut lines = text.lines();
    let cut = kde_cut || mime == "x-special/gnome-copied-files" && lines.next() == Some("cut");
    Ok((
        cut,
        paths_from_uri_list(&lines.collect::<Vec<_>>().join("\n")),
    ))
}

#[cfg(target_os = "windows")]
pub(crate) fn publish_file_clipboard(paths: &[PathBuf], cut: bool) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows::Win32::{
        Foundation::HANDLE,
        System::{
            DataExchange::{
                CloseClipboard, EmptyClipboard, OpenClipboard, RegisterClipboardFormatW,
                SetClipboardData,
            },
            Memory::{GHND, GlobalAlloc, GlobalLock, GlobalUnlock},
        },
        UI::Shell::DROPFILES,
    };
    use windows::core::w;
    const CF_HDROP: u32 = 15;
    let names = paths
        .iter()
        .flat_map(|path| path.as_os_str().encode_wide().chain(Some(0)))
        .chain(Some(0))
        .collect::<Vec<_>>();
    let bytes = std::mem::size_of::<DROPFILES>() + names.len() * 2;
    unsafe {
        OpenClipboard(None).map_err(|error| error.to_string())?;
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseClipboard();
                }
            }
        }
        let _guard = Guard;
        EmptyClipboard().map_err(|error| error.to_string())?;
        let memory = GlobalAlloc(GHND, bytes).map_err(|error| error.to_string())?;
        let pointer = GlobalLock(memory);
        if pointer.is_null() {
            return Err("GlobalLock failed".into());
        }
        let header = pointer.cast::<DROPFILES>();
        (*header).pFiles = std::mem::size_of::<DROPFILES>() as u32;
        (*header).fWide = true.into();
        std::ptr::copy_nonoverlapping(
            names.as_ptr().cast::<u8>(),
            pointer.cast::<u8>().add(std::mem::size_of::<DROPFILES>()),
            names.len() * 2,
        );
        let _ = GlobalUnlock(memory);
        SetClipboardData(CF_HDROP, Some(HANDLE(memory.0))).map_err(|error| error.to_string())?;
        let effect_format = RegisterClipboardFormatW(w!("Preferred DropEffect"));
        if effect_format == 0 {
            return Err("could not register Preferred DropEffect clipboard format".into());
        }
        let effect_memory =
            GlobalAlloc(GHND, std::mem::size_of::<u32>()).map_err(|error| error.to_string())?;
        let effect_pointer = GlobalLock(effect_memory).cast::<u32>();
        if effect_pointer.is_null() {
            return Err("GlobalLock failed for Preferred DropEffect".into());
        }
        effect_pointer.write(if cut { 2 } else { 1 });
        let _ = GlobalUnlock(effect_memory);
        SetClipboardData(effect_format, Some(HANDLE(effect_memory.0)))
            .map_err(|error| error.to_string())?;
    }
    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) fn read_file_clipboard() -> Result<(bool, Vec<PathBuf>), String> {
    use windows::Win32::{
        System::{
            DataExchange::{
                CloseClipboard, GetClipboardData, OpenClipboard, RegisterClipboardFormatW,
            },
            Memory::{GlobalLock, GlobalUnlock},
        },
        UI::Shell::DragQueryFileW,
    };
    use windows::core::w;
    const CF_HDROP: u32 = 15;
    unsafe {
        OpenClipboard(None).map_err(|error| error.to_string())?;
        struct Guard;
        impl Drop for Guard {
            fn drop(&mut self) {
                unsafe {
                    let _ = CloseClipboard();
                }
            }
        }
        let _guard = Guard;
        let drop = GetClipboardData(CF_HDROP).map_err(|error| error.to_string())?;
        let count = DragQueryFileW(windows::Win32::UI::Shell::HDROP(drop.0), u32::MAX, None);
        let mut paths = Vec::with_capacity(count as usize);
        for index in 0..count {
            let length = DragQueryFileW(windows::Win32::UI::Shell::HDROP(drop.0), index, None);
            let mut buffer = vec![0_u16; length as usize + 1];
            DragQueryFileW(
                windows::Win32::UI::Shell::HDROP(drop.0),
                index,
                Some(&mut buffer),
            );
            paths.push(PathBuf::from(String::from_utf16_lossy(
                &buffer[..length as usize],
            )));
        }
        let effect_format = RegisterClipboardFormatW(w!("Preferred DropEffect"));
        let cut = if effect_format == 0 {
            false
        } else {
            GetClipboardData(effect_format).ok().is_some_and(|memory| {
                let pointer =
                    GlobalLock(windows::Win32::Foundation::HGLOBAL(memory.0)).cast::<u32>();
                if pointer.is_null() {
                    return false;
                }
                let effect = pointer.read();
                let _ = GlobalUnlock(windows::Win32::Foundation::HGLOBAL(memory.0));
                effect & 2 != 0
            })
        };
        Ok((cut, paths))
    }
}

#[cfg(all(test, target_os = "linux"))]
mod clipboard_contract_tests {
    use super::*;

    #[test]
    fn file_uri_round_trip_preserves_reserved_and_unicode_characters() {
        let paths = vec![PathBuf::from("/tmp/a b#c.txt"), PathBuf::from("/tmp/café")];
        let payload = file_uri_list(&paths).unwrap();
        assert!(payload.contains("a%20b%23c.txt"));
        assert_eq!(paths_from_uri_list(&payload), paths);
    }

    #[test]
    fn uri_list_ignores_comments_and_non_file_urls() {
        assert_eq!(
            paths_from_uri_list("# copied files\r\nhttps://example.test/x\r\nfile:///tmp/x\r\n"),
            vec![PathBuf::from("/tmp/x")]
        );
    }
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) fn publish_file_clipboard(_paths: &[PathBuf], _cut: bool) -> Result<(), String> {
    Err("native file clipboard adapter unavailable".into())
}
#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) fn read_file_clipboard() -> Result<(bool, Vec<PathBuf>), String> {
    Err("native file clipboard adapter unavailable".into())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum OpenPathError {
    AssociationMissing,
    PermissionDenied,
    TargetMissing,
    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    Unsupported,
    Platform(String),
}

impl fmt::Display for OpenPathError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AssociationMissing => formatter.write_str("no default application is available"),
            Self::PermissionDenied => formatter.write_str("permission was denied"),
            Self::TargetMissing => formatter.write_str("the target no longer exists"),
            #[cfg(not(any(target_os = "windows", target_os = "linux")))]
            Self::Unsupported => {
                formatter.write_str("opening files is unsupported on this platform")
            }
            Self::Platform(error) => formatter.write_str(error),
        }
    }
}

#[cfg(target_os = "linux")]
fn spawn_open_error(error: io::Error) -> OpenPathError {
    match error.kind() {
        io::ErrorKind::NotFound => OpenPathError::AssociationMissing,
        io::ErrorKind::PermissionDenied => OpenPathError::PermissionDenied,
        _ => OpenPathError::Platform(error.to_string()),
    }
}

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

#[cfg(target_os = "linux")]
pub(crate) fn open_launcher(launcher: &Path) -> Result<(), OpenPathError> {
    if !launcher.exists() {
        return Err(OpenPathError::TargetMissing);
    }
    std::process::Command::new("gio")
        .args([std::ffi::OsStr::new("launch"), launcher.as_os_str()])
        .status()
        .map_err(spawn_open_error)
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| OpenPathError::Platform(format!("gio launch exited with {status}")))
        })
}

#[cfg(target_os = "windows")]
pub(crate) fn open_launcher(launcher: &Path) -> Result<(), OpenPathError> {
    nickel_platform::open_with_default(launcher).map_err(|error| match error {
        nickel_platform::DefaultLaunchError::TargetMissing => OpenPathError::TargetMissing,
        nickel_platform::DefaultLaunchError::PermissionDenied => OpenPathError::PermissionDenied,
        nickel_platform::DefaultLaunchError::AssociationMissing => {
            OpenPathError::AssociationMissing
        }
        nickel_platform::DefaultLaunchError::Platform(detail) => OpenPathError::Platform(detail),
    })
}

#[cfg(not(any(target_os = "linux", target_os = "windows")))]
pub(crate) fn open_launcher(_launcher: &Path) -> Result<(), OpenPathError> {
    Err(OpenPathError::Unsupported)
}

#[cfg(target_os = "linux")]
pub(crate) fn open_with_launcher(launcher: &Path, source: &Path) -> Result<(), OpenPathError> {
    if !launcher.exists() || !source.exists() {
        return Err(OpenPathError::TargetMissing);
    }
    std::process::Command::new("gio")
        .args([
            std::ffi::OsStr::new("launch"),
            launcher.as_os_str(),
            source.as_os_str(),
        ])
        .status()
        .map_err(spawn_open_error)
        .and_then(|status| {
            status
                .success()
                .then_some(())
                .ok_or_else(|| OpenPathError::Platform(format!("gio launch exited with {status}")))
        })
}

#[cfg(target_os = "windows")]
pub(crate) fn open_with_launcher(launcher: &Path, source: &Path) -> Result<(), OpenPathError> {
    if !launcher.exists() || !source.exists() {
        return Err(OpenPathError::TargetMissing);
    }
    use std::os::windows::ffi::OsStrExt;
    use windows::{
        Win32::UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
        core::PCWSTR,
    };
    let launcher = launcher
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let source = source
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect::<Vec<_>>();
    let verb = "open\0".encode_utf16().collect::<Vec<_>>();
    // SAFETY: all UTF-16 strings are NUL-terminated and live for this synchronous call.
    let result = unsafe {
        ShellExecuteW(
            None,
            PCWSTR(verb.as_ptr()),
            PCWSTR(launcher.as_ptr()),
            PCWSTR(source.as_ptr()),
            None,
            SW_SHOWNORMAL,
        )
    };
    let code = result.0 as isize;
    match code {
        value if value > 32 => Ok(()),
        5 => Err(OpenPathError::PermissionDenied),
        31 => Err(OpenPathError::AssociationMissing),
        _ => Err(OpenPathError::Platform(format!(
            "Windows shell error {code}"
        ))),
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub(crate) fn open_with_launcher(_launcher: &Path, _source: &Path) -> Result<(), OpenPathError> {
    Err(OpenPathError::Unsupported)
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
