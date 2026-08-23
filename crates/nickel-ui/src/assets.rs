use std::{collections::HashMap, sync::Arc};

use cosmic_text::{
    Attrs, Buffer, Color, Family, FontSystem, Metrics, Shaping, SwashCache, Weight, Wrap,
};
use image::RgbaImage;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub(crate) enum TextWeight {
    Normal,
    Bold,
}

pub(crate) struct TextRequest<'a> {
    pub text: &'a str,
    pub size: f32,
    pub line_height: f32,
    pub max_width: Option<u32>,
    pub color: [u8; 4],
    pub weight: TextWeight,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
struct TextKey {
    text: Arc<str>,
    locale: Arc<str>,
    size: u32,
    line_height: u32,
    max_width: Option<u32>,
    color: [u8; 4],
    weight: TextWeight,
}

pub(crate) struct RgbaAsset(RgbaImage);

impl RgbaAsset {
    pub fn pixels(&self) -> &[u8] {
        self.0.as_raw()
    }

    pub fn width(&self) -> u32 {
        self.0.width()
    }

    pub fn height(&self) -> u32 {
        self.0.height()
    }

    pub fn pitch(&self) -> usize {
        self.width() as usize * 4
    }
}

pub(crate) struct TextAssetCache {
    font_system: FontSystem,
    swash_cache: SwashCache,
    assets: HashMap<TextKey, Arc<RgbaAsset>>,
}

impl TextAssetCache {
    pub fn new() -> Self {
        Self {
            font_system: FontSystem::new(),
            swash_cache: SwashCache::new(),
            assets: HashMap::new(),
        }
    }

    pub fn clear(&mut self) {
        self.assets.clear();
    }

    pub fn get(&mut self, request: TextRequest<'_>) -> Arc<RgbaAsset> {
        let key = TextKey {
            text: Arc::from(request.text),
            locale: Arc::from(self.font_system.locale()),
            size: request.size.to_bits(),
            line_height: request.line_height.to_bits(),
            max_width: request.max_width,
            color: request.color,
            weight: request.weight,
        };
        if let Some(asset) = self.assets.get(&key) {
            return Arc::clone(asset);
        }
        let asset = Arc::new(self.rasterize(&key));
        self.assets.insert(key, Arc::clone(&asset));
        asset
    }

    fn rasterize(&mut self, key: &TextKey) -> RgbaAsset {
        let size = f32::from_bits(key.size).max(1.0);
        let line_height = f32::from_bits(key.line_height).max(size);
        let mut buffer = Buffer::new(&mut self.font_system, Metrics::new(size, line_height));
        buffer.set_wrap(if key.max_width.is_some() {
            Wrap::WordOrGlyph
        } else {
            Wrap::None
        });
        buffer.set_size(key.max_width.map(|width| width.max(1) as f32), None);
        buffer.set_text(
            &key.text,
            &Attrs::new()
                .family(Family::SansSerif)
                .weight(match key.weight {
                    TextWeight::Normal => Weight::NORMAL,
                    TextWeight::Bold => Weight::BOLD,
                }),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.font_system, false);
        let (layout_width, layout_height) =
            buffer
                .layout_runs()
                .fold((0.0_f32, line_height), |(width, height), run| {
                    (
                        width.max(run.line_w),
                        height.max(run.line_top + run.line_height),
                    )
                });
        let width = key
            .max_width
            .unwrap_or_else(|| layout_width.ceil().max(1.0) as u32)
            .max(1);
        let height = layout_height.ceil().max(1.0) as u32;
        let mut pixels = RgbaImage::new(width, height);
        let color = Color::rgba(key.color[0], key.color[1], key.color[2], key.color[3]);
        buffer.draw(
            &mut self.font_system,
            &mut self.swash_cache,
            color,
            |x, y, glyph_width, glyph_height, glyph_color| {
                blend_rect(
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
        RgbaAsset(pixels)
    }
}

fn blend_rect(target: &mut RgbaImage, x: i32, y: i32, width: u32, height: u32, source: [u8; 4]) {
    for row in 0..height {
        for column in 0..width {
            let px = x + column as i32;
            let py = y + row as i32;
            if px < 0 || py < 0 || px >= target.width() as i32 || py >= target.height() as i32 {
                continue;
            }
            let destination = target.get_pixel_mut(px as u32, py as u32);
            let source_alpha = u32::from(source[3]);
            let inverse = 255 - source_alpha;
            for channel in 0..3 {
                destination[channel] = ((u32::from(source[channel]) * source_alpha
                    + u32::from(destination[channel]) * inverse)
                    / 255) as u8;
            }
            destination[3] = (source_alpha + u32::from(destination[3]) * inverse / 255) as u8;
        }
    }
}
