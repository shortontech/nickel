use std::{
    collections::{HashMap, VecDeque},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
};

use image::{Rgba, RgbaImage, imageops::FilterType};
use nickel_core::shell_settings::FileIconPreference;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SemanticIconKind {
    Folder,
    HomeFolder,
    PicturesFolder,
    MusicFolder,
    TextFile,
    ImageFile,
    UnknownFile,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ArtworkAppearance {
    Light,
    Dark,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtworkRequest<'a> {
    pub path: &'a Path,
    pub kind: SemanticIconKind,
    pub logical_size: u16,
    pub scale_milli: u16,
    pub appearance: ArtworkAppearance,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ArtworkSource {
    Nickel,
    System,
}

#[derive(Clone)]
pub struct ResolvedArtwork {
    pub pixels: Arc<RgbaImage>,
    pub source: ArtworkSource,
    pub provider_revision: u64,
}

pub trait IconProvider: Send + Sync {
    fn source(&self) -> ArtworkSource;
    fn revision(&self) -> u64;
    fn resolve(&self, request: &ArtworkRequest<'_>) -> Option<RgbaImage>;
}

pub(crate) struct ArtworkCache {
    entries: HashMap<PathBuf, (u16, Arc<RgbaImage>)>,
    order: VecDeque<PathBuf>,
    retained_bytes: usize,
    byte_capacity: usize,
}

impl Default for ArtworkCache {
    fn default() -> Self {
        Self::with_capacity(32 * 1024 * 1024)
    }
}

impl ArtworkCache {
    fn with_capacity(byte_capacity: usize) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            retained_bytes: 0,
            byte_capacity,
        }
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&(u16, Arc<RgbaImage>)> {
        self.entries.get(path)
    }

    pub(crate) fn contains_key(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.order.clear();
        self.retained_bytes = 0;
    }

    pub(crate) fn retain(&mut self, mut keep: impl FnMut(&Path) -> bool) {
        let removed = self
            .entries
            .keys()
            .filter(|path| !keep(path))
            .cloned()
            .collect::<Vec<_>>();
        for path in removed {
            self.remove(&path);
        }
    }

    pub(crate) fn insert(&mut self, path: PathBuf, value: (u16, Arc<RgbaImage>)) {
        self.remove(&path);
        let bytes = value.1.as_raw().len();
        if bytes > self.byte_capacity {
            return;
        }
        while self.retained_bytes.saturating_add(bytes) > self.byte_capacity {
            let Some(oldest) = self.order.pop_front() else {
                break;
            };
            if let Some((_, image)) = self.entries.remove(&oldest) {
                self.retained_bytes = self.retained_bytes.saturating_sub(image.as_raw().len());
            }
        }
        self.retained_bytes = self.retained_bytes.saturating_add(bytes);
        self.order.push_back(path.clone());
        self.entries.insert(path, value);
    }

    fn remove(&mut self, path: &Path) {
        if let Some((_, image)) = self.entries.remove(path) {
            self.retained_bytes = self.retained_bytes.saturating_sub(image.as_raw().len());
        }
        self.order.retain(|candidate| candidate != path);
    }
}

#[derive(Default)]
pub struct NickelIconProvider;

impl IconProvider for NickelIconProvider {
    fn source(&self) -> ArtworkSource {
        ArtworkSource::Nickel
    }

    fn revision(&self) -> u64 {
        1
    }

    fn resolve(&self, request: &ArtworkRequest<'_>) -> Option<RgbaImage> {
        Some(contain(
            nickel_runtime_master(request.kind),
            physical_size(request),
        ))
    }
}

fn nickel_runtime_master(kind: SemanticIconKind) -> &'static RgbaImage {
    static FOLDER: OnceLock<RgbaImage> = OnceLock::new();
    static HOME: OnceLock<RgbaImage> = OnceLock::new();
    static PICTURES: OnceLock<RgbaImage> = OnceLock::new();
    static MUSIC: OnceLock<RgbaImage> = OnceLock::new();
    static IMAGE: OnceLock<RgbaImage> = OnceLock::new();
    static TEXT: OnceLock<RgbaImage> = OnceLock::new();
    let (slot, bytes): (&OnceLock<RgbaImage>, &[u8]) = match kind {
        SemanticIconKind::Folder => (
            &FOLDER,
            include_bytes!("../../../assets/concepts/nickel-file-icon-family/folder.png"),
        ),
        SemanticIconKind::HomeFolder => (
            &HOME,
            include_bytes!("../../../assets/concepts/nickel-file-icon-family/home-folder.png"),
        ),
        SemanticIconKind::PicturesFolder => (
            &PICTURES,
            include_bytes!("../../../assets/concepts/nickel-file-icon-family/pictures-folder.png"),
        ),
        SemanticIconKind::MusicFolder => (
            &MUSIC,
            include_bytes!("../../../assets/concepts/nickel-file-icon-family/music-folder.png"),
        ),
        SemanticIconKind::ImageFile => (
            &IMAGE,
            include_bytes!("../../../assets/concepts/nickel-file-icon-family/image-file.png"),
        ),
        SemanticIconKind::TextFile | SemanticIconKind::UnknownFile => (
            &TEXT,
            include_bytes!("../../../assets/concepts/nickel-file-icon-family/text-file.png"),
        ),
    };
    slot.get_or_init(|| {
        let source = image::load_from_memory(bytes)
            .expect("embedded Nickel file artwork must decode")
            .into_rgba8();
        contain(&source, 96)
    })
}

#[derive(Default)]
pub struct SystemIconProvider;

impl IconProvider for SystemIconProvider {
    fn source(&self) -> ArtworkSource {
        ArtworkSource::System
    }

    fn revision(&self) -> u64 {
        1
    }

    fn resolve(&self, request: &ArtworkRequest<'_>) -> Option<RgbaImage> {
        nickel_platform::path_icon(request.path)
            .filter(has_visible_pixels)
            .map(|image| contain(&image, physical_size(request)))
    }
}

pub fn resolve_artwork(
    preference: FileIconPreference,
    request: &ArtworkRequest<'_>,
) -> ResolvedArtwork {
    let nickel = NickelIconProvider;
    let system = SystemIconProvider;
    let preferred: &dyn IconProvider = match preference {
        FileIconPreference::Nickel => &nickel,
        FileIconPreference::System => &system,
    };
    let (pixels, provider): (RgbaImage, &dyn IconProvider) = preferred
        .resolve(request)
        .filter(has_visible_pixels)
        .map(|pixels| (pixels, preferred))
        .unwrap_or_else(|| {
            (
                nickel
                    .resolve(request)
                    .expect("embedded Nickel file artwork must decode"),
                &nickel,
            )
        });
    ResolvedArtwork {
        pixels: Arc::new(pixels),
        source: provider.source(),
        provider_revision: provider.revision(),
    }
}

pub fn semantic_kind(path: &Path, is_directory: bool) -> SemanticIconKind {
    if is_directory {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        return if name.eq_ignore_ascii_case("home") {
            SemanticIconKind::HomeFolder
        } else if name.eq_ignore_ascii_case("pictures") {
            SemanticIconKind::PicturesFolder
        } else if name.eq_ignore_ascii_case("music") {
            SemanticIconKind::MusicFolder
        } else {
            SemanticIconKind::Folder
        };
    }
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("png" | "jpg" | "jpeg" | "gif" | "webp" | "svg") => SemanticIconKind::ImageFile,
        Some("txt" | "md" | "rtf" | "log" | "toml" | "yaml" | "yml" | "json" | "rs") => {
            SemanticIconKind::TextFile
        }
        _ => SemanticIconKind::UnknownFile,
    }
}

