//! CPU-side text and image assets for Nickel render backends.
//!
//! This crate deliberately has no windowing or GPU dependency. SDL renderers can
//! upload [`RgbaAsset::pixels`] with a pitch of [`RgbaAsset::pitch`], while the
//! software renderer can blend the same bytes directly.

use std::{
    collections::{HashMap, VecDeque},
    hash::{Hash, Hasher},
    sync::Arc,
    time::Instant,
};

use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use image::{Rgba, RgbaImage, imageops::FilterType};

/// Immutable, tightly packed, straight-alpha RGBA pixels.
#[derive(Clone, Debug)]
pub struct RgbaAsset {
    pixels: Arc<RgbaImage>,
}

impl RgbaAsset {
    pub fn new(pixels: RgbaImage) -> Self {
        Self {
            pixels: Arc::new(pixels),
        }
    }

    pub fn pixels(&self) -> &[u8] {
        self.pixels.as_raw()
    }

    pub fn image(&self) -> &RgbaImage {
        &self.pixels
    }

    pub fn width(&self) -> u32 {
        self.pixels.width()
    }

    pub fn height(&self) -> u32 {
        self.pixels.height()
    }

    /// Row pitch accepted by `SDL_UpdateTexture`.
    pub fn pitch(&self) -> usize {
        self.width() as usize * 4
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum TextWeight {
    Normal,
    Bold,
}

/// Selects whether derived raster assets may be retained and reused.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AssetCacheMode {
    #[default]
    Enabled,
    Bypass,
}

#[derive(Clone, Debug)]
pub struct TextRequest<'a> {
    pub text: &'a str,
    pub size: f32,
    pub line_height: f32,
    pub max_width: Option<u32>,
    pub color: [u8; 4],
    pub weight: TextWeight,
}

impl<'a> TextRequest<'a> {
    pub fn label(text: &'a str, size: f32, color: [u8; 4]) -> Self {
        Self {
            text,
            size,
            line_height: (size * 1.35).ceil(),
            max_width: None,
            color,
            weight: TextWeight::Normal,
        }
    }
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextKey {
    text: Arc<str>,
    size_bits: u32,
    line_height_bits: u32,
    max_width: Option<u32>,
    color: [u8; 4],
    weight: TextWeight,
}

impl From<TextRequest<'_>> for TextKey {
    fn from(request: TextRequest<'_>) -> Self {
        Self {
            text: Arc::from(request.text),
            size_bits: request.size.to_bits(),
            line_height_bits: request.line_height.to_bits(),
            max_width: request.max_width,
            color: request.color,
            weight: request.weight,
        }
    }
}

/// Shapes with cosmic-text and caches the final CPU raster, independent of glyphon/wgpu.
pub struct TextAssetCache {
    font_system: FontSystem,
    swash_cache: SwashCache,
    assets: HashMap<TextKey, Arc<RgbaAsset>>,
    insertion_order: VecDeque<TextKey>,
    diagnostics: CacheActivity,
}

