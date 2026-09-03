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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ArtworkCacheKey {
    pub provider: ArtworkSource,
    pub provider_revision: u64,
    pub entry: PathBuf,
    pub kind: SemanticIconKind,
    pub logical_size: u16,
    pub scale_milli: u16,
    pub appearance: ArtworkAppearance,
}

impl ArtworkCacheKey {
    pub fn new(
        provider: ArtworkSource,
        provider_revision: u64,
        request: &ArtworkRequest<'_>,
    ) -> Self {
        Self {
            provider,
            provider_revision,
            entry: request.path.to_path_buf(),
            kind: request.kind,
            logical_size: request.logical_size,
            scale_milli: request.scale_milli,
            appearance: request.appearance,
        }
    }
}

#[derive(Clone)]
pub struct ResolvedArtwork {
    pub pixels: Arc<RgbaImage>,
    pub source: ArtworkSource,
    pub provider_revision: u64,
    pub fallback_reason: Option<ArtworkFallbackReason>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconProviderError {
    Malformed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtworkFallbackReason {
    Missing,
    Malformed,
    Transparent,
}

pub trait IconProvider: Send + Sync {
    fn source(&self) -> ArtworkSource;
    fn revision(&self) -> u64;
    fn resolve(&self, request: &ArtworkRequest<'_>)
    -> Result<Option<RgbaImage>, IconProviderError>;
}

pub(crate) struct ArtworkCache {
    entries: HashMap<PathBuf, (u16, Arc<RgbaImage>)>,
    keys: HashMap<PathBuf, ArtworkCacheKey>,
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
            keys: HashMap::new(),
            order: VecDeque::new(),
            retained_bytes: 0,
            byte_capacity,
        }
    }

    pub(crate) fn get(&self, path: &Path) -> Option<&(u16, Arc<RgbaImage>)> {
        self.entries.get(path)
    }

    pub(crate) fn matches(&self, key: &ArtworkCacheKey) -> bool {
        self.keys.get(&key.entry) == Some(key)
    }

    pub(crate) fn len(&self) -> usize {
        self.entries.len()
    }

    pub(crate) fn clear(&mut self) {
        self.entries.clear();
        self.keys.clear();
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
        self.insert_keyed(path, value, None);
    }

    pub(crate) fn insert_resolved(&mut self, key: ArtworkCacheKey, value: (u16, Arc<RgbaImage>)) {
        self.insert_keyed(key.entry.clone(), value, Some(key));
    }

    fn insert_keyed(
        &mut self,
        path: PathBuf,
        value: (u16, Arc<RgbaImage>),
        key: Option<ArtworkCacheKey>,
    ) {
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
            self.keys.remove(&oldest);
        }
        self.retained_bytes = self.retained_bytes.saturating_add(bytes);
        self.order.push_back(path.clone());
        if let Some(key) = key {
            self.keys.insert(path.clone(), key);
        }
        self.entries.insert(path, value);
    }

