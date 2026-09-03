use std::{
    collections::HashMap,
    env, fs,
    path::{Component, Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
};

use image::RgbaImage;

#[zbus::proxy(
    gen_async = false,
    interface = "org.freedesktop.portal.OpenURI",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait OpenUriPortal {
    fn open_uri(
        &self,
        parent_window: &str,
        uri: &str,
        options: HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

#[zbus::proxy(
    gen_async = false,
    interface = "org.freedesktop.portal.FileChooser",
    default_service = "org.freedesktop.portal.Desktop",
    default_path = "/org/freedesktop/portal/desktop"
)]
trait FileChooserPortal {
    fn open_file(
        &self,
        parent_window: &str,
        title: &str,
        options: HashMap<&str, zbus::zvariant::Value<'_>>,
    ) -> zbus::Result<zbus::zvariant::OwnedObjectPath>;
}

pub fn choose_image_file(
    callback: Box<dyn Fn(super::FileDialogOutcome) + Send + 'static>,
) -> Result<(), String> {
    thread::Builder::new()
        .name("nickel-portal-file-chooser".into())
        .spawn(move || callback(request_image_file()))
        .map(|_| ())
        .map_err(|error| format!("could not start the XDG FileChooser request: {error}"))
}

fn request_image_file() -> super::FileDialogOutcome {
    request_image_file_inner().unwrap_or_else(super::FileDialogOutcome::Failed)
}

fn request_image_file_inner() -> Result<super::FileDialogOutcome, String> {
    use zbus::zvariant::{OwnedObjectPath, OwnedValue};

    let connection = zbus::blocking::Connection::session().map_err(|error| error.to_string())?;
    let portal = FileChooserPortalProxy::new(&connection).map_err(|error| error.to_string())?;
    static REQUEST_ID: AtomicU64 = AtomicU64::new(1);
    let handle_token = format!(
        "nickel_{}_{}",
        std::process::id(),
        REQUEST_ID.fetch_add(1, Ordering::Relaxed)
    );
    let sender = connection
        .unique_name()
        .ok_or_else(|| "session bus did not assign a unique name".to_owned())?
        .as_str()
        .trim_start_matches(':')
        .replace('.', "_");
    let expected_handle = OwnedObjectPath::try_from(format!(
        "/org/freedesktop/portal/desktop/request/{sender}/{handle_token}"
    ))
    .map_err(|error| error.to_string())?;
    let request = zbus::blocking::Proxy::new(
        &connection,
        "org.freedesktop.portal.Desktop",
        expected_handle.as_str(),
        "org.freedesktop.portal.Request",
    )
    .map_err(|error| error.to_string())?;
    let mut responses = request
        .receive_signal("Response")
        .map_err(|error| error.to_string())?;
    let filters = vec![(
        "Images".to_owned(),
        vec![
            (0_u32, "*.png".to_owned()),
            (0, "*.jpg".to_owned()),
            (0, "*.jpeg".to_owned()),
            (0, "*.webp".to_owned()),
            (0, "*.bmp".to_owned()),
        ],
    )];
    let mut options = HashMap::new();
    options.insert(
        "handle_token",
        zbus::zvariant::Value::new(handle_token.clone()),
    );
    options.insert("filters", zbus::zvariant::Value::new(filters));
    let handle = portal
        .open_file("", "Choose an image", options)
        .map_err(|error| error.to_string())?;
    if handle != expected_handle {
        return Err(format!(
            "XDG FileChooser returned unexpected request handle {handle}"
        ));
    }
    let response = responses
        .next()
        .ok_or_else(|| "XDG FileChooser request ended without a response".to_owned())?
        .body()
        .deserialize::<(u32, HashMap<String, OwnedValue>)>()
        .map_err(|error| error.to_string())?;
    match response {
        (0, mut results) => {
            let uris = results
                .remove("uris")
                .ok_or_else(|| "XDG FileChooser response omitted selected URIs".to_owned())
                .and_then(|value| {
                    Vec::<String>::try_from(value).map_err(|error| error.to_string())
                })?;
            let uri = uris
                .into_iter()
                .next()
                .ok_or_else(|| "XDG FileChooser returned an empty selection".to_owned())?;
            decode_file_uri(&uri).map(super::FileDialogOutcome::Selected)
        }
        (1, _) => Ok(super::FileDialogOutcome::Cancelled),
        (code, _) => Err(format!("XDG FileChooser failed with response code {code}")),
    }
}

fn decode_file_uri(uri: &str) -> Result<std::path::PathBuf, String> {
    use std::os::unix::ffi::OsStringExt;

    let encoded = uri
        .strip_prefix("file://")
        .filter(|path| path.starts_with('/'))
        .ok_or_else(|| format!("XDG FileChooser returned a non-local URI: {uri}"))?;
    let bytes = encoded.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let high = bytes
                .get(index + 1)
                .and_then(|value| hex_digit(*value))
                .ok_or_else(|| format!("invalid percent escape in file URI: {uri}"))?;
            let low = bytes
                .get(index + 2)
                .and_then(|value| hex_digit(*value))
                .ok_or_else(|| format!("invalid percent escape in file URI: {uri}"))?;
            let value = high * 16 + low;
            if value == 0 {
                return Err("file URI contains a NUL byte".into());
            }
            decoded.push(value);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    Ok(std::ffi::OsString::from_vec(decoded).into())
}

fn hex_digit(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

pub fn open_external_url(url: &str) -> Result<(), String> {
    let url = url.to_owned();
    thread::Builder::new()
        .name("nickel-portal-open-uri".into())
        .spawn(move || {
            if let Err(error) = request_open_uri(&url) {
                tracing::warn!(%error, "XDG OpenURI request failed");
            }
        })
        .map(|_| ())
        .map_err(|error| format!("could not start the XDG OpenURI request: {error}"))
}

fn request_open_uri(uri: &str) -> Result<(), String> {
    let connection = zbus::blocking::Connection::session().map_err(|error| error.to_string())?;
    let portal = OpenUriPortalProxy::new(&connection).map_err(|error| error.to_string())?;
    portal
        .open_uri("", uri, HashMap::new())
        .map(|_| ())
        .map_err(|error| error.to_string())
}

pub fn path_icon(path: &Path) -> Option<RgbaImage> {
    let theme = system_icon_theme();
    path_icon_with_theme(path, Some(&theme))
}

pub fn path_icon_with_theme(path: &Path, theme: Option<&str>) -> Option<RgbaImage> {
    let name = icon_name(path);
    let theme = theme.filter(|theme| !theme.trim().is_empty())?;
    let mut components = Path::new(theme).components();
    if !matches!(components.next(), Some(Component::Normal(_)))
        || components.next().is_some()
        || !icon_search_roots()
            .into_iter()
            .any(|root| root.join(theme).join("index.theme").is_file())
    {
        return None;
    }
    freedesktop_icons::lookup(name)
        .with_size(96)
        .with_theme(theme)
        .with_cache()
        .find()
        .and_then(|path| load_icon(&path))
}

pub fn installed_icon_themes() -> Vec<String> {
    installed_icon_themes_in(icon_search_roots())
}

fn installed_icon_themes_in(roots: impl IntoIterator<Item = PathBuf>) -> Vec<String> {
    let mut themes = roots
        .into_iter()
        .filter_map(|root| fs::read_dir(root).ok())
        .flatten()
        .filter_map(Result::ok)
        .filter(|entry| entry.path().join("index.theme").is_file())
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect::<Vec<_>>();
    themes.sort_by_key(|theme| theme.to_ascii_lowercase());
    themes.dedup();
    themes
}

fn icon_search_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        roots.push(home.join(".icons"));
        roots.push(
            env::var_os("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".local/share"))
                .join("icons"),
        );
    }
    roots.extend(
        env::var_os("XDG_DATA_DIRS")
            .and_then(|value| value.into_string().ok())
            .unwrap_or_else(|| "/usr/local/share:/usr/share".to_owned())
            .split(':')
            .filter(|root| !root.is_empty())
            .map(|root| PathBuf::from(root).join("icons")),
    );
    roots
}

fn system_icon_theme() -> String {
    if let Ok(theme) = env::var("NICKEL_ICON_THEME")
        && !theme.trim().is_empty()
    {
        return theme;
    }
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")));
    config_home
        .and_then(|directory| fs::read_to_string(directory.join("kdeglobals")).ok())
        .and_then(|contents| value_in_section(&contents, "Icons", "Theme"))
        .unwrap_or_else(|| "hicolor".to_owned())
}

fn value_in_section(contents: &str, section: &str, key: &str) -> Option<String> {
    let mut in_section = false;
    for line in contents.lines().map(str::trim) {
        if line.starts_with('[') && line.ends_with(']') {
            in_section = &line[1..line.len() - 1] == section;
        } else if in_section
            && let Some(value) = line
                .strip_prefix(key)
                .and_then(|line| line.strip_prefix('='))
        {
            let value = value.trim();
            if !value.is_empty() {
                return Some(value.to_owned());
            }
        }
    }
    None
}

fn icon_name(path: &Path) -> &'static str {
    if path.is_dir() {
        return match path.file_name().and_then(|name| name.to_str()) {
            Some("Desktop") => "user-desktop",
            Some("Documents") => "folder-documents",
            Some("Downloads") => "folder-download",
            Some("Music") => "folder-music",
            Some("Pictures") => "folder-pictures",
            Some("Videos") => "folder-videos",
            _ => "folder",
        };
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg") => "image-x-generic",
        Some("mp3" | "flac" | "wav" | "ogg" | "m4a") => "audio-x-generic",
        Some("mp4" | "mkv" | "webm" | "avi" | "mov") => "video-x-generic",
        Some("pdf") => "application-pdf",
        Some("zip" | "tar" | "gz" | "bz2" | "xz" | "7z") => "package-x-generic",
        Some("desktop" | "appimage" | "exe") => "application-x-executable",
        _ => "text-x-generic",
    }
}

