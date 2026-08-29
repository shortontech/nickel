use std::{collections::HashMap, fs, path::Path, thread};

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
    let name = icon_name(path);
    ["breeze", "breeze-dark", "hicolor", "Adwaita"]
        .into_iter()
        .find_map(|theme| {
            freedesktop_icons::lookup(name)
                .with_size(96)
                .with_theme(theme)
                .with_cache()
                .find()
        })
        .and_then(|path| load_icon(&path))
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