const TEXT_ASSET_CAPACITY: usize = 512;
const TEXT_ASSET_BYTE_CAPACITY: usize = 32 * 1024 * 1024;
const IMAGE_ORIGINAL_CAPACITY: usize = 256;
const IMAGE_ORIGINAL_BYTE_CAPACITY: usize = 128 * 1024 * 1024;
const IMAGE_SCALED_CAPACITY: usize = 512;
const IMAGE_SCALED_BYTE_CAPACITY: usize = 64 * 1024 * 1024;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct CacheActivity {
    estimated_retained_bytes: usize,
    peak_estimated_retained_bytes: usize,
    hits: u64,
    misses: u64,
    insertions: u64,
    evictions: u64,
    invalidations: u64,
    recomputations: u64,
    recomputation_nanos: u128,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct CacheDiagnostics {
    pub entries: usize,
    pub capacity: usize,
    /// Sum of retained RGBA pixel payloads. This excludes map, key, queue, and allocator overhead.
    pub estimated_retained_bytes: usize,
    pub byte_capacity: usize,
    pub peak_estimated_retained_bytes: usize,
    pub hits: u64,
    pub misses: u64,
    pub insertions: u64,
    pub evictions: u64,
    pub invalidations: u64,
    pub recomputations: u64,
    pub recomputation_nanos: u128,
}

impl Default for TextAssetCache {
    fn default() -> Self {
        Self::new()
    }
}

impl TextAssetCache {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            assets: HashMap::new(),
            insertion_order: VecDeque::new(),
            diagnostics: CacheActivity::default(),
        }
    }

    pub fn get(&mut self, request: TextRequest<'_>) -> Arc<RgbaAsset> {
        self.get_with_mode(request, AssetCacheMode::Enabled)
    }

    pub fn get_with_mode(
        &mut self,
        request: TextRequest<'_>,
        mode: AssetCacheMode,
    ) -> Arc<RgbaAsset> {
        let key = TextKey::from(request);
        if mode == AssetCacheMode::Bypass {
            let recompute_started = Instant::now();
            let asset = Arc::new(self.rasterize(&key));
            self.diagnostics.recomputation_nanos = self
                .diagnostics
                .recomputation_nanos
                .saturating_add(recompute_started.elapsed().as_nanos());
            self.diagnostics.recomputations = self.diagnostics.recomputations.saturating_add(1);
            return asset;
        }
        if let Some(asset) = self.assets.get(&key) {
            self.diagnostics.hits = self.diagnostics.hits.saturating_add(1);
            return Arc::clone(asset);
        }
        self.diagnostics.misses = self.diagnostics.misses.saturating_add(1);
        let recompute_started = Instant::now();
        let asset = Arc::new(self.rasterize(&key));
        self.diagnostics.recomputation_nanos = self
            .diagnostics
            .recomputation_nanos
            .saturating_add(recompute_started.elapsed().as_nanos());
        self.diagnostics.recomputations = self.diagnostics.recomputations.saturating_add(1);
        let bytes = asset_pixel_bytes(&asset);
        if bytes > TEXT_ASSET_BYTE_CAPACITY {
            return asset;
        }
        while self.assets.len() >= TEXT_ASSET_CAPACITY
            || self
                .diagnostics
                .estimated_retained_bytes
                .saturating_add(bytes)
                > TEXT_ASSET_BYTE_CAPACITY
        {
            if !evict_text_oldest(
                &mut self.assets,
                &mut self.insertion_order,
                &mut self.diagnostics,
            ) {
                break;
            }
        }
        self.insertion_order.push_back(key.clone());
        self.assets.insert(key, Arc::clone(&asset));
        record_insertion(&mut self.diagnostics, bytes);
        asset
    }

    pub fn clear(&mut self) {
        self.diagnostics.invalidations = self
            .diagnostics
            .invalidations
            .saturating_add(self.assets.len() as u64);
        self.assets.clear();
        self.insertion_order.clear();
        self.diagnostics.estimated_retained_bytes = 0;
    }

    pub fn len(&self) -> usize {
        self.assets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
    }

    pub fn diagnostics(&self) -> CacheDiagnostics {
        cache_diagnostics(
            self.assets.len(),
            TEXT_ASSET_CAPACITY,
            TEXT_ASSET_BYTE_CAPACITY,
            self.diagnostics,
        )
    }

    fn rasterize(&mut self, key: &TextKey) -> RgbaAsset {
        let size = f32::from_bits(key.size_bits).max(1.0);
        let line_height = f32::from_bits(key.line_height_bits).max(size);
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(size, line_height));
        buffer.set_wrap(if key.max_width.is_some() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        buffer.set_size(key.max_width.map(|width| width.max(1) as f32), None);
        let weight = match key.weight {
            TextWeight::Normal => Weight::NORMAL,
            TextWeight::Bold => Weight::BOLD,
        };
        buffer.set_text(
            &key.text,
            &Attrs::new().family(Family::SansSerif).weight(weight),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);

        let (layout_width, layout_height) =
            buffer
                .layout_runs()
                .fold((0.0_f32, 0.0_f32), |(width, height), run| {
                    (
                        width.max(run.line_w),
                        height.max(run.line_top + run.line_height),
                    )
                });
        let width = key
            .max_width
            .unwrap_or_else(|| layout_width.ceil().max(1.0) as u32)
            .max(1);
        let height = layout_height.ceil().max(line_height).max(1.0) as u32;
        let mut pixels = RgbaImage::new(width, height);
        let color = Color::rgba(key.color[0], key.color[1], key.color[2], key.color[3]);
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            color,
            |x, y, glyph_width, glyph_height, glyph_color| {
                fill_blended(
                    &mut pixels,
                    x,
                    y,
                    glyph_width,
                    glyph_height,
                    [
                        glyph_color.r(),
                        glyph_color.g(),
                        glyph_color.b(),
                        glyph_color.a(),
                    ],
                );
            },
        );
        RgbaAsset::new(pixels)
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct ImageAssetId(pub u64);