fn physical_size(request: &ArtworkRequest<'_>) -> u32 {
    (u32::from(request.logical_size) * u32::from(request.scale_milli).max(1) / 1_000).max(1)
}

fn contain(source: &RgbaImage, size: u32) -> RgbaImage {
    let visible = visible_bounds(source).unwrap_or((0, 0, source.width(), source.height()));
    let cropped =
        image::imageops::crop_imm(source, visible.0, visible.1, visible.2, visible.3).to_image();
    // Keep a small, consistent optical inset after removing generation canvas whitespace.
    let target = ((size as f32) * 0.88).round().max(1.0) as u32;
    let scale =
        (target as f32 / cropped.width() as f32).min(target as f32 / cropped.height() as f32);
    let width = (cropped.width() as f32 * scale).round().max(1.0) as u32;
    let height = (cropped.height() as f32 * scale).round().max(1.0) as u32;
    let resized = image::imageops::resize(&cropped, width, height, FilterType::Lanczos3);
    let mut canvas = RgbaImage::from_pixel(size, size, Rgba([0, 0, 0, 0]));
    image::imageops::overlay(
        &mut canvas,
        &resized,
        i64::from((size - width) / 2),
        i64::from((size - height) / 2),
    );
    canvas
}

fn visible_bounds(image: &RgbaImage) -> Option<(u32, u32, u32, u32)> {
    let mut left = image.width();
    let mut top = image.height();
    let mut right = 0;
    let mut bottom = 0;
    for (x, y, pixel) in image.enumerate_pixels() {
        if pixel.0[3] > 8 {
            left = left.min(x);
            top = top.min(y);
            right = right.max(x + 1);
            bottom = bottom.max(y + 1);
        }
    }
    (right > left && bottom > top).then_some((left, top, right - left, bottom - top))
}