fn load_icon(path: &Path) -> Option<RgbaImage> {
    if path
        .extension()
        .is_some_and(|extension| extension.eq_ignore_ascii_case("svg"))
    {
        let data = fs::read(path).ok()?;
        let tree = resvg::usvg::Tree::from_data(&data, &Default::default()).ok()?;
        let size = tree.size();
        let scale = (96.0 / size.width()).min(96.0 / size.height());
        let width = (size.width() * scale).round().max(1.0) as u32;
        let height = (size.height() * scale).round().max(1.0) as u32;
        let mut pixmap = resvg::tiny_skia::Pixmap::new(width, height)?;
        resvg::render(
            &tree,
            resvg::tiny_skia::Transform::from_scale(scale, scale),
            &mut pixmap.as_mut(),
        );
        RgbaImage::from_raw(width, height, pixmap.data().to_vec())
    } else {
        image::open(path).ok().map(image::DynamicImage::into_rgba8)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{
        decode_file_uri, icon_name, installed_icon_themes_in, path_icon_with_theme,
        value_in_section,
    };

    #[test]
    fn portal_file_uris_preserve_unix_paths_and_percent_escapes() {
        assert_eq!(
            decode_file_uri("file:///run/user/1000/doc/a1/space%20and%23hash.png").unwrap(),
            Path::new("/run/user/1000/doc/a1/space and#hash.png")
        );
        assert!(decode_file_uri("https://example.test/image.png").is_err());
        assert!(decode_file_uri("file://remote/image.png").is_err());
        assert!(decode_file_uri("file:///tmp/bad%2").is_err());
        assert!(decode_file_uri("file:///tmp/nul%00byte").is_err());
    }

    #[test]
    fn icon_names_follow_path_kind() {
        let root = tempfile::tempdir().unwrap();
        let pictures = root.path().join("Pictures");
        std::fs::create_dir(&pictures).unwrap();
        assert_eq!(icon_name(&pictures), "folder-pictures");
        assert_eq!(icon_name(Path::new("report.pdf")), "application-pdf");
        assert_eq!(icon_name(Path::new("archive.tar")), "package-x-generic");
        assert_eq!(icon_name(Path::new("README")), "text-x-generic");
    }

    #[test]
    fn installed_theme_discovery_requires_an_index_and_is_stable() {
        let first = tempfile::tempdir().unwrap();
        let second = tempfile::tempdir().unwrap();
        std::fs::create_dir(first.path().join("not-a-theme")).unwrap();
        for (root, theme) in [(first.path(), "Zulu"), (second.path(), "alpha")] {
            let directory = root.join(theme);
            std::fs::create_dir(&directory).unwrap();
            std::fs::write(directory.join("index.theme"), "[Icon Theme]\n").unwrap();
        }
        assert_eq!(
            installed_icon_themes_in([first.path().into(), second.path().into()]),
            ["alpha", "Zulu"]
        );
    }

    #[test]
    fn kde_theme_parser_ignores_values_outside_icons_section() {
        let settings = "[General]\nTheme=Wrong\n[Icons]\nTheme=Papirus-Dark\n";
        assert_eq!(
            value_in_section(settings, "Icons", "Theme").as_deref(),
            Some("Papirus-Dark")
        );
    }

    #[test]
    fn unavailable_or_non_local_theme_names_return_no_artwork() {
        assert!(
            path_icon_with_theme(Path::new("report.txt"), Some("missing-nickel-test-theme"))
                .is_none()
        );
        assert!(path_icon_with_theme(Path::new("report.txt"), Some("../hicolor")).is_none());
    }
}