impl ImageAssetId {
    /// Stable within a process for immutable input pixels.
    pub fn from_rgba(image: &RgbaImage) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        image.dimensions().hash(&mut hasher);
        image.as_raw().hash(&mut hasher);
        Self(hasher.finish())
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ImageKey {
    id: ImageAssetId,
    width: u32,
    height: u32,
}

/// Caches decoded icons, wallpaper fragments, previews, and their scaled variants.
#[derive(Default)]
pub struct ImageAssetCache {
    originals: HashMap<ImageAssetId, Arc<RgbaAsset>>,
    scaled: HashMap<ImageKey, Arc<RgbaAsset>>,
    original_order: VecDeque<ImageAssetId>,
    scaled_order: VecDeque<ImageKey>,
    original_diagnostics: CacheActivity,
    scaled_diagnostics: CacheActivity,
}

impl ImageAssetCache {
    pub fn insert(&mut self, id: ImageAssetId, image: RgbaImage) -> Arc<RgbaAsset> {
        self.remove(id);
        let asset = Arc::new(RgbaAsset::new(image));
        let bytes = asset_pixel_bytes(&asset);
        if bytes > IMAGE_ORIGINAL_BYTE_CAPACITY {
            return asset;
        }
        while self.originals.len() >= IMAGE_ORIGINAL_CAPACITY
            || self
                .original_diagnostics
                .estimated_retained_bytes
                .saturating_add(bytes)
                > IMAGE_ORIGINAL_BYTE_CAPACITY
        {
            let Some(oldest) = self.original_order.pop_front() else {
                break;
            };
            if let Some(removed) = self.originals.remove(&oldest) {
                subtract_retained(&mut self.original_diagnostics, asset_pixel_bytes(&removed));
                self.original_diagnostics.evictions =
                    self.original_diagnostics.evictions.saturating_add(1);
                self.evict_scaled_for_original(oldest);
            }
        }
        self.original_order.push_back(id);
        self.originals.insert(id, Arc::clone(&asset));
        record_insertion(&mut self.original_diagnostics, bytes);
        asset
    }

    pub fn insert_content_addressed(&mut self, image: RgbaImage) -> (ImageAssetId, Arc<RgbaAsset>) {
        let id = ImageAssetId::from_rgba(&image);
        let asset = self.insert(id, image);
        (id, asset)
    }

    pub fn get(&mut self, id: ImageAssetId) -> Option<Arc<RgbaAsset>> {
        let asset = self.originals.get(&id).cloned();
        if asset.is_some() {
            self.original_diagnostics.hits = self.original_diagnostics.hits.saturating_add(1);
        } else {
            self.original_diagnostics.misses = self.original_diagnostics.misses.saturating_add(1);
        }
        asset
    }

    pub fn scaled(&mut self, id: ImageAssetId, width: u32, height: u32) -> Option<Arc<RgbaAsset>> {
        self.scaled_with_mode(id, width, height, AssetCacheMode::Enabled)
    }