    fn remove(&mut self, path: &Path) {
        if let Some((_, image)) = self.entries.remove(path) {
            self.retained_bytes = self.retained_bytes.saturating_sub(image.as_raw().len());
        }
        self.keys.remove(path);
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

    fn resolve(
        &self,
        request: &ArtworkRequest<'_>,
    ) -> Result<Option<RgbaImage>, IconProviderError> {
        Ok(Some(contain(
            nickel_runtime_master(request.kind),
            physical_size(request),
        )))
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

    fn resolve(
        &self,
        request: &ArtworkRequest<'_>,
    ) -> Result<Option<RgbaImage>, IconProviderError> {
        Ok(nickel_platform::path_icon(request.path)
            .map(|image| contain(&image, physical_size(request))))
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
    resolve_with_providers(preferred, &nickel, request)
}

pub fn cache_key(preference: FileIconPreference, request: &ArtworkRequest<'_>) -> ArtworkCacheKey {
    match preference {
        FileIconPreference::Nickel => ArtworkCacheKey::new(
            ArtworkSource::Nickel,
            NickelIconProvider.revision(),
            request,
        ),
        FileIconPreference::System => ArtworkCacheKey::new(
            ArtworkSource::System,
            SystemIconProvider.revision(),
            request,
        ),
    }
}

fn resolve_with_providers(
    preferred: &dyn IconProvider,
    fallback: &dyn IconProvider,
    request: &ArtworkRequest<'_>,
) -> ResolvedArtwork {
    let preferred_result = preferred.resolve(request);
    let fallback_reason = match &preferred_result {
        Ok(Some(pixels)) if has_visible_pixels(pixels) => None,
        Ok(Some(_)) => Some(ArtworkFallbackReason::Transparent),
        Ok(None) => Some(ArtworkFallbackReason::Missing),
        Err(IconProviderError::Malformed) => Some(ArtworkFallbackReason::Malformed),
    };
    let (pixels, provider): (RgbaImage, &dyn IconProvider) = if fallback_reason.is_none() {
        (preferred_result.unwrap().unwrap(), preferred)
    } else {
        (
            fallback
                .resolve(request)
                .expect("fallback provider must resolve without error")
                .filter(has_visible_pixels)
                .expect("fallback provider must return visible artwork"),
            fallback,
        )
    };
    ResolvedArtwork {
        pixels: Arc::new(pixels),
        source: provider.source(),
        provider_revision: provider.revision(),
        fallback_reason,
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

    enum FakeResult {
        Visible,
        Missing,
        Transparent,
        Malformed,
    }

    struct FakeProvider {
        source: ArtworkSource,
        revision: u64,
        result: FakeResult,
    }

    impl IconProvider for FakeProvider {
        fn source(&self) -> ArtworkSource {
            self.source
        }

        fn revision(&self) -> u64 {
            self.revision
        }

        fn resolve(&self, _: &ArtworkRequest<'_>) -> Result<Option<RgbaImage>, IconProviderError> {
            match self.result {
                FakeResult::Visible => {
                    Ok(Some(RgbaImage::from_pixel(8, 4, Rgba([20, 40, 60, 255]))))
                }
                FakeResult::Missing => Ok(None),
                FakeResult::Transparent => Ok(Some(RgbaImage::new(8, 4))),
                FakeResult::Malformed => Err(IconProviderError::Malformed),
            }
        }
    }

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
        assert!(cache.entries.contains_key(Path::new("item-2")));
    }

    #[test]
    fn cache_identity_covers_provider_revision_entry_kind_size_scale_and_appearance() {
        let path = PathBuf::from("/fixture/item.txt");
        let request = ArtworkRequest {
            path: &path,
            kind: SemanticIconKind::TextFile,
            logical_size: 48,
            scale_milli: 1_250,
            appearance: ArtworkAppearance::Dark,
        };
        let key = ArtworkCacheKey::new(ArtworkSource::System, 3, &request);
        let variants = [
            ArtworkCacheKey {
                provider: ArtworkSource::Nickel,
                ..key.clone()
            },
            ArtworkCacheKey {
                provider_revision: 4,
                ..key.clone()
            },
            ArtworkCacheKey {
                entry: PathBuf::from("/fixture/other.txt"),
                ..key.clone()
            },
            ArtworkCacheKey {
                kind: SemanticIconKind::ImageFile,
                ..key.clone()
            },
            ArtworkCacheKey {
                logical_size: 64,
                ..key.clone()
            },
            ArtworkCacheKey {
                scale_milli: 2_000,
                ..key.clone()
            },
            ArtworkCacheKey {
                appearance: ArtworkAppearance::Light,
                ..key.clone()
            },
        ];
        assert!(variants.iter().all(|variant| variant != &key));

        let mut cache = ArtworkCache::with_capacity(1_024);
        cache.insert_resolved(key.clone(), (1, Arc::new(RgbaImage::new(4, 4))));
        assert!(cache.matches(&key));
        assert!(variants.iter().all(|variant| !cache.matches(variant)));
    }

    #[test]
    fn provider_contract_reports_success_and_revision() {
        let provider = FakeProvider {
            source: ArtworkSource::System,
            revision: 41,
            result: FakeResult::Visible,
        };
        let resolved = resolve_with_providers(
            &provider,
            &NickelIconProvider,
            &request(SemanticIconKind::TextFile),
        );

        assert_eq!(resolved.source, ArtworkSource::System);
        assert_eq!(resolved.provider_revision, 41);
        assert_eq!(resolved.fallback_reason, None);
        assert_eq!((resolved.pixels.width(), resolved.pixels.height()), (8, 4));
    }

    #[test]
    fn provider_contract_distinguishes_fallback_causes() {
        for (result, reason) in [
            (FakeResult::Missing, ArtworkFallbackReason::Missing),
            (FakeResult::Transparent, ArtworkFallbackReason::Transparent),
            (FakeResult::Malformed, ArtworkFallbackReason::Malformed),
        ] {
            let provider = FakeProvider {
                source: ArtworkSource::System,
                revision: 7,
                result,
            };
            let resolved = resolve_with_providers(
                &provider,
                &NickelIconProvider,
                &request(SemanticIconKind::UnknownFile),
            );
            assert_eq!(resolved.source, ArtworkSource::Nickel);
            assert_eq!(resolved.provider_revision, 1);
            assert_eq!(resolved.fallback_reason, Some(reason));
            assert!(has_visible_pixels(&resolved.pixels));
        }
    }

    #[test]
    fn changed_provider_revision_is_visible_to_cache_diagnostics() {
        let resolve_revision = |revision| {
            resolve_with_providers(
                &FakeProvider {
                    source: ArtworkSource::System,
                    revision,
                    result: FakeResult::Visible,
                },
                &NickelIconProvider,
                &request(SemanticIconKind::ImageFile),
            )
        };

        assert_ne!(
            resolve_revision(8).provider_revision,
            resolve_revision(9).provider_revision
        );
    }
}