fn has_visible_pixels(image: &RgbaImage) -> bool {
    image.width() > 0 && image.height() > 0 && image.pixels().any(|pixel| pixel.0[3] != 0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: SemanticIconKind) -> ArtworkRequest<'static> {
        ArtworkRequest {
            path: Path::new("/fixture/item"),
            kind,
            logical_size: 48,
            scale_milli: 1_250,
            appearance: ArtworkAppearance::Dark,
        }
    }

    #[test]
    fn platform_defaults_follow_product_policy() {
        assert_eq!(
            FileIconPreference::default(),
            if cfg!(target_os = "windows") {
                FileIconPreference::System
            } else {
                FileIconPreference::Nickel
            }
        );
    }

    #[test]
    fn nickel_fallback_is_complete_and_deterministic() {
        for kind in [
            SemanticIconKind::Folder,
            SemanticIconKind::HomeFolder,
            SemanticIconKind::PicturesFolder,
            SemanticIconKind::MusicFolder,
            SemanticIconKind::TextFile,
            SemanticIconKind::ImageFile,
            SemanticIconKind::UnknownFile,
        ] {
            let first = resolve_artwork(FileIconPreference::Nickel, &request(kind));
            let second = resolve_artwork(FileIconPreference::Nickel, &request(kind));
            assert_eq!((first.pixels.width(), first.pixels.height()), (60, 60));
            assert_eq!(first.pixels.as_raw(), second.pixels.as_raw());
            assert!(has_visible_pixels(&first.pixels));
            assert_eq!(first.source, ArtworkSource::Nickel);
            let bounds = visible_bounds(&first.pixels).expect("derived icon has visible artwork");
            assert!(
                bounds.2 >= 48 || bounds.3 >= 48,
                "visible artwork must fill the optical box"
            );
        }
    }

    #[test]
    fn semantic_mapping_does_not_depend_on_native_types() {
        assert_eq!(
            semantic_kind(Path::new("/home/me/Pictures"), true),
            SemanticIconKind::PicturesFolder
        );
        assert_eq!(
            semantic_kind(Path::new("photo.PNG"), false),
            SemanticIconKind::ImageFile
        );
        assert_eq!(
            semantic_kind(Path::new("archive.bin"), false),
            SemanticIconKind::UnknownFile
        );
    }

    #[test]
    fn decoded_pixel_cache_evicts_to_its_byte_budget() {
        let mut cache = ArtworkCache::with_capacity(80);
        for index in 0..3 {
            cache.insert(
                PathBuf::from(format!("item-{index}")),
                (index, Arc::new(RgbaImage::new(4, 4))),
            );
        }
        assert_eq!(cache.len(), 1);
        assert!(cache.retained_bytes <= cache.byte_capacity);
        assert!(cache.contains_key(Path::new("item-2")));
    }
}