    pub fn scaled_with_mode(
        &mut self,
        id: ImageAssetId,
        width: u32,
        height: u32,
        mode: AssetCacheMode,
    ) -> Option<Arc<RgbaAsset>> {
        let key = ImageKey {
            id,
            width: width.max(1),
            height: height.max(1),
        };
        if mode == AssetCacheMode::Bypass {
            let original = self.originals.get(&id)?;
            let recompute_started = Instant::now();
            let pixels = image::imageops::resize(
                original.image(),
                key.width,
                key.height,
                FilterType::Triangle,
            );
            self.scaled_diagnostics.recomputation_nanos = self
                .scaled_diagnostics
                .recomputation_nanos
                .saturating_add(recompute_started.elapsed().as_nanos());
            self.scaled_diagnostics.recomputations =
                self.scaled_diagnostics.recomputations.saturating_add(1);
            return Some(Arc::new(RgbaAsset::new(pixels)));
        }
        if let Some(asset) = self.scaled.get(&key) {
            self.scaled_diagnostics.hits = self.scaled_diagnostics.hits.saturating_add(1);
            return Some(Arc::clone(asset));
        }
        self.scaled_diagnostics.misses = self.scaled_diagnostics.misses.saturating_add(1);
        let original = self.originals.get(&id)?;
        let recompute_started = Instant::now();
        let pixels = image::imageops::resize(
            original.image(),
            key.width,
            key.height,
            FilterType::Triangle,
        );
        self.scaled_diagnostics.recomputation_nanos = self
            .scaled_diagnostics
            .recomputation_nanos
            .saturating_add(recompute_started.elapsed().as_nanos());
        let asset = Arc::new(RgbaAsset::new(pixels));
        self.scaled_diagnostics.recomputations =
            self.scaled_diagnostics.recomputations.saturating_add(1);
        let bytes = asset_pixel_bytes(&asset);
        if bytes > IMAGE_SCALED_BYTE_CAPACITY {
            return Some(asset);
        }
        while self.scaled.len() >= IMAGE_SCALED_CAPACITY
            || self
                .scaled_diagnostics
                .estimated_retained_bytes
                .saturating_add(bytes)
                > IMAGE_SCALED_BYTE_CAPACITY
        {
            let Some(oldest) = self.scaled_order.pop_front() else {
                break;
            };
            if let Some(removed) = self.scaled.remove(&oldest) {
                subtract_retained(&mut self.scaled_diagnostics, asset_pixel_bytes(&removed));
                self.scaled_diagnostics.evictions =
                    self.scaled_diagnostics.evictions.saturating_add(1);
            }
        }
        self.scaled_order.push_back(key);
        self.scaled.insert(key, Arc::clone(&asset));
        record_insertion(&mut self.scaled_diagnostics, bytes);
        Some(asset)
    }

    pub fn remove(&mut self, id: ImageAssetId) {
        if let Some(removed) = self.originals.remove(&id) {
            subtract_retained(&mut self.original_diagnostics, asset_pixel_bytes(&removed));
            self.original_diagnostics.invalidations =
                self.original_diagnostics.invalidations.saturating_add(1);
        }
        self.invalidate_scaled_for_original(id);
        self.original_order.retain(|candidate| *candidate != id);
    }

    pub fn clear(&mut self) {
        self.original_diagnostics.invalidations = self
            .original_diagnostics
            .invalidations
            .saturating_add(self.originals.len() as u64);
        self.scaled_diagnostics.invalidations = self
            .scaled_diagnostics
            .invalidations
            .saturating_add(self.scaled.len() as u64);
        self.originals.clear();
        self.scaled.clear();
        self.original_order.clear();
        self.scaled_order.clear();
        self.original_diagnostics.estimated_retained_bytes = 0;
        self.scaled_diagnostics.estimated_retained_bytes = 0;
    }

    pub fn diagnostics(&self) -> (CacheDiagnostics, CacheDiagnostics) {
        (
            cache_diagnostics(
                self.originals.len(),
                IMAGE_ORIGINAL_CAPACITY,
                IMAGE_ORIGINAL_BYTE_CAPACITY,
                self.original_diagnostics,
            ),
            cache_diagnostics(
                self.scaled.len(),
                IMAGE_SCALED_CAPACITY,
                IMAGE_SCALED_BYTE_CAPACITY,
                self.scaled_diagnostics,
            ),
        )
    }

    fn evict_scaled_for_original(&mut self, id: ImageAssetId) {
        let keys = self
            .scaled
            .keys()
            .filter(|key| key.id == id)
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(removed) = self.scaled.remove(&key) {
                subtract_retained(&mut self.scaled_diagnostics, asset_pixel_bytes(&removed));
                self.scaled_diagnostics.evictions =
                    self.scaled_diagnostics.evictions.saturating_add(1);
            }
        }
        self.scaled_order.retain(|key| key.id != id);
    }

