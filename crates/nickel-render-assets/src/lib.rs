//! CPU-side text and image assets for Nickel render backends.
//!
//! This crate deliberately has no windowing or GPU dependency. SDL renderers can
//! upload [`RgbaAsset::pixels`] with a pitch of [`RgbaAsset::pitch`], while the
//! software renderer can blend the same bytes directly.

use std::{
    collections::HashMap,
    hash::{Hash, Hasher},
    sync::Arc,
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
        }
    }

    pub fn get(&mut self, request: TextRequest<'_>) -> Arc<RgbaAsset> {
        let key = TextKey::from(request);
        if let Some(asset) = self.assets.get(&key) {
            return Arc::clone(asset);
        }
        let asset = Arc::new(self.rasterize(&key));
        self.assets.insert(key, Arc::clone(&asset));
        asset
    }

    pub fn clear(&mut self) {
        self.assets.clear();
    }

    pub fn len(&self) -> usize {
        self.assets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.assets.is_empty()
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
}

impl ImageAssetCache {
    pub fn insert(&mut self, id: ImageAssetId, image: RgbaImage) -> Arc<RgbaAsset> {
        self.scaled.retain(|key, _| key.id != id);
        let asset = Arc::new(RgbaAsset::new(image));
        self.originals.insert(id, Arc::clone(&asset));
        asset
    }

    pub fn insert_content_addressed(&mut self, image: RgbaImage) -> (ImageAssetId, Arc<RgbaAsset>) {
        let id = ImageAssetId::from_rgba(&image);
        let asset = self.insert(id, image);
        (id, asset)
    }

    pub fn get(&self, id: ImageAssetId) -> Option<Arc<RgbaAsset>> {
        self.originals.get(&id).cloned()
    }

    pub fn scaled(&mut self, id: ImageAssetId, width: u32, height: u32) -> Option<Arc<RgbaAsset>> {
        let key = ImageKey {
            id,
            width: width.max(1),
            height: height.max(1),
        };
        if let Some(asset) = self.scaled.get(&key) {
            return Some(Arc::clone(asset));
        }
        let original = self.originals.get(&id)?;
        let pixels = image::imageops::resize(
            original.image(),
            key.width,
            key.height,
            FilterType::Triangle,
        );
        let asset = Arc::new(RgbaAsset::new(pixels));
        self.scaled.insert(key, Arc::clone(&asset));
        Some(asset)
    }

    pub fn remove(&mut self, id: ImageAssetId) {
        self.originals.remove(&id);
        self.scaled.retain(|key, _| key.id != id);
    }

    pub fn clear(&mut self) {
        self.originals.clear();
        self.scaled.clear();
    }
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

        cache.clear();
        assert!(cache.is_empty());
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
    }
}