    fn invalidate_scaled_for_original(&mut self, id: ImageAssetId) {
        let keys = self
            .scaled
            .keys()
            .filter(|key| key.id == id)
            .copied()
            .collect::<Vec<_>>();
        for key in keys {
            if let Some(removed) = self.scaled.remove(&key) {
                subtract_retained(&mut self.scaled_diagnostics, asset_pixel_bytes(&removed));
                self.scaled_diagnostics.invalidations =
                    self.scaled_diagnostics.invalidations.saturating_add(1);
            }
        }
        self.scaled_order.retain(|key| key.id != id);
    }
}

fn asset_pixel_bytes(asset: &RgbaAsset) -> usize {
    asset.pixels().len()
}

fn subtract_retained(activity: &mut CacheActivity, bytes: usize) {
    activity.estimated_retained_bytes = activity.estimated_retained_bytes.saturating_sub(bytes);
}

fn record_insertion(activity: &mut CacheActivity, bytes: usize) {
    activity.insertions = activity.insertions.saturating_add(1);
    activity.estimated_retained_bytes = activity.estimated_retained_bytes.saturating_add(bytes);
    activity.peak_estimated_retained_bytes = activity
        .peak_estimated_retained_bytes
        .max(activity.estimated_retained_bytes);
}

fn cache_diagnostics(
    entries: usize,
    capacity: usize,
    byte_capacity: usize,
    activity: CacheActivity,
) -> CacheDiagnostics {
    CacheDiagnostics {
        entries,
        capacity,
        estimated_retained_bytes: activity.estimated_retained_bytes,
        byte_capacity,
        peak_estimated_retained_bytes: activity.peak_estimated_retained_bytes,
        hits: activity.hits,
        misses: activity.misses,
        insertions: activity.insertions,
        evictions: activity.evictions,
        invalidations: activity.invalidations,
        recomputations: activity.recomputations,
        recomputation_nanos: activity.recomputation_nanos,
    }
}

fn evict_text_oldest(
    assets: &mut HashMap<TextKey, Arc<RgbaAsset>>,
    insertion_order: &mut VecDeque<TextKey>,
    diagnostics: &mut CacheActivity,
) -> bool {
    while let Some(oldest) = insertion_order.pop_front() {
        if let Some(removed) = assets.remove(&oldest) {
            subtract_retained(diagnostics, asset_pixel_bytes(&removed));
            diagnostics.evictions = diagnostics.evictions.saturating_add(1);
            return true;
        }
    }
    false
}

fn fill_blended(target: &mut RgbaImage, x: i32, y: i32, width: u32, height: u32, source: [u8; 4]) {
    for offset_y in 0..height {
        for offset_x in 0..width {
            let pixel_x = x + offset_x as i32;
            let pixel_y = y + offset_y as i32;
            if pixel_x < 0
                || pixel_y < 0
                || pixel_x >= target.width() as i32
                || pixel_y >= target.height() as i32
            {
                continue;
            }
            let destination = target.get_pixel_mut(pixel_x as u32, pixel_y as u32);
            *destination = Rgba(source_over(destination.0, source));
        }
    }
}

fn source_over(destination: [u8; 4], source: [u8; 4]) -> [u8; 4] {
    let source_alpha = source[3] as u32;
    let inverse_alpha = 255 - source_alpha;
    let output_alpha = source_alpha + (destination[3] as u32 * inverse_alpha + 127) / 255;
    if output_alpha == 0 {
        return [0; 4];
    }
    let channel = |index: usize| {
        let premultiplied = source[index] as u32 * source_alpha
            + (destination[index] as u32 * destination[3] as u32 * inverse_alpha + 127) / 255;
        ((premultiplied + output_alpha / 2) / output_alpha).min(255) as u8
    };
    [
        channel(0),
        channel(1),
        channel(2),
        output_alpha.min(255) as u8,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rasterized_text_contains_visible_pixels_and_is_cached() {
        let mut cache = TextAssetCache::new();
        let first = cache.get(TextRequest::label("Nickel", 18.0, [255, 255, 255, 255]));
        let second = cache.get(TextRequest::label("Nickel", 18.0, [255, 255, 255, 255]));

        assert!(first.width() > 0);
        assert!(first.height() > 0);
        assert_eq!(first.pitch(), first.width() as usize * 4);
        assert!(first.image().pixels().any(|pixel| pixel[3] != 0));
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(cache.len(), 1);

        let before_clear = cache.diagnostics();
        assert_eq!(before_clear.hits, 1);
        assert_eq!(before_clear.misses, 1);
        assert_eq!(before_clear.insertions, 1);
        assert_eq!(before_clear.recomputations, 1);
        assert!(before_clear.recomputation_nanos > 0);
        assert_eq!(before_clear.estimated_retained_bytes, first.pixels().len());

        cache.clear();
        assert!(cache.is_empty());
        let after_clear = cache.diagnostics();
        assert_eq!(after_clear.estimated_retained_bytes, 0);
        assert_eq!(after_clear.invalidations, 1);
        assert_eq!(
            after_clear.peak_estimated_retained_bytes,
            before_clear.estimated_retained_bytes
        );
    }

    #[test]
    fn scaled_image_is_cached() {
        let mut cache = ImageAssetCache::default();
        let (id, _) =
            cache.insert_content_addressed(RgbaImage::from_pixel(2, 2, Rgba([5, 10, 15, 255])));
        let first = cache.scaled(id, 16, 16).expect("inserted image");
        let second = cache.scaled(id, 16, 16).expect("inserted image");

        assert_eq!((first.width(), first.height()), (16, 16));
        assert!(Arc::ptr_eq(&first, &second));
        let (originals, scaled) = cache.diagnostics();
        assert_eq!(originals.insertions, 1);
        assert_eq!(scaled.hits, 1);
        assert_eq!(scaled.misses, 1);
        assert_eq!(scaled.insertions, 1);
        assert_eq!(scaled.recomputations, 1);
        assert!(scaled.recomputation_nanos > 0);
    }

    #[test]
    fn text_cache_reports_and_enforces_its_hard_bound() {
        let mut cache = TextAssetCache::new();
        for index in 0..=TEXT_ASSET_CAPACITY {
            cache.get(TextRequest::label(
                &format!("label-{index}"),
                8.0,
                [255, 255, 255, 255],
            ));
        }

        let diagnostics = cache.diagnostics();
        assert_eq!(diagnostics.entries, TEXT_ASSET_CAPACITY);
        assert_eq!(diagnostics.capacity, TEXT_ASSET_CAPACITY);
        assert_eq!(diagnostics.misses, (TEXT_ASSET_CAPACITY + 1) as u64);
        assert_eq!(diagnostics.insertions, diagnostics.misses);
        assert_eq!(diagnostics.evictions, 1);
        assert!(diagnostics.estimated_retained_bytes <= diagnostics.byte_capacity);
        assert!(diagnostics.peak_estimated_retained_bytes <= diagnostics.byte_capacity);
    }

    #[test]
    fn image_cache_bounds_originals_and_scaled_variants_independently() {
        let mut cache = ImageAssetCache::default();
        for index in 0..=IMAGE_ORIGINAL_CAPACITY {
            cache.insert(
                ImageAssetId(index as u64),
                RgbaImage::from_pixel(1, 1, Rgba([index as u8, 0, 0, 255])),
            );
        }
        let retained = ImageAssetId(IMAGE_ORIGINAL_CAPACITY as u64);
        for size in 1..=IMAGE_SCALED_CAPACITY + 1 {
            cache
                .scaled(retained, size as u32, 1)
                .expect("retained original");
        }

        let (originals, scaled) = cache.diagnostics();
        assert_eq!(originals.entries, IMAGE_ORIGINAL_CAPACITY);
        assert_eq!(originals.capacity, IMAGE_ORIGINAL_CAPACITY);
        assert_eq!(originals.insertions, (IMAGE_ORIGINAL_CAPACITY + 1) as u64);
        assert_eq!(originals.evictions, 1);
        assert!(originals.estimated_retained_bytes <= originals.byte_capacity);
        assert_eq!(scaled.entries, IMAGE_SCALED_CAPACITY);
        assert_eq!(scaled.capacity, IMAGE_SCALED_CAPACITY);
        assert_eq!(scaled.misses, (IMAGE_SCALED_CAPACITY + 1) as u64);
        assert_eq!(scaled.recomputations, scaled.misses);
        assert_eq!(scaled.evictions, 1);
        assert!(scaled.estimated_retained_bytes <= scaled.byte_capacity);
    }

    #[test]
    fn image_cache_byte_bounds_and_lifecycle_account_for_cascaded_variants() {
        let mut cache = ImageAssetCache::default();
        for index in 0..5 {
            cache.insert(
                ImageAssetId(index),
                RgbaImage::from_pixel(4096, 2048, Rgba([index as u8, 0, 0, 255])),
            );
        }
        let retained = ImageAssetId(4);
        cache
            .scaled(retained, 2048, 2048)
            .expect("retained original");
        cache
            .scaled(retained, 2048, 2047)
            .expect("retained original");
        cache
            .scaled(retained, 2048, 2046)
            .expect("retained original");
        cache
            .scaled(retained, 2048, 2045)
            .expect("retained original");
        cache
            .scaled(retained, 2048, 2044)
            .expect("retained original");

        let (originals, scaled) = cache.diagnostics();
        assert!(originals.estimated_retained_bytes <= originals.byte_capacity);
        assert!(originals.peak_estimated_retained_bytes <= originals.byte_capacity);
        assert!(originals.evictions > 0);
        assert!(scaled.estimated_retained_bytes <= scaled.byte_capacity);
        assert!(scaled.peak_estimated_retained_bytes <= scaled.byte_capacity);
        assert!(scaled.evictions > 0);

        cache.remove(retained);
        let (after_remove, after_scaled_remove) = cache.diagnostics();
        assert_eq!(after_remove.invalidations, 1);
        assert_eq!(after_scaled_remove.entries, 0);
        assert!(after_scaled_remove.invalidations > 0);

        cache.clear();
        let (after_clear, after_scaled_clear) = cache.diagnostics();
        assert_eq!(after_clear.entries, 0);
        assert_eq!(after_clear.estimated_retained_bytes, 0);
        assert_eq!(after_scaled_clear.estimated_retained_bytes, 0);
    }

    #[test]
    fn text_cache_byte_churn_returns_to_steady_state() {
        let mut cache = TextAssetCache::new();
        for index in 0..4 {
            cache.get(TextRequest {
                text: &format!("wide-{index}"),
                size: 8.0,
                line_height: 1024.0,
                max_width: Some(4096),
                color: [255, 255, 255, 255],
                weight: TextWeight::Normal,
            });
        }
        let diagnostics = cache.diagnostics();
        assert!(diagnostics.entries < TEXT_ASSET_CAPACITY);
        assert!(diagnostics.evictions > 0);
        assert!(diagnostics.estimated_retained_bytes <= diagnostics.byte_capacity);
        assert!(diagnostics.peak_estimated_retained_bytes <= diagnostics.byte_capacity);

        cache.clear();
        assert_eq!(cache.diagnostics().estimated_retained_bytes, 0);
    }

    #[derive(Clone, Copy)]
    struct AdmissionStats {
        median_us: f64,
        p95_us: f64,
    }

    fn admission_stats(mut samples: Vec<f64>) -> AdmissionStats {
        samples.sort_by(f64::total_cmp);
        let nearest_rank =
            |percent: usize| samples[(samples.len() * percent).div_ceil(100).saturating_sub(1)];
        AdmissionStats {
            median_us: nearest_rank(50),
            p95_us: nearest_rank(95),
        }
    }

    fn print_admission(
        cache: &str,
        workload: &str,
        cached: AdmissionStats,
        bypass: AdmissionStats,
        retained_bytes: usize,
        key_fields: usize,
        invalidation_triggers: usize,
    ) {
        println!(
            "{{\"schema\":\"nickel-cache-admission-v1\",\"cache\":\"{cache}\",\"workload\":\"{workload}\",\"fixture\":\"deterministic_headless\",\"profile\":\"release\",\"samples\":31,\"cached_median_us\":{:.3},\"cached_p95_us\":{:.3},\"bypass_median_us\":{:.3},\"bypass_p95_us\":{:.3},\"retained_bytes\":{retained_bytes},\"complexity\":{{\"key_fields\":{key_fields},\"invalidation_triggers\":{invalidation_triggers},\"storage_collections\":2}},\"output_equivalence\":\"exact_rgba\"}}",
            cached.median_us, cached.p95_us, bypass.median_us, bypass.p95_us
        );
    }

    fn admission_text_request(text: &str) -> TextRequest<'_> {
        TextRequest {
            text,
            size: 22.0,
            line_height: 30.0,
            max_width: Some(640),
            color: [232, 237, 244, 255],
            weight: TextWeight::Normal,
        }
    }

    #[test]
    #[ignore = "release-mode cache admission benchmark"]
    fn shared_cpu_text_rasters_admission_workloads() {
        const SAMPLES: usize = 31;
        const WARM_P95_BENEFIT_US: f64 = 100.0;
        let mut cache = TextAssetCache::new();
        cache.get(admission_text_request(
            "Nickel shared CPU text raster admission workload",
        ));

        for workload in ["cold", "warm", "churn", "low_reuse"] {
            let mut cached_samples = Vec::with_capacity(SAMPLES);
            let mut bypass_samples = Vec::with_capacity(SAMPLES);
            for sample in 0..SAMPLES {
                if workload == "cold" {
                    cache.clear();
                }
                let text = match workload {
                    "churn" => format!("Nickel palette generation {}", sample % 4),
                    "low_reuse" => format!("Nickel unique search result {sample}"),
                    _ => "Nickel shared CPU text raster admission workload".to_owned(),
                };
                let started = Instant::now();
                let cached =
                    cache.get_with_mode(admission_text_request(&text), AssetCacheMode::Enabled);
                cached_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
                let started = Instant::now();
                let bypass =
                    cache.get_with_mode(admission_text_request(&text), AssetCacheMode::Bypass);
                bypass_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
                assert_eq!(cached.image(), bypass.image());
            }
            let cached = admission_stats(cached_samples);
            let bypass = admission_stats(bypass_samples);
            print_admission(
                "shared_cpu_text_rasters",
                workload,
                cached,
                bypass,
                cache.diagnostics().estimated_retained_bytes,
                6,
                3,
            );
            if workload == "warm" {
                assert!(
                    bypass.p95_us - cached.p95_us > WARM_P95_BENEFIT_US,
                    "predeclared warm p95 benefit was not met"
                );
            }
        }
    }

    #[test]
    #[ignore = "release-mode cache admission benchmark"]
    fn shared_scaled_images_admission_workloads() {
        const SAMPLES: usize = 31;
        const WARM_P95_BENEFIT_US: f64 = 100.0;
        let source = RgbaImage::from_fn(512, 512, |x, y| {
            Rgba([(x % 251) as u8, (y % 241) as u8, ((x + y) % 239) as u8, 255])
        });
        let mut cache = ImageAssetCache::default();
        let id = ImageAssetId::from_rgba(&source);
        cache.insert(id, source);
        cache.scaled(id, 256, 256).expect("warm scaled image");

        for workload in ["cold", "warm", "churn", "low_reuse"] {
            let mut cached_samples = Vec::with_capacity(SAMPLES);
            let mut bypass_samples = Vec::with_capacity(SAMPLES);
            for sample in 0..SAMPLES {
                if workload == "cold" {
                    cache.invalidate_scaled_for_original(id);
                }
                let (width, height) = match workload {
                    "churn" => (224 + (sample % 4) as u32 * 8, 224 + (sample % 4) as u32 * 8),
                    "low_reuse" => (180 + sample as u32, 220 + sample as u32),
                    _ => (256, 256),
                };
                let started = Instant::now();
                let cached = cache
                    .scaled_with_mode(id, width, height, AssetCacheMode::Enabled)
                    .expect("cached scale");
                cached_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
                let started = Instant::now();
                let bypass = cache
                    .scaled_with_mode(id, width, height, AssetCacheMode::Bypass)
                    .expect("bypassed scale");
                bypass_samples.push(started.elapsed().as_secs_f64() * 1_000_000.0);
                assert_eq!(cached.image(), bypass.image());
            }
            let cached = admission_stats(cached_samples);
            let bypass = admission_stats(bypass_samples);
            print_admission(
                "shared_scaled_images",
                workload,
                cached,
                bypass,
                cache.diagnostics().1.estimated_retained_bytes,
                3,
                3,
            );
            if workload == "warm" {
                assert!(
                    bypass.p95_us - cached.p95_us > WARM_P95_BENEFIT_US,
                    "predeclared warm p95 benefit was not met"
                );
            }
        }
    }
}
